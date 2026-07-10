//! Indistinct cleanup: absorbs the resample and anti-alias residue the cliff
//! merge leaves behind into an adjacent neighbor.

use super::common::{absorb, boundary_edge_counts, is_ink, Lab, UnionFind};
use super::{Feature, FeatureId, FeatureLabels, Partition};
use std::collections::HashMap;

/// Mean-thickness ceiling (source px) under which [`Partition::fold_residue`]
/// will consider a feature a blend ribbon. With contiguous features, every
/// true fringe measured on the corpus is at most ~1.7 thick, while authored
/// soft shading crescents start at ~2.5 (the ear-tip shadow measures 2.49);
/// the thicker values seen before contiguity were pooled features, not fringe.
const INDISTINCT_RIBBON: f32 = 2.0;

/// Max OKLab distance from the segment between its two dominant neighbors'
/// colors for a feature to count as their blend. Measured fringe sits at
/// 0.0000-0.0005; the nearest authored tone measured 0.0197 (the head fur
/// against the fill-blush segment), so 0.01 splits them with margin on both
/// sides.
const RIBBON_DEV: f32 = 0.01;

/// Min OKLab ΔE a blend ribbon must keep from each endpoint of its pair's
/// segment. A feature this close to an endpoint reads as a shade of that
/// neighbor, not a mixture: authored darker shading measured 0.064 from its
/// fill where true fringe sits 0.094+ from both sides.
const RIBBON_END_MARGIN: f32 = 0.075;

/// Area (source px) below which the endpoint shelter does not apply: a
/// near-endpoint SLIVER is that endpoint's own edge residue and folds into
/// it, where a substantial near-endpoint feature is authored shading and
/// keeps the shelter. The measured margin is tight and deliberate: the culled
/// class tops out at 14 px (a 1x14 seam strip) and the kept class starts at
/// 15 px (the smallest Bed crease).
const RIBBON_SHELTER_FLOOR: u32 = 15;

/// Compact-size floor: the bbox diagonal in source px below which a feature
/// is a speck and folds into its neighbor.
const INDISTINCT_SPECK: f32 = 6.0;

/// Near-duplicate color ceiling, OKLab ΔE. A feature within this of its
/// most-similar neighbor is an imperceptible color step; the closest
/// deliberate detail measured on the goldens (a fur-depth highlight at ΔE
/// ~0.037) stays well clear.
const INDISTINCT_COLOR_JND: f32 = 0.02;

impl Partition {
    /// Absorbs resample and anti-alias residue left by
    /// [`Partition::merge_shades`] into an adjacent neighbor. A feature is
    /// absorbed when ANY of three signals matches an artifact class
    /// (OR-combined):
    ///
    /// - Blend ribbon: mean thickness `area / (0.5 * perimeter)` at most
    ///   [`INDISTINCT_RIBBON`] (source px) AND its color reads as a mixture
    ///   of some pair of its three dominant boundary neighbors: within
    ///   [`RIBBON_DEV`] of the OKLab segment between that pair's means and at
    ///   least [`RIBBON_END_MARGIN`] from each endpoint. This is what
    ///   anti-aliasing physically is, so stroke-seam and fill-boundary
    ///   slivers match while an authored thin stroke does not: its color is
    ///   its own, not a mixture of what it borders (a dark crease on a sheet
    ///   sits far off the sheet-to-sheet segment, and a stroke darker than
    ///   everything it touches is outside every pair's segment), and authored
    ///   shading hugs one endpoint where a mixture sits mid-segment. The
    ///   endpoint shelter needs substance: below [`RIBBON_SHELTER_FLOOR`] a
    ///   near-endpoint on-segment sliver is edge residue and folds into the
    ///   endpoint it hugs.
    /// - Too small: bbox diagonal below [`INDISTINCT_SPECK`] (source px), a
    ///   compact speck.
    /// - Near-duplicate color: OKLab ΔE to its salient parent (below) is
    ///   under [`INDISTINCT_COLOR_JND`]. Deliberate detail stays clear: fur
    ///   stripes sit at ΔE ~0.06, the cheek fur-depth highlight at ~0.037.
    ///
    /// Separately, same ink is an equivalence: every pair of ADJACENT
    /// near-black features (dark and neutral, where the cube root makes
    /// imperceptible steps read as large ΔE) unions into one feature, so an
    /// outline network stays whole across its AA slivers.
    ///
    /// A feature absorbs into its salient parent: the most-similar-color
    /// adjacent neighbor of strictly greater area. Absorption therefore
    /// always climbs in area to a local maximum, which is kept, so a cluster
    /// of imperceptible features collapses into the fill it borders instead
    /// of pooling into a surviving feature of its own, and a ring of
    /// near-black fragments collapses into the near-black stroke it borders
    /// rather than the brighter fill. A feature with no larger neighbor is a
    /// local area-maximum and is always kept.
    pub fn fold_residue(&mut self) {
        if self.features.is_empty() {
            return;
        }
        let (out, remap) = fold(&self.features, &self.labels);
        self.apply(out, &remap);
    }
}

