//! Palette selection over the merged partition: grouping features of one
//! authored color, then greedy salience-ordered slot assignment.

use crate::color::{Lab, Srgb};
use super::{Feature, SelectParams};

/// OKLab distance under which feature means count as one authored color.
/// Independent detections of the same fill land a few thousandths apart
/// (mean jitter from the AA edge pixels each component absorbs); 0.015
/// covers that and stays under the closest deliberate pair measured on the
/// goldens (soft fur highlight vs base, 0.037).
const FEATURE_DEDUP: f32 = 0.015;

/// Area floor (source px) for a feature to join grouping at all. Purely
/// anti-explosion: keeps degenerate slivers from blowing up the O(F*G)
/// grouping, not an evidence gate.
const GROUP_PRUNE: u32 = 3;

/// Features of one authored color, aggregated as a palette candidate.
#[derive(Debug, Clone)]
pub struct FeatureGroup {
    /// Group color: the mean of its largest member, sRGB.
    pub color: Srgb,
    /// Largest member area, source px.
    pub largest: u32,
    /// Total member area, source px.
    pub aggregate: u64,
}

/// Groups features whose means sit within [`FEATURE_DEDUP`] of each other,
/// so a color drawn as many features (spots, stripes) pools its evidence.
/// Groups come out in salience order: largest member desc, then aggregate
/// desc, then color.
pub fn group_features(features: &[Feature]) -> Vec<FeatureGroup> {
    let mut feats: Vec<&Feature> = features.iter().filter(|f| f.area >= GROUP_PRUNE).collect();
    feats.sort_unstable_by_key(|f| (std::cmp::Reverse(f.area), f.mean));

    let mut groups: Vec<FeatureGroup> = Vec::new();
    let mut group_lab: Vec<Lab> = Vec::new();

    for f in feats {
        let l = Lab::from(f.mean);

        // The founding (largest) member fixes the group color and its lab
        // anchor: an exact fill stays exact instead of drifting with every
        // joining stripe.
        match group_lab.iter().position(|&g| g.dist(l) <= FEATURE_DEDUP) {
            Some(gi) => groups[gi].aggregate += f.area as u64,
            None => {
                groups.push(FeatureGroup {
                    color: f.mean,
                    largest: f.area,
                    aggregate: f.area as u64,
                });

                group_lab.push(l);
            }
        }
    }

    groups.sort_unstable_by_key(|g| {
        (
            std::cmp::Reverse(g.largest),
            std::cmp::Reverse(g.aggregate),
            g.color,
        )
    });

    groups
}

/// A color held for the output palette: its sRGB value, its OKLab position
/// (the eviction metric), and the source area backing it. Locked colors carry
/// zero area; they are never evicted and only serve as gap neighbors.
struct Slot {
    color: Srgb,
    lab: Lab,
    area: u64,
}

