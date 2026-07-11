//! Merge-plan stage: fold speckle regions and honor the layer pins.

use super::super::artifact::{write_raster, Artifact};
use super::super::Img;
use super::PlanCtx;
use crate::pipeline;
use crate::raster::Prepared;
use crate::regions::{self, MergePlan, PlanParams, Region};
use iced::widget::image as iced_image;
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

/// Trace-fate tint over the segmentation, for the fates overlay to composite on
/// the Regions view. Transparent everywhere except regions the trace will not
/// keep as their own shape: red marks a culled region (below the speckle floor,
/// no neighbor to merge into, unpinned; it vanishes silently), orange one the
/// speckle merge folds into a neighbor (it survives as pixels, losing its color
/// and path). Returns `None` when every region survives, so the overlay draws
/// nothing.
pub(super) fn fate_tint_handle(
    regs: &[Region],
    (w, h): (u32, u32),
    fates: &[regions::Fate],
) -> Option<Img> {
    // Straight alpha matching the baked 0.5 blend once composited over the
    // segmentation: 128/255 ≈ 0.5.
    const TINT_A: u8 = 128;

    let mut bytes = vec![0u8; (w * h * 4) as usize];

    let mut any = false;

    for (i, r) in regs.iter().enumerate() {
        let tint = match fates.get(i) {
            Some(regions::Fate::Culled) => [230, 55, 45],
            Some(regions::Fate::MergedInto(_)) => [240, 150, 40],
            _ => continue,
        };

        any = true;

        for &(px, py) in &r.pixels {
            let idx = (((r.y0 + py) * w + (r.x0 + px)) * 4) as usize;
            bytes[idx..idx + 3].copy_from_slice(&tint);
            bytes[idx + 3] = TINT_A;
        }
    }

    any.then(|| Img {
        handle: iced_image::Handle::from_rgba(w, h, bytes),
        size: (w, h),
    })
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
