//! Visual golden tests for the full tracing pipeline. For each fixture PSD,
//! traces every layer exactly as `src/main.rs` does, rasterizes the assembled
//! document SVG at native size, and compares against a blessed baseline PNG
//! with a perceptual (OKLab ΔE) metric. A separate stats guard catches anchor
//! explosions that a pixel diff cannot see.
//!
//! Run: `cargo test --features preview` (heavy in debug; see Cargo.toml's
//! `[profile.test]` opt bumps).
//! Re-bless: `UPDATE_GOLDENS=1 cargo test --features preview` rewrites the
//! baseline PNGs and `fixtures/golden/stats.toml`, then passes.

#![cfg(feature = "preview")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use image::{Rgba, RgbaImage};
use rayon::prelude::*;
use resvg::{tiny_skia, usvg};
use serde::{Deserialize, Serialize};

use pawtrace::config::srgb_to_oklab;
use pawtrace::output::{self, Stroke, SvgLayer};
use pawtrace::profiles::ProfileStack;
use pawtrace::trace::TracedPath;
use pawtrace::{pipeline, psd_import};

const FIXTURES: &[&str] = &["a good throat swabbing.psd", "between the buck's legs.psd"];

/// Per-pixel OKLab ΔE at or above which a pixel counts as visibly changed.
/// Below this is anti-alias jitter along edges; a dropped outline or a
/// re-colored region lands far above it (ΔE > 0.3).
const VISIBLE_DELTA_E: f32 = 0.06;
/// Mean ΔE budget over the compared pixel set. The pipeline is deterministic,
/// so an unregressed render matches its baseline exactly at mean 0. The budget
/// is headroom for a future resvg or AA shift, not expected drift.
const MEAN_DELTA_E_BUDGET: f32 = 0.01;
/// Budget on the fraction of compared pixels that are visibly changed. Tight
/// enough that a missing thin feature (which paints a contiguous band of
/// high-ΔE pixels) trips it, loose enough for edge AA reflow.
const VISIBLE_FRACTION_BUDGET: f32 = 0.02;
/// Stats may grow this much over the blessed totals before failing. Anchor
/// explosions multiply the count several-fold, well past this.
const STATS_GROWTH: f64 = 1.25;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn golden_dir() -> PathBuf {
    fixtures_dir().join("golden")
}

