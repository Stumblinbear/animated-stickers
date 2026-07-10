//! Palette selection over the merged partition: grouping features of one
//! authored color, then greedy salience-ordered slot assignment.

use super::common::Lab;
use super::Feature;
use crate::config::Config;

/// OKLab distance under which feature means count as one authored color.
/// Independent detections of the same fill land a few thousandths apart
/// (mean jitter from the AA edge pixels each component absorbs); 0.015
/// covers that and stays under the closest deliberate pair measured on the
/// goldens (soft fur highlight vs base, 0.037).
const FEATURE_DEDUP: f32 = 0.015;

/// A group's aggregate area must reach this many detail areas to earn a
/// palette slot when no single member does.
const AGGREGATE_EVIDENCE: f32 = 4.0;

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
pub fn group_features(features: &[Feature], cfg: &Config, dim: u32) -> Vec<FeatureGroup> {
    let scale2 = (cfg.scale * cfg.scale).max(1) as f32;
    let detail_src = cfg.detail_area_scaled(dim) / scale2;
    // Slivers of a few px are resample fringe. They cannot found a palette
    // color, and pooling tens of thousands of them would both fabricate
    // aggregate evidence for blend colors and blow up the O(F*G) grouping.
    let prune = (detail_src / 8.0).max(3.0);
    let mut feats: Vec<&Feature> = features.iter().filter(|f| f.area as f32 >= prune).collect();
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
        (std::cmp::Reverse(g.largest), std::cmp::Reverse(g.aggregate), g.color)
    });
    groups
}

/// Palette slots from grouped features: `cfg.locked` first, unconditionally,
/// then groups in salience order until `cfg.max_colors`. A group earns a
/// slot when its largest member covers a detail area (source px) or its
/// aggregate covers [`AGGREGATE_EVIDENCE`] of them; kept colors dedup at
/// [`FEATURE_DEDUP`], with no merge radius beyond it.
pub fn select_features(groups: &[FeatureGroup], cfg: &Config, dim: u32) -> Vec<[u8; 3]> {
    let scale2 = (cfg.scale * cfg.scale).max(1) as f32;
    let detail_src = cfg.detail_area_scaled(dim) / scale2;
    let mut palette: Vec<[u8; 3]> = Vec::new();
    for &c in &cfg.locked {
        if !palette.contains(&c) {
            palette.push(c);
        }
    }
    let mut kept: Vec<Lab> = palette.iter().map(|&c| Lab::of(c)).collect();
    for g in groups {
        if palette.len() >= cfg.max_colors {
            break;
        }
        if (g.largest as f32) < detail_src
            && (g.aggregate as f32) < AGGREGATE_EVIDENCE * detail_src
        {
            continue;
        }
        let l = Lab::of(g.color);
        if kept.iter().any(|&k| l.dist(k) < FEATURE_DEDUP) {
            continue;
        }
        palette.push(g.color);
        kept.push(l);
    }
    // A layer smaller than the detail floor still needs one color to remap
    // to, as the histogram path guarantees with its top entry.
    if palette.is_empty() {
        if let Some(g) = groups.first() {
            palette.push(g.color);
        }
    }
    palette
}
