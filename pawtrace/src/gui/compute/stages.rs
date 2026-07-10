//! The stage-strip worker: given a snapshot of the memo, computes only the
//! stages missing from it and streams each display image the moment it is
//! ready. A stage whose value the snapshot already holds is not recomputed,
//! and its image is re-emitted only when it is stale (a different layer or a
//! changed input), so an unchanged stage never flickers.

use super::memo::{ShapeCache, StageKeys};
use super::render::{masked, region_fates_handle, render_debug, render_svg, rgba_img};
use super::{shape_memo, Img, LayerTrace, Shown, StagePart, STAGE_COUNT};
use crate::config::Config;
use crate::gui::ids::LayerId;
use crate::gui::msg::{ComputeMsg, Msg};
use crate::raster::Prepared;
use crate::regions::{MergePlan, Region};
use crate::{output, palette, pipeline, regions};
use iced::Task;
use image::{RgbImage, RgbaImage};
use std::sync::Arc;

/// The memo entries a stage run may reuse, each `None` when absent or stale.
pub(super) struct Snapshot {
    pub prep: Option<Arc<Prepared>>,
    pub detect: Option<Arc<palette::Partition>>,
    pub quant: Option<Arc<RgbImage>>,
    pub palette: Option<Arc<Vec<[u8; 3]>>>,
    pub regions: Option<Arc<Vec<Region>>>,
    pub plan: Option<Arc<MergePlan>>,
    pub smooth: Option<Option<Img>>,
    pub fit: Option<Arc<LayerTrace>>,
    pub simplify: Option<Arc<LayerTrace>>,
}

/// Everything one stage run needs, moved into the worker.
pub(super) struct StageJob {
    pub doc: usize,
    pub generation: u64,
    pub img: RgbaImage,
    pub offset: (u32, u32),
    pub doc_dim: u32,
    pub cfg: Config,
    pub pending: [bool; STAGE_COUNT],
    pub shown: Shown,
    pub snap: Snapshot,
    pub shape_cache: ShapeCache,
}

/// Which stage images are stale against the ones currently shown. Source
/// depends only on the layer; each later image on its stage's key, and the
/// fit/simplify renders additionally on the stroke they paint.
pub(super) fn pending(
    shown: Option<&Shown>,
    layer: LayerId,
    keys: &StageKeys,
    stroke_bits: u32,
    stroke_color: [u8; 3],
) -> [bool; STAGE_COUNT] {
    let same_layer = shown.is_some_and(|s| s.layer == layer);
    let cur =
        |sel: fn(&StageKeys) -> u64| same_layer && shown.is_some_and(|s| sel(&s.keys) == sel(keys));
    let stroke_same =
        shown.is_some_and(|s| s.stroke_bits == stroke_bits && s.stroke_color == stroke_color);
    [
        !same_layer,
        !cur(|k| k.prep),
        !cur(|k| k.quant),
        !cur(|k| k.regions_view),
        !cur(|k| k.fit),
        !(cur(|k| k.fit) && stroke_same),
        !(cur(|k| k.simplify) && stroke_same),
    ]
}

