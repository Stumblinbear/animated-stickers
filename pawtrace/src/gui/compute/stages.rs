//! The stage-strip worker: given a snapshot of the memo, computes only the
//! stages missing from it and streams each display image the moment it is
//! ready. A stage whose value the snapshot already holds is not recomputed,
//! and its image is re-emitted only when it is stale (a different layer or a
//! changed input), so an unchanged stage never flickers.

use super::memo::StageKeys;
use super::render::{masked, region_fates_handle, render_debug, render_svg, rgba_img};
use super::{Img, LayerTrace, Shown, StagePart, STAGE_COUNT};
use crate::config::Config;
use crate::gui::ids::LayerId;
use crate::gui::msg::{ComputeMsg, Msg};
use crate::raster::Prepared;
use crate::regions::Region;
use crate::{output, palette, pipeline, regions};
use iced::Task;
use image::{RgbImage, RgbaImage};
use std::sync::Arc;

/// The memo entries a stage run may reuse, each `None` when absent or stale.
pub(super) struct Snapshot {
    pub prep: Option<Arc<Prepared>>,
    pub quant: Option<Arc<RgbImage>>,
    pub palette: Option<Arc<Vec<[u8; 3]>>>,
    pub regions: Option<Arc<Vec<Region>>>,
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
    let cur = |sel: fn(&StageKeys) -> u64| same_layer && shown.is_some_and(|s| sel(&s.keys) == sel(keys));
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
                doc, generation, img, offset, doc_dim, cfg, pending, shown, snap,
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
                emit!(StagePart::Flat(rgba_img(&masked(&prep.flat, &prep.alpha)), prep.clone()));
            }

            let (quant, palette, quant_c) = match (snap.quant, snap.palette) {
                (Some(q), Some(p)) => (q, p, false),
                _ => {
                    let pal = palette::extract_palette(&prep.flat, &prep.alpha, &cfg, doc_dim);
                    let mut q = palette::remap(&prep.flat, &prep.alpha, &pal);
                    if cfg.color_cleanup > 0 {
                        q = palette::label_smooth(&q, &prep.alpha, cfg.color_cleanup);
                    }
                    (Arc::new(q), Arc::new(pal), true)
                }
            };
            if pending[2] || quant_c {
                let px = masked(&quant, &prep.alpha);
                emit!(StagePart::Quant(rgba_img(&px), px, quant.clone(), palette.clone()));
            }

            let pins = pipeline::scale_pins(&cfg.pins, offset, cfg.scale, img.dimensions());
            let (regs, regs_c) = match snap.regions {
                Some(r) => (r, false),
                None => (Arc::new(regions::segment_absorbed(&quant, &prep.alpha, &cfg)), true),
            };
            if pending[3] || regs_c {
                let report = regions::region_report(&regs, &prep.alpha, &cfg, doc_dim, &pins);
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
                let anchors = colors.iter().flat_map(|(_, ps)| ps.iter()).map(|p| p.cubics.len()).sum();
                let svg = output::svg(
                    w,
                    h,
                    cfg.scale,
                    pad,
                    &[output::SvgLayer { name: "layer", stroke: stroke.as_ref(), colors }],
                );
                (render_svg(&svg, w * 2, h * 2), anchors)
            };

            let (smooth, smooth_c) = match snap.smooth {
                Some(s) => (s, false),
                None => {
                    let contours = pipeline::debug_contours(&regs, &prep.alpha, &cfg, doc_dim, &pins);
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
                    _ => (Arc::new(pipeline::trace_regions(&regs, &prep.alpha, &cfg, doc_dim, &pins)), true),
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