fn fold(features: &[Feature], labels: &FeatureLabels) -> (Vec<Feature>, Vec<FeatureId>) {
    let n = features.len();
    let lab: Vec<Lab> = features.iter().map(|f| Lab::of(f.mean)).collect();
    // Adjacency with shared-boundary lengths: the ribbon test needs each
    // feature's two DOMINANT neighbors (most shared edges), not just any two.
    let mut adjw: Vec<Vec<(FeatureId, u32)>> = vec![Vec::new(); n];
    for ((a, b), c) in boundary_edge_counts(labels) {
        adjw[a.ix()].push((b, c));
        adjw[b.ix()].push((a, c));
    }
    let per = feature_perimeters(labels, n);

    // nearest[i] is the salient parent feature i folds toward: its closest
    // neighbor in mean-color ΔE among neighbors of strictly greater area. Every
    // fold climbs in area, so a chain of imperceptible features always
    // terminates at a local area-maximum instead of pooling into a surviving
    // feature of its own.
    let mut keep = vec![false; n];
    let mut nearest = vec![FeatureId::NONE; n];
    let dark = is_ink;
    for i in 0..n {
        let (mut best, mut tgt) = (f32::MAX, FeatureId::NONE);
        for &(j, _) in &adjw[i] {
            // Strictly greater area, ties to the lower index: a total order that
            // keeps the fold relation acyclic and deterministic run to run
            // (adjacency comes from a hashed edge set with no inherent order).
            let larger = features[j.ix()].area > features[i].area
                || (features[j.ix()].area == features[i].area && j.ix() < i);
            if !larger {
                continue;
            }
            let d = lab[i].dist(lab[j.ix()]);
            if d < best || (d == best && j < tgt) {
                best = d;
                tgt = j;
            }
        }
        // A local area-maximum has no parent to fold into, so it survives
        // whatever its thinness or size. This is the anchor every fold chain
        // reaches.
        if tgt == FeatureId::NONE {
            keep[i] = true;
            continue;
        }
        nearest[i] = tgt;
        let (x0, y0, x1, y1) = features[i].bbox;
        let bw = (x1 - x0 + 1) as f32;
        let bh = (y1 - y0 + 1) as f32;
        let diag = (bw * bw + bh * bh).sqrt();
        let thickness = features[i].area as f32 / (0.5 * per[i].max(1) as f32);
        // Three dominant neighbors by shared boundary, ties to the lower index
        // for run-to-run determinism. Testing every pair among them matters
        // because connected fringe often spans several boundaries (the
        // silhouette ring runs outline-fill in places and fill-white in
        // others), so its pooled mean is the blend of its two EXTREME sides,
        // not of the two longest contacts.
        let mut dom = [(FeatureId::NONE, 0u32); 3];
        for &(j, c) in &adjw[i] {
            let mut cand = (j, c);
            for d in &mut dom {
                if cand.1 > d.1 || (cand.1 == d.1 && cand.0 < d.0) {
                    std::mem::swap(d, &mut cand);
                }
            }
        }
        let blend = dom[1].0 != FeatureId::NONE
            && (0..3).any(|a| {
                (a + 1..3).any(|b| {
                    dom[b].0 != FeatureId::NONE
                        && lab[i].blend_between(
                            lab[dom[a].0.ix()],
                            lab[dom[b].0.ix()],
                            RIBBON_END_MARGIN,
                            RIBBON_DEV,
                        )
                })
            });
        // A thin sliver hugging one endpoint of an on-segment pair is that
        // endpoint's own edge residue: the shelter that keeps substantial
        // near-endpoint shading does not apply, and the fold lands in the
        // hugged endpoint rather than the salient parent.
        let mut sliver_tgt = FeatureId::NONE;
        if features[i].area < RIBBON_SHELTER_FLOOR && dom[1].0 != FeatureId::NONE {
            'pairs: for a in 0..3 {
                for b in a + 1..3 {
                    if dom[b].0 == FeatureId::NONE {
                        continue;
                    }
                    let (ca, cb) = (lab[dom[a].0.ix()], lab[dom[b].0.ix()]);
                    if lab[i].seg_dev(ca, cb) < RIBBON_DEV {
                        let (da, db) = (lab[i].dist(ca), lab[i].dist(cb));
                        // The hugged endpoint must be strictly larger, keeping
                        // every fold area-climbing; two same-size slivers
                        // pointing at each other would found a feature of
                        // their own.
                        let near = if da <= db { dom[a].0 } else { dom[b].0 };
                        if (da <= RIBBON_END_MARGIN || db <= RIBBON_END_MARGIN)
                            && features[near.ix()].area > features[i].area
                        {
                            sliver_tgt = near;
                            break 'pairs;
                        }
                    }
                }
            }
        }
        let ribbon =
            thickness <= INDISTINCT_RIBBON && (blend || sliver_tgt != FeatureId::NONE);
        if ribbon && sliver_tgt != FeatureId::NONE {
            nearest[i] = sliver_tgt;
        }
        let small = diag < INDISTINCT_SPECK;
        // Measured against the salient parent only, so a large feature is never
        // ruled a duplicate of a tiny same-color speck that happens to touch it.
        let color_dup = best < INDISTINCT_COLOR_JND;
        keep[i] = !(ribbon || small || color_dup);
    }

    // Ink membership: a dark feature with a dark neighbor belongs to the ink
    // closure below, and must NOT star-fold toward a non-ink parent; one dark
    // speck folding into the fill would otherwise drag the whole ink network
    // in with it once the closure unions their roots.
    let inky: Vec<bool> = (0..n)
        .map(|i| {
            dark(features[i].mean) && adjw[i].iter().any(|&(j, _)| dark(features[j.ix()].mean))
        })
        .collect();

    // A failing feature unions toward its salient parent. The chain climbs in
    // area to a local maximum, which is always kept, so every component holds a
    // survivor.
    let mut uf = UnionFind::new(n);
    for i in 0..n {
        if !keep[i] && !inky[i] && nearest[i] != FeatureId::NONE {
            uf.union(i as u32, nearest[i].0);
        }
    }
    // Same ink is an equivalence, not a fold: EVERY adjacent pair of
    // near-black features unions, so an ink network whose halves only connect
    // through a shared sliver (or through each other) becomes one feature. A
    // star fold toward one parent left two outline halves separate whenever
    // the bridging sliver could only join one of them.
    for i in 0..n {
        if !inky[i] {
            continue;
        }
        for &(j, _) in &adjw[i] {
            if !dark(features[j.ix()].mean) {
                continue;
            }
            uf.union(i as u32, j.0);
        }
    }
    // Ascending-root order keeps the label raster deterministic, as in the
    // shade merge.
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
        // The representative is the component's kept local-area-maximum;
        // ranking by (keep, area) selects it and makes every other member
        // absorb into it.
        let rep = *members
            .iter()
            .max_by(|&&a, &&b| {
                let ka = (keep[a as usize], features[a as usize].area);
                let kb = (keep[b as usize], features[b as usize].area);
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
    (out, remap)
}

/// Full 4-connected outline length of each feature in `labels`, in edge units:
/// every side of a feature pixel facing a different feature, the background, or
/// the image border counts one. Indexed by feature; `n` is the feature count.
///
/// This is the whole geometric perimeter, not just the shared-with-a-neighbor
/// part, so a thin ring hugging the silhouette counts both its outer (against
/// background) and inner (against the fill) edges. That is what makes
/// `area / (0.5 * perimeter)` read its mean thickness rather than double it.
fn feature_perimeters(labels: &FeatureLabels, n: usize) -> Vec<u32> {
    let (w, h) = (labels.w as usize, labels.h as usize);
    let mut per = vec![0u32; n];
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
            };
            face(if x > 0 { Some(labels.at[y * w + x - 1]) } else { None });
            face(if x + 1 < w { Some(labels.at[y * w + x + 1]) } else { None });
            face(if y > 0 { Some(labels.at[(y - 1) * w + x]) } else { None });
            face(if y + 1 < h { Some(labels.at[(y + 1) * w + x]) } else { None });
        }
    }
    per
}

