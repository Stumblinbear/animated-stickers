//! Background compute: the per-layer stage strip (streamed part by part) and
//! the full-document preview. Both read and fill a per-document content-keyed
//! [`memo::Memo`], so an edit recomputes only the stages whose inputs changed
//! and a layer the full render already traced needs no re-trace.

mod full;
mod memo;
mod render;
mod shape_memo;
mod stages;

use super::app::App;
use crate::regions;
use iced::widget::image as iced_image;
use iced::Task;
use image::RgbaImage;
use std::sync::Arc;

pub(super) use full::export_doc;
pub(in crate::gui) use memo::{Memo, StageKeys};

use super::msg::Msg;

pub(super) const STAGE_COUNT: usize = 7;

/// A display handle with the pixel dimensions it was built from.
#[derive(Debug, Clone)]
pub struct Img {
    pub handle: iced_image::Handle,
    pub size: (u32, u32),
}

/// One layer's traced colors: color hex -> paths, in bottom-first paint order.
pub(super) type LayerTrace = Vec<(String, Vec<crate::trace::TracedPath>)>;

/// The layer and content keys the current stage images reflect, so the next
/// spawn can tell which images are stale without re-deriving them.
#[derive(Debug, Clone)]
pub(super) struct Shown {
    pub(super) layer: super::ids::LayerId,
    pub(super) keys: StageKeys,
    pub(super) stroke_bits: u32,
    pub(super) stroke_color: [u8; 3],
}

/// Stage outputs for the selected layer, as display images.
#[derive(Debug, Clone, Default)]
pub(super) struct StageImages {
    pub(super) source: Option<Img>,
    pub(super) flat: Option<Img>,
    pub(super) quant: Option<Img>,
    /// Quantized pixels with the alpha mask applied, kept for the
    /// click-to-lock color picker.
    pub(super) quant_px: Option<RgbaImage>,
    pub(super) regions: Option<Img>,
    /// Per-region trace fates and floor for the regions hover readout,
    /// aligned with the cached regions.
    pub(super) region_report: Option<regions::RegionReport>,
    /// Smoothed boundary with corner markers, pre-fit.
    pub(super) smooth: Option<Img>,
    /// Fitted render, pre-simplification.
    pub(super) render: Option<Img>,
    /// Final render, after the simplify pass.
    pub(super) simplified: Option<Img>,
    pub(super) palette: Vec<[u8; 3]>,
    pub(super) region_count: usize,
    pub(super) anchor_count: usize,
    pub(super) simplify_anchor_count: usize,
    pub(super) shown: Option<Shown>,
}

/// Whole-document totals, computed alongside the full preview.
#[derive(Debug, Clone, Copy)]
pub struct DocStats {
    pub shapes: usize,
    pub anchors: usize,
}

/// Full-preview result: the composite image, totals, per-layer anchor counts,
/// and the pre-transform traces newly computed this run, for the memo.
#[derive(Debug, Clone)]
pub struct FullResult {
    pub(super) img: Img,
    pub(super) stats: DocStats,
    pub(super) anchors: Vec<usize>,
    pub(super) merges: Vec<full::FullMerge>,
}

/// One stage's output, streamed the moment it finishes so the fast early
/// stages appear without waiting for the trace. Each part carries the value to
/// merge into the memo alongside the image to display.
#[derive(Debug, Clone)]
pub enum StagePart {
    Source(Img),
    Flat(Img, Arc<crate::raster::Prepared>),
    /// The merge plan computed this run, memoized under the `regions_view`
    /// key for the report, contours, and trace of later runs.
    Plan(Arc<regions::MergePlan>),
    Quant(Img, RgbaImage, Arc<image::RgbImage>, Arc<Vec<[u8; 3]>>),
    Regions(Img, usize, regions::RegionReport, Arc<Vec<regions::Region>>),
    /// Smoothed boundary, stored under the fit key.
    Smooth(Option<Img>),
    /// Fitted render, anchor count, and the fitted paths.
    Fit(Option<Img>, usize, Arc<LayerTrace>),
    /// Always the final part: completion is detected by it. Carries the
    /// simplified render, its anchor count, the simplified paths, and the
    /// keys the images now reflect.
    Simplify(Option<Img>, usize, Arc<LayerTrace>, Shown),
}