pub(super) fn stream(job: StageJob) -> Task<Msg> {
    Task::stream(iced::stream::channel(
        0,
        move |mut tx: iced::futures::channel::mpsc::Sender<Msg>| async move {
            use iced::futures::SinkExt;
            let StageJob {
                doc,
                generation,
                img,
                offset,
                doc_dim,
                cfg,
                pending,
                shown,
                snap,
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

            if pending[0] {
                emit!(StagePart::Source(rgba_img(&img)));
            }

            let (prep, prep_c) = match snap.prep {
                Some(p) => (p, false),
                None => (Arc::new(crate::raster::prepare(&img, &cfg)), true),
            };
            if pending[1] || prep_c {
                emit!(StagePart::Flat(
                    rgba_img(&masked(&prep.flat, &prep.alpha)),
                    prep.clone()
                ));
            }

            let (quant, palette, quant_c) = match (snap.quant, snap.palette) {
                (Some(q), Some(p)) => (q, p, false),
                _ => {
                    let detect = match snap.detect {
                        Some(d) => d,
                        None => {
                            let d = Arc::new(palette::Partition::detect(&img, &cfg));
                            emit!(StagePart::Detect(d.clone()));
                            d
                        }
                    };
                    let mut part = (*detect).clone();
                    part.merge_shades(&cfg);
                    part.fold_residue();
                    let plan = part.plan(&cfg);
                    let mut q = palette::remap_constrained(&prep.flat, &prep.alpha, &plan, cfg.scale);
                    if cfg.color_cleanup > 0 {
                        q = palette::label_smooth(&q, &prep.alpha, cfg.color_cleanup);
                    }
                    (Arc::new(q), Arc::new(plan.palette), true)
                }
            };
            if pending[2] || quant_c {
                let px = masked(&quant, &prep.alpha);
                emit!(StagePart::Quant(
                    rgba_img(&px),
                    px,
                    quant.clone(),
                    palette.clone()
                ));
            }

            let pins = pipeline::scale_pins(&cfg.pins, offset, cfg.scale, img.dimensions());
            let (regs, regs_c) = match snap.regions {
                Some(r) => (r, false),
                None => (
                    Arc::new(regions::segment_absorbed(&quant, &prep.alpha, &cfg)),
                    true,
                ),
            };
            // One merge plan feeds the report, the debug contours, and the
            // trace below; each used to re-run the speckle merge and shape
            // build for itself. Skipped when every consumer is cached.
            let fit_shortcut = snap.simplify.is_some() && cfg.simplify <= 0.0;
            let need_plan = pending[3]
                || regs_c
                || snap.smooth.is_none()
                || (snap.fit.is_none() && !fit_shortcut);
            let plan = match snap.plan {
                Some(p) => Some(p),
                None if need_plan => {
                    let p = Arc::new(regions::merge_plan(
                        &regs,
                        &prep.alpha,
                        &cfg,
                        doc_dim,
                        &pins,
                    ));
                    emit!(StagePart::Plan(p.clone()));
                    Some(p)
                }
                None => None,
            };
            if pending[3] || regs_c {
                let report = regions::report_of(plan.as_ref().unwrap());
                emit!(StagePart::Regions(
                    region_fates_handle(&regs, quant.dimensions(), &report.fates, &pins),
                    regs.len(),
                    report,
                    regs.clone(),
                ));
            }

            let (w, h) = img.dimensions();
            let stroke = output::stroke_of(&cfg);
            let pad = cfg.stroke_width * cfg.scale as f32 / 2.0;
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

            // One shape build serves the contour view and the trace: the
            // spanning-tree mask union is most of their cost.
            let need_shapes = snap.smooth.is_none() || (snap.fit.is_none() && !fit_shortcut);
            let shapes = need_shapes
                .then(|| pipeline::planned_shapes(plan.as_ref().unwrap(), &prep.alpha, &cfg));

            let (smooth, smooth_c) = match snap.smooth {
                Some(s) => (s, false),
                None => {
                    let contours = pipeline::debug_from_shapes(shapes.as_ref().unwrap(), &cfg);
                    (render_debug(&contours, w, h, cfg.scale), true)
                }
            };
            if pending[4] || smooth_c {
                emit!(StagePart::Smooth(smooth.clone()));
            }

            // With simplify off, the simplify pass is a no-op, so a cached
            // simplify trace is the fit trace and needs no re-tracing.
            let (fit, fit_c) = match snap.fit {
                Some(f) => (f, false),
                None => match &snap.simplify {
                    Some(s) if cfg.simplify <= 0.0 => (s.clone(), true),
                    _ => (
                        Arc::new(shape_memo::trace_shapes_memo(
                            &shape_cache,
                            shapes.as_ref().unwrap(),
                            &cfg,
                        )),
                        true,
                    ),
                },
            };
            if pending[5] || fit_c {
                let (im, an) = render_paths(&fit);
                emit!(StagePart::Fit(im, an, fit.clone()));
            }

            let simpl = match snap.simplify {
                Some(s) => s,
                None if cfg.simplify <= 0.0 => fit.clone(),
                None => Arc::new(pipeline::simplify_paths((*fit).clone(), &cfg)),
            };
            let (im, an) = render_paths(&simpl);
            emit!(StagePart::Simplify(im, an, simpl.clone(), shown));
        },
    ))
}
