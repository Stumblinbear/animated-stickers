//! Background compute: the per-layer stage strip (streamed part by part) and
//! the full-document preview. Both read and fill a per-document content-keyed
//! [`cache::DocStages`], so an edit recomputes only the stages whose inputs
//! changed and a layer the full render already traced needs no re-trace.

mod artifact;
mod cache;
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

pub(in crate::gui) use cache::DocStages;
pub(super) use full::export_doc;
pub(in crate::gui) use stages::LayerStages;

use super::msg::Msg;
use super::phases::{PerStage, Stage};

/// A display handle with the pixel dimensions it was built from.
#[derive(Debug, Clone)]
pub struct Img {
    pub handle: iced_image::Handle,
    pub size: (u32, u32),
}

/// One layer's traced colors: color hex -> paths, in bottom-first paint order.
pub(super) type LayerTrace = Vec<(String, Vec<crate::trace::TracedPath>)>;

/// The layer, inputs, and per-stage keys the current stage images reflect, so
/// the next spawn can tell whether the strip changed at all (the `cfg`/`pins`
/// compare) and, when it did, which stage images went stale (the per-stage key
/// compares) without re-deriving them.
#[derive(Debug, Clone)]
pub(super) struct Shown {
    pub(super) layer: super::ids::LayerId,
    pub(super) cfg: crate::config::Config,
    pub(super) pins: Vec<[u32; 2]>,
    pub(super) stroke_bits: u32,
    pub(super) stroke_color: [u8; 3],
    /// Per-display-stage keys the shown images were built under. `None` for a
    /// stage never yet shown (the very first run for this layer), which reads as
    /// stale so the worker draws it.
    pub(super) prep: Option<stages::PrepInputs>,
    pub(super) remap: Option<stages::RemapInputs>,
    pub(super) regions: Option<stages::RegionsInputs>,
    pub(super) plan: Option<stages::PlanInputs>,
    pub(super) contours: Option<stages::ContoursInputs>,
    pub(super) fit: Option<stages::FitInputs>,
    pub(super) simplify: Option<stages::SimplifyInputs>,
}

#[cfg(test)]
impl Shown {
    /// A shown record carrying only the strip inputs, its per-stage keys unset,
    /// for exercising the "did anything change" early-out.
    pub(super) fn inputs(
        layer: super::ids::LayerId,
        cfg: crate::config::Config,
        pins: Vec<[u32; 2]>,
        stroke_bits: u32,
        stroke_color: [u8; 3],
    ) -> Self {
        Shown {
            layer,
            cfg,
            pins,
            stroke_bits,
            stroke_color,
            prep: None,
            remap: None,
            regions: None,
            plan: None,
            contours: None,
            fit: None,
            simplify: None,
        }
    }
}

