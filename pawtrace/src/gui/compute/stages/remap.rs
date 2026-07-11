//! Palette-remap stage: consolidate the detected partition and remap the
//! flattened raster onto the selected palette.

use crate::color::Srgb;
use super::super::artifact::{write_raster, Artifact};
use crate::palette::{self, MergeParams, Partition, RemapParams, SelectParams};
use crate::raster::Prepared;
use image::RgbImage;
use std::sync::Arc;

/// Palette-remap inputs: the flattened raster, the detected partition, the
/// consolidation and selection params, and the remap params (`scale` and
/// `color_cleanup`) the remap passes to the constrained remap and the label
/// smooth.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gui) struct RemapInputs {
    pub prep: Artifact<Prepared>,
    pub detect: Artifact<Partition>,
    pub merge: MergeParams,
    pub select: SelectParams,
    pub remap: RemapParams,
}

/// The remapped raster and its palette, produced together by the remap stage.
pub(in crate::gui) type RemapOutput = (Artifact<RgbImage>, Arc<Vec<Srgb>>);

pub(super) fn compute_remap(k: &RemapInputs, _ctx: ()) -> RemapOutput {
    // A uniform layer's palette is its single color, and the remap is a copy of
    // the mask painted in it. The regions stage segments straight from the mask
    // here, so this raster only backs the Remap preview.
    if let Some(color) = k.prep.uniform {
        let (w, h) = k.prep.alpha.dimensions();
        let mut solid = RgbImage::new(w, h);
        {
            let sr: &mut [u8] = &mut solid;

            for (i, &a) in k.prep.alpha.as_raw().iter().enumerate() {
                if a != 0 {
                    sr[3 * i..3 * i + 3].copy_from_slice(&color.0);
                }
            }
        }
        let remap = Artifact::new_with(Arc::new(solid), |q, h| write_raster(h, q));

        return (remap, Arc::new(vec![color]));
    }

    let mut part = (*k.detect).clone();
    {
        part.merge_shades(&k.merge);
        part.fold_residue();
        part.fold_rim_residue();
    }
    let plan = part.plan(&k.select);

    let mut remapped =
        palette::remap_constrained(&k.prep.flat, &k.prep.alpha, &plan, k.remap.scale);

    if k.remap.color_cleanup > 0 {
        remapped = palette::label_smooth(&remapped, &k.prep.alpha, k.remap.color_cleanup);
    }

    let remap = Artifact::new_with(Arc::new(remapped), |q, h| write_raster(h, q));

    (remap, Arc::new(plan.palette))
}
