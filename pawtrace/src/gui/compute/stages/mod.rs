//! The stage-strip worker: runs the per-layer pipeline through a clone of its
//! cache slots, streaming each stage's result the moment it finishes. A slot
//! serves a cached value when its inputs are unchanged, so an unedited stage
//! costs an `Arc` clone and no recompute; its image is re-emitted only when the
//! stage's key differs from the shown one (or, for the stroked renders, the
//! stroke changed), so an unchanged stage never flickers. Each streamed part
//! carries the stage's `(key, value)` memo update, which the handler installs
//! into the session memo at once, so session-state readers see each fresh value
//! mid-stream instead of at completion.
//!
//! Each stage is a submodule owning its `Inputs` struct, its `compute_*`
//! function, and that function's content hashing. This driver names them all:
//! it threads the artifacts between stages and keys each slot, but holds no
//! per-stage compute logic of its own.

mod detect;
mod fit;
mod plan;
mod prep;
mod regions;
mod remap;
mod shapes;
mod simplify;

pub(in crate::gui) use detect::DetectInputs;
pub(in crate::gui) use fit::{FitInputs, TraceOutput};
pub(in crate::gui) use plan::PlanInputs;
pub(in crate::gui) use prep::PrepInputs;
pub(in crate::gui) use regions::RegionsInputs;
pub(in crate::gui) use remap::{RemapInputs, RemapOutput};
pub(in crate::gui) use shapes::ShapesInputs;
pub(in crate::gui) use simplify::SimplifyInputs;

use detect::compute_detect;
use fit::compute_fit;
use plan::compute_plan;
use prep::compute_prep;
use regions::compute_regions;
use remap::compute_remap;
use shapes::compute_shapes;
use simplify::compute_simplify;

use super::artifact::Artifact;
use super::cache::ShapeCache;
use super::memo::Memo;
use super::render::{masked, rgba_img};
use super::{LayerTrace, Shown, StagePart};
use crate::config::Config;
use crate::gui::ids::{DocId, LayerId};
use crate::gui::msg::{ComputeMsg, Msg};
use crate::gui::phases::Stage;
use crate::color::Srgb;
use crate::palette::Partition;
use crate::pipeline::Shape;
use crate::raster::Prepared;
use crate::regions::{MergePlan, Region};
use crate::trace::{ContourParams, FitParams};
use crate::{palette, pipeline};
use iced::Task;
use image::RgbaImage;
use std::sync::Arc;

/// Every stage memo for one layer: one [`Memo`] per pipeline stage, keyed by
/// that stage's `Inputs`. Cloning clones each memo's `Arc`s, so a copy is cheap
/// and moves into the stage worker; the worker's updated copy replaces the
/// cached one when its run completes.
#[derive(Clone, Default, Debug)]
pub(in crate::gui) struct LayerStages {
    pub prep: Memo<PrepInputs, Artifact<Prepared>>,
    pub detect: Memo<DetectInputs, Artifact<Partition>>,
    pub remap: Memo<RemapInputs, RemapOutput>,
    pub regions: Memo<RegionsInputs, Artifact<Vec<Region>>>,
    pub plan: Memo<PlanInputs, Artifact<MergePlan>>,
    pub shapes: Memo<ShapesInputs, Artifact<Vec<Shape>>>,
    pub fit: Memo<FitInputs, TraceOutput>,
    pub simplify: Memo<SimplifyInputs, TraceOutput>,
}

/// Everything one stage run needs, moved into the worker.
pub(super) struct StageJob {
    pub doc: DocId,
    pub generation: u64,
    pub layer: LayerId,
    pub img: RgbaImage,
    pub offset: (u32, u32),
    pub doc_dim: u32,
    pub cfg: Config,
    /// The selected layer's speckle-floor exemption points, document source px.
    pub pins: Vec<[u32; 2]>,
    /// The keys and inputs the currently shown images reflect, or `None` when
    /// nothing is shown for this layer yet.
    pub shown: Option<Shown>,
    pub slots: LayerStages,
    pub shape_cache: ShapeCache,
}

/// The layer-fixed inputs the plan needs beyond its key: the crop origin and
/// dimensions the pins scale against, and the document dimension the floor
/// derives from.
#[derive(Clone, Copy)]
pub(super) struct PlanCtx {
    pub offset: (u32, u32),
    pub dims: (u32, u32),
    pub doc_dim: u32,
}

