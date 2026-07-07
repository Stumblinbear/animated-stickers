//! Background compute: the per-layer stage strip (streamed card by card)
//! and the full-document preview, both cached so an edit recomputes only
//! the stages whose inputs actually changed.

use super::doc::{Doc, Layer};
use super::{App, Msg};
use crate::config::Config;
use crate::trace::TracedPath;
use crate::{output, palette, pipeline, profiles, raster, regions};
use iced::widget::image as iced_image;
use iced::Task;
use image::RgbaImage;
use std::sync::Arc;

pub(super) const STAGE_COUNT: usize = 7;

/// Stage outputs for the selected layer, as display handles.
#[derive(Debug, Clone, Default)]
pub(super) struct StageImages {
    pub(super) source: Option<iced_image::Handle>,
    pub(super) flat: Option<iced_image::Handle>,
    pub(super) quant: Option<iced_image::Handle>,
    /// Quantized pixels with the alpha mask applied, kept for the
    /// click-to-lock color picker.
    pub(super) quant_px: Option<RgbaImage>,
    pub(super) regions: Option<iced_image::Handle>,
    /// Per-region trace fates and floor for the stage-4 hover readout, aligned
    /// with the cached regions.
    pub(super) region_report: Option<regions::RegionReport>,
    /// Stage 5: smoothed boundary with corner markers, pre-fit.
    pub(super) smooth: Option<iced_image::Handle>,
    /// Stage 6 (fit), pre-simplification.
    pub(super) render: Option<iced_image::Handle>,
    /// Stage 7 (final), after the simplify pass.
    pub(super) simplified: Option<iced_image::Handle>,
    pub(super) palette: Vec<[u8; 3]>,
    pub(super) region_count: usize,
    pub(super) anchor_count: usize,
    pub(super) simplify_anchor_count: usize,
}

/// Whole-document totals, computed alongside the full preview.
#[derive(Debug, Clone, Copy)]
pub struct DocStats {
    pub layers: usize,
    pub shapes: usize,
    pub anchors: usize,
}

/// Intermediates from the last stage run, reused when a setting change
/// leaves earlier stages' inputs untouched (each stage reads a known subset
/// of Config; see the same_* predicates).
#[derive(Debug, Clone)]
pub struct StageCache {
    doc: usize,
    layer: usize,
    cfg: Config,
    prep: Arc<raster::Prepared>,
    quant: Arc<image::RgbImage>,
    palette: Vec<[u8; 3]>,
    regs: Arc<Vec<regions::Region>>,
    /// Fitted (pre-simplify) paths, so a simplify-slider drag re-runs only
    /// the cheap simplify pass, not the fit.
    fit: Arc<LayerTrace>,
    /// Rendered smooth-and-corners debug view, reused while the fit inputs
    /// hold (it does not depend on the stroke).
    smooth: Option<iced_image::Handle>,
}

fn same_prep(a: &Config, b: &Config) -> bool {
    a.scale == b.scale && a.alpha_threshold == b.alpha_threshold && a.mode_filter == b.mode_filter
}

fn same_quant(a: &Config, b: &Config) -> bool {
    same_prep(a, b)
        && a.detail == b.detail
        && a.max_colors == b.max_colors
        && a.merge_dist == b.merge_dist
        && a.gradient_dist == b.gradient_dist
        && a.hist_bits == b.hist_bits
        && a.locked == b.locked
        && a.color_cleanup == b.color_cleanup
}

fn same_regions(a: &Config, b: &Config) -> bool {
    same_quant(a, b)
        && a.absorb_dist == b.absorb_dist
        && a.absorb_aggr == b.absorb_aggr
        && a.stroke_merge_dist == b.stroke_merge_dist
        && a.stroke_merge_width == b.stroke_merge_width
}