impl App {
    /// Recompute the stage strip off the UI thread, streaming each stage as it
    /// finishes. One stream in flight at a time: further edits set the dirty
    /// latch and re-spawn on completion.
    pub(super) fn spawn_stages(&mut self) -> Task<Msg> {
        let doc_idx = self.selected_doc;
        let Some(doc) = self.docs.get(doc_idx) else {
            return Task::none();
        };
        let layer = doc.session.selected_layer;
        let idx = layer.index();
        let Some(src) = doc.layers.get(idx) else {
            return self.drain_full_queued();
        };
        let img = src.img.clone();
        let offset = src.offset;
        if doc.session.stages_running {
            self.docs[doc_idx].session.stages_dirty = true;
            return Task::none();
        }
        let cfg = doc.session.cfg.clone();
        let keys = StageKeys::of(&cfg);
        let stroke_bits = cfg.stroke_width.to_bits();
        let stroke_color = cfg.stroke_color;
        let doc_dim = doc.size.0.max(doc.size.1);

        let shown = doc.session.stages.shown.clone();
        let pending = stages::pending(shown.as_ref(), layer, &keys, stroke_bits, stroke_color);
        // Nothing changed against the shown images: leave them and clear any
        // stale pending flags without spawning. A full render queued behind
        // this run must still happen (a flag flip re-composites without
        // changing the strip's config), so the latch drains here.
        if pending.iter().all(|&p| !p) {
            self.docs[doc_idx].session.stage_pending = [false; STAGE_COUNT];
            return self.drain_full_queued();
        }

        let (snap, shape_cache) = {
            let m = &mut self.docs[doc_idx].session.memo;
            let snap = stages::Snapshot {
                prep: m.prep(layer, keys.prep),
                quant: m.quant(layer, keys.quant),
                palette: m.palette(layer, keys.quant),
                regions: m.regions(layer, keys.regions),
                plan: m.plan(layer, keys.regions_view),
                smooth: m.smooth(layer, keys.fit),
                fit: m.fit(layer, keys.fit),
                simplify: m.simplify(layer, keys.simplify),
            };
            (snap, m.shape_cache())
        };

        self.stages_gen += 1;
        let generation = self.stages_gen;
        {
            let s = &mut self.docs[doc_idx].session;
            s.stages_running = true;
            s.stage_pending = pending;
            s.stage_gen = generation;
            s.stage_keys = keys;
        }
        stages::stream(stages::StageJob {
            doc: doc_idx,
            generation,
            img,
            offset,
            doc_dim,
            cfg,
            pending,
            shown: Shown { layer, keys, stroke_bits, stroke_color },
            snap,
            shape_cache,
        })
    }

    /// Consumes the selected document's queued-full-render latch: spawns the
    /// full render and clears the latch if it was set, otherwise does nothing.
    fn drain_full_queued(&mut self) -> Task<Msg> {
        if !self.session().is_some_and(|s| s.full_queued) {
            return Task::none();
        }
        if let Some(s) = self.session_mut() {
            s.full_queued = false;
        }
        self.spawn_full()
    }

    /// Recompute the full-document preview off the UI thread; same
    /// one-in-flight + dirty-latch scheme as the stage strip.
    pub(super) fn spawn_full(&mut self) -> Task<Msg> {
        let doc_idx = self.selected_doc;
        let Some(doc) = self.docs.get(doc_idx) else {
            return Task::none();
        };
        if doc.session.full_busy {
            self.docs[doc_idx].session.full_dirty = true;
            return Task::none();
        }
        let layers = doc.layers.clone();
        let flags = doc.flags.clone();
        let size = doc.size;
        let profiles = self.stack(doc_idx).to_owned();
        let doc_dim = size.0.max(size.1);
        // Snapshot each enabled layer's cached simplify trace, so an unchanged
        // layer is reused rather than re-traced.
        let snap: Vec<Option<Arc<LayerTrace>>> = {
            let m = &mut self.docs[doc_idx].session.memo;
            layers
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    if !flags[i].enabled {
                        return None;
                    }
                    let cfg = profiles.resolve(&l.name).0;
                    m.simplify(super::ids::LayerId(i), StageKeys::of(&cfg).simplify)
                })
                .collect()
        };
        self.full_gen += 1;
        let generation = self.full_gen;
        {
            let s = &mut self.docs[doc_idx].session;
            s.full_busy = true;
            s.full_gen = generation;
        }
        Task::perform(
            async move {
                let result = full::render_full(&layers, &flags, size, &profiles, doc_dim, snap)
                    .map_err(|e| e.to_string());
                (generation, result)
            },
            move |(generation, result)| {
                Msg::Compute(super::msg::ComputeMsg::FullReady(doc_idx, generation, result))
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::doc::{Doc, Layer, LayerFlags};
    use crate::gui::ids::LayerId;

    #[test]
    fn unchanged_stage_config_still_drains_a_queued_full_render() {
        let mut app = App::default();
        let layer = Layer {
            name: "layer".into(),
            img: RgbaImage::new(4, 4),
            offset: (0, 0),
        };
        let mut doc = Doc {
            path: "test.png".into(),
            size: (4, 4),
            layers: Arc::new(vec![layer]),
            flags: vec![LayerFlags::default()],
            session: Default::default(),
        };
        // The shown images already reflect the current config, so
        // spawn_stages will take its nothing-changed early return.
        let s = &mut doc.session;
        let keys = StageKeys::of(&s.cfg);
        s.stages.shown = Some(Shown {
            layer: LayerId(0),
            keys,
            stroke_bits: s.cfg.stroke_width.to_bits(),
            stroke_color: s.cfg.stroke_color,
        });
        app.docs.push(doc);
        app.selected_doc = 0;

        // A flag flip undone through preview_tasks: config unchanged, full
        // render queued.
        let _task = app.preview_tasks();
        let s = app.session().unwrap();
        assert!(!s.full_queued, "the latch is consumed, not stranded");
        assert!(s.full_busy, "the full render was spawned");
        assert!(!s.stages_running, "no stage run for an unchanged config");
    }
}
