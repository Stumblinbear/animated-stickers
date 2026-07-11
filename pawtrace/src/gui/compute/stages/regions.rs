//! Region-segmentation stage: label the remapped raster into flood-fill regions.

use super::super::artifact::Artifact;
use super::super::Img;
use crate::raster::Prepared;
use crate::regions::{self, Region, SegmentParams};
use iced::widget::image as iced_image;
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

/// Regions view: each region painted in its own quantized color, so the image
/// reads as the art. The trace-fate tint ([`super::plan::fate_tint_handle`])
/// and the pin markers are drawn as overlays over this, not baked in, so a pin
/// edit re-tints without rebuilding this raster.
pub(super) fn regions_handle(regs: &[Region], (w, h): (u32, u32)) -> Img {
    let mut bytes = vec![0u8; (w * h * 4) as usize];

    for r in regs {
        for &(px, py) in &r.pixels {
            let idx = (((r.y0 + py) * w + (r.x0 + px)) * 4) as usize;

            bytes[idx..idx + 3].copy_from_slice(&r.color.0);
            bytes[idx + 3] = 255;
        }
    }

    Img {
        handle: iced_image::Handle::from_rgba(w, h, bytes),
        size: (w, h),
    }
}

pub(super) fn compute_regions(k: &RegionsInputs, _ctx: ()) -> Artifact<Vec<Region>> {
    // A uniform layer's mask already determines its regions: its components are
    // the regions, with no transition bands to absorb. Segmenting from the mask
    // matches the monolithic run's fast path for solid layers.
    let regs = match k.prep.uniform {
        Some(color) => regions::from_mask(&k.prep.alpha, color),
        None => regions::segment_absorbed(&k.remap, &k.prep.alpha, &k.params),
    };

    Artifact::new(Arc::new(regs))
}
