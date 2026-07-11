//! Shape-build stage: turn the merge plan into per-color masks. The one shape
//! build feeds both the contour view and the trace.

use super::super::artifact::{write_raster, Artifact};
use crate::pipeline::{self, Shape, ShapeParams};
use crate::raster::Prepared;
use crate::regions::MergePlan;
use std::hash::Hasher;
use std::sync::Arc;

/// Shape-build inputs: the merge plan, the alpha mask (via `prep`), and the
/// shape params.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gui) struct ShapesInputs {
    pub plan: Artifact<MergePlan>,
    pub prep: Artifact<Prepared>,
    pub params: ShapeParams,
}

pub(super) fn compute_shapes(k: &ShapesInputs, _ctx: ()) -> Artifact<Vec<Shape>> {
    let shapes = pipeline::planned_shapes(&k.plan, &k.prep.alpha, &k.params);

    Artifact::new_with(Arc::new(shapes), |shapes, h| {
        // Shape is `(color, mask, seam-slack mask, bbox origin)`; the masks have
        // no `Hash`, so feed each of the four elements by hand.
        h.write_usize(shapes.len());

        for (color, mask, slack, (bx, by)) in shapes {
            h.write(color);
            write_raster(h, mask);
            match slack {
                Some(s) => {
                    h.write_u8(1);
                    write_raster(h, s);
                }
                None => h.write_u8(0),
            }
            h.write_u32(*bx);
            h.write_u32(*by);
        }
    })
}