/// Whether the fitted (pre-simplify) paths are unchanged. Pins gate which
/// small regions get traced, so they belong here even though they leave the
/// regions themselves untouched.
fn same_fit(a: &Config, b: &Config) -> bool {
    same_regions(a, b)
        && a.alphamax == b.alphamax
        && a.opttolerance == b.opttolerance
        && a.seam_slack == b.seam_slack
        && a.smoothing == b.smoothing
        && a.pins == b.pins
}

/// One layer's traced colors, positioned in document space.
pub(super) type LayerTrace = Vec<(String, Vec<TracedPath>)>;

/// Full-preview result plus the per-layer trace cache that produced it.
#[derive(Debug, Clone)]
pub struct FullResult {
    pub(super) handle: iced_image::Handle,
    pub(super) stats: DocStats,
    /// The document scale the cached traces are normalized to; they are
    /// only reusable while it holds.
    pub(super) doc_scale: u32,
    pub(super) cache: Vec<Option<(Config, Arc<LayerTrace>)>>,
}

/// One stage's output, streamed as soon as that stage finishes so the fast
/// early stages appear without waiting for the trace.
#[derive(Debug, Clone)]
pub enum StagePart {
    Source(iced_image::Handle),
    Flat(iced_image::Handle),
    Quant(iced_image::Handle, Vec<[u8; 3]>, RgbaImage),
    Regions(iced_image::Handle, usize, regions::RegionReport),
    /// Stage 5: smoothed boundary with corner markers.
    Smooth(Option<iced_image::Handle>),
    /// Stage 6: fitted render and anchor count, before simplification.
    Fit(Option<iced_image::Handle>, usize),
    /// Always the final part: completion is detected by it. Carries the
    /// simplified render, its anchor count, and the intermediates for reuse.
    Simplify(Option<iced_image::Handle>, usize, Box<StageCache>),
}

