//! Shared support for the visual harness: loads the curated manifest, traces
//! a manifest entry through the same per-layer pipeline `src/main.rs` uses,
//! and renders per-layer stage previews plus the final composite.
//!
//! An entry is either a `[[subset]]` (a named list of SFW layers from an
//! otherwise-NSFW PSD) or a `[[whole]]` (an SFW file, every layer). Tracing a
//! layer is independent of the others, so a subset's traced geometry is
//! byte-identical to that layer's geometry in the full document.

// Each test binary compiles this module and uses a different subset of it.
#![allow(dead_code)]

mod font;

use std::path::{Path, PathBuf};

use image::{GrayImage, Rgb, RgbImage, Rgba, RgbaImage};
use rayon::prelude::*;
use resvg::{tiny_skia, usvg};
use serde::Deserialize;

use pawtrace::config::Config;
use pawtrace::output::{self, Stroke, SvgLayer};
use pawtrace::profiles::ProfileStack;
use pawtrace::trace::TracedPath;
use pawtrace::{palette, pipeline, psd_import, raster, regions};

pub fn manifest_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

pub fn visual_golden_dir() -> PathBuf {
    manifest_dir().join("fixtures").join("visual").join("golden")
}

/// Filesystem-safe stem: ASCII alphanumerics kept, everything else `_`.
pub fn slug(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// A curated manifest entry: a subset carries its layer allow-list, a whole
/// file carries `None` (every layer traced).
pub struct Entry {
    pub name: String,
    pub source: PathBuf,
    pub layers: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct RawSubset {
    name: String,
    source: String,
    layers: Vec<String>,
}

#[derive(Deserialize)]
struct RawWhole {
    name: String,
    source: String,
}

#[derive(Deserialize)]
struct RawManifest {
    #[serde(default)]
    subset: Vec<RawSubset>,
    #[serde(default)]
    whole: Vec<RawWhole>,
}

/// Every entry in the manifest, subsets first, then whole files, in file
/// order.
pub fn load_manifest() -> Vec<Entry> {
    let path = manifest_dir().join("fixtures").join("visual").join("subsets.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let raw: RawManifest = toml::from_str(&text)
        .unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()));
    let root = manifest_dir();
    let mut entries = Vec::new();
    for s in raw.subset {
        entries.push(Entry {
            name: s.name,
            source: root.join(&s.source),
            layers: Some(s.layers),
        });
    }
    for w in raw.whole {
        entries.push(Entry { name: w.name, source: root.join(&w.source), layers: None });
    }
    entries
}

/// One entry layer selected for tracing: its name, its resolved config, and
/// its document-sized source raster.
pub struct KeptLayer {
    pub name: String,
    pub cfg: Config,
    pub img: RgbaImage,
}

/// The layers an entry selects, plus the shared document facts: size in
/// source px, the profile scale, and any manifest layers that named no
/// non-empty PSD layer.
pub struct Resolved {
    pub w: u32,
    pub h: u32,
    pub scale: u32,
    pub layers: Vec<KeptLayer>,
    pub missing: Vec<String>,
}

/// Reads the entry's PSD and keeps only its manifest layers (all of them for
/// a whole file), each paired with its resolved config, in the document's
/// bottom-first paint order.
pub fn resolve_entry(entry: &Entry) -> Resolved {
    let profiles = ProfileStack::load_near(&entry.source);
    let bytes = std::fs::read(&entry.source)
        .unwrap_or_else(|e| panic!("reading {}: {e}", entry.source.display()));
    let all = psd_import::layers(&bytes).unwrap();
    let (w, h) = (all[0].1.width(), all[0].1.height());
    let scale = profiles.resolve("").0.scale;

    let mut layers = Vec::new();
    for (name, img) in all {
        if let Some(want) = &entry.layers {
            if !want.contains(&name) {
                continue;
            }
        }
        let (cfg, _) = profiles.resolve(&name);
        layers.push(KeptLayer { name, cfg, img });
    }
    let missing = match &entry.layers {
        None => Vec::new(),
        Some(want) => want
            .iter()
            .filter(|n| !layers.iter().any(|l| &l.name == *n))
            .cloned()
            .collect(),
    };
    Resolved { w, h, scale, layers, missing }
}

/// One traced layer of an entry, in document scaled space.
pub struct Layer {
    pub name: String,
    pub stroke: Option<Stroke>,
    pub colors: Vec<(String, Vec<TracedPath>)>,
}

/// A traced entry: the document size in source px, the profile scale, and the
/// assembled layers bottom-first.
pub struct Document {
    pub w: u32,
    pub h: u32,
    pub scale: u32,
    pub layers: Vec<Layer>,
}

/// Traces each kept layer exactly as `src/main.rs`'s full-document path does:
/// `pipeline::run` on the document-sized layer, then per-layer paths rescaled
/// from the layer's own supersample space into the document's.
pub fn trace(r: &Resolved) -> Document {
    let doc_dim = r.w.max(r.h);
    let layers = r
        .layers
        .par_iter()
        .map(|l| {
            let mut colors = pipeline::run(&l.img, &l.cfg, doc_dim, (0, 0)).unwrap();
            let ratio = r.scale as f64 / l.cfg.scale as f64;
            if ratio != 1.0 {
                for (_, paths) in &mut colors {
                    for p in paths {
                        p.scale(ratio);
                    }
                }
            }
            Layer { name: l.name.clone(), stroke: output::stroke_of(&l.cfg), colors }
        })
        .collect();
    Document { w: r.w, h: r.h, scale: r.scale, layers }
}

/// Rasterizes the assembled document SVG at its native (source-pixel) size.
pub fn render(doc: &Document) -> RgbaImage {
    let svg_layers: Vec<SvgLayer> = doc
        .layers
        .iter()
        .map(|l| SvgLayer { name: &l.name, stroke: l.stroke.as_ref(), colors: &l.colors })
        .collect();
    rasterize(doc.w, doc.h, doc.scale, &svg_layers)
}

fn rasterize(w: u32, h: u32, scale: u32, layers: &[SvgLayer]) -> RgbaImage {
    let svg = output::svg(w, h, scale, 0.0, layers);
    let tree = usvg::Tree::from_data(svg.as_bytes(), &usvg::Options::default()).unwrap();
    let mut pix = tiny_skia::Pixmap::new(w, h).unwrap();
    resvg::render(&tree, tiny_skia::Transform::identity(), &mut pix.as_mut());
    RgbaImage::from_raw(w, h, pix.take()).unwrap()
}

/// Like [`rasterize`] but renders into a `scale`x larger pixmap, so vector
/// output lands at the same supersampled density as the flat/quant/region
/// rasters instead of at the 1x crop size. `w`/`h` are the unscaled crop size
/// and `scale` the supersample the paths were built in.
fn rasterize_scaled(w: u32, h: u32, scale: u32, layers: &[SvgLayer]) -> RgbaImage {
    let svg = output::svg(w, h, scale, 0.0, layers);
    let tree = usvg::Tree::from_data(svg.as_bytes(), &usvg::Options::default()).unwrap();
    let (sw, sh) = (w * scale, h * scale);
    let mut pix = tiny_skia::Pixmap::new(sw, sh).unwrap();
    let t = tiny_skia::Transform::from_scale(scale as f32, scale as f32);
    resvg::render(&tree, t, &mut pix.as_mut());
    RgbaImage::from_raw(sw, sh, pix.take()).unwrap()
}

/// Total traced path count and anchor count over a document. A closed cubic
/// path has one anchor per cubic segment, so anchors sum `cubics.len()`.
pub fn counts(doc: &Document) -> (usize, usize) {
    let mut paths = 0;
    let mut anchors = 0;
    for l in &doc.layers {
        for (_, ps) in &l.colors {
            paths += ps.len();
            anchors += ps.iter().map(|p| p.cubics.len()).sum::<usize>();
        }
    }
    (paths, anchors)
}

/// The five stage rasters for one layer, each at its own native size. `crop`
/// is 1x source; the rest are at the profile's supersample scale.
pub struct Stages {
    pub crop: RgbaImage,
    pub flat: RgbaImage,
    pub quant: RgbaImage,
    pub regions: RgbaImage,
    pub traced: RgbaImage,
}

/// Replays `pipeline::run`'s stages for one layer as images: the alpha crop,
/// the flattened supersample, the quantized labels, the segmented regions
/// (each painted its own region color), and the final single-layer trace.
/// `None` for a fully transparent layer. `doc_dim` is `max(doc W, H)`.
pub fn layer_stages(img: &RgbaImage, cfg: &Config, doc_dim: u32) -> Option<Stages> {
    let (src, ox, oy) = pipeline::crop_to_alpha(img, cfg)?;
    let pins = pipeline::scale_pins(&cfg.pins, (ox, oy), cfg.scale, (src.width(), src.height()));

    let (alpha, flat, quant_rgb, regs) =
        if let Some(color) = raster::uniform_color(&src, cfg.alpha_threshold) {
            let alpha = raster::scale_alpha(&src, cfg);
            let flat = RgbImage::from_pixel(alpha.width(), alpha.height(), Rgb(color));
            let regs = regions::from_mask(&alpha, color);
            (alpha, flat.clone(), flat, regs)
        } else {
            let prep = raster::prepare(&src, cfg);
            let pal = palette::extract_palette(&prep.flat, &prep.alpha, cfg, doc_dim);
            let mut quant = palette::remap(&prep.flat, &prep.alpha, &pal);
            if cfg.color_cleanup > 0 {
                quant = palette::label_smooth(&quant, &prep.alpha, cfg.color_cleanup);
            }
            let regs = regions::segment_absorbed(&quant, &prep.alpha, cfg);
            (prep.alpha, prep.flat, quant, regs)
        };

    let traced = {
        let colors = pipeline::simplify_paths(
            pipeline::trace_regions(&regs, &alpha, cfg, doc_dim, &pins),
            cfg,
        );
        let stroke = output::stroke_of(cfg);
        let layer = SvgLayer { name: "layer", stroke: stroke.as_ref(), colors: &colors };
        // trace_regions leaves paths in the crop's scaled space; render at that
        // supersampled size so the tile is as crisp as the region tile once the
        // sheet fits both to a common height.
        rasterize_scaled(src.width(), src.height(), cfg.scale, std::slice::from_ref(&layer))
    };

    Some(Stages {
        crop: src,
        flat: rgba_over_alpha(&flat, &alpha),
        quant: rgba_over_alpha(&quant_rgb, &alpha),
        regions: region_image(&regs, alpha.width(), alpha.height()),
        traced,
    })
}

/// An RGBA image showing `rgb` only where `alpha` is set, transparent
/// elsewhere. The two planes share dimensions.
fn rgba_over_alpha(rgb: &RgbImage, alpha: &GrayImage) -> RgbaImage {
    let (w, h) = alpha.dimensions();
    let mut out = RgbaImage::new(w, h);
    for (o, (p, a)) in out.pixels_mut().zip(rgb.pixels().zip(alpha.pixels())) {
        if a.0[0] != 0 {
            *o = Rgba([p.0[0], p.0[1], p.0[2], 255]);
        }
    }
    out
}

/// Paints each region its own post-segmentation color onto a transparent
/// canvas, so the segmentation (absorption and stroke merges included) is
/// visible independently of the quantized labels.
fn region_image(regs: &[regions::Region], w: u32, h: u32) -> RgbaImage {
    let mut out = RgbaImage::new(w, h);
    for r in regs {
        let c = Rgba([r.color[0], r.color[1], r.color[2], 255]);
        for &(px, py) in &r.pixels {
            out.put_pixel(r.x0 + px, r.y0 + py, c);
        }
    }
    out
}

const TILE_H: u32 = 256;
const PAD: u32 = 8;
const SEP_W: u32 = 2;
/// Divider rule color, chosen to read against the checkerboard.
const SEP: [u8; 3] = [96, 96, 104];
/// Label text color, chosen to read against the checkerboard.
const LABEL: [u8; 3] = [210, 210, 215];
/// Caption per tile, mirroring the GUI's stage names in
/// `src/gui/view/strip.rs` (crop is shown as the Source stage), uppercased.
const STAGE_LABELS: [&str; 5] = ["SOURCE", "FLATTEN", "PALETTE", "REGIONS", "TRACE"];

/// Alpha checkerboard, mirrored from the GUI preview's
/// `src/gui/view/checkerboard.rs` (CHECK_LIGHT / CHECK_DARK / TILE) as 8-bit.
const CHECK_LIGHT: [u8; 3] = [41, 41, 46];
const CHECK_DARK: [u8; 3] = [28, 28, 33];
const CHECK_TILE: u32 = 8;

/// An opaque `w`x`h` transparency-grid raster: the two-tone alpha checker the
/// GUI draws behind preview art, so composited alpha stays legible.
fn checkerboard(w: u32, h: u32) -> RgbaImage {
    let mut img = RgbaImage::new(w, h);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let c = if (x / CHECK_TILE + y / CHECK_TILE) % 2 == 1 { CHECK_DARK } else { CHECK_LIGHT };
        *p = Rgba([c[0], c[1], c[2], 255]);
    }
    img
}

