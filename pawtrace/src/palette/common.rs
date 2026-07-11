//! Color and geometry helpers shared by the palette stages.

use crate::color::Srgb;
use super::{Feature, FeatureId, FeatureLabels};
use std::collections::HashMap;

/// Max sRGB channel under which an achromatic color counts as black ink;
/// adjacent ink features union regardless of OKLab distance, whose cube root
/// inflates imperceptible near-black steps to ΔE 0.05-0.13, past any workable
/// duplicate threshold. Inner-ear strokes measure up to [30,30,30].
pub const INK_BLACK_ZONE: u8 = 30;

/// Max sRGB channel spread for a color to count as ink. Ink is dark AND
/// neutral: a chromatic dark like the brown outline [31,9,0] (spread 22+) is
/// authored linework in another color and must never union with black.
pub const INK_SPREAD: u8 = 8;

/// Whether `m` counts as black ink under [`INK_BLACK_ZONE`]/[`INK_SPREAD`].
pub fn is_ink(m: Srgb) -> bool {
    let [r, g, b] = m.0;
    let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
    hi <= INK_BLACK_ZONE && hi - lo <= INK_SPREAD
}

/// Union-find over dense u32 ids with path halving. Unions settle the root on
/// the LOWER member id, which keeps every consumer's output in first-encounter
/// order deterministically.
pub struct UnionFind(Vec<u32>);

impl UnionFind {
    pub fn new(n: usize) -> UnionFind {
        UnionFind((0..n as u32).collect())
    }

    /// Adds a new singleton set, returning its id.
    pub fn push(&mut self) -> u32 {
        let id = self.0.len() as u32;
        self.0.push(id);
        id
    }

    pub fn find(&mut self, mut i: u32) -> u32 {
        while self.0[i as usize] != i {
            self.0[i as usize] = self.0[self.0[i as usize] as usize];
            i = self.0[i as usize];
        }
        i
    }

    /// Unions the sets holding `a` and `b`; the lower root wins.
    pub fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.0[ra.max(rb) as usize] = ra.min(rb);
        }
    }

    pub fn is_root(&mut self, i: u32) -> bool {
        self.find(i) == i
    }
}

/// Unique unordered feature-id pairs that share a 4-connected boundary in
/// `labels`, background excluded.
pub fn boundary_edges(labels: &FeatureLabels) -> Vec<(FeatureId, FeatureId)> {
    boundary_edge_counts(labels).into_keys().collect()
}

/// Unordered feature-id pairs sharing a 4-connected boundary in `labels`,
/// each with its shared boundary length in pixel-side units, background
/// excluded.
pub fn boundary_edge_counts(labels: &FeatureLabels) -> HashMap<(FeatureId, FeatureId), u32> {
    let (w, h) = (labels.w as usize, labels.h as usize);
    let mut counts: HashMap<(FeatureId, FeatureId), u32> = HashMap::new();
    for y in 0..h {
        for x in 0..w {
            let a = labels.at[y * w + x];
            if a == FeatureId::NONE {
                continue;
            }
            let mut edge = |b: FeatureId| {
                if b != FeatureId::NONE && b != a {
                    *counts.entry((a.min(b), a.max(b))).or_default() += 1;
                }
            };
            if x + 1 < w {
                edge(labels.at[y * w + x + 1]);
            }
            if y + 1 < h {
                edge(labels.at[(y + 1) * w + x]);
            }
        }
    }
    counts
}

/// Adds `src`'s area and bbox to `dst`, leaving `dst`'s color unchanged as
/// the color of the absorbing feature.
pub fn absorb(dst: &mut Feature, src: &Feature) {
    dst.area = dst.area.saturating_add(src.area);
    grow_bbox(&mut dst.bbox, src.bbox);
}

pub fn grow_bbox(dst: &mut (u32, u32, u32, u32), s: (u32, u32, u32, u32)) {
    dst.0 = dst.0.min(s.0);
    dst.1 = dst.1.min(s.1);
    dst.2 = dst.2.max(s.2);
    dst.3 = dst.3.max(s.3);
}