#[cfg(test)]
mod tests {
    use super::*;

    fn partition(features: Vec<Feature>, labels: FeatureLabels) -> Partition {
        Partition { features, labels }
    }

    #[test]
    fn fold_residue_folds_a_speck_cluster_into_the_fill_not_a_turd() {
        // 8x8 fill (feature 0) with two adjacent single-pixel specks (1, 2) of
        // near-identical color embedded in it. Folding toward the nearest color
        // let the two specks union to each other into a surviving turd; folding
        // toward the larger-area parent collapses both into the fill.
        let (w, h) = (8u32, 8u32);
        let mut at = vec![FeatureId(0); (w * h) as usize];
        at[(3 * w + 3) as usize] = FeatureId(1);
        at[(3 * w + 4) as usize] = FeatureId(2);
        let labels = FeatureLabels { w, h, at };
        let features = vec![
            Feature { mean: [120, 120, 120], area: w * h - 2, bbox: (0, 0, 7, 7) },
            Feature { mean: [122, 121, 120], area: 1, bbox: (3, 3, 3, 3) },
            Feature { mean: [122, 121, 120], area: 1, bbox: (4, 3, 4, 3) },
        ];
        let mut part = partition(features, labels);
        part.fold_residue();
        assert_eq!(part.features.len(), 1, "specks must fold into the fill");
        assert_eq!(part.features[0].area, w * h);
        assert!(part.labels.at.iter().all(|&l| l == FeatureId(0)));
    }

