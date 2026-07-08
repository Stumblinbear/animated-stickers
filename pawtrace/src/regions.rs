//! Connected same-color regions: segmentation, transition-band absorption,
//! and per-region paintable shapes. The region is the pipeline's unit of
//! output: each becomes one filled path, painted as an outside-in stack.

use crate::config::{color_dist, Config};
use image::{GrayImage, RgbImage};
#[cfg(test)]
use image::Luma;

#[derive(Debug, Clone)]
pub struct Region {
    pub color: [u8; 3],
    /// Bbox in scaled px, inclusive.
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
    /// Pixels, relative to (x0, y0).
    pub pixels: Vec<(u32, u32)>,
}

impl Region {
    /// Whether the scaled-space point (x, y) is one of this region's pixels.
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x0
            && y >= self.y0
            && x <= self.x1
            && y <= self.y1
            && self.pixels.contains(&(x - self.x0, y - self.y0))
    }
}

/// One region per connected component of a mask, all in the given color.
/// For uniform-color layers, where the mask already determines the regions
/// and quantization would only rediscover them. Components stay separate:
/// trace_mask walks one component per shape.
pub fn from_mask(mask: &GrayImage, color: [u8; 3]) -> Vec<Region> {
    // A uniform image quantizes to itself, so the mask's connected
    // components are exactly the regions and no color comparison is needed.
    let (w, h) = mask.dimensions();
    let am = mask.as_raw();
    let idx = |x: u32, y: u32| (y * w + x) as usize;
    let mut seen = vec![false; (w * h) as usize];
    let mut regions = Vec::new();
    let mut queue: Vec<(u32, u32)> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if seen[idx(x, y)] || am[idx(x, y)] == 0 {
                continue;
            }
            let mut pixels = Vec::new();
            let (mut x0, mut y0, mut x1, mut y1) = (x, y, x, y);
            seen[idx(x, y)] = true;
            queue.push((x, y));
            while let Some((px, py)) = queue.pop() {
                pixels.push((px, py));
                x0 = x0.min(px);
                y0 = y0.min(py);
                x1 = x1.max(px);
                y1 = y1.max(py);
                let mut visit = |nx: u32, ny: u32, queue: &mut Vec<(u32, u32)>| {
                    if !seen[idx(nx, ny)] && am[idx(nx, ny)] != 0 {
                        seen[idx(nx, ny)] = true;
                        queue.push((nx, ny));
                    }
                };
                if px > 0 { visit(px - 1, py, &mut queue); }
                if py > 0 { visit(px, py - 1, &mut queue); }
                if px + 1 < w { visit(px + 1, py, &mut queue); }
                if py + 1 < h { visit(px, py + 1, &mut queue); }
            }
            for p in &mut pixels {
                p.0 -= x0;
                p.1 -= y0;
            }
            regions.push(Region { color, x0, y0, x1, y1, pixels });
        }
    }
    regions
}

/// Connected same-color regions (4-connectivity) over art pixels.
pub fn segment(quant: &RgbImage, alpha: &GrayImage) -> Vec<Region> {
    let (w, h) = quant.dimensions();
    let q3 = quant.as_raw();
    let am = alpha.as_raw();
    let idx = |x: u32, y: u32| (y * w + x) as usize;
    let color_at = |i: usize| -> [u8; 3] { [q3[3 * i], q3[3 * i + 1], q3[3 * i + 2]] };
    let mut seen = vec![false; (w * h) as usize];
    let mut regions = Vec::new();
    let mut queue: Vec<(u32, u32)> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if seen[idx(x, y)] || am[idx(x, y)] == 0 {
                continue;
            }
            let color = color_at(idx(x, y));
            let mut pixels = Vec::new();
            let (mut x0, mut y0, mut x1, mut y1) = (x, y, x, y);
            seen[idx(x, y)] = true;
            queue.push((x, y));
            while let Some((px, py)) = queue.pop() {
                pixels.push((px, py));
                x0 = x0.min(px);
                y0 = y0.min(py);
                x1 = x1.max(px);
                y1 = y1.max(py);
                let mut visit = |nx: u32, ny: u32, queue: &mut Vec<(u32, u32)>| {
                    let i = idx(nx, ny);
                    if !seen[i] && am[i] != 0 && color_at(i) == color {
                        seen[i] = true;
                        queue.push((nx, ny));
                    }
                };
                if px > 0 { visit(px - 1, py, &mut queue); }
                if py > 0 { visit(px, py - 1, &mut queue); }
                if px + 1 < w { visit(px + 1, py, &mut queue); }
                if py + 1 < h { visit(px, py + 1, &mut queue); }
            }
            for p in &mut pixels {
                p.0 -= x0;
                p.1 -= y0;
            }
            regions.push(Region { color, x0, y0, x1, y1, pixels });
        }
    }
    regions
}

/// Merges every region smaller than `min_area` into its nearest-color
/// neighbor (by shared boundary among ties), cascading until every surviving
/// region clears `min_area` or has no neighbor. Regions holding a pin are
/// left untouched, neither merged away nor receiving neighbors. `(w, h)` is
/// the segmented image's size in scaled px.
pub fn merge_speckles(
    regs: &[Region],
    (w, h): (u32, u32),
    min_area: u64,
    pins: &[(u32, u32)],
) -> Vec<Region> {
    let (roots, colors) = merge_speckle_roots(regs, (w, h), min_area, pins);
    gather_speckle_merged(regs, &roots, &colors).into_values().collect()
}