impl App {
    /// Recompute the stage strip off the UI thread, streaming each stage's
    /// card the moment it finishes. One stream in flight at a time: further
    /// edits set the dirty latch and re-spawn on completion, so a slider
    /// drag computes the latest state, not every intermediate.
    pub(super) fn spawn_stages(&mut self) -> Task<Msg> {
        let Some(doc) = self.docs.get(self.selected_doc) else {
            return Task::none();
        };
        if doc.layers.get(self.selected_layer).is_none() {
            return Task::none();
        }
        if self.stages_running {
            self.stages_dirty = true;
            return Task::none();
        }
        let cache = self
            .stage_cache
            .clone()
            .filter(|c| c.doc == self.selected_doc && c.layer == self.selected_layer);
        // Identical config: every displayed card, including the render, came
        // from exactly this state. Re-running would repeat the whole trace
        // for pixel-identical output.
        if cache.as_ref().is_some_and(|c| c.cfg == self.cfg) {
            self.stage_pending = [false; STAGE_COUNT];
            return Task::none();
        }
        self.stages_running = true;
        self.stages_gen += 1;
        let generation = self.stages_gen;
        let layers = doc.layers.clone();
        let doc_idx = self.selected_doc;
        let idx = self.selected_layer;
        let cfg = self.cfg.clone();
        let doc_dim = doc.size.0.max(doc.size.1);
        // Cache hits keep their card as-is: no pending flag, and the stream
        // skips their emit, since the displayed image came from this same
        // cache state. Trace always re-runs.
        let hit_prep = cache.as_ref().is_some_and(|c| same_prep(&c.cfg, &cfg));
        let hit_quant = cache.as_ref().is_some_and(|c| same_quant(&c.cfg, &cfg));
        let hit_regions = cache.as_ref().is_some_and(|c| same_regions(&c.cfg, &cfg));
        // Pins don't change the regions themselves, only their card (the
        // pin markers) and the trace filter, so a pin toggle reuses the
        // cached regions but still redraws the card.
        let hit_regions_view =
            hit_regions && cache.as_ref().is_some_and(|c| c.cfg.pins == cfg.pins);
        let hit_fit = cache.as_ref().is_some_and(|c| same_fit(&c.cfg, &cfg));
        // Fit and simplify always re-render (cheap): the stroke, which only
        // acts at render time, can change with everything upstream cached.
        // The smooth debug view ignores the stroke, so it tracks the fit.
        self.stage_pending = [
            cache.is_none(),
            !hit_prep,
            !hit_quant,
            !hit_regions_view,
            !hit_fit,
            true,
            true,
        ];
        // Rendezvous channel (capacity 0): each send suspends the compute
        // until the UI has taken the message. Any buffer lets the whole
        // pipeline run in one poll and every card appears at once.
        Task::stream(iced::stream::channel(
            0,
            move |mut tx: iced::futures::channel::mpsc::Sender<Msg>| async move {
            use iced::futures::SinkExt;
            // Send failures mean the app dropped the stream (stale gen or
            // shutdown); the remaining work would be wasted either way.
            let img = &layers[idx].img;
            macro_rules! emit {
                ($part:expr) => {
                    if tx.send(Msg::StagePart(generation, $part)).await.is_err() {
                        return;
                    }
                };
            }

            if cache.is_none() {
                emit!(StagePart::Source(rgba_handle(img)));
            }

            let prep = if hit_prep {
                cache.as_ref().unwrap().prep.clone()
            } else {
                let prep = Arc::new(raster::prepare(img, &cfg));
                emit!(StagePart::Flat(rgba_handle(&masked(
                    &prep.flat,
                    &prep.alpha
                ))));
                prep
            };

            let (quant, pal) = if hit_quant {
                let c = cache.as_ref().unwrap();
                (c.quant.clone(), c.palette.clone())
            } else {
                let pal = palette::extract_palette(&prep.flat, &prep.alpha, &cfg, doc_dim);
                let quant = palette::remap(&prep.flat, &prep.alpha, &pal);
                let quant = if cfg.color_cleanup > 0 {
                    palette::label_smooth(&quant, &prep.alpha, cfg.color_cleanup)
                } else {
                    quant
                };
                let quant = Arc::new(quant);
                let px = masked(&quant, &prep.alpha);
                emit!(StagePart::Quant(rgba_handle(&px), pal.clone(), px));
                (quant, pal)
            };

            let pins =
                pipeline::scale_pins(&cfg.pins, layers[idx].offset, cfg.scale, img.dimensions());
            let regs = if hit_regions {
                cache.as_ref().unwrap().regs.clone()
            } else {
                Arc::new(regions::segment_absorbed(&quant, &prep.alpha, &cfg))
            };
            if !hit_regions_view {
                let report = regions::region_report(&regs, &prep.alpha, &cfg, doc_dim, &pins);
                emit!(StagePart::Regions(
                    region_fates_handle(&regs, quant.dimensions(), &report.fates, &pins),
                    regs.len(),
                    report
                ));
            }

            let (w, h) = img.dimensions();
            let stroke = output::stroke_of(&cfg);
            let pad = cfg.stroke_width * cfg.scale as f32 / 2.0;
            // Renders `colors` to a stage card image at 2x source (crisp at
            // card width) and returns its anchor total.
            let render_paths = |colors: &LayerTrace| -> (Option<iced_image::Handle>, usize) {
                let anchors = colors
                    .iter()
                    .flat_map(|(_, ps)| ps.iter())
                    .map(|p| p.cubics.len())
                    .sum();
                let svg = output::svg(
                    w,
                    h,
                    cfg.scale,
                    pad,
                    &[output::SvgLayer { name: "layer", stroke: stroke.as_ref(), colors }],
                );
                (render_svg(&svg, w * 2, h * 2), anchors)
            };

            // Stage 5: smoothed boundary with corner markers, the geometry
            // the fit runs on. Independent of the stroke, so it rides the
            // fit cache.
            let smooth = if hit_fit {
                cache.as_ref().unwrap().smooth.clone()
            } else {
                let contours = pipeline::debug_contours(&regs, &prep.alpha, &cfg, doc_dim, &pins);
                let handle = render_debug(&contours, w, h, cfg.scale);
                emit!(StagePart::Smooth(handle.clone()));
                handle
            };

            // Stage 6: the fitted paths. Reuse the cache when only the
            // simplify pass (or the stroke) changed.
            let fit = if hit_fit {
                cache.as_ref().unwrap().fit.clone()
            } else {
                Arc::new(pipeline::trace_regions(&regs, &prep.alpha, &cfg, doc_dim, &pins))
            };
            let (fit_render, fit_anchors) = render_paths(&fit);
            emit!(StagePart::Fit(fit_render, fit_anchors));

            // Stage 7: the final simplified paths.
            let simplified = pipeline::simplify_paths((*fit).clone(), &cfg);
            let (simplify_render, simplify_anchors) = render_paths(&simplified);
            emit!(StagePart::Simplify(
                simplify_render,
                simplify_anchors,
                Box::new(StageCache {
                    doc: doc_idx,
                    layer: idx,
                    cfg,
                    prep,
                    quant,
                    palette: pal,
                    regs,
                    fit,
                    smooth,
                })
            ));
        }))
    }

