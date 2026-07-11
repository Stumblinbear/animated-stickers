//! Constrained remap: quantizing the supersampled art to the palette while
//! restricting each pixel to the colors of the features under it, so
//! anti-alias blends never precipitate a third color along a seam.

use super::common::Lab;
use super::{group_features, select_features, FeatureId, Partition, SelectParams};
use image::{GrayImage, RgbImage};
use std::collections::HashMap;

/// A palette plus the per-source-pixel feature-color raster that
/// [`remap_constrained`] needs to keep anti-alias blends from precipitating a
/// third color along a feature seam.
pub struct RemapPlan {
    /// Selected palette colors, as [`select_features`], sRGB.
    pub palette: Vec<[u8; 3]>,

    w: u32,
    h: u32,

    // feat_color[y * w + x] indexes `palette` for the feature owning source
    // pixel (x, y), or u32::MAX outside the alpha. Every feature is pinned to
    // the palette color nearest its mean, which is the color its solid interior
    // remaps to, so an interior supersample pixel resolves the same whether or
    // not it sits near a seam.
    feat_color: Vec<u32>,
}

impl Partition {
    /// Builds the palette and the source-pixel feature-color raster for
    /// [`remap_constrained`] from this merged partition (its labels pin each
    /// 1x source pixel to a feature).
    pub fn plan(&self, cfg: &SelectParams) -> RemapPlan {
        self.plan_with(select_features(&group_features(&self.features), cfg))
    }

    /// [`Partition::plan`] with the palette supplied instead of selected, for
    /// harnesses and overrides that choose the colors themselves.
    pub fn plan_with(&self, palette: Vec<[u8; 3]>) -> RemapPlan {
        let pal_lab: Vec<Lab> = palette.iter().map(|&c| Lab::of(c)).collect();

        let feat_pal: Vec<u32> = self
            .features
            .iter()
            .map(|f| nearest_palette(Lab::of(f.mean), &pal_lab))
            .collect();

        let feat_color: Vec<u32> = self
            .labels
            .at
            .iter()
            .map(|&f| {
                if f == FeatureId::NONE {
                    u32::MAX
                } else {
                    feat_pal[f.ix()]
                }
            })
            .collect();

        RemapPlan {
            palette,
            w: self.labels.w,
            h: self.labels.h,
            feat_color,
        }
    }
}

/// Index of the palette color nearest `lab` in OKLab, or [`u32::MAX`] when
/// `pal_lab` is empty.
fn nearest_palette(lab: Lab, pal_lab: &[Lab]) -> u32 {
    pal_lab
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| lab.dist2(**a).partial_cmp(&lab.dist2(**b)).unwrap())
        .map(|(i, _)| i as u32)
        .unwrap_or(u32::MAX)
}

/// Remaps `flat` (supersampled art, `scale` pixels per source px) to `plan`'s
/// palette by nearest OKLab color, but restricts each supersampled pixel to the
/// palette colors of the features in its 1x source neighborhood. A blend pixel
/// on a seam between features A and B can then only resolve to A's or B's color,
/// never a third palette color it happens to sit nearest, so anti-alias
/// transition bands are never born. Pixels outside `alpha` keep their zero fill.
pub fn remap_constrained(
    flat: &RgbImage,
    alpha: &GrayImage,
    plan: &RemapPlan,
    scale: u32,
) -> RgbImage {
    let mut out = flat.clone();

    if plan.palette.is_empty() {
        return out;
    }

    let (w, h) = (plan.w as usize, plan.h as usize);
    let scale = scale.max(1) as usize;
    let pal_lab: Vec<Lab> = plan.palette.iter().map(|&c| Lab::of(c)).collect();

    // Per source pixel, the distinct palette colors across its 3x3
    // neighborhood. A blend pixel lies in the interior of a source pixel's
    // supersample block, so its neighborhood always spans both sides of the
    // seam it straddles. MULTI marks a seam pixel whose candidate list sits in
    // `multi`; EMPTY a pixel with no 1x feature under its neighborhood (the
    // supersample silhouette runs a touch wider than the thresholded source),
    // resolved to a single index otherwise.
    const MULTI: u32 = u32::MAX;
    const EMPTY: u32 = u32::MAX - 1;

    let mut resolved = vec![EMPTY; w * h];

    let mut multi: HashMap<usize, Vec<u32>> = HashMap::new();

    for y in 0..h {
        for x in 0..w {
            let mut cand: Vec<u32> = Vec::new();

            for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                    let f = plan.feat_color[ny * w + nx];

                    if f != u32::MAX && !cand.contains(&f) {
                        cand.push(f);
                    }
                }
            }

            let p = y * w + x;

            match cand.len() {
                0 => {}
                1 => resolved[p] = cand[0],
                _ => {
                    resolved[p] = MULTI;
                    multi.insert(p, cand);
                }
            }
        }
    }

    // Nearest palette color to `cl` among `cands` (indices into `pal_lab`).
    // `cands` is only ever `all` or a `multi` entry, both non-empty, so the
    // min is always present.
    let nearest = |cl: Lab, cands: &[u32]| -> [u8; 3] {
        let i = *cands
            .iter()
            .min_by(|&&a, &&b| {
                cl.dist2(pal_lab[a as usize])
                    .partial_cmp(&cl.dist2(pal_lab[b as usize]))
                    .unwrap()
            })
            .unwrap();

        plan.palette[i as usize]
    };

    let all: Vec<u32> = (0..plan.palette.len() as u32).collect();

    let sw = out.width() as usize;
    let amask = alpha.as_raw();

    for (i, p) in out.pixels_mut().enumerate() {
        if amask[i] == 0 {
            continue;
        }

        let sp = ((i / sw) / scale).min(h - 1) * w + ((i % sw) / scale).min(w - 1);

        p.0 = match resolved[sp] {
            EMPTY => nearest(Lab::of(p.0), &all),
            MULTI => nearest(Lab::of(p.0), &multi[&sp]),
            idx => plan.palette[idx as usize],
        };
    }

    out
}