/// Whether a display stage is stale: no prior shown key, or the fresh key
/// differs from it.
fn stale<K: PartialEq>(shown: Option<&K>, fresh: &K) -> bool {
    shown.is_none_or(|k| k != fresh)
}

/// Total anchors across a trace: the sum of every path's cubic count over all
/// color runs.
fn anchor_total(trace: &LayerTrace) -> usize {
    trace
        .iter()
        .flat_map(|(_, ps)| ps.iter())
        .map(|p| p.cubics.len())
        .sum()
}

pub(super) fn stream(job: StageJob) -> Task<Msg> {
    Task::stream(iced::stream::channel(
        0,
        move |mut tx: iced::futures::channel::mpsc::Sender<Msg>| async move {
            use iced::futures::SinkExt;

            let StageJob {
                doc,
                generation,
                layer,
                img,
                offset,
                doc_dim,
                cfg,
                pins,
                shown,
                mut slots,
                shape_cache,
            } = job;

            // A send failure means the app dropped this stream, superseded or
            // shut down. The remaining work would be wasted either way.
            macro_rules! emit {
                ($part:expr) => {
                    if tx
                        .send(Msg::Compute(ComputeMsg::StagePart(doc, generation, $part)))
                        .await
                        .is_err()
                    {
                        return;
                    }
                };
            }

            let (w, h) = img.dimensions();

            // Source: the raw layer raster, unchanged unless nothing was shown.
            if shown.is_none() {
                emit!(StagePart::Source(rgba_img(&img)));
            } else {
                emit!(StagePart::Unchanged(Stage::Source));
            }

            // Flatten.
            let prep_key = PrepInputs::of(&cfg);
            let prep = slots.prep.get_or(prep_key.clone(), &img, compute_prep);
            if stale(shown.as_ref().and_then(|s| s.prep.as_ref()), &prep_key) {
                let flat = rgba_img(&masked(&prep.flat, &prep.alpha));
                emit!(StagePart::Flat(flat, prep_key.clone(), prep.clone()));
            } else {
                emit!(StagePart::Unchanged(Stage::Flatten));
            }

            // Detection feeds the remap key but paints nothing, so it rides its
            // own part to seed the memo the remap keys against.
            let detect_key = DetectInputs::of(&cfg);
            let detect = slots.detect.get_or(detect_key.clone(), &img, compute_detect);
            emit!(StagePart::Detect(detect_key, detect.clone()));

            // Remap: constrained remap onto the detected palette.
            let remap_key = RemapInputs {
                prep: prep.clone(),
                detect,
                merge: palette::MergeParams::of(&cfg),
                select: palette::SelectParams::of(&cfg),
                remap: palette::RemapParams::of(&cfg),
            };
            let (remap, palette) = slots.remap.get_or(remap_key.clone(), (), compute_remap);
            if stale(shown.as_ref().and_then(|s| s.remap.as_ref()), &remap_key) {
                let px = masked(&remap, &prep.alpha);
                emit!(StagePart::Remap(
                    rgba_img(&px),
                    px,
                    remap_key.clone(),
                    (remap.clone(), palette),
                ));
            } else {
                emit!(StagePart::Unchanged(Stage::Remap));
            }

            // Regions.
            let regions_key = RegionsInputs {
                remap: remap.clone(),
                prep: prep.clone(),
                params: crate::regions::SegmentParams::of(&cfg),
            };
            let regs = slots
                .regions
                .get_or(regions_key.clone(), (), compute_regions);
            if stale(
                shown.as_ref().and_then(|s| s.regions.as_ref()),
                &regions_key,
            ) {
                emit!(StagePart::Regions(
                    regions::regions_handle(&regs, remap.dimensions()),
                    regions_key.clone(),
                    regs.clone(),
                ));
            } else {
                emit!(StagePart::Unchanged(Stage::Regions));
            }

            // Merge plan feeds the fates and, through the shapes it plans, the trace.
            let plan_key = PlanInputs {
                regs: regs.clone(),
                prep: prep.clone(),
                pins: pins.clone(),
                params: crate::regions::PlanParams::of(&cfg),
            };
            let plan_ctx = PlanCtx {
                offset,
                dims: (w, h),
                doc_dim,
            };
            let plan = slots.plan.get_or(plan_key.clone(), plan_ctx, compute_plan);
            // Fates track the plan: a pin edit reworks the merge without touching
            // the segmentation raster. No busy flag, so only emit on a change.
            if stale(shown.as_ref().and_then(|s| s.plan.as_ref()), &plan_key) {
                let report = crate::regions::report_of(&plan);
                let tint = plan::fate_tint_handle(&regs, remap.dimensions(), &report.fates);
                emit!(StagePart::Fates(tint, report, plan_key.clone(), plan.clone()));
            }

            // One shape build feeds the fit stage, which walks each shape's
            // contours before fitting them.
            let shapes_key = ShapesInputs {
                plan: plan.clone(),
                prep: prep.clone(),
                params: pipeline::ShapeParams::of(&cfg),
            };
            let shapes = slots.shapes.get_or(shapes_key.clone(), (), compute_shapes);
            // Shape planning paints nothing; install it so the fit keys hit.
            emit!(StagePart::Shapes(shapes_key, shapes.clone()));

            // Fit: the boundary walk and the cubic fit, keyed on the shapes
            // artifact so the full render shares it. The fit and simplify views
            // draw the trace as vectors from the memo the part installs.
            let fit_key = FitInputs {
                shapes: shapes.clone(),
                contour: ContourParams::of(&cfg),
                fit: FitParams::of(&cfg),
            };
            let fit = slots.fit.get_or(fit_key.clone(), shape_cache, compute_fit);
            if stale(shown.as_ref().and_then(|s| s.fit.as_ref()), &fit_key) {
                emit!(StagePart::Fit(fit_key.clone(), fit.clone()));
            } else {
                emit!(StagePart::Unchanged(Stage::Fit));
            }

            // Simplify: the final trace (the fit trace when simplify is off).
            let simp_key = SimplifyInputs {
                fit: fit_key.clone(),
                params: crate::pipeline::SimplifyParams::of(&cfg),
            };
            let simpl = slots
                .simplify
                .get_or(simp_key.clone(), fit, compute_simplify);
            if stale(shown.as_ref().and_then(|s| s.simplify.as_ref()), &simp_key) {
                emit!(StagePart::Simplify(simp_key.clone(), simpl.clone()));
            } else {
                emit!(StagePart::Unchanged(Stage::Simplify));
            }

            let now_shown = Shown {
                layer,
                cfg,
                pins,
                prep: Some(prep_key),
                remap: Some(remap_key),
                regions: Some(regions_key),
                plan: Some(plan_key),
                fit: Some(fit_key),
                simplify: Some(simp_key),
            };
            emit!(StagePart::Done(Box::new(now_shown)));
        },
    ))
}

