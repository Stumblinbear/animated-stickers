//! Feature-detection stage: partition the source raster into palette bands.

use super::super::artifact::Artifact;
use crate::palette::{self, DetectParams};
use image::RgbaImage;
use std::sync::Arc;

/// Feature-detection inputs: the source raster is fixed per layer, so the
/// params are the whole key.
pub(in crate::gui) type DetectInputs = DetectParams;

pub(super) fn compute_detect(k: &DetectInputs, img: &RgbaImage) -> Artifact<palette::Partition> {
    let part = palette::Partition::detect(img, k);
    Artifact::new(Arc::new(part))
}
