//! Region-segmentation stage: label the remapped raster into flood-fill regions.

use super::super::artifact::Artifact;
use crate::raster::Prepared;
use crate::regions::{self, Region, SegmentParams};
use image::RgbImage;
use std::sync::Arc;

/// Region-segmentation inputs: the remapped raster, the alpha mask (via
/// `prep`), and the segmentation params.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gui) struct RegionsInputs {
    pub remap: Artifact<RgbImage>,
    pub prep: Artifact<Prepared>,
    pub params: SegmentParams,
}

pub(super) fn compute_regions(k: &RegionsInputs, _ctx: ()) -> Artifact<Vec<Region>> {
    let regs = regions::segment_absorbed(&k.remap, &k.prep.alpha, &k.params);
    Artifact::new(Arc::new(regs))
}
