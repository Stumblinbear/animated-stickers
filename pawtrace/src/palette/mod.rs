//! Region-first palette selection (NOT k-means, NOT histogram). Flat sticker
//! art authors color as spatial features: fills, stripes, highlights. Feature
//! detection over the 1x source crop finds those directly; a color-space-only
//! method cannot separate a deliberate low-contrast feature from anti-alias
//! fringe, since both live at the same OKLab distance. Selection is greedy by
//! feature salience with an OKLab dedup floor.

mod cleanup;
mod common;
mod detect;
mod merge;
mod remap;
mod select;

pub use remap::{label_smooth, remap_constrained, RemapPlan};
pub use select::{group_features, select_features, FeatureGroup};

use crate::config::Config;
use image::RgbaImage;

/// One color-uniform connected component of the 1x source crop, the evidence
/// unit of region-first palette selection.
#[derive(Debug, Clone)]
pub struct Feature {
    /// Mean member color, sRGB.
    pub mean: [u8; 3],
    /// Member pixel count, source px.
    pub area: u32,
    /// Bbox in source-crop px, inclusive: (x0, y0, x1, y1).
    pub bbox: (u32, u32, u32, u32),
}

/// Index of a [`Feature`] in its [`Partition`]'s feature list, or
/// [`FeatureId::NONE`] where no feature owns the pixel.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
#[repr(transparent)]
pub struct FeatureId(pub u32);

impl FeatureId {
    /// Background: below the alpha threshold, owned by no feature.
    pub const NONE: FeatureId = FeatureId(u32::MAX);

    /// The id as an index into the partition's feature list.
    pub fn ix(self) -> usize {
        self.0 as usize
    }
}

/// Feature id per pixel of the 1x source crop, [`FeatureId::NONE`] where the
/// pixel is below the alpha threshold. Indexes into the owning
/// [`Partition`]'s feature list, so callers can read which feature owns a
/// pixel and which features share a boundary.
#[derive(Debug, Clone)]
pub struct FeatureLabels {
    pub w: u32,
    pub h: u32,
    pub at: Vec<FeatureId>,
}

/// A feature segmentation of the 1x source crop: the feature records and the
/// label raster pinning each opaque pixel to its owner, moving together
/// through the palette stages. [`Partition::detect`] builds the fine
/// segmentation; [`Partition::merge_shades`] and [`Partition::fold_residue`]
/// consolidate it in place; [`Partition::plan`] derives the palette and the
/// raster the constrained remap consumes.
#[derive(Debug, Clone)]
pub struct Partition {
    pub features: Vec<Feature>,
    pub labels: FeatureLabels,
}

impl Partition {
    /// Color-uniform connected components (4-connectivity) over the opaque
    /// pixels of the 1x source crop `src`, grown under a fine tolerance.
    /// Components come out in first-encounter scan order. Deliberately
    /// over-segmented: compression and anti-alias fringe spawns fragment
    /// features freely, and [`Partition::merge_shades`] plus
    /// [`Partition::fold_residue`] consolidate them.
    pub fn detect(src: &RgbaImage, cfg: &Config) -> Partition {
        detect::grow_features(src, cfg)
    }

    /// The merged partition palette selection runs on: fine detection,
    /// cliff-bounded consolidation at `cfg.shade_split`, then indistinct
    /// cleanup. Visualization and digest harnesses build the same partition,
    /// so their downstream stages match the palette the pipeline builds.
    pub fn build(src: &RgbaImage, cfg: &Config) -> Partition {
        let mut part = Partition::detect(src, cfg);
        part.merge_shades(cfg);
        part.fold_residue();
        part
    }

    // Rewrites the partition onto a consolidated feature set in place:
    // remap[old] is the slot in `features` that old feature's pixels now
    // belong to. The single point where features and labels move together.
    fn apply(&mut self, features: Vec<Feature>, remap: &[FeatureId]) {
        for l in &mut self.labels.at {
            if *l != FeatureId::NONE {
                *l = remap[l.ix()];
            }
        }
        self.features = features;
    }
}