/// Stage outputs for the selected layer, as display images.
#[derive(Debug, Clone, Default)]
pub(super) struct StageImages {
    pub(super) source: Option<Img>,
    pub(super) flat: Option<Img>,
    pub(super) remap: Option<Img>,
    /// Remapped pixels with the alpha mask applied, kept for the
    /// click-to-lock color picker.
    pub(super) remap_px: Option<RgbaImage>,
    pub(super) regions: Option<Img>,
    /// Trace-fate tint over the segmentation, composited by the fates overlay on
    /// the Regions view; `None` when every region survives.
    pub(super) fate_tint: Option<Img>,
    /// Per-region trace fates and floor for the regions hover readout,
    /// aligned with the cached regions.
    pub(super) region_report: Option<regions::RegionReport>,
    /// Smoothed boundary with corner markers, pre-fit.
    pub(super) contours: Option<Img>,
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

/// A failed full render: the human-readable message and, when the failure was
/// one layer's trace, which layer it came from. `layer` is `None` for a failure
/// not tied to a specific layer, such as the final composite render.
#[derive(Debug, Clone)]
pub struct FullError {
    pub layer: Option<super::ids::LayerId>,
    pub msg: String,
}

/// Full-preview result: the composite image, totals, per-layer derived render
/// outputs keyed by layer id, and the pre-transform traces newly computed this
/// run, for the memo.
#[derive(Debug, Clone)]
pub struct FullResult {
    pub(super) img: Img,
    pub(super) stats: DocStats,
    pub(super) outputs: rustc_hash::FxHashMap<super::ids::LayerId, super::doc::LayerOutputs>,
    pub(super) merges: Vec<full::FullMerge>,
}

/// One stage's display output, streamed the moment it finishes so the fast
/// early stages appear without waiting for the trace. A part carries only what
/// the display needs; the recomputed cache values ride home once, in
/// [`StagePart::Done`]. A stage whose inputs are unchanged emits
/// [`StagePart::Unchanged`] instead, clearing its busy flag without a redraw.
#[derive(Debug, Clone)]
pub enum StagePart {
    Source(Img),
    Flat(Img),
    Remap(Img, RgbaImage, Vec<[u8; 3]>),
    Regions(Img, usize),
    /// The fate tint (`None` when every region survives) and the region report,
    /// emitted whenever the merge plan changes, including on a pin edit.
    Fates(Option<Img>, regions::RegionReport),
    Contours(Option<Img>),
    /// Fitted render and its anchor count.
    Fit(Option<Img>, usize),
    /// Simplified render and its anchor count.
    Simplify(Option<Img>, usize),
    /// A display stage whose inputs matched the shown ones: no redraw, just
    /// clear the pending flag.
    Unchanged(Stage),
    /// Always the final part; completion is detected by it. Carries the worker's
    /// completed slots to install in the memo and the keys the images now
    /// reflect.
    Done(Box<LayerStages>, Box<Shown>),
}

impl App {
    /// Recompute the stage strip off the UI thread, streaming each stage as it
    /// finishes. One stream in flight at a time: further edits set the dirty
    /// latch and re-spawn on completion.
    pub(super) fn spawn_stages(&mut self) -> Task<Msg> {
        let doc_id = self.selected_doc;

        let Some(pos) = self.doc_pos(doc_id) else {
            return Task::none();
        };

        let doc = &self.docs[pos];
        let layer = doc.session.selected_layer;

        let Some(src) = doc.layer(layer) else {
            return self.drain_full_queued();
        };

        if doc.session.stages_running {
            self.docs[pos].session.stages_dirty = true;
            return Task::none();
        }

        let cfg = doc.session.cfg.clone();

        // The worker needs the layer pins for the trace, and the plan key
        // folds them in.
        let pins = doc
            .inputs
            .get(&layer)
            .map(|i| i.pins.clone())
            .unwrap_or_default();

        let stroke_bits = cfg.stroke_width.to_bits();
        let stroke_color = cfg.stroke_color;
        let doc_dim = doc.size.0.max(doc.size.1);

        let shown = doc.session.preview.shown.clone();

        // Nothing the strip reads changed against the shown images: leave them,
        // clear any stale pending flags, and let a queued full render proceed
        // without spawning (a flag flip re-composites without touching the
        // strip's inputs), so the latch drains here.
        if shown.as_ref().is_some_and(|s| {
            s.layer == layer
                && s.cfg == cfg
                && s.pins == pins
                && s.stroke_bits == stroke_bits
                && s.stroke_color == stroke_color
        }) {
            self.docs[pos].session.stage_pending = PerStage::from_fn(|_| false);
            return self.drain_full_queued();
        }

        let img = src.img.clone();
        let offset = src.offset;

        let (slots, shape_cache) = {
            let m = &mut self.docs[pos].session.stages;
            (m.stages(layer), m.shape_cache())
        };

        self.stages_gen += 1;

        let generation = self.stages_gen;
        {
            let s = &mut self.docs[pos].session;
            s.stages_running = true;
            // Which stages recompute depends on cached-Arc identities the worker
            // discovers as it runs, so mark all busy up front; the worker clears
            // each as it emits a redraw or an unchanged marker.
            s.stage_pending = PerStage::from_fn(|_| true);
            s.stage_gen = generation;
        }

        stages::stream(stages::StageJob {
            doc: doc_id,
            generation,
            layer,
            img,
            offset,
            doc_dim,
            cfg,
            pins,
            stroke_bits,
            stroke_color,
            shown,
            slots,
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
        let doc_id = self.selected_doc;

        let Some(pos) = self.doc_pos(doc_id) else {
            return Task::none();
        };

        let doc = &self.docs[pos];

        if doc.session.full_busy {
            self.docs[pos].session.full_dirty = true;
            return Task::none();
        }

        let layers = doc.layers.clone();
        let inputs = doc.inputs.clone();
        let size = doc.size;
        let profiles = self.stack(pos).to_owned();
        let doc_dim = size.0.max(size.1);

        // Snapshot each enabled layer's cached simplify trace, so an unchanged
        // layer is reused rather than re-traced.
        let snap: Vec<Option<Arc<LayerTrace>>> = {
            let m = &self.docs[pos].session.stages;

            layers
                .iter()
                .map(|l| {
                    let inp = &inputs[&l.id];

                    if !inp.enabled {
                        return None;
                    }

                    let cfg = profiles.resolve(&l.name).0;

                    m.peek(l.id)
                        .and_then(|s| s.simplify.get(&stages::SimplifyInputs::of(&cfg, &inp.pins)))
                })
                .collect()
        };

        self.full_gen += 1;

        let generation = self.full_gen;
        {
            let s = &mut self.docs[pos].session;
            s.full_busy = true;
            s.full_gen = generation;
        }

        Task::perform(
            async move {
                let result = full::render_full(&layers, &inputs, size, &profiles, doc_dim, snap);
                (generation, result)
            },
            move |(generation, result)| {
                Msg::Compute(super::msg::ComputeMsg::FullReady(
                    doc_id, generation, result,
                ))
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::doc::{Doc, Layer, LayerInputs};
    use crate::gui::ids::{DocId, LayerId};

    #[test]
    fn unchanged_stage_config_still_drains_a_queued_full_render() {
        let mut app = App::default();

        let lid = LayerId::from_raw(1);

        let layer = Layer {
            id: lid,
            name: "layer".into(),
            img: RgbaImage::new(4, 4),
            offset: (0, 0),
        };

        let mut doc = Doc {
            id: DocId::from_raw(0),
            path: "test.png".into(),
            size: (4, 4),
            layers: Arc::new(vec![layer]),
            inputs: [(lid, LayerInputs::default())].into_iter().collect(),
            session: Default::default(),
        };

        // The shown images already reflect the current config for the selected
        // layer, so spawn_stages will take its nothing-changed early return.
        let s = &mut doc.session;

        s.selected_layer = lid;
        s.preview.shown = Some(Shown::inputs(
            lid,
            s.cfg.clone(),
            Vec::new(),
            s.cfg.stroke_width.to_bits(),
            s.cfg.stroke_color,
        ));
        app.docs.push(doc);
        app.selected_doc = DocId::from_raw(0);

        // A flag flip undone through preview_tasks: config unchanged, full
        // render queued.
        let _task = app.preview_tasks();
        let s = app.session().unwrap();
        assert!(!s.full_queued, "the latch is consumed, not stranded");
        assert!(s.full_busy, "the full render was spawned");
        assert!(!s.stages_running, "no stage run for an unchanged config");
    }
}
