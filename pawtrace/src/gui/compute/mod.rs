//! Background compute: the per-layer stage strip (streamed part by part) and
//! the full-document preview. Both read and fill a per-document content-keyed
//! [`cache::DocStages`], so an edit recomputes only the stages whose inputs
//! changed and a layer the full render already traced needs no re-trace.

mod artifact;
mod cache;
mod full;
mod memo;
mod render;
mod stages;

use super::app::App;
use crate::palette::Partition;
use crate::pipeline::Shape;
use crate::raster::Prepared;
use crate::regions::{self, MergePlan, Region};
pub(in crate::gui) use artifact::Artifact;
use iced::widget::image as iced_image;
use iced::Task;
use image::RgbaImage;
use stages::{
    DetectInputs, PlanInputs, PrepInputs, RegionsInputs, RemapInputs, RemapOutput, ShapesInputs,
    SimplifyInputs,
};
pub(in crate::gui) use stages::{FitInputs, TraceOutput};
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
pub(super) type LayerTrace = crate::output::LayerColors;

/// A drawable vector view: bottom-first color-run layers in a supersample
/// coordinate space, and the crop-space size they fill. Dividing a path
/// coordinate by `scale` places it in the `dims`-sized crop-space rectangle the
/// viewport maps to screen.
#[derive(Debug, Clone)]
pub(super) struct VectorScene {
    pub(super) dims: (u32, u32),
    pub(super) scale: u32,
    pub(super) layers: Vec<VectorLayer>,
}

/// One layer of a [`VectorScene`]: its color runs, the per-path culling boxes
/// aligned with those runs, and the optional centered stroke drawn on every
/// run, matching the SVG export.
#[derive(Debug, Clone)]
pub(super) struct VectorLayer {
    pub(super) colors: Arc<LayerTrace>,
    /// Per-path bounding boxes grouped exactly like `colors`, so `bboxes[r][p]`
    /// bounds `colors[r].1[p]`. Derived from `colors` at production, cloned as
    /// an `Arc` alongside it.
    pub(super) bboxes: Arc<LayerBboxes>,
    pub(super) stroke: Option<crate::output::Stroke>,
}

/// An axis-aligned bounding box in a trace's supersample coordinate space (a
/// path coordinate before the `/scale` to crop px). Conservative: taken over
/// each path's start point and every cubic control point, so it contains the
/// curve without evaluating it.
#[derive(Debug, Clone, Copy)]
pub(super) struct Bbox {
    pub(super) min: (f64, f64),
    pub(super) max: (f64, f64),
}

impl Bbox {
    /// Whether this box overlaps the axis-aligned rect `[lo, hi]`, touching
    /// edges included. Used to cull paths against the visible viewport.
    pub(super) fn overlaps(&self, lo: (f64, f64), hi: (f64, f64)) -> bool {
        self.max.0 >= lo.0 && self.min.0 <= hi.0 && self.max.1 >= lo.1 && self.min.1 <= hi.1
    }
}

/// Per-path bounding boxes for one layer's trace, grouped like the trace's
/// color runs.
pub(super) type LayerBboxes = Vec<Vec<Bbox>>;

/// The conservative box of one path: the hull of its start point and every
/// cubic control point.
fn path_bbox(p: &crate::trace::TracedPath) -> Bbox {
    let mut min = p.start;
    let mut max = p.start;
    let mut fold = |(x, y): (f64, f64)| {
        min.0 = min.0.min(x);
        min.1 = min.1.min(y);
        max.0 = max.0.max(x);
        max.1 = max.1.max(y);
    };
    for &(c1, c2, end) in &p.cubics {
        fold(c1);
        fold(c2);
        fold(end);
    }
    Bbox { min, max }
}

/// The per-path culling boxes for `trace`, grouped like its color runs so
/// `result[r][p]` bounds `trace[r].1[p]`.
pub(super) fn layer_bboxes(trace: &LayerTrace) -> LayerBboxes {
    trace
        .iter()
        .map(|(_, paths)| paths.iter().map(path_bbox).collect())
        .collect()
}

impl VectorScene {
    /// An empty scene that renders nothing, for the moment before an active
    /// vector view has produced content.
    pub(super) fn empty() -> Self {
        Self {
            dims: (1, 1),
            scale: 1,
            layers: Vec::new(),
        }
    }
}

/// What the preview paints for the active view: raster pixels for the genuine
/// image stages, or vector color runs for the trace-backed views. Both resolve
/// to the same crop-space rectangle, so pan, zoom, and the overlays align
/// regardless of which the active view draws.
pub(super) enum Art<'a> {
    Raster { img: &'a Img, factor: f32 },
    Vector(VectorScene),
}

impl Art<'_> {
    /// The crop-space dimensions the art fills, the size the viewport places it
    /// against.
    pub(super) fn dims(&self) -> (f32, f32) {
        match self {
            Art::Raster { img, factor } => (img.size.0 as f32 / factor, img.size.1 as f32 / factor),
            Art::Vector(s) => (s.dims.0 as f32, s.dims.1 as f32),
        }
    }
}