impl LayerStages {
    /// The extracted palette from the remap memo's current value, empty before
    /// the remap stage has run for this layer.
    pub(in crate::gui) fn palette(&self) -> Arc<Vec<Srgb>> {
        self.remap.current().map(|(_, pal)| pal).unwrap_or_default()
    }

    /// The segmented region count from the regions memo's current value, 0
    /// before the regions stage has run.
    pub(in crate::gui) fn region_count(&self) -> usize {
        self.regions.current().map_or(0, |r| r.len())
    }

    /// The pre-simplify anchor total from the fit memo's current trace, 0 before
    /// the fit stage has run.
    pub(in crate::gui) fn fit_anchors(&self) -> usize {
        self.fit.current().map_or(0, |o| anchor_total(&o.trace))
    }

    /// The final anchor total from the simplify memo's current trace, 0 before
    /// the simplify stage has run.
    pub(in crate::gui) fn simplify_anchors(&self) -> usize {
        self.simplify.current().map_or(0, |o| anchor_total(&o.trace))
    }

    /// Runs the per-layer stage chain into these memo slots and returns the
    /// final trace, emitting no images and recording no shown keys. A slot hit
    /// reuses its cached value.
    pub(super) fn trace(
        &mut self,
        img: &RgbaImage,
        cfg: &Config,
        pins: &[[u32; 2]],
        plan_ctx: PlanCtx,
        shape_cache: &ShapeCache,
    ) -> Arc<LayerTrace> {
        // The get_or sequence mirrors stream's, so the strip and the full
        // render share every slot they both touch.
        let prep = self.prep.get_or(PrepInputs::of(cfg), img, compute_prep);

        let detect = self
            .detect
            .get_or(DetectInputs::of(cfg), img, compute_detect);
        let (remap, _palette) = self.remap.get_or(
            RemapInputs {
                prep: prep.clone(),
                detect,
                merge: palette::MergeParams::of(cfg),
                select: palette::SelectParams::of(cfg),
                remap: palette::RemapParams::of(cfg),
            },
            (),
            compute_remap,
        );

        let regs = self.regions.get_or(
            RegionsInputs {
                remap,
                prep: prep.clone(),
                params: crate::regions::SegmentParams::of(cfg),
            },
            (),
            compute_regions,
        );

        let plan = self.plan.get_or(
            PlanInputs {
                regs,
                prep: prep.clone(),
                pins: pins.to_vec(),
                params: crate::regions::PlanParams::of(cfg),
            },
            plan_ctx,
            compute_plan,
        );

        let shapes = self.shapes.get_or(
            ShapesInputs {
                plan,
                prep: prep.clone(),
                params: pipeline::ShapeParams::of(cfg),
            },
            (),
            compute_shapes,
        );

        let fit_key = FitInputs {
            shapes,
            contour: ContourParams::of(cfg),
            fit: FitParams::of(cfg),
        };
        let fit = self
            .fit
            .get_or(fit_key.clone(), shape_cache.clone(), compute_fit);

        self.simplify
            .get_or(
                SimplifyInputs {
                    fit: fit_key,
                    params: pipeline::SimplifyParams::of(cfg),
                },
                fit,
                compute_simplify,
            )
            .trace
    }
}

