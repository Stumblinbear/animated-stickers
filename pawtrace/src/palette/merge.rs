//! Cliff-bounded consolidation: adjacent features merge smallest ΔE first
//! until every remaining gap reaches the shade-split stop.

use super::common::{boundary_edges, grow_bbox, Lab, UnionFind};
use super::{Feature, FeatureId, FeatureLabels, Partition};
use crate::config::Config;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// Identity slack for the merge comparison: a cluster's rep may sit up to
/// this many tolerances from its partner's rep. Slack 2 lets a noisy stroke's
/// dusky fragments (reps up to ~0.1 from the core pink) rejoin it, while a
/// shadow-to-fill walk (reps 0.13+ apart at large sizes, tolerance ~0.03)
/// stays blocked.
const GUARD_SLACK: f32 = 2.0;

impl Partition {
    /// Consolidates the partition into contiguous regions bounded by color
    /// cliffs: adjacent clusters merge smallest mean-color ΔE first until
    /// every remaining gap reaches `cfg.shade_split`, with small clusters
    /// tolerating up to `cfg.shade_noise / min(area)` more because their
    /// means are noise-dominated estimates. A soft mark's interior is
    /// gradual, so the mark collapses to one feature with its soft edge
    /// inside it, while its boundary against the surround is a cliff and
    /// survives. A wide smooth gradient re-bands itself: as a chain absorbs
    /// bands its area-weighted mean drifts until the next band falls outside
    /// the stop, cutting a band roughly every stop-width of ramp. Each output
    /// feature keeps the color of its largest original member, so an authored
    /// fill stays exact.
    pub fn merge_shades(&mut self, cfg: &Config) {
        if self.features.is_empty() {
            return;
        }
        let (out, remap) =
            consolidate(&self.features, &self.labels, cfg.shade_split, cfg.shade_noise);
        self.apply(out, &remap);
    }
}

