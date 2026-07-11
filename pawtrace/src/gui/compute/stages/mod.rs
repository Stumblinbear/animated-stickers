//! The stage-strip worker: runs the per-layer pipeline through its cache slots,
//! streaming each display image the moment its stage finishes. A slot serves a
//! cached value when its inputs are unchanged, so an unedited stage costs an
//! `Arc` clone and no recompute; its image is re-emitted only when the stage's
//! key differs from the shown one (or, for the stroked renders, the stroke
//! changed), so an unchanged stage never flickers. The worker owns a clone of
//! the layer's slots, fills it as it runs, and ships it home in the final part.
//!
//! Each stage is a submodule owning its `Inputs` struct, its `compute_*`
//! function, and that function's content hashing. This driver names them all:
//! it threads the artifacts between stages and keys each slot, but holds no
//! per-stage compute logic of its own.

mod contours;
mod detect;
mod fit;
mod plan;
mod prep;
mod regions;
mod remap;
mod shapes;
mod simplify;

pub(in crate::gui) use contours::ContoursInputs;
pub(in crate::gui) use detect::DetectInputs;
pub(in crate::gui) use fit::FitInputs;
pub(in crate::gui) use plan::PlanInputs;
pub(in crate::gui) use prep::PrepInputs;
pub(in crate::gui) use regions::RegionsInputs;
pub(in crate::gui) use remap::{RemapInputs, RemapOut};
pub(in crate::gui) use shapes::ShapesInputs;
pub(in crate::gui) use simplify::SimplifyInputs;