    /// Recompute the full-document preview off the UI thread; same
    /// one-in-flight + dirty-latch scheme as the stage strip.
    pub(super) fn spawn_full(&mut self) -> Task<Msg> {
        let Some(doc) = self.docs.get(self.selected_doc) else {
            return Task::none();
        };
        if self.full_busy {
            self.full_dirty = true;
            return Task::none();
        }
        self.full_busy = true;
        self.full_gen += 1;
        let generation = self.full_gen;
        let layers = doc.layers.clone();
        let size = doc.size;
        let profiles = self.profiles.clone();
        // Cached traces are normalized to the document scale; a default-
        // profile scale change moves that space out from under them.
        let doc_scale = profiles.resolve("").0.scale;
        let mut cache = if self.full_cache_scale == doc_scale {
            self.full_cache.clone()
        } else {
            Vec::new()
        };
        cache.resize(layers.len(), None);
        Task::perform(
            async move {
                let result = render_full(&layers, size, &profiles, doc_scale, cache)
                    .map_err(|e| e.to_string());
                (generation, result)
            },
            |(generation, result)| Msg::FullReady(generation, result),
        )
    }
}

impl App {
    /// Pin/unpin the region under a click on the regions card: an unpinned
    /// region gains a pin at the clicked point; a region already holding a
    /// pin loses it. Pins are stored in document source px so they survive
    /// re-segmentation and follow the layer through exports.
    pub(super) fn toggle_pin(&mut self, p: iced::Point) -> Task<Msg> {
        let (regs, (qw, _)) = match &self.stage_cache {
            Some(c) => (c.regs.clone(), c.quant.dimensions()),
            None => return Task::none(),
        };
        let Some(offset) = self
            .doc()
            .and_then(|d| d.layers.get(self.selected_layer))
            .map(|l| l.offset)
        else {
            return Task::none();
        };
        let display_scale = qw as f32 / super::view::CARD_IMG_WIDTH;
        let (sx, sy) = ((p.x * display_scale) as u32, (p.y * display_scale) as u32);
        let Some(region) = regs.iter().find(|r| r.contains(sx, sy)) else {
            return Task::none();
        };
        let s = self.cfg.scale;
        let existing = self.cfg.pins.iter().position(|pin| {
            let Some(x) = pin[0].checked_sub(offset.0) else { return false };
            let Some(y) = pin[1].checked_sub(offset.1) else { return false };
            region.contains(x * s + s / 2, y * s + s / 2)
        });
        match existing {
            Some(i) => {
                self.cfg.pins.remove(i);
            }
            None => self.cfg.pins.push([sx / s + offset.0, sy / s + offset.1]),
        }
        self.write_pins();
        self.preview_tasks()
    }