    // Column-striped label raster: each x maps to the feature index `of(x)`
    // returns, constant down the column.
    fn column_labels(w: u32, h: u32, of: impl Fn(u32) -> u32) -> FeatureLabels {
        let at = (0..w * h).map(|i| FeatureId(of(i % w))).collect();
        FeatureLabels { w, h, at }
    }

    #[test]
    fn fold_residue_folds_a_blend_ribbon_between_two_fills() {
        // Dark fill | 2px ribbon of their mixture | light fill. Greys share an
        // OKLab line, so the ribbon sits exactly on the segment between its two
        // dominant neighbors and must fold into one of them.
        let labels = column_labels(22, 20, |x| if x < 10 { 0 } else if x < 12 { 1 } else { 2 });
        let features = vec![
            Feature { mean: [40, 40, 40], area: 200, bbox: (0, 0, 9, 19) },
            Feature { mean: [120, 120, 120], area: 40, bbox: (10, 0, 11, 19) },
            Feature { mean: [200, 200, 200], area: 200, bbox: (12, 0, 21, 19) },
        ];
        let mut part = partition(features, labels);
        part.fold_residue();
        assert_eq!(part.features.len(), 2, "the blend ribbon must fold");
    }

    #[test]
    fn fold_residue_keeps_a_thin_stroke_on_a_uniform_fill() {
        // A 1px-thick dark crease with the sheet on both sides has one distinct
        // neighbor, so it can never read as a blend of two.
        let (w, h) = (20u32, 20u32);
        let mut at = vec![FeatureId(0); (w * h) as usize];
        for x in 4..16 {
            at[(10 * w + x) as usize] = FeatureId(1);
        }
        let labels = FeatureLabels { w, h, at };
        let features = vec![
            Feature { mean: [230, 230, 230], area: w * h - 12, bbox: (0, 0, 19, 19) },
            Feature { mean: [60, 60, 60], area: 12, bbox: (4, 10, 15, 10) },
        ];
        let mut part = partition(features, labels);
        part.fold_residue();
        assert_eq!(part.features.len(), 2, "the crease stroke must survive");
    }

    #[test]
    fn fold_residue_folds_a_near_endpoint_sliver_into_the_hugged_fill() {
        // A 1x14 seam sliver whose color is a near-fill mixture of its two
        // sides: on the segment between them but within the endpoint margin of
        // the dark fill, and too small for the shading shelter, so it folds
        // into that fill instead of surviving as a strip.
        let labels = column_labels(21, 14, |x| if x < 10 { 0 } else if x < 11 { 1 } else { 2 });
        let features = vec![
            Feature { mean: [41, 41, 41], area: 140, bbox: (0, 0, 9, 13) },
            Feature { mean: [51, 51, 51], area: 14, bbox: (10, 0, 10, 13) },
            Feature { mean: [217, 217, 217], area: 140, bbox: (11, 0, 20, 13) },
        ];
        let mut part = partition(features, labels);
        part.fold_residue();
        assert_eq!(part.features.len(), 2, "the seam sliver must fold");
        // The sliver's pixels land in the dark fill it hugs, not the white.
        assert_eq!(part.labels.at[10], part.labels.at[0]);
    }

    #[test]
    fn fold_residue_keeps_a_dark_stroke_between_two_fills() {
        // A thin stroke darker than BOTH fills it separates is off the segment
        // between them (its color is its own, not their mixture), so it
        // survives where a blend ribbon of the same shape folds.
        let labels = column_labels(20, 20, |x| if x < 9 { 0 } else if x < 11 { 1 } else { 2 });
        let features = vec![
            Feature { mean: [120, 120, 120], area: 180, bbox: (0, 0, 8, 19) },
            Feature { mean: [30, 30, 30], area: 40, bbox: (9, 0, 10, 19) },
            Feature { mean: [220, 220, 220], area: 180, bbox: (11, 0, 19, 19) },
        ];
        let mut part = partition(features, labels);
        part.fold_residue();
        assert_eq!(part.features.len(), 3, "the dark stroke must survive");
    }
}
