//! Rim-residue fold: removes the color chips a limb cut-out leaves embedded
//! in the outside edge of its ink outline.

use super::common::{absorb, boundary_edge_counts, is_ink, UnionFind};
use super::{Feature, FeatureId, Partition};
use std::collections::HashMap;

/// Mean-thickness ceiling (source px) for a cut chip. Measured residue on
/// the corpus tops out at 1.7; authored shared-color regions and joint
/// shading start at 2.9.
const RESIDUE_THICK: f32 = 2.0;

/// Area ceiling (source px). Measured residue tops out at 44 px; the
/// smallest legitimate rim region of a foreign color measured 300+.
const RESIDUE_MAX_AREA: u32 = 100;

/// Max fraction of the layer's opaque area a chip may hold. Residue is edge
/// contamination, marginal by definition; a small overlay layer (an eyelid
/// on a rig-complete head) legitimately shares most of its area with the
/// paint beneath it, and its dominant features must never read as residue.
const RESIDUE_LAYER_SHARE: f32 = 0.25;

/// Min share of a chip's interior contact (non-background boundary) that
/// must be ink. Embedded-in-the-outline is the discriminating structure: cut
/// residue clings to the silhouette outline with background on the far side,
/// while authored rim detail (mane wisps, iris glints) sits on its FILL, and
/// art outlined in a chromatic color has no ink to embed into at all.
const RESIDUE_INK_SHARE: f32 = 0.6;