    /// The stage-4 readout for the region under the last hover: its floor-
    /// tested area, the floor, and its fate. `None` when nothing is hovered or
    /// no region sits under the cursor.
    pub(super) fn region_hover_info(&self) -> Option<String> {
        let p = self.region_hover?;
        let cache = self.stage_cache.as_ref()?;
        let report = self.stages.region_report.as_ref()?;
        let (qw, _) = cache.quant.dimensions();
        let display_scale = qw as f32 / super::view::CARD_IMG_WIDTH;
        let (sx, sy) = ((p.x * display_scale) as u32, (p.y * display_scale) as u32);
        let i = cache.regs.iter().position(|r| r.contains(sx, sy))?;
        let fate = match report.fates.get(i)? {
            regions::Fate::Traced => "traces".to_string(),
            regions::Fate::MergedInto(t) => {
                let c = cache.regs[*t].color;
                format!("merges into #{:02x}{:02x}{:02x}", c[0], c[1], c[2])
            }
            regions::Fate::Culled => "culled".to_string(),
        };
        Some(format!(
            "area {} px\u{b2} / floor {} px\u{b2} - {fate}",
            report.areas.get(i)?,
            report.floor
        ))
    }
}

fn rgba_handle(img: &RgbaImage) -> iced_image::Handle {
    iced_image::Handle::from_rgba(img.width(), img.height(), img.as_raw().clone())
}

/// RGB plus its alpha mask as displayable RGBA: pixels outside the mask
/// become fully transparent instead of exposing the meaningless zero fill.
fn masked(img: &image::RgbImage, alpha: &image::GrayImage) -> RgbaImage {
    let mut out = RgbaImage::new(img.width(), img.height());
    for (o, (p, a)) in out.pixels_mut().zip(img.pixels().zip(alpha.pixels())) {
        o.0 = [p.0[0], p.0[1], p.0[2], a.0[0]];
    }
    out
}

/// Fates view: each region in its own quantized color, so the card reads as
/// the art, with a translucent tint over regions the trace will not keep as
/// their own shape. Red marks a culled region (below the speckle floor, no
/// neighbor to merge into, unpinned; it vanishes silently). Orange marks one
/// the speckle merge folds into a neighbor (it survives as pixels, losing its
/// color and path). Untinted regions trace. Pins draw as white dots with a
/// dark ring.
fn region_fates_handle(
    regs: &[regions::Region],
    (w, h): (u32, u32),
    fates: &[regions::Fate],
    pins: &[(u32, u32)],
) -> iced_image::Handle {
    // Tint weight for a non-surviving region: enough to read the fate, light
    // enough to keep the underlying color legible.
    const TINT: f32 = 0.5;
    let blend = |base: [u8; 3], tint: [u8; 3]| -> [u8; 3] {
        std::array::from_fn(|k| {
            (base[k] as f32 * (1.0 - TINT) + tint[k] as f32 * TINT) as u8
        })
    };
    let mut bytes = vec![0u8; (w * h * 4) as usize];
    for (i, r) in regs.iter().enumerate() {
        let c = match fates.get(i) {
            Some(regions::Fate::Culled) => blend(r.color, [230, 55, 45]),
            Some(regions::Fate::MergedInto(_)) => blend(r.color, [240, 150, 40]),
            _ => r.color,
        };
        for &(px, py) in &r.pixels {
            let idx = (((r.y0 + py) * w + (r.x0 + px)) * 4) as usize;
            bytes[idx..idx + 3].copy_from_slice(&c);
            bytes[idx + 3] = 255;
        }
    }
    // Marker size follows the image so it stays visible at card width.
    let radius = (w.max(h) as i64 / 64).clamp(3, 12);
    for &(px, py) in pins {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let (x, y) = (px as i64 + dx, py as i64 + dy);
                if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                    continue;
                }
                let d2 = dx * dx + dy * dy;
                if d2 > radius * radius {
                    continue;
                }
                let inner = radius * 2 / 3;
                let c = if d2 <= inner * inner { 255u8 } else { 20 };
                let idx = ((y as u32 * w + x as u32) * 4) as usize;
                bytes[idx..idx + 4].copy_from_slice(&[c, c, c, 255]);
            }
        }
    }
    iced_image::Handle::from_rgba(w, h, bytes)
}