/// Runs the speckle-merge cascade and returns, per input region: the index of
/// the region it settled into (its own index when it survives as a merge
/// root), and a color per index whose entry at each root is that root's
/// merged color (the color of its largest member).
fn merge_speckle_roots(
    regs: &[Region],
    (w, h): (u32, u32),
    min_area: u64,
    pins: &[(u32, u32)],
) -> (Vec<u32>, Vec<[u8; 3]>) {
    let n = regs.len();
    let mut color: Vec<[u8; 3]> = regs.iter().map(|r| r.color).collect();
    let small = |r: &Region| (r.pixels.len() as u64) < min_area;
    if n < 2 || !regs.iter().any(small) {
        return ((0..n as u32).collect(), color);
    }

    let mut label = vec![u32::MAX; (w * h) as usize];
    for (id, r) in regs.iter().enumerate() {
        for &(px, py) in &r.pixels {
            label[((r.y0 + py) * w + (r.x0 + px)) as usize] = id as u32;
        }
    }

    let mut area: Vec<u64> = regs.iter().map(|r| r.pixels.len() as u64).collect();
    let mut neighbors: Vec<Vec<(u32, u64)>> = Vec::with_capacity(n);
    // Dense-id scratch counter, as in census(): first-touch order is fine
    // because the merge loop re-aggregates by root with its own tie-breaks.
    let mut shared = vec![0u64; n];
    let mut touched: Vec<u32> = Vec::new();
    for (id, r) in regs.iter().enumerate() {
        for &(px, py) in &r.pixels {
            let (x, y) = (r.x0 + px, r.y0 + py);
            for (nx, ny) in [
                (x.wrapping_sub(1), y),
                (x + 1, y),
                (x, y.wrapping_sub(1)),
                (x, y + 1),
            ] {
                if nx < w && ny < h {
                    let other = label[(ny * w + nx) as usize];
                    if other != u32::MAX && other != id as u32 {
                        if shared[other as usize] == 0 {
                            touched.push(other);
                        }
                        shared[other as usize] += 1;
                    }
                }
            }
        }
        neighbors.push(
            touched
                .drain(..)
                .map(|o| {
                    let e = (o, shared[o as usize]);
                    shared[o as usize] = 0;
                    e
                })
                .collect(),
        );
    }

    let pinned: Vec<bool> = regs
        .iter()
        .map(|r| pins.iter().any(|&(x, y)| r.contains(x, y)))
        .collect();

    let mut nodes: Vec<u32> = (0..n as u32).collect();
    fn find(parent: &mut [u32], id: u32) -> u32 {
        let p = parent[id as usize];
        if p == id {
            return id;
        }
        let root = find(parent, p);
        parent[id as usize] = root;
        root
    }

    let mut by_len = vec![0u64; n];
    let mut btouched: Vec<u32> = Vec::new();
    loop {
        // Roots snapshot once per round: merges land as a batch below, so
        // the snapshot equals per-neighbor find() calls.
        let root_of: Vec<u32> = (0..n as u32).map(|id| find(&mut nodes, id)).collect();
        let mut merges: Vec<(u32, u32)> = Vec::new();
        for id in 0..n as u32 {
            if root_of[id as usize] != id || pinned[id as usize] || area[id as usize] >= min_area
            {
                continue;
            }
            for &(nid, len) in &neighbors[id as usize] {
                let root = root_of[nid as usize];
                // Pinned regions take no part in merging, as source or
                // target: a pin marks the region as deliberate exactly as
                // drawn, and receiving a neighbor would grow it.
                if root != id && !pinned[root as usize] {
                    if by_len[root as usize] == 0 {
                        btouched.push(root);
                    }
                    by_len[root as usize] += len;
                }
            }
            let by_root: Vec<(u32, u64)> = btouched
                .drain(..)
                .map(|o| {
                    let e = (o, by_len[o as usize]);
                    by_len[o as usize] = 0;
                    e
                })
                .collect();
            // Nearest color picks the target, not dominant boundary: a
            // border arc shares more edge with the fill it outlines than
            // with the sibling arcs at its two ends, and boundary-major
            // merging would bleed the line into the fill instead of
            // reuniting it.
            let target = by_root.into_iter().min_by(|&(ra, la), &(rb, lb)| {
                color_dist(color[id as usize], color[ra as usize])
                    .total_cmp(&color_dist(color[id as usize], color[rb as usize]))
                    .then(lb.cmp(&la))
                    .then(ra.cmp(&rb))
            });
            if let Some((t, _)) = target {
                merges.push((id, t));
            }
        }
        if merges.is_empty() {
            break;
        }
        for (id, target) in merges {
            let t = find(&mut nodes, target);
            let s = find(&mut nodes, id);
            if s == t {
                continue;
            }
            // The larger side keeps its color, so a chain of gradient arcs
            // converges on its biggest member, not on merge order.
            if area[s as usize] > area[t as usize] {
                color[t as usize] = color[s as usize];
            }
            area[t as usize] += area[s as usize];
            let sn = std::mem::take(&mut neighbors[s as usize]);
            neighbors[t as usize].extend(sn);
            nodes[s as usize] = t;
        }
    }

    let roots: Vec<u32> = (0..n as u32).map(|id| find(&mut nodes, id)).collect();
    (roots, color)
}