impl Partition {
    /// Folds cut-edge residue into the ink outline it is embedded in: a
    /// feature touching the alpha silhouette that is thin, small, marginal
    /// to its layer, not ink itself, and whose interior contact is mostly
    /// ink is a chip another body part left behind when this layer was cut
    /// out of a shared source. It folds INTO the outline, thickening it
    /// imperceptibly, never notching it with a fill color. Authored rim
    /// detail does not match: it sits on its fill, not embedded in the
    /// outline, and art outlined in a chromatic color (never ink) is
    /// untouched entirely. Folding relabels the chip's pixels; the
    /// silhouette never moves.
    pub fn fold_rim_residue(&mut self) {
        let n = self.features.len();
        if n == 0 {
            return;
        }
        let labels = &self.labels;
        let features = &self.features;
        let (w, h) = (labels.w as usize, labels.h as usize);

        // Per feature: total interior contact, ink share of it, and the
        // dominant ink neighbor (most shared boundary), the fold target.
        let mut contact = vec![0u64; n];
        let mut ink_contact = vec![0u64; n];
        let mut ink_dom: Vec<(FeatureId, u32)> = vec![(FeatureId::NONE, 0); n];
        for ((a, b), c) in boundary_edge_counts(labels) {
            contact[a.ix()] += c as u64;
            contact[b.ix()] += c as u64;
            if is_ink(features[b.ix()].mean) {
                ink_contact[a.ix()] += c as u64;
                if c > ink_dom[a.ix()].1 || (c == ink_dom[a.ix()].1 && b < ink_dom[a.ix()].0) {
                    ink_dom[a.ix()] = (b, c);
                }
            }
            if is_ink(features[a.ix()].mean) {
                ink_contact[b.ix()] += c as u64;
                if c > ink_dom[b.ix()].1 || (c == ink_dom[b.ix()].1 && a < ink_dom[b.ix()].0) {
                    ink_dom[b.ix()] = (a, c);
                }
            }
        }
        let mut per = vec![0u32; n];
        let mut rim = vec![false; n];
        for y in 0..h {
            for x in 0..w {
                let a = labels.at[y * w + x];
                if a == FeatureId::NONE {
                    continue;
                }
                let mut face = |nb: Option<FeatureId>| {
                    if nb != Some(a) {
                        per[a.ix()] += 1;
                    }
                    if nb.is_none() || nb == Some(FeatureId::NONE) {
                        rim[a.ix()] = true;
                    }
                };
                face(if x > 0 { Some(labels.at[y * w + x - 1]) } else { None });
                face(if x + 1 < w { Some(labels.at[y * w + x + 1]) } else { None });
                face(if y > 0 { Some(labels.at[(y - 1) * w + x]) } else { None });
                face(if y + 1 < h { Some(labels.at[(y + 1) * w + x]) } else { None });
            }
        }

        let opaque: u64 = features.iter().map(|f| f.area as u64).sum();
        let fold: Vec<bool> = (0..n)
            .map(|i| {
                let f = &features[i];
                let thickness = f.area as f32 / (0.5 * per[i].max(1) as f32);
                rim[i]
                    && f.area <= RESIDUE_MAX_AREA
                    && (f.area as f32) <= RESIDUE_LAYER_SHARE * opaque as f32
                    && thickness <= RESIDUE_THICK
                    && !is_ink(f.mean)
                    && ink_dom[i].0 != FeatureId::NONE
                    && ink_contact[i] as f32 >= RESIDUE_INK_SHARE * contact[i] as f32
            })
            .collect();
        if !fold.iter().any(|&f| f) {
            return;
        }

        // Collapse each folded chip into its dominant ink neighbor, exactly
        // as the cleanup collapse does: union toward the target, one kept
        // representative per component.
        let mut uf = UnionFind::new(n);
        for i in 0..n {
            if fold[i] {
                uf.union(i as u32, ink_dom[i].0 .0);
            }
        }
        let mut region_at: HashMap<u32, usize> = HashMap::new();
        let mut regions: Vec<Vec<u32>> = Vec::new();
        for i in 0..n as u32 {
            let root = uf.find(i);
            let idx = *region_at.entry(root).or_insert_with(|| {
                regions.push(Vec::new());
                regions.len() - 1
            });
            regions[idx].push(i);
        }
        let mut remap = vec![FeatureId::NONE; n];
        let mut out: Vec<Feature> = Vec::new();
        for members in &regions {
            let rep = *members
                .iter()
                .max_by(|&&a, &&b| {
                    let ka = (!fold[a as usize], features[a as usize].area);
                    let kb = (!fold[b as usize], features[b as usize].area);
                    ka.cmp(&kb)
                })
                .unwrap();
            let slot = FeatureId(out.len() as u32);
            out.push(features[rep as usize].clone());
            for &m in members {
                remap[m as usize] = slot;
                if m != rep {
                    absorb(&mut out[slot.ix()], &features[m as usize]);
                }
            }
        }
        self.apply(out, &remap);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Srgb;
    use crate::palette::FeatureLabels;

    // 20-wide raster: a 1px chip strip at y=0 x=8..13 (touching the image
    // border, so on the rim), an outline band over y=0..3 elsewhere, and a
    // fill below. Feature 0 is the outline, 1 the chip, 2 the fill.
    fn partition(outline: Srgb, chip: Srgb) -> Partition {
        let (w, h) = (20u32, 12u32);
        let of = |x: u32, y: u32| {
            if y == 0 && (8..13).contains(&x) {
                1
            } else if y < 3 {
                0
            } else {
                2
            }
        };
        let at = (0..w * h).map(|i| FeatureId(of(i % w, i / w))).collect();
        Partition {
            features: vec![
                Feature { mean: outline, area: 55, bbox: (0, 0, 19, 2) },
                Feature { mean: chip, area: 5, bbox: (8, 0, 12, 0) },
                Feature { mean: Srgb([200, 200, 200]), area: 180, bbox: (0, 3, 19, 11) },
            ],
            labels: FeatureLabels { w, h, at },
        }
    }

    #[test]
    fn fold_rim_residue_folds_a_chip_embedded_in_the_ink_outline() {
        // Fur-grey chip on the outer edge of a black outline: background
        // above, ink everywhere else. The cut-residue signature.
        let mut part = partition(Srgb([0, 0, 0]), Srgb([57, 57, 57]));
        part.fold_rim_residue();
        assert_eq!(part.features.len(), 2, "the chip must fold");
        // Its pixels land in the outline, not the fill.
        assert_eq!(part.labels.at[8], part.labels.at[0]);
    }

    #[test]
    fn fold_rim_residue_spares_a_chip_on_a_chromatic_outline() {
        // The same chip embedded in a BROWN outline: brown is authored
        // linework, not ink, so there is nothing to embed into and the chip
        // is left for the artist to judge.
        let mut part = partition(Srgb([46, 17, 16]), Srgb([120, 120, 120]));
        part.fold_rim_residue();
        assert_eq!(part.features.len(), 3, "no ink, no fold");
    }

    #[test]
    fn fold_rim_residue_spares_a_wisp_sitting_on_its_fill() {
        // A thin rim wisp whose interior contact is its own fill (a mane
        // strand tip): no ink contact, so it never reads as residue.
        let (w, h) = (20u32, 12u32);
        let of = |x: u32, y: u32| u32::from(y == 0 && (8..13).contains(&x));
        let at = (0..w * h).map(|i| FeatureId(of(i % w, i / w))).collect();
        let mut part = Partition {
            features: vec![
                Feature { mean: Srgb([180, 180, 180]), area: 235, bbox: (0, 0, 19, 11) },
                Feature { mean: Srgb([230, 230, 230]), area: 5, bbox: (8, 0, 12, 0) },
            ],
            labels: FeatureLabels { w, h, at },
        };
        part.fold_rim_residue();
        assert_eq!(part.features.len(), 2, "the wisp must survive");
    }
}
