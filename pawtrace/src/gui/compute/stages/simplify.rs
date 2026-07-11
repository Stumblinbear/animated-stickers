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

    let trace = pipeline::simplify_paths((*fit.trace).clone(), &k.params);
    let bboxes = layer_bboxes(&trace);

    TraceOutput {
        trace: Arc::new(trace),
        bboxes: Arc::new(bboxes),
        scale: fit.scale,
    }
}