/// Gathers each merge root's members into one region keyed by root index: the
/// root's color and union bbox, every member's pixels re-based onto it.
/// `roots[i]` is region `i`'s root and `colors[root]` its color.
fn gather_speckle_merged(
    regs: &[Region],
    roots: &[u32],
    colors: &[[u8; 3]],
) -> std::collections::BTreeMap<u32, Region> {
    let mut merged: std::collections::BTreeMap<u32, Region> = Default::default();
    for (id, r) in regs.iter().enumerate() {
        let root = roots[id];
        let m = merged.entry(root).or_insert_with(|| Region {
            color: colors[root as usize],
            x0: u32::MAX,
            y0: u32::MAX,
            x1: 0,
            y1: 0,
            pixels: Vec::new(),
        });
        m.x0 = m.x0.min(r.x0);
        m.y0 = m.y0.min(r.y0);
        m.x1 = m.x1.max(r.x1);
        m.y1 = m.y1.max(r.y1);
    }
    for (id, r) in regs.iter().enumerate() {
        let m = merged.get_mut(&roots[id]).unwrap();
        let (ox, oy) = (r.x0 - m.x0, r.y0 - m.y0);
        m.pixels
            .extend(r.pixels.iter().map(|&(px, py)| (px + ox, py + oy)));
    }
    merged
}

/// The trace-time outcome of a segmented region, one per input region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fate {
    /// Clears the speckle floor (or holds a pin) and traces as its own shape.
    Traced,
    /// Below the floor; folded into the neighbor region at this index by the
    /// speckle merge, surviving as pixels but losing its own color and path.
    MergedInto(usize),
    /// Below the floor, with no neighbor to merge into and no pin: dropped
    /// silently at trace time.
    Culled,
}

/// Per-region trace fate plus the areas the speckle floor tested, matching the
/// decisions [`crate::pipeline::trace_regions`] makes. `areas[i]` is the
/// hole-filled area of the merged region that region `i` settled into (the
/// value weighed against `floor`), shared by every member of a merge.
#[derive(Debug, Clone)]
pub struct RegionReport {
    pub fates: Vec<Fate>,
    pub areas: Vec<u64>,
    pub floor: u64,
}

/// Classifies each region in `regs` by what the trace will do with it, under
/// the same speckle merge and floor the pipeline applies. `pins` are
/// floor-exemption points in the regions' scaled space.
pub fn region_report(
    regs: &[Region],
    alpha: &GrayImage,
    cfg: &Config,
    doc_dim: u32,
    pins: &[(u32, u32)],
) -> RegionReport {
    let floor = cfg.turdsize(doc_dim) as u64;
    let (roots, colors) = merge_speckle_roots(regs, alpha.dimensions(), floor, pins);
    let merged = gather_speckle_merged(regs, &roots, &colors);
    // The floor tests the merged region: its hole-filled area against the
    // floor, exempting any pin it covers, exactly as surviving_shapes does.
    use rayon::prelude::*;
    let mut root_area: std::collections::HashMap<u32, u64> = Default::default();
    let mut root_survives: std::collections::HashMap<u32, bool> = Default::default();
    let entries: Vec<(u32, u64, bool)> = merged
        .par_iter()
        .map(|(&root, mr)| {
            let area = region_shape(mr, alpha, floor).1;
            let survives = area >= floor || pins.iter().any(|&(x, y)| mr.contains(x, y));
            (root, area, survives)
        })
        .collect();
    for (root, area, survives) in entries {
        root_area.insert(root, area);
        root_survives.insert(root, survives);
    }
    let fates = roots
        .iter()
        .enumerate()
        .map(|(id, &root)| {
            if root as usize != id {
                Fate::MergedInto(root as usize)
            } else if root_survives[&root] {
                Fate::Traced
            } else {
                Fate::Culled
            }
        })
        .collect();
    let areas = roots.iter().map(|&root| root_area[&root]).collect();
    RegionReport { fates, areas, floor }
}

/// Classifies each region in `regs` by what the trace will do with it. See
/// [`region_report`] for the shared area and floor detail.
pub fn region_fates(
    regs: &[Region],
    alpha: &GrayImage,
    cfg: &Config,
    doc_dim: u32,
    pins: &[(u32, u32)],
) -> Vec<Fate> {
    region_report(regs, alpha, cfg, doc_dim, pins).fates
}

/// Static per-region facts gathered in one pass, plus union-find state for
/// the merge cascade.
struct Node {
    color: [u8; 3],
    area: u64,
    perimeter: u64,
    /// Max BFS depth from the boundary; 2x this bounds the inscribed width.
    /// Computed lazily: most regions fail the cheaper tests first, and the
    /// BFS with its two bbox-sized allocations dominates the census cost.
    depth: Option<u32>,
    /// Boundary pixels, relative to the region bbox; depth's BFS seeds.
    ring: Vec<(u32, u32)>,
    interior: u32,
    bbox: (u32, u32, u32, u32),
    /// (region id, shared boundary length), raw ids resolved at use.
    neighbors: Vec<(u32, u64)>,
    parent: u32,
}

fn find(nodes: &mut Vec<Node>, id: u32) -> u32 {
    let p = nodes[id as usize].parent;
    if p == id {
        return id;
    }
    let root = find(nodes, p);
    nodes[id as usize].parent = root;
    root
}