/// Renders the smooth-and-corners debug view: each contour as a thin
/// polyline (blue, or green over stretches fit at the slackened seam
/// tolerance) with an orange dot on every corner vertex, on transparent
/// backing. Coordinates are in scaled space, so the viewBox matches.
fn render_debug(
    contours: &[pipeline::DebugContour],
    w: u32,
    h: u32,
    scale: u32,
) -> Option<iced_image::Handle> {
    let (vw, vh) = (w * scale, h * scale);
    let mut s = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {vw} {vh}">"#
    );
    let line = scale as f32 * 0.5;
    for c in contours {
        if c.points.len() < 2 {
            continue;
        }
        let mut d = String::new();
        for (x, y) in &c.points {
            d += &format!("{x:.1},{y:.1} ");
        }
        // Close the ring back to its first vertex.
        let (fx, fy) = c.points[0];
        d += &format!("{fx:.1},{fy:.1}");
        s += &format!(
            r##"<polyline points="{d}" fill="none" stroke="#6ea8ff" stroke-width="{line}"/>"##
        );
        // Overlay each edge touching a slack vertex, so slackened seams read
        // apart from the base outline. Drawn on top at the same width.
        let n = c.points.len();
        for i in 0..n {
            let j = (i + 1) % n;
            if !c.slack.get(i).copied().unwrap_or(false)
                && !c.slack.get(j).copied().unwrap_or(false)
            {
                continue;
            }
            let (x1, y1) = c.points[i];
            let (x2, y2) = c.points[j];
            s += &format!(
                r##"<line x1="{x1:.1}" y1="{y1:.1}" x2="{x2:.1}" y2="{y2:.1}" stroke="#48d597" stroke-width="{line}"/>"##
            );
        }
    }
    let dot = scale as f32 * 1.2;
    for c in contours {
        for (x, y) in &c.corners {
            s += &format!(r##"<circle cx="{x:.1}" cy="{y:.1}" r="{dot}" fill="#ff9d3c"/>"##);
        }
    }
    s += "</svg>";
    render_svg(&s, w * 2, h * 2)
}

fn render_svg(svg: &str, w: u32, h: u32) -> Option<iced_image::Handle> {
    let tree = resvg::usvg::Tree::from_data(svg.as_bytes(), &Default::default()).ok()?;
    let sz = tree.size();
    let scale = (w as f32 / sz.width()).min(h as f32 / sz.height());
    let (pw, ph) = ((sz.width() * scale) as u32, (sz.height() * scale) as u32);
    let mut pix = resvg::tiny_skia::Pixmap::new(pw.max(1), ph.max(1))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pix.as_mut(),
    );
    Some(iced_image::Handle::from_rgba(
        pix.width(),
        pix.height(),
        pix.take(),
    ))
}

/// One layer through the pipeline, paths converted to the document's scale
/// space and translated to document position.
fn trace_layer(l: &Layer, cfg: &Config, doc_scale: u32, doc_dim: u32) -> anyhow::Result<LayerTrace> {
    let mut colors = pipeline::run(&l.img, cfg, doc_dim, l.offset)?;
    // The pipeline traces in the layer's own scale space; a per-layer scale
    // override would land displaced and mis-sized against the shared
    // document viewBox without this conversion.
    let ratio = doc_scale as f64 / cfg.scale as f64;
    let (dx, dy) = (
        (l.offset.0 * doc_scale) as f64,
        (l.offset.1 * doc_scale) as f64,
    );
    for (_, paths) in &mut colors {
        for p in paths {
            if ratio != 1.0 {
                p.scale(ratio);
            }
            p.translate(dx, dy);
        }
    }
    Ok(colors)
}

