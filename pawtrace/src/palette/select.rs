//! Palette selection over the merged partition: grouping features of one
//! authored color, then greedy salience-ordered slot assignment.

use super::common::Lab;
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
    pub color: [u8; 3],
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
        let l = Lab::of(f.mean);

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

/// Palette slots from grouped features: `cfg.locked` first, unconditionally,
/// then every group in salience order, deduped at [`FEATURE_DEDUP`] (a group
/// that near-duplicates a kept color pulls from it: its regions remap there).
/// There is no area floor: the constrained remap confines each color to its
/// own features, so a small authored mark costs one slot and cannot bleed.
/// Over `cfg.max_colors`, the least distinct color is evicted first: the
/// non-locked entry with the smallest OKLab gap to its nearest surviving
/// neighbor, whose regions then degrade onto that neighbor.
pub fn select_features(groups: &[FeatureGroup], cfg: &SelectParams) -> Vec<[u8; 3]> {
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut locked_n = 0;

    for &c in &cfg.locked {
        if !palette.contains(&c) {
            palette.push(c);
        }
    }

    let mut kept: Vec<Lab> = palette.iter().map(|&c| Lab::of(c)).collect();
    locked_n += kept.len();

    for g in groups {
        let l = Lab::of(g.color);

        if kept.iter().any(|&k| l.dist(k) < FEATURE_DEDUP) {
            continue;
        }

        palette.push(g.color);
        kept.push(l);
    }

    while palette.len() > cfg.max_colors.max(locked_n) {
        let nearest_gap = |i: usize| {
            kept.iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(_, &k)| kept[i].dist(k))
                .fold(f32::MAX, f32::min)
        };

        // Locked colors occupy the front of the vec and are never evicted.
        let evict = (locked_n..palette.len())
            .min_by(|&a, &b| nearest_gap(a).partial_cmp(&nearest_gap(b)).unwrap());

        match evict {
            Some(i) => {
                palette.remove(i);
                kept.remove(i);
            }
            None => break,
        }
    }

    palette
}