#[cfg(test)]
mod tests {
    use super::super::artifact::Artifact;
    use super::*;
    use crate::color::Srgb;
    use crate::palette::DetectParams;
    use crate::pipeline::SimplifyParams;
    use crate::raster::{PrepParams, Prepared};
    use crate::regions::{PlanParams, Region, SegmentParams};
    use crate::trace::{ContourParams, FitParams};
    use image::RgbImage;
    use std::hash::Hasher;
    use std::sync::Arc;

    fn dummy_prep() -> Artifact<Prepared> {
        let prep = crate::raster::prepare(
            &image::RgbaImage::new(2, 2),
            &PrepParams::of(&Config::default()),
        );

        Artifact::new_with(Arc::new(prep), |_, _| {})
    }

    fn rgb(px: Srgb) -> Artifact<RgbImage> {
        let mut img = RgbImage::new(1, 1);

        img.put_pixel(0, 0, px.into());

        Artifact::new_with(Arc::new(img), |img, h| {
            h.write_u32(img.width());
            h.write_u32(img.height());
            h.write(img.as_raw());
        })
    }

    // An artifact's identity is its content hash, not its allocation: two
    // artifacts with the same hash but distinct `Arc`s are equal, so a
    // downstream key built from either matches and a recompute to identical
    // content cuts off. A differing hash moves the key.
    #[test]
    fn equal_content_hits_and_different_content_misses() {
        let a = rgb(Srgb([0, 0, 0]));
        let b = rgb(Srgb([0, 0, 0]));
        let c = rgb(Srgb([9, 9, 9]));
        assert_eq!(a, b, "equal content hashes compare equal");
        assert_ne!(a, c, "different content hashes compare unequal");

        let prep = dummy_prep();
        let params = SegmentParams::of(&Config::default());
        let key = |q: Artifact<RgbImage>| RegionsInputs {
            remap: q,
            prep: prep.clone(),
            params: params.clone(),
        };
        assert_eq!(key(a.clone()), key(b), "equal content keeps the key equal");
        assert_ne!(key(a), key(c), "a content change moves the key");
    }

    // Pins enter at the plan: a pin edit moves the plan key (and thus the fates
    // and, through the chained shapes artifact, the trace), but leaves the
    // segmentation keys untouched.
    #[test]
    fn a_pin_edit_moves_the_plan_key_only() {
        let prep = dummy_prep();
        let remap = rgb(Srgb([0, 0, 0]));
        let regs = Artifact::new(Arc::new(Vec::<Region>::new()));
        let regions = RegionsInputs {
            remap: remap.clone(),
            prep: prep.clone(),
            params: SegmentParams::of(&Config::default()),
        };
        let plan = |pins: Vec<[u32; 2]>| PlanInputs {
            regs: regs.clone(),
            prep: prep.clone(),
            pins,
            params: PlanParams::of(&Config::default()),
        };
        assert_eq!(regions, regions.clone(), "segmentation is pin-independent");
        assert_ne!(plan(vec![]), plan(vec![[3, 4]]), "the plan folds the pins");
    }