/// Full document render: every layer traced under its matched profile,
/// composited at document position, with whole-document totals. Layers
/// whose resolved config matches their cache entry reuse it, so a profile
/// edit re-traces only that profile's layers.
fn render_full(
    layers: &[Layer],
    size: (u32, u32),
    profiles: &profiles::ProfileStack,
    doc_scale: u32,
    cache: Vec<Option<(Config, Arc<LayerTrace>)>>,
) -> anyhow::Result<Box<FullResult>> {
    let doc_dim = size.0.max(size.1);
    use rayon::prelude::*;
    let entries: Vec<(Config, Arc<LayerTrace>)> = layers
        .par_iter()
        .enumerate()
        .map(|(i, l)| {
            let (cfg, _) = profiles.resolve(&l.name);
            if let Some((cached_cfg, t)) = &cache[i] {
                if *cached_cfg == cfg {
                    return Ok((cfg, t.clone()));
                }
            }
            let colors = trace_layer(l, &cfg, doc_scale, doc_dim)?;
            Ok((cfg, Arc::new(colors)))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let stats = DocStats {
        layers: entries.len(),
        shapes: entries
            .iter()
            .flat_map(|(_, t)| t.iter())
            .map(|(_, paths)| paths.len())
            .sum(),
        anchors: entries
            .iter()
            .flat_map(|(_, t)| t.iter())
            .flat_map(|(_, paths)| paths.iter())
            .map(|p| p.cubics.len())
            .sum(),
    };
    let strokes: Vec<Option<output::Stroke>> =
        entries.iter().map(|(cfg, _)| output::stroke_of(cfg)).collect();
    let svg_layers: Vec<output::SvgLayer> = layers
        .iter()
        .zip(&entries)
        .zip(&strokes)
        .map(|((l, (_, t)), stroke)| output::SvgLayer {
            name: &l.name,
            stroke: stroke.as_ref(),
            colors: t,
        })
        .collect();
    let svg = output::svg(size.0, size.1, doc_scale, 0.0, &svg_layers);
    let handle = render_svg(&svg, size.0, size.1)
        .ok_or_else(|| anyhow::anyhow!("full preview render failed"))?;
    Ok(Box::new(FullResult {
        handle,
        stats,
        doc_scale,
        cache: entries.into_iter().map(Some).collect(),
    }))
}

/// Batch export: Tailmovin JSON next to each document.
pub(super) fn export_doc(
    doc: &Doc,
    profiles: &profiles::ProfileStack,
) -> anyhow::Result<std::path::PathBuf> {
    let doc_dim = doc.size.0.max(doc.size.1);
    let doc_scale = profiles.resolve("").0.scale;
    use rayon::prelude::*;
    let traced: Vec<(String, Option<output::Stroke>, LayerTrace)> = doc
        .layers
        .par_iter()
        .map(|l| {
            let (cfg, _) = profiles.resolve(&l.name);
            Ok((
                l.name.clone(),
                output::stroke_of(&cfg),
                trace_layer(l, &cfg, doc_scale, doc_dim)?,
            ))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let scale = doc_scale as f64;
    let out = output::Doc {
        width: doc.size.0,
        height: doc.size.1,
        layers: traced
            .into_iter()
            .map(|(name, stroke, colors)| output::Layer {
                name,
                stroke,
                colors: colors
                    .into_iter()
                    .map(|(hex, paths)| output::ColorGroup {
                        hex,
                        paths: paths.iter().map(|p| output::to_json_path(p, scale)).collect(),
                    })
                    .collect(),
            })
            .collect(),
    };
    let path = doc.path.with_extension("json");
    std::fs::write(&path, serde_json::to_vec_pretty(&out)?)?;
    Ok(path)
}