/// Palette slots from grouped features: `cfg.locked` first, unconditionally,
/// then every group in salience order, deduped at [`FEATURE_DEDUP`] (a group
/// that near-duplicates a kept color pulls from it: its regions remap there).
/// There is no area floor: the constrained remap confines each color to its
/// own features, so a small authored mark costs one slot and cannot bleed.
/// Over `cfg.max_colors`, the color whose loss is least visible is evicted
/// first: the non-locked entry minimizing `area * OKLab-gap-to-nearest`, whose
/// regions then degrade onto that neighbor.
pub fn select_features(groups: &[FeatureGroup], cfg: &SelectParams) -> Vec<Srgb> {
    let mut slots: Vec<Slot> = Vec::new();

    for &c in &cfg.locked {
        if !slots.iter().any(|s| s.color == c) {
            slots.push(Slot { color: c, lab: Lab::from(c), area: 0 });
        }
    }

    let locked_n = slots.len();

    for g in groups {
        let l = Lab::from(g.color);

        if slots.iter().any(|s| l.dist(s.lab) < FEATURE_DEDUP) {
            continue;
        }

        slots.push(Slot { color: g.color, lab: l, area: g.aggregate });
    }

    while slots.len() > cfg.max_colors.max(locked_n) {
        let nearest_gap = |i: usize| {
            slots
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(_, s)| slots[i].lab.dist(s.lab))
                .fold(f32::MAX, f32::min)
        };

        // Weighting the gap by area protects large fills: a big region evicts
        // only when its gap is tiny, i.e. when remapping it onto the surviving
        // neighbor is perceptually near-free, while a small distinct accent can
        // still outrank a near-duplicate fill.
        let cost = |i: usize| slots[i].area as f32 * nearest_gap(i);

        // Locked colors occupy the front of the vec and are never evicted.
        let evict = (locked_n..slots.len())
            .min_by(|&a, &b| cost(a).partial_cmp(&cost(b)).unwrap());

        match evict {
            Some(i) => {
                slots.remove(i);
            }
            None => break,
        }
    }

    slots.into_iter().map(|s| s.color).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{Lab, Srgb};
    use crate::palette::SelectParams;

    fn hex(s: &str) -> Srgb {
        Srgb::from_hex(s).unwrap()
    }

    fn group(color: Srgb, aggregate: u64) -> FeatureGroup {
        FeatureGroup { color, largest: aggregate.min(u32::MAX as u64) as u32, aggregate }
    }

    fn cfg(max_colors: usize, locked: Vec<Srgb>) -> SelectParams {
        SelectParams { locked, max_colors }
    }

    fn gap(a: Srgb, b: Srgb) -> f32 {
        Lab::from(a).dist(Lab::from(b))
    }

    #[test]
    fn large_fill_survives_when_smaller_accent_can_drop() {
        let big = hex("#c83232");
        let neighbor = hex("#b84848");
        let accent = hex("#3050c0");

        let near = gap(big, neighbor);
        let far = gap(big, accent);
        assert!(near > FEATURE_DEDUP, "near pair must survive dedup: {near}");
        assert!(near < far, "accent must be the more distinct color: {near} < {far}");

        // Old rule (min nearest-gap) evicts big or neighbor, both near-duplicates
        // of each other; the new rule drops the tiny distinct accent instead.
        let groups = vec![group(big, 100_000), group(neighbor, 100_000), group(accent, 3)];
        let out = select_features(&groups, &cfg(2, vec![]));

        assert_eq!(out.len(), 2);
        assert!(out.contains(&big), "large fill kept");
        assert!(!out.contains(&accent), "small accent evicted");
    }

    #[test]
    fn large_fill_evicted_when_its_gap_is_tiny() {
        let partner = hex("#c83232");
        let big = hex("#bc3c3c");
        let distinct = hex("#3050c0");

        let near = gap(partner, big);
        let far = gap(big, distinct);
        assert!(near > FEATURE_DEDUP, "near pair must survive dedup: {near}");
        assert!(near < far, "distinct color must be farther: {near} < {far}");

        // big is large (100k px) but near-duplicates the even larger partner, so
        // partner*near < big*near is beaten and big*near is the global min: the
        // near-free remap onto partner is taken.
        let groups =
            vec![group(partner, 200_000), group(big, 100_000), group(distinct, 100_000)];
        let out = select_features(&groups, &cfg(2, vec![]));

        assert_eq!(out.len(), 2);
        assert!(out.contains(&partner), "larger near-duplicate kept");
        assert!(out.contains(&distinct), "distinct color kept");
        assert!(!out.contains(&big), "large fill evicted on the near-free move");
    }

    #[test]
    fn locked_never_evicted_regardless_of_area() {
        let locked = hex("#101010");
        let huge = hex("#c83232");
        assert!(gap(locked, huge) > FEATURE_DEDUP);

        let groups = vec![group(huge, 10_000_000)];
        let out = select_features(&groups, &cfg(1, vec![locked]));

        assert_eq!(out, vec![locked], "locked survives, the huge fill is evicted");
    }

    #[test]
    fn under_budget_palette_returned_unchanged() {
        let a = hex("#c83232");
        let b = hex("#3050c0");
        let c = hex("#30c050");

        let groups = vec![group(a, 100), group(b, 100), group(c, 100)];
        let out = select_features(&groups, &cfg(5, vec![]));

        assert_eq!(out, vec![a, b, c], "no eviction below budget, insertion order kept");
    }
}