use contours::compute_contours;
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
use super::render::{fate_tint_handle, masked, regions_handle, render_svg, rgba_img};
use super::{Img, LayerTrace, Shown, StagePart};
use crate::config::Config;
use crate::gui::ids::{DocId, LayerId};
use crate::gui::msg::{ComputeMsg, Msg};
use crate::gui::phases::Stage;
use crate::palette::Partition;
use crate::pipeline::Shape;
use crate::raster::Prepared;
use crate::regions::{MergePlan, Region};
use crate::{output, palette, pipeline};
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
    pub remap: Memo<RemapInputs, RemapOut>,
    pub regions: Memo<RegionsInputs, Artifact<Vec<Region>>>,
    pub plan: Memo<PlanInputs, Artifact<MergePlan>>,
    pub shapes: Memo<ShapesInputs, Artifact<Vec<Shape>>>,
    pub contours: Memo<ContoursInputs, Option<Img>>,
    pub fit: Memo<FitInputs, Arc<LayerTrace>>,
    pub simplify: Memo<SimplifyInputs, Arc<LayerTrace>>,
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
    pub stroke_bits: u32,
    pub stroke_color: [u8; 3],
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
                stroke_bits,
                stroke_color,
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

            let stroke_same = shown
                .as_ref()
                .is_some_and(|s| s.stroke_bits == stroke_bits && s.stroke_color == stroke_color);
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
                emit!(StagePart::Flat(rgba_img(&masked(&prep.flat, &prep.alpha))));
            } else {
                emit!(StagePart::Unchanged(Stage::Flatten));
            }

            // Remap: detection then constrained remap, palette alongside.
            let detect = slots
                .detect
                .get_or(DetectInputs::of(&cfg), &img, compute_detect);
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
                emit!(StagePart::Remap(rgba_img(&px), px, (*palette).clone()));
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
                    regions_handle(&regs, remap.dimensions()),
                    regs.len(),
                ));
            } else {
                emit!(StagePart::Unchanged(Stage::Regions));
            }

            // Merge plan feeds the fates, the contours, and the trace.
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
                let tint = fate_tint_handle(&regs, remap.dimensions(), &report.fates);
                emit!(StagePart::Fates(tint, report));
            }

            // One shape build serves the contour view and the trace.
            let shapes_key = ShapesInputs {
                plan: plan.clone(),
                prep: prep.clone(),
                params: pipeline::ShapeParams::of(&cfg),
            };
            let shapes = slots.shapes.get_or(shapes_key, (), compute_shapes);

            // Contours.
            let contours_key = ContoursInputs {
                shapes: shapes.clone(),
                params: crate::trace::TraceParams::of(&cfg),
            };
            let contours = slots
                .contours
                .get_or(contours_key.clone(), (w, h), compute_contours);
            if stale(
                shown.as_ref().and_then(|s| s.contours.as_ref()),
                &contours_key,
            ) {
                emit!(StagePart::Contours(contours));
            } else {
                emit!(StagePart::Unchanged(Stage::Contours));
            }

            let pad = cfg.stroke_width * cfg.scale as f32 / 2.0;
            let stroke = output::stroke_of(&cfg);
            let render_paths = |colors: &LayerTrace| -> (Option<Img>, usize) {
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
                    &[output::SvgLayer {
                        name: "layer",
                        stroke: stroke.as_ref(),
                        colors,
                    }],
                );
                (render_svg(&svg, w * 2, h * 2), anchors)
            };

            // Fit: the pre-simplify trace, keyed on every input its geometry
            // reads so the full render shares it.
            let fit_key = FitInputs::of(&cfg, &pins);
            let fit = slots
                .fit
                .get_or(fit_key.clone(), (shapes.arc(), shape_cache), compute_fit);
            let fit_stale =
                !stroke_same || stale(shown.as_ref().and_then(|s| s.fit.as_ref()), &fit_key);
            if fit_stale {
                let (im, an) = render_paths(&fit);
                emit!(StagePart::Fit(im, an));
            } else {
                emit!(StagePart::Unchanged(Stage::Fit));
            }

            // Simplify: the final trace (the fit trace when simplify is off).
            let simp_key = SimplifyInputs::of(&cfg, &pins);
            let simpl = slots
                .simplify
                .get_or(simp_key.clone(), fit, compute_simplify);
            let simp_stale =
                !stroke_same || stale(shown.as_ref().and_then(|s| s.simplify.as_ref()), &simp_key);
            if simp_stale {
                let (im, an) = render_paths(&simpl);
                emit!(StagePart::Simplify(im, an));
            } else {
                emit!(StagePart::Unchanged(Stage::Simplify));
            }

            let now_shown = Shown {
                layer,
                cfg,
                pins,
                stroke_bits,
                stroke_color,
                prep: Some(prep_key),
                remap: Some(remap_key),
                regions: Some(regions_key),
                plan: Some(plan_key),
                contours: Some(contours_key),
                fit: Some(fit_key),
                simplify: Some(simp_key),
            };
            emit!(StagePart::Done(Box::new(slots), Box::new(now_shown)));
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::super::artifact::{write_raster, Artifact};
    use super::*;
    use crate::palette::DetectParams;
    use crate::raster::{PrepParams, Prepared};
    use crate::regions::{PlanParams, Region, SegmentParams};
    use crate::trace::TraceParams;
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

    fn rgb(px: [u8; 3]) -> Artifact<RgbImage> {
        let mut img = RgbImage::new(1, 1);

        img.put_pixel(0, 0, image::Rgb(px));

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
        let a = rgb([0, 0, 0]);
        let b = rgb([0, 0, 0]);
        let c = rgb([9, 9, 9]);
        assert!(!Arc::ptr_eq(&a.arc(), &b.arc()), "distinct allocations");
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

    // A pin edit changes the plan key (and thus fates and trace) but leaves the
    // segmentation keys untouched.
    #[test]
    fn a_pin_edit_moves_the_plan_and_trace_keys_only() {
        let prep = dummy_prep();
        let remap = rgb([0, 0, 0]);
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

        let cfg = Config::default();
        let fit0 = FitInputs::of(&cfg, &[]);
        let fit1 = FitInputs::of(&cfg, &[[3, 4]]);
        assert_ne!(fit0, fit1, "the trace folds the pins");
    }

    // A pin-only edit leaves every key up to the segmentation raster equal.
    // Only the plan key (fates) and the trace keys move.
    #[test]
    fn a_pin_edit_dirties_only_the_plan_and_the_trace() {
        let cfg = Config::default();
        let prep = {
            let p = crate::raster::prepare(&RgbaImage::new(2, 2), &PrepParams::of(&cfg));
            Artifact::new_with(Arc::new(p), |_, _| {})
        };
        let remap = Artifact::new_with(Arc::new(RgbImage::new(1, 1)), |q, h| write_raster(h, q));
        let regs = Artifact::new(Arc::new(Vec::<Region>::new()));
        let regions = RegionsInputs {
            remap,
            prep: prep.clone(),
            params: SegmentParams::of(&cfg),
        };
        let plan = |pins: Vec<[u32; 2]>| PlanInputs {
            regs: regs.clone(),
            prep: prep.clone(),
            pins,
            params: PlanParams::of(&cfg),
        };
        // Segmentation is pin-independent; the plan and the trace fold pins.
        assert_eq!(regions, regions.clone());
        assert_ne!(plan(vec![]), plan(vec![[3, 4]]));
        assert_ne!(FitInputs::of(&cfg, &[]), FitInputs::of(&cfg, &[[3, 4]]));
        assert_ne!(
            SimplifyInputs::of(&cfg, &[]),
            SimplifyInputs::of(&cfg, &[[3, 4]])
        );
    }

    // A config edit ripples exactly as far as its params reach: simplify only
    // to the final trace, detail to the plan (the speckle floor) and the
    // trace, scale to segmentation, alphamax to the contours and the trace,
    // alpha_threshold to detection.
    #[test]
    fn config_edits_ripple_by_field() {
        let base = Config::default();
        let simp = Config {
            simplify: 5.0,
            ..base.clone()
        };
        assert_eq!(
            FitInputs::of(&base, &[]),
            FitInputs::of(&simp, &[]),
            "fit is simplify-independent"
        );
        assert_ne!(
            SimplifyInputs::of(&base, &[]),
            SimplifyInputs::of(&simp, &[]),
            "simplify moves the final trace"
        );

        // detail drives the speckle floor: the plan params move, the
        // segmentation params do not, and the trace inherits the ripple.
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
        assert_ne!(
            FitInputs::of(&base, &[]),
            FitInputs::of(&detail, &[]),
            "detail reaches the trace"
        );

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

        // alphamax is the corner threshold: the contour walk and the trace
        // move, segmentation and the plan do not.
        let alpha_max = Config {
            alphamax: 1.3,
            ..base.clone()
        };
        assert_ne!(
            TraceParams::of(&base),
            TraceParams::of(&alpha_max),
            "alphamax moves the contours"
        );
        assert_eq!(SegmentParams::of(&base), SegmentParams::of(&alpha_max));
        assert_eq!(PlanParams::of(&base), PlanParams::of(&alpha_max));
        assert_ne!(
            FitInputs::of(&base, &[]),
            FitInputs::of(&alpha_max, &[]),
            "alphamax reaches the trace"
        );

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
}