/// Filesystem-safe stem for a fixture's baseline artifacts.
fn slug(psd: &str) -> String {
    Path::new(psd)
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Path count and cubic-segment count summed across every layer of a document.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct DocStats {
    paths: usize,
    cubics: usize,
}

struct Traced {
    name: String,
    stroke: Option<Stroke>,
    colors: Vec<(String, Vec<TracedPath>)>,
}

/// Traces every layer of `psd_path` the way `src/main.rs` does: profiles
/// resolved per layer, `pipeline::run` on the full document-sized layer, and
/// per-layer paths rescaled from the layer's own supersample space into the
/// document's. Returns the document size and the assembled layers.
fn trace_document(psd_path: &Path) -> (u32, u32, u32, Vec<Traced>) {
    let profiles = ProfileStack::load_near(psd_path);
    let bytes = std::fs::read(psd_path).unwrap();
    let layers = psd_import::layers(&bytes).unwrap();
    let (w, h) = (layers[0].1.width(), layers[0].1.height());
    let doc_scale = profiles.resolve("").0.scale;

    let traced = layers
        .par_iter()
        .map(|(name, img)| {
            let (layer_cfg, _) = profiles.resolve(name);
            let mut colors = pipeline::run(img, &layer_cfg, w.max(h), (0, 0)).unwrap();
            let ratio = doc_scale as f64 / layer_cfg.scale as f64;
            if ratio != 1.0 {
                for (_, paths) in &mut colors {
                    for p in paths {
                        p.scale(ratio);
                    }
                }
            }
            Traced {
                name: name.clone(),
                stroke: output::stroke_of(&layer_cfg),
                colors,
            }
        })
        .collect();

    (w, h, doc_scale, traced)
}

/// Rasterizes the assembled document SVG at its native (source-pixel) size.
fn render(w: u32, h: u32, scale: u32, layers: &[Traced]) -> RgbaImage {
    let svg_layers: Vec<SvgLayer> = layers
        .iter()
        .map(|l| SvgLayer {
            name: &l.name,
            stroke: l.stroke.as_ref(),
            colors: &l.colors,
        })
        .collect();
    let svg = output::svg(w, h, scale, 0.0, &svg_layers);
    let tree = usvg::Tree::from_data(svg.as_bytes(), &usvg::Options::default()).unwrap();
    let mut pix = tiny_skia::Pixmap::new(w, h).unwrap();
    resvg::render(&tree, tiny_skia::Transform::identity(), &mut pix.as_mut());
    RgbaImage::from_raw(w, h, pix.take()).unwrap()
}

fn doc_stats(layers: &[Traced]) -> DocStats {
    let mut paths = 0;
    let mut cubics = 0;
    for l in layers {
        for (_, ps) in &l.colors {
            paths += ps.len();
            cubics += ps.iter().map(|p| p.cubics.len()).sum::<usize>();
        }
    }
    DocStats { paths, cubics }
}

/// Straight sRGB of a tiny_skia premultiplied-RGBA pixel composited over
/// white, so an alpha drop (a hole where the baseline was opaque) reads as a
/// large color change instead of vanishing.
fn over_white(p: &Rgba<u8>) -> [u8; 3] {
    let bg = 255 - p.0[3];
    [
        p.0[0].saturating_add(bg),
        p.0[1].saturating_add(bg),
        p.0[2].saturating_add(bg),
    ]
}

fn delta_e(a: &Rgba<u8>, b: &Rgba<u8>) -> f32 {
    let la = srgb_to_oklab(over_white(a));
    let lb = srgb_to_oklab(over_white(b));
    let d0 = la[0] - lb[0];
    let d1 = la[1] - lb[1];
    let d2 = la[2] - lb[2];
    (d0 * d0 + d1 * d1 + d2 * d2).sqrt()
}

/// Mean ΔE and visibly-changed fraction over pixels painted in either image.
fn compare(current: &RgbaImage, baseline: &RgbaImage) -> (f32, f32) {
    let mut sum = 0.0f64;
    let mut compared = 0u64;
    let mut visible = 0u64;
    for (c, b) in current.pixels().zip(baseline.pixels()) {
        if c.0[3] == 0 && b.0[3] == 0 {
            continue;
        }
        let d = delta_e(c, b);
        sum += d as f64;
        compared += 1;
        if d >= VISIBLE_DELTA_E {
            visible += 1;
        }
    }
    if compared == 0 {
        return (0.0, 0.0);
    }
    ((sum / compared as f64) as f32, visible as f32 / compared as f32)
}

#[test]
fn golden_documents_match_baselines() {
    let bless = std::env::var_os("UPDATE_GOLDENS").is_some();
    let dir = golden_dir();
    std::fs::create_dir_all(&dir).unwrap();

    let stats_path = dir.join("stats.toml");
    let mut blessed_stats: BTreeMap<String, DocStats> = std::fs::read_to_string(&stats_path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default();

    let mut failures = Vec::new();
    for &fixture in FIXTURES {
        let psd = fixtures_dir().join(fixture);
        assert!(psd.exists(), "missing fixture: {}", psd.display());
        let slug = slug(fixture);
        let png = dir.join(format!("{slug}.png"));

        let (w, h, scale, layers) = trace_document(&psd);
        let current = render(w, h, scale, &layers);
        let stats = doc_stats(&layers);

        if bless {
            current.save(&png).unwrap();
            blessed_stats.insert(slug, stats);
            continue;
        }

        let baseline = image::open(&png)
            .unwrap_or_else(|e| panic!("no baseline {} ({e}); bless with UPDATE_GOLDENS=1", png.display()))
            .to_rgba8();
        if baseline.dimensions() != current.dimensions() {
            failures.push(format!(
                "{fixture}: size {:?} != baseline {:?}",
                current.dimensions(),
                baseline.dimensions()
            ));
            continue;
        }

        let (mean, visible) = compare(&current, &baseline);
        if mean > MEAN_DELTA_E_BUDGET || visible > VISIBLE_FRACTION_BUDGET {
            failures.push(format!(
                "{fixture}: mean ΔE {mean:.4} (budget {MEAN_DELTA_E_BUDGET}), \
                 visible {:.3}% (budget {:.1}%)",
                visible * 100.0,
                VISIBLE_FRACTION_BUDGET * 100.0
            ));
        }

        match blessed_stats.get(&slug) {
            None => failures.push(format!("{fixture}: no blessed stats; bless with UPDATE_GOLDENS=1")),
            Some(blessed) => {
                let grew = |cur: usize, base: usize| cur as f64 > base as f64 * STATS_GROWTH;
                if grew(stats.paths, blessed.paths) || grew(stats.cubics, blessed.cubics) {
                    failures.push(format!(
                        "{fixture}: stats grew past {STATS_GROWTH}x — paths {} vs {}, cubics {} vs {}",
                        stats.paths, blessed.paths, stats.cubics, blessed.cubics
                    ));
                }
            }
        }
    }

    if bless {
        let toml = toml::to_string_pretty(&blessed_stats).unwrap();
        std::fs::write(&stats_path, toml).unwrap();
        return;
    }
    assert!(failures.is_empty(), "golden mismatches:\n{}", failures.join("\n"));
}