/// Segments the quantized labels into regions and collapses AA transition
/// bands into their dominant-boundary neighbor. This is the pipeline's one
/// entry point for regions; absorption happens on the region graph, so the
/// image is never repainted or re-segmented.
pub fn segment_absorbed(quant: &RgbImage, alpha: &GrayImage, cfg: &Config) -> Vec<Region> {
    let regions = crate::timing::SEGMENT.time(|| segment(quant, alpha));
    if cfg.absorb_dist <= 0.0 {
        return regions;
    }
    let (w, h) = quant.dimensions();
    crate::timing::ABSORB.time(|| {
        let regions = absorb(regions, w, h, cfg);
        if cfg.stroke_merge_dist > 0.0 && cfg.stroke_merge_width > 0.0 {
            merge_strokes(regions, w, h, cfg)
        } else {
            regions
        }
    })
}

/// Merges AA transition bands into their dominant-boundary neighbor,
/// cascading so multi-step gradient chains collapse to one representative
/// band. A transition band, and nothing else, has all four signatures at
/// once: near a neighbor in color, thin everywhere (resample blur width is
/// constant), separating at least two regions, and colored BETWEEN its two
/// dominant neighbors. Linework fails on contrast, highlights and spots
/// fail as single-neighbor islands and as color extrema, deliberate bands
/// like layered fur fail on max width at their spikes, and nested stroke
/// families fail on bbox containment.
fn absorb(regions: Vec<Region>, w: u32, h: u32, cfg: &Config) -> Vec<Region> {
    // Mean width of a band: area = width * length, perimeter ~ 2 * length.
    let aggr = cfg.absorb_aggr.max(0.0);
    let max_mean_width = 2.5 * cfg.scale as f32 * aggr;
    let max_max_width = 4.0 * cfg.scale as f32 * aggr;

    let mut nodes = census(&regions, w, h);

    // Merge cascade on the region graph: each round evaluates every root
    // against round-start state and applies the merges as a batch. Pixels
    // are repainted once at the end.
    let mut by_len = vec![0u64; nodes.len()];
    let mut btouched: Vec<u32> = Vec::new();
    for _ in 0..8 {
        // Roots snapshot once per round: merges only land as a batch below,
        // so this equals a per-neighbor find() and lets the scan borrow
        // neighbor lists instead of cloning them.
        let root_of: Vec<u32> = (0..nodes.len() as u32)
            .map(|id| find(&mut nodes, id))
            .collect();
        let roots: Vec<u32> = (0..nodes.len() as u32)
            .filter(|&id| root_of[id as usize] == id)
            .collect();
        let mut merges: Vec<(u32, u32)> = Vec::new();
        for &id in &roots {
            let n = &nodes[id as usize];
            let (color, area, perimeter, bbox) = (n.color, n.area, n.perimeter, n.bbox);
            // Resolve neighbor ids to current roots, summing boundary.
            for &(nid, len) in &n.neighbors {
                let root = root_of[nid as usize];
                if root != id {
                    if by_len[root as usize] == 0 {
                        btouched.push(root);
                    }
                    by_len[root as usize] += len;
                }
            }
            let mut neigh: Vec<(u32, u64)> = btouched
                .drain(..)
                .map(|o| {
                    let e = (o, by_len[o as usize]);
                    by_len[o as usize] = 0;
                    e
                })
                .collect();
            // The sort's id tie-break makes the accumulation order above
            // unobservable: boundary-length ties would otherwise pick
            // different dominant neighbors run to run.
            neigh.sort_by_key(|&(rid, l)| (std::cmp::Reverse(l), rid));
            // Islands bordering a single region are deliberate marks
            // (highlights, spots), never transitions.
            if neigh.len() < 2 {
                continue;
            }
            // A nested family (bands wrapping a core, bbox-contained) is a
            // soft brush stroke, deliberate art, not a transition: a
            // transition band separates two regions that both extend beyond
            // it. Absorbing a stroke destroys it whichever neighbor receives
            // it.
            let (ba, bb) = (
                nodes[neigh[0].0 as usize].bbox,
                nodes[neigh[1].0 as usize].bbox,
            );
            let contains = |o: (u32, u32, u32, u32), i: (u32, u32, u32, u32)| {
                o.0 <= i.0 && o.1 <= i.1 && o.2 >= i.2 && o.3 >= i.3
            };
            if contains(bbox, ba) || contains(bbox, bb) {
                continue;
            }
            let (a, b) = (
                nodes[neigh[0].0 as usize].color,
                nodes[neigh[1].0 as usize].color,
            );
            let (da, db) = (color_dist(color, a), color_dist(color, b));
            if da.min(db) >= cfg.absorb_dist {
                continue;
            }
            // A transition lies on the color segment between its dominant
            // neighbors; an extremum (highlight against two fills) does not.
            if da + db > 1.25 * color_dist(a, b) {
                continue;
            }
            let mean_width = 2.0 * area as f32 / perimeter.max(1) as f32;
            if mean_width >= max_mean_width {
                continue;
            }
            // A blur band is equally thin everywhere; a deliberate band
            // (layered fur) has wide spikes even where its mean is small.
            if 2.0 * depth_of(&mut nodes, &regions, id) as f32 >= max_max_width {
                continue;
            }
            // Absorb into the dominant-boundary neighbor, the outer shell of
            // a nested gradient stroke, never the darker core: merging by
            // nearest color concentrated soft creases into their darkest
            // member. Outward merging also self-terminates, since each merge
            // widens the color gap to the next shell, so one representative
            // band at or beyond absorb_dist survives per stroke.
            let target = if da < cfg.absorb_dist { neigh[0].0 } else { neigh[1].0 };
            merges.push((id, target));
        }
        if merges.is_empty() {
            break;
        }
        for (id, target) in merges {
            let t = find(&mut nodes, target);
            let s = find(&mut nodes, id);
            if s == t {
                continue;
            }
            // Aggregate the absorbed band into the target root. Shared
            // boundary becomes interior, so it leaves both perimeters. Depth
            // adds as a bound: stacked thin bands can be that wide together.
            let sn = std::mem::take(&mut nodes[s as usize].neighbors);
            let shared: u64 = sn
                .iter()
                .filter(|&&(nid, _)| find(&mut nodes, nid) == t)
                .map(|&(_, l)| l)
                .sum();
            let sd = depth_of(&mut nodes, &regions, s);
            let td = depth_of(&mut nodes, &regions, t);
            let (sa, sp, sb) = {
                let n = &nodes[s as usize];
                (n.area, n.perimeter, n.bbox)
            };
            let tn = &mut nodes[t as usize];
            tn.area += sa;
            tn.perimeter = (tn.perimeter + sp).saturating_sub(2 * shared);
            tn.depth = Some(td + sd);
            tn.bbox = (
                tn.bbox.0.min(sb.0),
                tn.bbox.1.min(sb.1),
                tn.bbox.2.max(sb.2),
                tn.bbox.3.max(sb.3),
            );
            tn.neighbors.extend(sn);
            nodes[s as usize].parent = t;
        }
    }

    gather_merged(regions, &mut nodes)
}

