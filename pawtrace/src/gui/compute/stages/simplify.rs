//! Simplify stage: the final trace (the fit trace when simplify is off).

use super::super::LayerTrace;
use super::FitInputs;
use crate::config::Config;
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

impl SimplifyInputs {
    /// The final trace key for `cfg` and the layer's document-space `pins`.
    pub(in crate::gui) fn of(cfg: &Config, pins: &[[u32; 2]]) -> Self {
        SimplifyInputs {
            fit: FitInputs::of(cfg, pins),
            params: SimplifyParams::of(cfg),
        }
    }
}

pub(super) fn compute_simplify(k: &SimplifyInputs, fit: Arc<LayerTrace>) -> Arc<LayerTrace> {
    // Simplify off is a no-op, so the fit trace is the final trace: keep its
    // Arc so the full render and downstream pointer-match it.
    if k.params.simplify <= 0.0 {
        return fit;
    }

    Arc::new(pipeline::simplify_paths((*fit).clone(), &k.params))
}
