//! Fit stage: the pre-simplify trace, keyed on every input its geometry reads
//! so the full-document render shares the same cache entry.

use super::super::cache::ShapeCache;
use super::super::{shape_memo, LayerTrace};
use crate::config::Config;
use crate::palette::{DetectParams, MergeParams, RemapParams, SelectParams};
use crate::pipeline::{Shape, ShapeParams};
use crate::raster::PrepParams;
use crate::regions::{PlanParams, SegmentParams};
use crate::trace::TraceParams;
use std::sync::Arc;

/// Pre-simplify trace inputs: the layer pins plus every upstream params struct
/// the trace geometry depends on. Excludes `simplify` (its own pass) and the
/// stroke (a render property).
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gui) struct FitInputs {
    pub pins: Vec<[u32; 2]>,
    pub prep: PrepParams,
    pub detect: DetectParams,
    pub merge: MergeParams,
    pub select: SelectParams,
    pub remap: RemapParams,
    pub segment: SegmentParams,
    pub plan: PlanParams,
    pub shape: ShapeParams,
    pub trace: TraceParams,
}

impl FitInputs {
    /// The pre-simplify trace key for `cfg` and the layer's document-space `pins`.
    pub(in crate::gui) fn of(cfg: &Config, pins: &[[u32; 2]]) -> Self {
        FitInputs {
            pins: pins.to_vec(),
            prep: PrepParams::of(cfg),
            detect: DetectParams::of(cfg),
            merge: MergeParams::of(cfg),
            select: SelectParams::of(cfg),
            remap: RemapParams::of(cfg),
            segment: SegmentParams::of(cfg),
            plan: PlanParams::of(cfg),
            shape: ShapeParams::of(cfg),
            trace: TraceParams::of(cfg),
        }
    }
}

pub(super) fn compute_fit(k: &FitInputs, ctx: (Arc<Vec<Shape>>, ShapeCache)) -> Arc<LayerTrace> {
    let (shapes, cache) = ctx;
    Arc::new(shape_memo::trace_shapes_memo(&cache, &shapes, &k.trace))
}