/// One census pass over a label map of the regions: per-region perimeter,
/// shared boundary per neighbor, boundary ring, and interior count, as
/// union-find nodes for a merge cascade.
fn census(regions: &[Region], w: u32, h: u32) -> Vec<Node> {
    let mut label = vec![u32::MAX; (w * h) as usize];
    for (id, r) in regions.iter().enumerate() {
        for &(px, py) in &r.pixels {
            label[((r.y0 + py) * w + (r.x0 + px)) as usize] = id as u32;
        }
    }
    let mut nodes: Vec<Node> = Vec::with_capacity(regions.len());
    // Dense-id scratch counter instead of a hash map: neighbor ids index it
    // directly, and `touched` limits the reset to the ids actually seen.
    // Neighbor list order becomes first-touch order, which no consumer
    // observes: they re-aggregate by root or sort with explicit tie-breaks.
    let mut shared = vec![0u64; regions.len()];
    let mut touched: Vec<u32> = Vec::new();
    for (id, r) in regions.iter().enumerate() {
        let mut perimeter = 0u64;
        let mut interior = 0u32;
        let mut ring: Vec<(u32, u32)> = Vec::new();
        for &(px, py) in &r.pixels {
            let (x, y) = (r.x0 + px, r.y0 + py);
            let mut foreign = false;
            for (nx, ny) in [
                (x.wrapping_sub(1), y),
                (x + 1, y),
                (x, y.wrapping_sub(1)),
                (x, y + 1),
            ] {
                let other = if nx < w && ny < h {
                    label[(ny * w + nx) as usize]
                } else {
                    u32::MAX
                };
                if other == id as u32 {
                    continue;
                }
                foreign = true;
                perimeter += 1;
                if other != u32::MAX {
                    if shared[other as usize] == 0 {
                        touched.push(other);
                    }
                    shared[other as usize] += 1;
                }
            }
            if foreign {
                ring.push((px, py));
            } else {
                interior += 1;
            }
        }
        let neighbors = touched
            .drain(..)
            .map(|o| {
                let n = (o, shared[o as usize]);
                shared[o as usize] = 0;
                n
            })
            .collect();
        nodes.push(Node {
            color: r.color,
            area: r.pixels.len() as u64,
            perimeter,
            depth: None,
            ring,
            interior,
            bbox: (r.x0, r.y0, r.x1, r.y1),
            neighbors,
            parent: id as u32,
        });
    }
    nodes
}

/// Gathers each union-find root's members into one merged region: the
/// root's color and merged bbox, with every member's pixels re-based onto
/// it.
fn gather_merged(regions: Vec<Region>, nodes: &mut Vec<Node>) -> Vec<Region> {
    let mut merged: std::collections::BTreeMap<u32, Region> = Default::default();
    for (id, r) in regions.into_iter().enumerate() {
        let root = find(nodes, id as u32);
        let bbox = nodes[root as usize].bbox;
        let m = merged.entry(root).or_insert_with(|| Region {
            color: nodes[root as usize].color,
            x0: bbox.0,
            y0: bbox.1,
            x1: bbox.2,
            y1: bbox.3,
            pixels: Vec::new(),
        });
        let (ox, oy) = (r.x0 - m.x0, r.y0 - m.y0);
        m.pixels
            .extend(r.pixels.into_iter().map(|(px, py)| (px + ox, py + oy)));
    }
    merged.into_values().collect()
}