fn consolidate(
    features: &[Feature],
    labels: &FeatureLabels,
    shade_split: f32,
    noise_k: f32,
) -> (Vec<Feature>, Vec<FeatureId>) {
    let n = features.len();
    let mut uf = UnionFind::new(n);
    // Cluster accumulators, indexed by root. The merge decision uses the
    // area-weighted mean in OKLab (f64 sums: a cluster can span millions of
    // px), while `rep` remembers the largest original member for the output
    // color.
    let mut version = vec![0u32; n];
    let mut area: Vec<u64> = features.iter().map(|f| f.area as u64).collect();
    let mut lab_sum: Vec<[f64; 3]> = features
        .iter()
        .map(|f| {
            let l = Lab::of(f.mean).0;
            let a = f.area as f64;
            [l[0] as f64 * a, l[1] as f64 * a, l[2] as f64 * a]
        })
        .collect();
    let mut bbox: Vec<(u32, u32, u32, u32)> = features.iter().map(|f| f.bbox).collect();
    let mut rep: Vec<u32> = (0..n as u32).collect();
    let mut rep_area: Vec<u32> = features.iter().map(|f| f.area).collect();
    let mut nbrs: Vec<HashSet<u32>> = vec![HashSet::new(); n];
    for (a, b) in boundary_edges(labels) {
        nbrs[a.ix()].insert(b.0);
        nbrs[b.ix()].insert(a.0);
    }
    let lab0: Vec<Lab> = features.iter().map(|f| Lab::of(f.mean)).collect();
    let mean = |lab_sum: &[[f64; 3]], area: &[u64], i: usize| -> Lab {
        let a = area[i] as f64;
        Lab([
            (lab_sum[i][0] / a) as f32,
            (lab_sum[i][1] / a) as f32,
            (lab_sum[i][2] / a) as f32,
        ])
    };
    // Pair distance is the max of the mean distance and the rep distance over
    // [`GUARD_SLACK`]: means still average noise away, while a cluster's
    // identity (its rep) can stretch at most slack-times the tolerance, which
    // blocks a mean-drift walk from a shadow into its fill.
    let pair_d = |lab_sum: &[[f64; 3]], area: &[u64], rep: &[u32], i: usize, j: usize| -> f32 {
        let dr = lab0[rep[i] as usize].dist(lab0[rep[j] as usize]);
        (dr / GUARD_SLACK).max(mean(lab_sum, area, i).dist(mean(lab_sum, area, j)))
    };

    // Min-heap of candidate merges as (ΔE bits, a, b, version a, version b)
    // with a < b. ΔE is non-negative, so its bit pattern orders like the
    // float; the index tie-breaks make equal-distance pops deterministic
    // regardless of hash-order pushes. A popped entry whose endpoint is no
    // longer a root, or whose version moved, is stale and skipped.
    type Merges = BinaryHeap<Reverse<(u32, u32, u32, u32, u32)>>;
    let mut heap: Merges = BinaryHeap::new();
    for (a, ns) in nbrs.iter().enumerate() {
        for &b in ns {
            let b = b as usize;
            if a < b {
                let d = pair_d(&lab_sum, &area, &rep, a, b);
                heap.push(Reverse((d.to_bits(), a as u32, b as u32, 0, 0)));
            }
        }
    }
    while let Some(Reverse((dbits, a, b, va, vb))) = heap.pop() {
        let d = f32::from_bits(dbits);
        // Nothing in the heap can beat the 1px-cluster tolerance, and pops
        // arrive in ascending d, so everything past this point fails too.
        if d >= shade_split + noise_k {
            break;
        }
        if !uf.is_root(a) || !uf.is_root(b) {
            continue;
        }
        let (a, b) = (a as usize, b as usize);
        if version[a] != va || version[b] != vb {
            continue;
        }
        // Areas only grow, so a pair failing its tolerance now fails forever;
        // it re-enters the heap with fresh distance if either side merges.
        if d >= shade_split + noise_k / area[a].min(area[b]) as f32 {
            continue;
        }
        // b folds into a (a < b), keeping ascending-root determinism.
        uf.union(a as u32, b as u32);
        area[a] += area[b];
        for k in 0..3 {
            lab_sum[a][k] += lab_sum[b][k];
        }
        let bb = bbox[b];
        grow_bbox(&mut bbox[a], bb);
        if rep_area[b] > rep_area[a] || (rep_area[b] == rep_area[a] && rep[b] < rep[a]) {
            rep[a] = rep[b];
            rep_area[a] = rep_area[b];
        }
        version[a] += 1;
        let moved: Vec<u32> = nbrs[b].iter().copied().collect();
        for nb in moved {
            let nbu = nb as usize;
            nbrs[nbu].remove(&(b as u32));
            if nbu != a {
                nbrs[nbu].insert(a as u32);
                nbrs[a].insert(nb);
            }
        }
        nbrs[a].remove(&(a as u32));
        nbrs[a].remove(&(b as u32));
        nbrs[b].clear();
        for &nb in &nbrs[a] {
            let nbu = nb as usize;
            let d = pair_d(&lab_sum, &area, &rep, a, nbu);
            let (x, y) = (a.min(nbu) as u32, a.max(nbu) as u32);
            heap.push(Reverse((
                d.to_bits(),
                x,
                y,
                version[x as usize],
                version[y as usize],
            )));
        }
    }

    // Output in ascending-root order so the label raster is deterministic.
    let mut remap = vec![FeatureId::NONE; n];
    let mut out: Vec<Feature> = Vec::new();
    let mut slot: HashMap<u32, u32> = HashMap::new();
    for i in 0..n as u32 {
        let r = uf.find(i);
        if r == i {
            slot.insert(r, out.len() as u32);
            out.push(Feature {
                mean: features[rep[r as usize] as usize].mean,
                area: area[r as usize] as u32,
                bbox: bbox[r as usize],
            });
        }
    }
    for i in 0..n as u32 {
        remap[i as usize] = FeatureId(slot[&uf.find(i)]);
    }
    (out, remap)
}
