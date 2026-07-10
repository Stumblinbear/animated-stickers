//! Color and geometry helpers shared by the palette stages.

use super::{Feature, FeatureId, FeatureLabels};
use crate::config::srgb_to_oklab;
use std::collections::HashMap;

/// A color in OKLab, the space every palette comparison runs in. Wrapping the
/// triple keeps an sRGB byte color from entering a ΔE comparison unconverted.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Lab(pub [f32; 3]);

impl Lab {
    /// Converts an sRGB color.
    pub fn of(srgb: [u8; 3]) -> Lab {
        Lab(srgb_to_oklab(srgb))
    }

    /// Euclidean ΔE to `o`.
    pub fn dist(self, o: Lab) -> f32 {
        self.dist2(o).sqrt()
    }

    /// Squared ΔE to `o`, saving the sqrt where only comparisons happen.
    pub fn dist2(self, o: Lab) -> f32 {
        let (d0, d1, d2) = (self.0[0] - o.0[0], self.0[1] - o.0[1], self.0[2] - o.0[2]);
        d0 * d0 + d1 * d1 + d2 * d2
    }

    /// Distance to the segment between `a` and `b`.
    pub fn seg_dev(self, a: Lab, b: Lab) -> f32 {
        let ab = [b.0[0] - a.0[0], b.0[1] - a.0[1], b.0[2] - a.0[2]];
        let ap = [self.0[0] - a.0[0], self.0[1] - a.0[1], self.0[2] - a.0[2]];
        let len2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
        let t = if len2 > 0.0 {
            ((ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let on = Lab([a.0[0] + t * ab[0], a.0[1] + t * ab[1], a.0[2] + t * ab[2]]);
        self.dist(on)
    }

    /// Whether this color reads as an anti-alias mixture of `a` and `b`:
    /// within `dev` of the segment between them and at least `jnd` away from
    /// each endpoint, so a mark sharing an endpoint's color never matches.
    pub fn blend_between(self, a: Lab, b: Lab, jnd: f32, dev: f32) -> bool {
        if self.dist(a) <= jnd || self.dist(b) <= jnd {
            return false;
        }
        self.seg_dev(a, b) < dev
    }
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