/// Merges adjacent thin regions whose colors sit within `stroke_merge_dist`
/// of each other, cascading until stable. Quantizing shaded artwork cuts a
/// single drawn stroke (an outline crossing a gradient) into segments of
/// interchangeable colors, each otherwise traced as its own path with a
/// visible joint. The `stroke_merge_width` gate keeps wide regions out, so
/// a gradient's interior banding survives.
fn merge_strokes(regions: Vec<Region>, w: u32, h: u32, cfg: &Config) -> Vec<Region> {
    let max_max_width = cfg.stroke_merge_width.max(0.0) * cfg.scale as f32;
    let mut nodes = census(&regions, w, h);
    let n = nodes.len();
    // Thin-ness survives merging: segments of one stroke join end to end,
    // and even a side-by-side join only widens what already reads as a
    // single mark.
    let mut thin: Vec<Option<bool>> = vec![None; n];
    let is_thin = |id: u32, nodes: &mut Vec<Node>, thin: &mut Vec<Option<bool>>| -> bool {
        if let Some(t) = thin[id as usize] {
            return t;
        }
        let t = (2.0 * depth_of(nodes, &regions, id) as f32) < max_max_width;
        thin[id as usize] = Some(t);
        t
    };

    let mut by_len = vec![0u64; n];
    let mut btouched: Vec<u32> = Vec::new();
    for _ in 0..8 {
        // Roots snapshot once per round, as in absorb(): merges land as a
        // batch, so the snapshot equals per-neighbor find() calls.
        let root_of: Vec<u32> = (0..n as u32).map(|id| find(&mut nodes, id)).collect();
        let roots: Vec<u32> = (0..n as u32)
            .filter(|&id| root_of[id as usize] == id)
            .collect();
        let mut merges: Vec<(u32, u32)> = Vec::new();
        for &id in &roots {
            if !is_thin(id, &mut nodes, &mut thin) {
                continue;
            }
            let color = nodes[id as usize].color;
            for &(nid, len) in &nodes[id as usize].neighbors {
                let root = root_of[nid as usize];
                if root != id {
                    if by_len[root as usize] == 0 {
                        btouched.push(root);
                    }
                    by_len[root as usize] += len;
                }
            }
            let mut neigh: Vec<(u32, u64)> = btouched
                .drain(..)
                .map(|o| {
                    let e = (o, by_len[o as usize]);
                    by_len[o as usize] = 0;
                    e
                })
                .collect();
            neigh.sort_by_key(|&(rid, l)| (std::cmp::Reverse(l), rid));
            let target = neigh.into_iter().find(|&(rid, _)| {
                color_dist(color, nodes[rid as usize].color) < cfg.stroke_merge_dist
                    && is_thin(rid, &mut nodes, &mut thin)
            });
            if let Some((t, _)) = target {
                merges.push((id, t));
            }
        }
        if merges.is_empty() {
            break;
        }
        for (id, target) in merges {
            let t = find(&mut nodes, target);
            let s = find(&mut nodes, id);
            if s == t {
                continue;
            }
            // The larger side keeps its color, so a chain of stroke segments
            // converges on its biggest member, not on merge order.
            let sn = std::mem::take(&mut nodes[s as usize].neighbors);
            let (sa, sb, sc) = {
                let node = &nodes[s as usize];
                (node.area, node.bbox, node.color)
            };
            let tn = &mut nodes[t as usize];
            if sa > tn.area {
                tn.color = sc;
            }
            tn.area += sa;
            tn.bbox = (
                tn.bbox.0.min(sb.0),
                tn.bbox.1.min(sb.1),
                tn.bbox.2.max(sb.2),
                tn.bbox.3.max(sb.3),
            );
            tn.neighbors.extend(sn);
            thin[t as usize] = Some(true);
            nodes[s as usize].parent = t;
        }
    }
    gather_merged(regions, &mut nodes)
}

/// Memoized max-depth for node `id`. Valid only while the node's geometry is
/// its original region's; merged roots get their depth summed at merge time,
/// which forces both sides through here first.
fn depth_of(nodes: &mut [Node], regions: &[Region], id: u32) -> u32 {
    if let Some(d) = nodes[id as usize].depth {
        return d;
    }
    // The ring is only needed for this one BFS; take it rather than clone.
    let ring = std::mem::take(&mut nodes[id as usize].ring);
    let d = region_depth(&regions[id as usize], &ring, nodes[id as usize].interior);
    nodes[id as usize].depth = Some(d);
    d
}

/// Max BFS depth (px) from the boundary ring into the region interior.
fn region_depth(r: &Region, ring: &[(u32, u32)], interior: u32) -> u32 {
    if interior == 0 {
        return 1;
    }
    let (bw, bh) = (r.x1 - r.x0 + 1, r.y1 - r.y0 + 1);
    let idx = |x: u32, y: u32| (y * bw + x) as usize;
    // One grid carries both roles: 0 = not a member, u32::MAX = member not
    // yet reached, anything else = assigned depth. Real depths never reach
    // u32::MAX.
    let mut depth = vec![0u32; (bw * bh) as usize];
    for &(px, py) in &r.pixels {
        depth[idx(px, py)] = u32::MAX;
    }
    // Vec plus head cursor is the same FIFO a VecDeque gives, without the
    // ring-buffer bookkeeping.
    let mut q: Vec<(u32, u32)> = Vec::with_capacity(ring.len());
    for &(px, py) in ring {
        depth[idx(px, py)] = 1;
        q.push((px, py));
    }
    let mut head = 0;
    let mut max_d = 1;
    while head < q.len() {
        let (x, y) = q[head];
        head += 1;
        let d = depth[idx(x, y)];
        max_d = max_d.max(d);
        for (nx, ny) in [
            (x.wrapping_sub(1), y),
            (x + 1, y),
            (x, y.wrapping_sub(1)),
            (x, y + 1),
        ] {
            if nx < bw && ny < bh && depth[idx(nx, ny)] == u32::MAX {
                depth[idx(nx, ny)] = d + 1;
                q.push((nx, ny));
            }
        }
    }
    max_d
}