/// Composites straight-alpha `top` over an opaque transparency grid of the
/// same size, so a golden with transparent areas shows its alpha as a checker.
pub fn composite_over_grid(top: &RgbaImage) -> RgbaImage {
    let mut bg = checkerboard(top.width(), top.height());
    blit_over(&mut bg, top, 0, 0);
    bg
}

/// Tiles the five stage rasters left-to-right over a transparency grid, each
/// captioned and separated by a vertical rule: crop (Source), flattened,
/// quantized (Palette), regions, final trace. A fixed label band sits above
/// the tiles, each caption left-aligned over its tile; tiles keep their aspect
/// ratio at a fixed height and read left-to-right in pipeline order.
pub fn contact_sheet(s: &Stages) -> RgbaImage {
    let tiles: [&RgbaImage; 5] = [&s.crop, &s.flat, &s.quant, &s.regions, &s.traced];
    let scaled: Vec<RgbaImage> = tiles.iter().map(|t| fit_height(t, TILE_H)).collect();

    let gap = 2 * PAD + SEP_W;
    let tiles_w: u32 = scaled.iter().map(|t| t.width()).sum();
    let width = 2 * PAD + tiles_w + (scaled.len() as u32 - 1) * gap;
    let tile_top = PAD + font::TEXT_H + PAD;
    let height = tile_top + TILE_H + PAD;

    let mut sheet = checkerboard(width, height);
    let mut x = PAD;
    for (i, tile) in scaled.iter().enumerate() {
        font::draw_text(&mut sheet, STAGE_LABELS[i], x, PAD, tile.width(), LABEL);
        blit_over(&mut sheet, tile, x, tile_top);
        x += tile.width();
        if i + 1 < scaled.len() {
            fill_rect(&mut sheet, x + PAD, tile_top, SEP_W, TILE_H, SEP);
            x += gap;
        }
    }
    sheet
}