/// The layer, inputs, and per-stage keys the current stage images reflect, so
/// the next spawn can tell whether the strip changed at all (the `cfg`/`pins`
/// compare) and, when it did, which stage images went stale (the per-stage key
/// compares) without re-deriving them.
#[derive(Debug, Clone)]
pub(super) struct Shown {
    pub(super) layer: super::ids::LayerId,
    pub(super) cfg: crate::config::Config,
    pub(super) pins: Vec<[u32; 2]>,
    /// Per-display-stage keys the shown images were built under. `None` for a
    /// stage never yet shown (the very first run for this layer), which reads as
    /// stale so the worker draws it.
    pub(super) prep: Option<stages::PrepInputs>,
    pub(super) remap: Option<stages::RemapInputs>,
    pub(super) regions: Option<stages::RegionsInputs>,
    pub(super) plan: Option<stages::PlanInputs>,
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
    ) -> Self {
        Shown {
            layer,
            cfg,
            pins,
            prep: None,
            remap: None,
            regions: None,
            plan: None,
            fit: None,
            simplify: None,
        }
    }
}

/// Worker-baked display rasters for the selected layer's stage strip. Only
/// state that the worker paints and no memo holds lives here; the palette,
/// region count, and anchor counts derive from the session stage memos at view
/// time (see [`DocState`](crate::gui::app::DocState)).
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
    pub(super) shown: Option<Shown>,
}

/// Whole-document totals, computed alongside the full preview.
#[derive(Debug, Clone, Copy)]
pub struct DocStats {
    pub shapes: usize,
    pub anchors: usize,
}

/// Full-preview result: the composite vector scene, totals, per-layer derived
/// render outputs keyed by layer id, and each recomputed layer's filled stage
/// slots to reinstall in the cache.
#[derive(Debug, Clone)]
pub struct FullResult {
    pub(super) scene: VectorScene,
    pub(super) stats: DocStats,
    pub(super) outputs: rustc_hash::FxHashMap<super::ids::LayerId, super::doc::LayerOutputs>,
    pub(super) stages: rustc_hash::FxHashMap<super::ids::LayerId, LayerStages>,
}

/// One stage's streamed result: the display raster the view needs, plus the
/// stage's `(key, value)` memo update, which the handler installs into the
/// session memo the moment it arrives so session-state readers (the vector
/// preview, the anchors overlay, the pin hit test) see the fresh value
/// mid-stream rather than at completion. A display stage whose inputs match the
/// shown ones emits [`StagePart::Unchanged`]: its session memo already holds
/// the current value, so it only clears the pending flag. The internal `Detect`
/// and `Shapes` memos have no display, so they ride dedicated payload-less
/// parts that install unconditionally.
#[derive(Debug, Clone)]
pub enum StagePart {
    Source(Img),
    Flat(Img, PrepInputs, Artifact<Prepared>),
    /// The detection memo update; detection has no display, so this part only
    /// seeds the session memo the remap stage keys against.
    Detect(DetectInputs, Artifact<Partition>),
    Remap(Img, RgbaImage, RemapInputs, RemapOutput),
    Regions(Img, RegionsInputs, Artifact<Vec<Region>>),
    /// The fate tint (`None` when every region survives), the region report, and
    /// the merge-plan memo update, emitted whenever the plan changes, including
    /// on a pin edit. The plan's display is the tint, so it carries the update.
    Fates(
        Option<Img>,
        regions::RegionReport,
        PlanInputs,
        Artifact<MergePlan>,
    ),
    /// The shapes memo update; shape planning has no display, so this part only
    /// seeds the session memo the fit stage keys against.
    Shapes(ShapesInputs, Artifact<Vec<Shape>>),
    /// The fit memo update; the fit view draws as vectors from this memo.
    Fit(FitInputs, TraceOutput),
    /// The simplify memo update; the simplify view draws as vectors from it.
    Simplify(SimplifyInputs, TraceOutput),
    /// A display stage whose inputs matched the shown ones: no redraw, just
    /// clear the pending flag.
    Unchanged(Stage),
    /// Always the final part; completion is detected by it. The memos are
    /// already current, so it carries only the keys the images now reflect.
    Done(Box<Shown>),
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

        let doc_dim = doc.size.0.max(doc.size.1);

        let shown = doc.session.preview.shown.clone();

        // Nothing the strip reads changed against the shown images: leave them,
        // clear any stale pending flags, and let a queued full render proceed
        // without spawning (a flag flip re-composites without touching the
        // strip's inputs), so the latch drains here. The config carries the
        // stroke, so its equality covers a stroke edit too.
        if shown
            .as_ref()
            .is_some_and(|s| s.layer == layer && s.cfg == cfg && s.pins == pins)
        {
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

        // Clone each enabled layer's stage slots for the worker to run its
        // chain against, so a layer already traced hits its cached values.
        let (slots, shape_cache) = {
            let m = &mut self.docs[pos].session.stages;
            m.ensure_layers(layers.len());

            let slots: rustc_hash::FxHashMap<_, _> = layers
                .iter()
                .filter(|l| inputs[&l.id].enabled)
                .map(|l| (l.id, m.stages(l.id)))
                .collect();

            (slots, m.shape_cache())
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
                let result = full::render_full(
                    &layers,
                    &inputs,
                    size,
                    &profiles,
                    doc_dim,
                    slots,
                    shape_cache,
                );
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
    use std::sync::Arc;

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
        s.preview.shown = Some(Shown::inputs(lid, s.cfg.clone(), Vec::new()));
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