/// Mode-filters the quantized labels so color boundaries settle where the
/// local majority sits. Nearest-color remap assigns the resize blend band
/// noisily when two palette colors are perceptually close (dark linework
/// against dark fur), pinching thin lines to nothing in places; majority
/// voting reclaims those pixels. Only art pixels vote: nothing outside the
/// alpha can outvote art, so the silhouette cannot erode.
pub fn label_smooth(quant: &RgbImage, alpha: &GrayImage, k: u32) -> RgbImage {
    crate::raster::majority_vote(quant, alpha, k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Luma, Rgb};

    fn plan(w: u32, h: u32, palette: Vec<[u8; 3]>, feat_color: Vec<u32>) -> RemapPlan {
        RemapPlan {
            palette,
            w,
            h,
            feat_color,
        }
    }

    #[test]
    fn constrained_remap_blocks_a_third_color_on_a_seam() {
        // Two source px: 0 -> feature A (red), 1 -> feature B (blue), with a
        // green third slot. A green blend over source 0 is nearest green
        // unconstrained; the seam neighborhood {A, B} must exclude it.
        let plan = plan(
            2,
            1,
            vec![[255, 0, 0], [0, 0, 255], [0, 255, 0]],
            vec![0, 1],
        );

        let mut flat = RgbImage::new(4, 1);

        flat.put_pixel(0, 0, Rgb([255, 0, 0]));
        flat.put_pixel(1, 0, Rgb([0, 200, 0]));
        flat.put_pixel(2, 0, Rgb([0, 0, 255]));
        flat.put_pixel(3, 0, Rgb([0, 0, 255]));

        let alpha = GrayImage::from_pixel(4, 1, Luma([255]));
        let out = remap_constrained(&flat, &alpha, &plan, 2);
        let seam = out.get_pixel(1, 0).0;

        assert_ne!(seam, [0, 255, 0]);
        assert!(seam == [255, 0, 0] || seam == [0, 0, 255]);
    }

    #[test]
    fn constrained_remap_pins_interior_to_its_feature_color() {
        // The middle pixel's 3x3 neighborhood is pure feature A, so a green
        // blend there resolves to A without consulting its own color.
        let plan = plan(
            3,
            1,
            vec![[255, 0, 0], [0, 0, 255], [0, 255, 0]],
            vec![0, 0, 0],
        );

        let mut flat = RgbImage::new(3, 1);

        flat.put_pixel(0, 0, Rgb([255, 0, 0]));
        flat.put_pixel(1, 0, Rgb([0, 200, 0]));
        flat.put_pixel(2, 0, Rgb([255, 0, 0]));

        let alpha = GrayImage::from_pixel(3, 1, Luma([255]));
        let out = remap_constrained(&flat, &alpha, &plan, 1);

        assert_eq!(out.get_pixel(1, 0).0, [255, 0, 0]);
    }

    #[test]
    fn constrained_remap_falls_back_to_full_palette_off_every_feature() {
        // No feature under the pixel (silhouette rim): the full palette is the
        // candidate set, so a green blend snaps to the green slot.
        let plan = plan(
            1,
            1,
            vec![[255, 0, 0], [0, 0, 255], [0, 255, 0]],
            vec![u32::MAX],
        );

        let mut flat = RgbImage::new(1, 1);
        flat.put_pixel(0, 0, Rgb([0, 200, 0]));
        let alpha = GrayImage::from_pixel(1, 1, Luma([255]));
        let out = remap_constrained(&flat, &alpha, &plan, 1);

        assert_eq!(out.get_pixel(0, 0).0, [0, 255, 0]);
    }
}
