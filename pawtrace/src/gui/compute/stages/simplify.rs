//! Simplify stage: the final trace (the fit trace when simplify is off).

use super::super::layer_bboxes;
use super::{FitInputs, TraceOutput};
use crate::pipeline::{self, SimplifyParams};
use std::sync::Arc;

/// Final trace inputs: the pre-simplify inputs plus the simplify params. When
/// `simplify <= 0` the simplify pass is a no-op and this trace is the fit
/// trace.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gui) struct SimplifyInputs {
    pub fit: FitInputs,
    pub params: SimplifyParams,
}

pub(super) fn compute_simplify(k: &SimplifyInputs, fit: TraceOutput) -> TraceOutput {
    // Simplify off is a no-op, so the fit trace is the final trace: keep its
    // Arc so the full render and downstream pointer-match it.
    if k.params.simplify <= 0.0 {
        return fit;
    }

    let (trace, seams) = pipeline::simplify_paths((*fit.trace).clone(), &fit.seams, &k.params);
    let bboxes = layer_bboxes(&trace);

    TraceOutput {
        trace: Arc::new(trace),
        bboxes: Arc::new(bboxes),
        // Simplify remaps the sidecar onto the post-simplify anchors, so the
        // seams overlay reads it off this stage's output too.
        seams: Arc::new(seams),
        scale: fit.scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::compute::artifact::Artifact;
    use crate::color::Srgb;
    use crate::config::Config;
    use crate::pipeline::{group_traced, Shape};
    use crate::seams::{self, StitchParams};
    use crate::trace::{self, ContourParams, FitParams};
    use image::{GrayImage, Luma};

    /// A shape whose art covers the scaled pixels `[x, x+w) x [y, y+h)`: bbox
    /// origin `(x, y)`, mask with the pipeline's 1px border.
    fn rect_shape(color: Srgb, (x, y): (u32, u32), w: u32, h: u32) -> Shape {
        let mut mask = GrayImage::new(w + 2, h + 2);
        for my in 1..=h {
            for mx in 1..=w {
                mask.put_pixel(mx, my, Luma([255]));
            }
        }
        (color, mask, None, (x, y))
    }

    // With simplify on, the shared boundary two abutting siblings splice into
    // both paths survives into the stage output, remapped onto the
    // post-simplify anchors, so the seams overlay finds a non-empty sidecar on
    // the Simplify view.
    #[test]
    fn simplify_output_carries_the_seam_sidecar() {
        let shapes = vec![
            rect_shape(Srgb([200, 30, 30]), (1, 1), 8, 6),
            rect_shape(Srgb([30, 30, 200]), (9, 1), 8, 6),
        ];
        let cfg = Config {
            scale: 1,
            simplify: 5.0,
            ..Default::default()
        };
        let cp = ContourParams::of(&cfg);
        let fp = FitParams::of(&cfg);
        let sp = StitchParams::of(&cfg);

        let traced = seams::stitched_contours(&shapes, &cp, &sp)
            .into_iter()
            .zip(&shapes)
            .map(|((contours, (tx, ty)), (color, ..))| {
                let mut paths = trace::fit_contours(&contours, &fp);
                for (p, _) in &mut paths {
                    p.translate(tx, ty);
                }
                (*color, paths)
            })
            .collect();
        let (trace, seams) = group_traced(traced);
        assert!(
            seams.iter().flatten().any(|s| !s.is_empty()),
            "the siblings stitch a shared span before simplify"
        );

        let bboxes = layer_bboxes(&trace);
        let fit = TraceOutput {
            trace: Arc::new(trace),
            bboxes: Arc::new(bboxes),
            seams: Arc::new(seams),
            scale: cfg.scale,
        };

        // compute_simplify keys on FitInputs but reads only params and the fit
        // output, so a no-op content feed on the shapes artifact is enough here.
        let k = SimplifyInputs {
            fit: FitInputs {
                shapes: Artifact::new_with(Arc::new(shapes), |_, _| {}),
                contour: cp,
                fit: fp,
                stitch: sp,
            },
            params: SimplifyParams::of(&cfg),
        };

        let out = compute_simplify(&k, fit);
        assert!(
            out.seams.iter().flatten().any(|s| !s.is_empty()),
            "the simplify stage output keeps a non-empty seam sidecar"
        );
    }
}
