//! Merge-plan stage: fold speckle regions and honor the layer pins.

use super::super::artifact::{write_raster, Artifact};
use super::PlanCtx;
use crate::pipeline;
use crate::raster::Prepared;
use crate::regions::{self, MergePlan, PlanParams, Region};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Merge-plan inputs: the regions, the alpha mask (via `prep`), the layer
/// pins, and the speckle-floor params. The pins enter here, so a pin edit
/// invalidates the plan (and its fates and trace) but nothing earlier.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gui) struct PlanInputs {
    pub regs: Artifact<Vec<Region>>,
    pub prep: Artifact<Prepared>,
    pub pins: Vec<[u32; 2]>,
    pub params: PlanParams,
}

pub(super) fn compute_plan(k: &PlanInputs, ctx: PlanCtx) -> Artifact<MergePlan> {
    let scaled = pipeline::scale_pins(&k.pins, ctx.offset, k.params.scale, ctx.dims);
    let plan = regions::merge_plan(&k.regs, &k.prep.alpha, &k.params, ctx.doc_dim, &scaled);

    Artifact::new_with(Arc::new(plan), |plan, h| {
        // The masks are `GrayImage`, which has no `Hash`, so the plan can't
        // derive it: feed the derivable fields through `Hash` and the masks by
        // raster content, covering all seven fields.
        plan.floor.hash(h);
        plan.roots.hash(h);
        plan.regs.hash(h);
        plan.root_ids.hash(h);
        h.write_usize(plan.masks.len());
        for m in &plan.masks {
            write_raster(h, m);
        }
        plan.areas.hash(h);
        plan.survives.hash(h);
    })
}