/// A region's paintable shape over its bbox (1px border so the outside is
/// connected): the region plus every enclosed hole that is either free of
/// transparency or smaller than `min_hole` (sub-speckle pinholes). Returns
/// the mask, whose origin is (x0-1, y0-1), and its area.
pub fn region_shape(r: &Region, alpha: &GrayImage, min_hole: u64) -> (GrayImage, u64) {
    let (bw, bh) = (r.x1 - r.x0 + 3, r.y1 - r.y0 + 3);
    let mut mask = GrayImage::new(bw, bh);
    let m: &mut [u8] = &mut mask;
    for &(px, py) in &r.pixels {
        m[((py + 1) * bw + px + 1) as usize] = 255;
    }
    let area = fill_holes(&mut mask, (r.x0, r.y0), alpha, min_hole);
    (mask, area)
}

/// Fills every hole of `mask` (off-pixel components unreachable from its
/// border) that is either free of transparency or smaller than `min_hole`
/// (sub-speckle pinholes), leaving genuine cutouts open. Returns the
/// on-pixel count after filling. `mask` must carry an empty 1px border;
/// `origin` is the position of mask pixel (1, 1) in `alpha`.
pub fn fill_holes(
    mask: &mut GrayImage,
    origin: (u32, u32),
    alpha: &GrayImage,
    min_hole: u64,
) -> u64 {
    let (bw, bh) = mask.dimensions();
    let m: &mut [u8] = mask;
    let (aw, araw) = (alpha.width(), alpha.as_raw());

    // Flood the off-pixels from the border; unreached off-components are
    // holes.
    let idx = |x: u32, y: u32| (y * bw + x) as usize;
    let mut outside = vec![false; (bw * bh) as usize];
    let mut q: Vec<(u32, u32)> = vec![(0, 0)];
    outside[0] = true;
    while let Some((x, y)) = q.pop() {
        let mut visit = |nx: u32, ny: u32, q: &mut Vec<(u32, u32)>| {
            if !outside[idx(nx, ny)] && m[idx(nx, ny)] == 0 {
                outside[idx(nx, ny)] = true;
                q.push((nx, ny));
            }
        };
        if x > 0 { visit(x - 1, y, &mut q); }
        if y > 0 { visit(x, y - 1, &mut q); }
        if x + 1 < bw { visit(x + 1, y, &mut q); }
        if y + 1 < bh { visit(x, y + 1, &mut q); }
    }

    // Collect hole components; fill unless a hole both touches transparency
    // and is at least min_hole (a genuine cutout, not an alpha pinhole).
    let mut hole_id = vec![0u32; (bw * bh) as usize]; // 0 = none
    let mut hole_keep: Vec<bool> = vec![false]; // index 0 unused
    for y in 0..bh {
        for x in 0..bw {
            if m[idx(x, y)] != 0 || outside[idx(x, y)] || hole_id[idx(x, y)] != 0 {
                continue;
            }
            let id = hole_keep.len() as u32;
            let mut transparent = false;
            let mut size = 0u64;
            let mut q = vec![(x, y)];
            hole_id[idx(x, y)] = id;
            while let Some((hx, hy)) = q.pop() {
                size += 1;
                let (dx, dy) = (origin.0 + hx - 1, origin.1 + hy - 1);
                if araw[(dy * aw + dx) as usize] == 0 {
                    transparent = true;
                }
                let mut visit = |nx: u32, ny: u32, q: &mut Vec<(u32, u32)>| {
                    if hole_id[idx(nx, ny)] == 0
                        && !outside[idx(nx, ny)]
                        && m[idx(nx, ny)] == 0
                    {
                        hole_id[idx(nx, ny)] = id;
                        q.push((nx, ny));
                    }
                };
                if hx > 0 { visit(hx - 1, hy, &mut q); }
                if hy > 0 { visit(hx, hy - 1, &mut q); }
                if hx + 1 < bw { visit(hx + 1, hy, &mut q); }
                if hy + 1 < bh { visit(hx, hy + 1, &mut q); }
            }
            hole_keep.push(transparent && size >= min_hole);
        }
    }
    let mut area = 0u64;
    for (i, mv) in m.iter_mut().enumerate() {
        let id = hole_id[i];
        if id != 0 && !hole_keep[id as usize] {
            *mv = 255;
        }
        if *mv != 0 {
            area += 1;
        }
    }
    area
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 12x4: a top row of near-black arcs (two alternating colors, each arc
    /// well under the floor) over a solid gray fill.
    fn dithered_border() -> (RgbImage, GrayImage) {
        let gray = image::Rgb([128u8, 128, 128]);
        let mut quant = RgbImage::from_pixel(12, 4, gray);
        for x in 0..12 {
            let dark = if (x / 4) % 2 == 0 { [16, 16, 16] } else { [24, 24, 24] };
            quant.put_pixel(x, 0, image::Rgb(dark));
        }
        let alpha = GrayImage::from_pixel(12, 4, Luma([255]));
        (quant, alpha)
    }

    #[test]
    fn merge_speckles_reunites_dithered_linework() {
        let (quant, alpha) = dithered_border();
        let regs = segment(&quant, &alpha);
        assert_eq!(regs.len(), 4); // three arcs + fill

        let merged = merge_speckles(&regs, (12, 4), 10, &[]);
        assert_eq!(merged.len(), 2);
        // The arcs coalesce into one dark region instead of bleeding into
        // the gray fill (nearest color, not dominant boundary).
        let dark = merged.iter().find(|r| r.color[0] < 100).unwrap();
        assert_eq!(dark.pixels.len(), 12);
        assert_eq!((dark.x0, dark.y0, dark.x1, dark.y1), (0, 0, 11, 0));
        let fill = merged.iter().find(|r| r.color[0] == 128).unwrap();
        assert_eq!(fill.pixels.len(), 36);
    }

    #[test]
    fn merge_strokes_reunites_a_quantization_split_outline() {
        let (quant, alpha) = dithered_border();
        let regs = segment(&quant, &alpha);
        let cfg = Config { scale: 1, ..Default::default() };
        let merged = merge_strokes(regs, 12, 4, &cfg);
        // The three thin near-black arcs fuse into one stroke; the wide gray
        // fill keeps its color and stays out of it.
        assert_eq!(merged.len(), 2);
        let dark = merged.iter().find(|r| r.color[0] < 100).unwrap();
        assert_eq!(dark.pixels.len(), 12);
        let fill = merged.iter().find(|r| r.color == [128, 128, 128]).unwrap();
        assert_eq!(fill.pixels.len(), 36);
    }

    #[test]
    fn merge_strokes_leaves_wide_bands_apart() {
        // Two wide near-identical bands: gradient banding, not a cut stroke.
        let mut quant = RgbImage::from_pixel(8, 8, image::Rgb([100u8, 100, 100]));
        for y in 0..8 {
            for x in 4..8 {
                quant.put_pixel(x, y, image::Rgb([108, 108, 108]));
            }
        }
        let alpha = GrayImage::from_pixel(8, 8, Luma([255]));
        let regs = segment(&quant, &alpha);
        let cfg = Config { scale: 1, ..Default::default() };
        assert_eq!(merge_strokes(regs, 8, 8, &cfg).len(), 2);
    }

    #[test]
    fn merge_speckles_spares_pinned_regions() {
        let (quant, alpha) = dithered_border();
        let regs = segment(&quant, &alpha);
        // Pin the middle arc: it must survive as its own region.
        let merged = merge_speckles(&regs, (12, 4), 10, &[(5, 0)]);
        assert!(merged.iter().any(|r| r.color == [24, 24, 24] && r.pixels.len() == 4));
    }

    #[test]
    fn merge_speckles_leaves_isolated_specks_for_the_floor() {
        // A 2px speck on transparency has no neighbor to join.
        let mut quant = RgbImage::from_pixel(8, 8, image::Rgb([0, 0, 0]));
        quant.put_pixel(3, 3, image::Rgb([200, 10, 10]));
        quant.put_pixel(4, 3, image::Rgb([200, 10, 10]));
        let mut alpha = GrayImage::new(8, 8);
        alpha.put_pixel(3, 3, Luma([255]));
        alpha.put_pixel(4, 3, Luma([255]));
        let regs = segment(&quant, &alpha);
        let merged = merge_speckles(&regs, (8, 8), 10, &[]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].pixels.len(), 2);
    }

    #[test]
    fn region_fates_classify_survive_merge_cull_and_pin() {
        // Four regions on transparency, scanned in this order: a 36px block
        // that clears the floor; a 2px nub touching it, below the floor; a
        // 2px isolated speck; a 2px isolated speck that a pin exempts.
        let mut quant = RgbImage::from_pixel(24, 8, image::Rgb([0, 0, 0]));
        let mut alpha = GrayImage::new(24, 8);
        let mut opaque = |q: &mut RgbImage, x: u32, y: u32, c: [u8; 3]| {
            q.put_pixel(x, y, image::Rgb(c));
            alpha.put_pixel(x, y, Luma([255]));
        };
        for y in 0..6 {
            for x in 0..6 {
                opaque(&mut quant, x, y, [200, 30, 30]);
            }
        }
        for y in 0..2 {
            opaque(&mut quant, 6, y, [40, 200, 40]);
        }
        opaque(&mut quant, 10, 0, [40, 40, 200]);
        opaque(&mut quant, 11, 0, [40, 40, 200]);
        opaque(&mut quant, 15, 0, [200, 200, 40]);
        opaque(&mut quant, 16, 0, [200, 200, 40]);

        let regs = segment(&quant, &alpha);
        assert_eq!(regs.len(), 4);
        // detail 5 at scale 1 over a 512 doc gives a floor of 12 px: the block
        // clears it, every 2px speck is under it.
        let cfg = Config { scale: 1, detail: 5.0, ..Default::default() };
        let fates = region_fates(&regs, &alpha, &cfg, 512, &[(15, 0)]);
        assert_eq!(
            fates,
            vec![Fate::Traced, Fate::MergedInto(0), Fate::Culled, Fate::Traced]
        );
    }
}