/// Paints an opaque `color` rectangle, clipped to the image bounds.
fn fill_rect(dst: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, color: [u8; 3]) {
    let px = Rgba([color[0], color[1], color[2], 255]);
    for yy in y..(y + h).min(dst.height()) {
        for xx in x..(x + w).min(dst.width()) {
            dst.put_pixel(xx, yy, px);
        }
    }
}

/// Resizes to `h` px tall, width scaled to preserve aspect (at least 1 px).
fn fit_height(img: &RgbaImage, h: u32) -> RgbaImage {
    let (w0, h0) = img.dimensions();
    let w = ((w0 as f32 / h0.max(1) as f32) * h as f32).round().max(1.0) as u32;
    image::imageops::resize(img, w, h, image::imageops::FilterType::Triangle)
}

/// Source-over composite of a straight-alpha `tile` onto opaque `dst` at
/// `(dx, dy)`.
fn blit_over(dst: &mut RgbaImage, tile: &RgbaImage, dx: u32, dy: u32) {
    for (tx, ty, p) in tile.enumerate_pixels() {
        let a = p.0[3] as u32;
        if a == 0 {
            continue;
        }
        let d = dst.get_pixel_mut(dx + tx, dy + ty);
        for c in 0..3 {
            d.0[c] = ((p.0[c] as u32 * a + d.0[c] as u32 * (255 - a)) / 255) as u8;
        }
    }
}