    // A config edit ripples exactly as far as its params reach. Stage keys
    // downstream of the split chain by content, so a config field is tracked to
    // the param struct that reads it: simplify to the simplify params, opttol
    // and seam_slack to the fit params, alphamax to the contour params, detail
    // to the plan, scale to segmentation, alpha_threshold to detection.
    #[test]
    fn config_edits_ripple_by_field() {
        let base = Config::default();
        let simp = Config {
            simplify: 5.0,
            ..base.clone()
        };
        assert_eq!(
            FitParams::of(&base),
            FitParams::of(&simp),
            "the fit is simplify-independent"
        );
        assert_ne!(
            SimplifyParams::of(&base),
            SimplifyParams::of(&simp),
            "simplify moves the final trace"
        );

        // opttolerance and seam_slack are the fit's alone: they move the fit
        // params, never the contour walk.
        let optt = Config {
            opttolerance: base.opttolerance + 0.1,
            ..base.clone()
        };
        assert_ne!(
            FitParams::of(&base),
            FitParams::of(&optt),
            "opttolerance moves the fit"
        );
        assert_eq!(
            ContourParams::of(&base),
            ContourParams::of(&optt),
            "opttolerance leaves the contour walk"
        );

        // detail drives the speckle floor: the plan params move, the
        // segmentation params do not.
        let detail = Config {
            detail: 9.0,
            ..base.clone()
        };
        assert_ne!(
            PlanParams::of(&base),
            PlanParams::of(&detail),
            "detail moves the plan"
        );
        assert_eq!(SegmentParams::of(&base), SegmentParams::of(&detail));

        // scale sizes the absorption ceilings, so it moves the segmentation.
        let scale = Config {
            scale: 4,
            ..base.clone()
        };
        assert_ne!(
            SegmentParams::of(&base),
            SegmentParams::of(&scale),
            "scale moves segmentation"
        );

        // alphamax is the corner threshold: the contour walk moves,
        // segmentation and the plan do not.
        let alpha_max = Config {
            alphamax: 1.3,
            ..base.clone()
        };
        assert_ne!(
            ContourParams::of(&base),
            ContourParams::of(&alpha_max),
            "alphamax moves the contours"
        );
        assert_eq!(
            FitParams::of(&base),
            FitParams::of(&alpha_max),
            "alphamax leaves the fit params (it reaches the fit through the contours)"
        );
        assert_eq!(SegmentParams::of(&base), SegmentParams::of(&alpha_max));
        assert_eq!(PlanParams::of(&base), PlanParams::of(&alpha_max));

        // Detection reads only alpha_threshold; a palette-shaping edit leaves it.
        let bands = Config {
            shade_split: base.shade_split + 0.01,
            ..base.clone()
        };
        assert_eq!(DetectParams::of(&base), DetectParams::of(&bands));
        let alpha = Config {
            alpha_threshold: base.alpha_threshold.wrapping_add(1),
            ..base.clone()
        };
        assert_ne!(
            DetectParams::of(&base),
            DetectParams::of(&alpha),
            "alpha_threshold invalidates detection"
        );
    }

    // A uniform-color layer must trace identically through the staged chain and
    // the monolithic run: the run skips palette and remap for a solid layer, so
    // the chain's regions stage segments straight from the mask to match. The
    // art is a strict interior crop, so this also exercises the crop's
    // shift-equivariance the run relies on.
    #[test]
    fn a_uniform_layer_traces_identically_through_the_staged_chain() {
        use lru::LruCache;
        use std::num::NonZeroUsize;
        use std::sync::Mutex;

        let mut img = RgbaImage::new(40, 40);
        for y in 8..32u32 {
            for x in 8..32u32 {
                img.put_pixel(x, y, image::Rgba([40, 160, 150, 255]));
            }
        }

        let cfg = Config {
            scale: 3,
            detail: 1.0,
            ..Default::default()
        };
        let doc_dim = 40;

        let mono = pipeline::run(&img, &cfg, doc_dim, (0, 0), &[]).unwrap();

        let mut slots = LayerStages::default();
        let cache: ShapeCache = Arc::new(Mutex::new(LruCache::new(NonZeroUsize::new(64).unwrap())));
        let plan_ctx = PlanCtx {
            offset: (0, 0),
            dims: img.dimensions(),
            doc_dim,
        };
        let staged = slots.trace(&img, &cfg, &[], plan_ctx, &cache);

        assert_eq!(mono.len(), staged.len());
        assert!(!mono.is_empty());

        for ((h1, p1), (h2, p2)) in mono.iter().zip(staged.iter()) {
            assert_eq!(h1, h2);
            assert_eq!(p1.len(), p2.len());

            for (a, b) in p1.iter().zip(p2) {
                assert_eq!(a.start, b.start);
                assert_eq!(a.cubics, b.cubics);
            }
        }
    }
}
