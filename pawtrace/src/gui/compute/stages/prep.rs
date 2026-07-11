//! Flatten stage: supersample and flatten the source raster.

use super::super::artifact::{write_raster, Artifact};
use crate::raster::{PrepParams, Prepared};
use image::RgbaImage;
use std::sync::Arc;

/// Supersampled, flattened raster inputs.
pub(in crate::gui) type PrepInputs = PrepParams;

pub(super) fn compute_prep(k: &PrepInputs, img: &RgbaImage) -> Artifact<Prepared> {
    let prep = crate::raster::prepare(img, k);

    Artifact::new_with(Arc::new(prep), |p, h| {
        write_raster(h, &p.flat);
        write_raster(h, &p.alpha);
    })
}
