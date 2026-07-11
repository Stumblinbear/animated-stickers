//! Cross-shape seam matching. Sibling shapes (adjacent shapes whose seam is
//! not a containment-tree edge) each walk their shared boundary, and two
//! independent fits of the same pixel seam diverge into hairline gaps. This
//! module matches undirected unit segments across every walked ring to find
//! each shared stretch, canonicalizes the stretch's geometry once
//! (orientation, corners, smoothing, slack), and embeds the identical bytes
//! in both rings, so the pure per-shape fit emits bitwise-equal curves on
//! both sides.

use crate::color::Srgb;
use crate::config::Config;
use crate::fit;
use crate::pipeline::Shape;
use crate::trace::{self, ContourParams, SeamSpan, SmoothedContour};
use image::GrayImage;
use rayon::prelude::*;
use std::collections::HashMap;

/// Every config value the cross-shape seam match reads beyond the contour
/// walk's own params: the stitch switch and the inputs of the uniform
/// pair-color slack flag on shared stretches.
#[derive(Debug, Clone, PartialEq)]
pub struct StitchParams {
    pub seam_stitch: bool,
    pub seam_slack: f64,
    pub stroke_merge_dist: f32,
}

impl StitchParams {
    pub fn of(cfg: &Config) -> Self {
        Self {
            seam_stitch: cfg.seam_stitch,
            seam_slack: cfg.seam_slack,
            stroke_merge_dist: cfg.stroke_merge_dist,
        }
    }
}

/// A lattice point in scaled space.
type Pt = (i64, i64);

/// One walked ring across the whole shape list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct RingId(u32);

/// An undirected unit boundary segment, keyed by its two lattice endpoints
/// in lexicographic order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct UnitSeg(Pt, Pt);

impl UnitSeg {
    fn of(a: Pt, b: Pt) -> Self {
        if a <= b {
            Self(a, b)
        } else {
            Self(b, a)
        }
    }
}

type V = (f64, f64);

fn sub(a: V, b: V) -> V {
    (a.0 - b.0, a.1 - b.1)
}
fn add(a: V, b: V) -> V {
    (a.0 + b.0, a.1 + b.1)
}
fn mul(a: V, s: f64) -> V {
    (a.0 * s, a.1 * s)
}
fn dot(a: V, b: V) -> f64 {
    a.0 * b.0 + a.1 * b.1
}
fn len(a: V) -> f64 {
    dot(a, a).sqrt()
}
fn norm(a: V) -> V {
    let l = len(a);
    if l > 1e-12 {
        mul(a, 1.0 / l)
    } else {
        (0.0, 0.0)
    }
}

/// Translation from a shape mask's local coordinates to scaled space: the
/// mask origin sits one border pixel above and left of the region bbox.
fn translation(origin: (u32, u32)) -> (i64, i64) {
    (origin.0 as i64 - 1, origin.1 as i64 - 1)
}

/// A walked ring densified to unit lattice steps in scaled space. `orig[k]`
/// marks the dense points that are vertices of the walked ring.
struct DenseRing {
    shape: usize,
    ring: usize,
    pts: Vec<Pt>,
    orig: Vec<bool>,
}

fn densify(shape: usize, ring: usize, verts: &[(f64, f64)], t: (i64, i64)) -> DenseRing {
    let n = verts.len();
    let at = |i: usize| -> Pt {
        (
            verts[i % n].0 as i64 + t.0,
            verts[i % n].1 as i64 + t.1,
        )
    };
    let mut pts = Vec::new();
    let mut orig = Vec::new();
    for i in 0..n {
        let p = at(i);
        let q = at(i + 1);
        debug_assert!(p.0 == q.0 || p.1 == q.1, "boundary edges are axis-aligned");
        let steps = (q.0 - p.0).abs().max((q.1 - p.1).abs());
        if steps == 0 {
            continue;
        }
        let d = ((q.0 - p.0).signum(), (q.1 - p.1).signum());
        pts.push(p);
        orig.push(true);
        for s in 1..steps {
            pts.push((p.0 + d.0 * s, p.1 + d.1 * s));
            orig.push(false);
        }
    }
    DenseRing { shape, ring, pts, orig }
}

/// A maximal run of consecutive unit segments shared with one partner set:
/// dense segments `start..start + len` (wrapping).
struct Run {
    start: usize,
    len: usize,
}

/// Groups a ring's shared segments into maximal same-partner runs, in ring
/// order from a partner-set boundary so no run wraps the scan start. A ring
/// entirely shared with one set yields a single full run.
fn shared_runs(partners: &[Vec<RingId>]) -> Vec<Run> {
    let p = partners.len();
    if !(0..p).any(|k| !partners[k].is_empty()) {
        return Vec::new();
    }
    if (0..p).all(|k| partners[k] == partners[0]) {
        return vec![Run { start: 0, len: p }];
    }
    let k0 = (0..p)
        .find(|&k| partners[k] != partners[(k + p - 1) % p])
        .unwrap();
    let mut runs = Vec::new();
    let mut i = 0;
    while i < p {
        let k = (k0 + i) % p;
        if partners[k].is_empty() {
            i += 1;
            continue;
        }
        let mut l = 1;
        while i + l < p && partners[(k0 + i + l) % p] == partners[k] {
            l += 1;
        }
        runs.push(Run { start: k, len: l });
        i += l;
    }
    runs
}

/// The stretch's vertex chain in the ring's traversal order: both endpoints
/// plus every interior direction change of the dense segment run.
fn run_chain(dense: &[Pt], start: usize, len: usize) -> Vec<Pt> {
    let p = dense.len();
    let at = |i: usize| dense[(start + i) % p];
    let mut out = vec![at(0)];
    for i in 1..len {
        let (a, b, c) = (at(i - 1), at(i), at(i + 1));
        if (b.0 - a.0, b.1 - a.1) != (c.0 - b.0, c.1 - b.1) {
            out.push(b);
        }
    }
    out.push(at(len));
    out
}

/// The direction-change vertices of a whole dense ring, in ring order.
fn cycle_vertices(dense: &[Pt]) -> Vec<Pt> {
    let p = dense.len();
    (0..p)
        .filter(|&i| {
            let a = dense[(i + p - 1) % p];
            let b = dense[i];
            let c = dense[(i + 1) % p];
            (b.0 - a.0, b.1 - a.1) != (c.0 - b.0, c.1 - b.1)
        })
        .map(|i| dense[i])
        .collect()
}

/// Canonical orientation of an open stretch chain: the reading whose vertex
/// sequence is lexicographically smaller (endpoint comparison, extended to
/// the full sequence for equal endpoints). Returns the canonical chain and
/// whether `chain`'s own order is canonical.
fn canonical_open(chain: Vec<Pt>) -> (Vec<Pt>, bool) {
    let rev: Vec<Pt> = chain.iter().rev().copied().collect();
    if chain <= rev {
        (chain, true)
    } else {
        (rev, false)
    }
}

/// Canonical form of a closed vertex cycle: the lexicographically smallest
/// reading over every rotation starting at a minimal vertex, in either
/// direction, closed with a duplicate of its first vertex. Also returns
/// whether `cycle`'s own traversal direction is the canonical one.
fn canonical_cycle(cycle: &[Pt]) -> (Vec<Pt>, bool) {
    let v = cycle.len();
    let pmin = *cycle.iter().min().unwrap();
    let mut best: Option<(Vec<Pt>, bool)> = None;
    for s in 0..v {
        if cycle[s] != pmin {
            continue;
        }
        let fwd: Vec<Pt> = (0..v).map(|i| cycle[(s + i) % v]).collect();
        let bwd: Vec<Pt> = (0..v).map(|i| cycle[(s + v - i) % v]).collect();
        for (cand, is_fwd) in [(fwd, true), (bwd, false)] {
            if best.as_ref().is_none_or(|(b, _)| cand < *b) {
                best = Some((cand, is_fwd));
            }
        }
    }
    let (mut chain, forward) = best.unwrap();
    chain.push(chain[0]);
    (chain, forward)
}

/// A canonicalized stretch, processed once as an open chain. Both rings
/// through the stretch derive it from the same canonical bytes, so every
/// field below is bitwise-agreed between them except `forward`, which is
/// each ring's own traversal direction.
struct Stretch {
    raw: Vec<V>,
    smoothed: Vec<V>,
    /// Interior corners, canonical indices; the endpoints are junctions and
    /// always pinned.
    corners: Vec<usize>,
    forward: bool,
    slack: bool,
}

fn stretch_of(canon: &[Pt], forward: bool, slack: bool, cfg: &ContourParams) -> Stretch {
    let raw: Vec<V> = canon.iter().map(|&(x, y)| (x as f64, y as f64)).collect();
    let corners = open_corners(&raw, cfg.corner_threshold(), cfg.corner_arm());
    let smoothed = smooth_open(&raw, &corners, cfg.smooth_radius());
    Stretch { raw, smoothed, corners, forward, slack }
}

/// [`fit::find_corners`] adapted to an open chain: arm windows clamp at the
/// endpoints, no run wraps, and the endpoints themselves are excluded (they
/// are junctions, pinned by the caller regardless of angle).
fn open_corners(pts: &[V], threshold: f64, arm: f64) -> Vec<usize> {
    let n = pts.len();
    if n < 3 {
        return Vec::new();
    }
    let mut turns = vec![0.0f64; n];
    for (i, turn) in turns.iter_mut().enumerate().take(n - 1).skip(1) {
        let mut back = (0.0, 0.0);
        let mut d = 0.0;
        let mut j = i;
        while d < arm && j > 0 {
            j -= 1;
            back = sub(pts[j], pts[i]);
            d = len(back);
        }
        let mut fwd = (0.0, 0.0);
        d = 0.0;
        j = i;
        while d < arm && j < n - 1 {
            j += 1;
            fwd = sub(pts[j], pts[i]);
            d = len(fwd);
        }
        let (vin, vout) = (norm(mul(back, -1.0)), norm(fwd));
        *turn = dot(vin, vout).clamp(-1.0, 1.0).acos();
    }

    let mut corners = Vec::new();
    let mut i = 1;
    while i < n - 1 {
        if turns[i] < threshold {
            i += 1;
            continue;
        }
        let mut best = i;
        let mut j = i;
        while j + 1 < n - 1 && turns[j + 1] >= threshold {
            j += 1;
            if turns[j] > turns[best] {
                best = j;
            }
        }
        corners.push(best);
        i = j + 1;
    }
    corners
}

/// [`fit::smooth_pinned`] adapted to an open chain: both endpoints join the
/// pinned set alongside the corners, and windows never reach past a pin.
fn smooth_open(pts: &[V], corners: &[usize], radius: usize) -> Vec<V> {
    let n = pts.len();
    if radius == 0 || n < 3 {
        return pts.to_vec();
    }
    let radius = radius as f64;
    let mut pinned = vec![false; n];
    pinned[0] = true;
    pinned[n - 1] = true;
    for &c in corners {
        if c < n {
            pinned[c] = true;
        }
    }
    // edge[i] is the arclength from pts[i] to pts[i+1].
    let edge: Vec<f64> = (0..n - 1).map(|i| len(sub(pts[i + 1], pts[i]))).collect();

    // Arclength to the nearest pin in each direction; the pinned endpoints
    // make both finite everywhere with a single sweep each way.
    let mut fwd = vec![0.0f64; n];
    let mut bwd = vec![0.0f64; n];
    for k in (0..n).rev() {
        fwd[k] = if pinned[k] { 0.0 } else { edge[k] + fwd[k + 1] };
    }
    for k in 0..n {
        bwd[k] = if pinned[k] { 0.0 } else { edge[k - 1] + bwd[k - 1] };
    }

    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let half = radius.min(fwd[i].min(bwd[i]));
        if pinned[i] || half <= 0.0 {
            out.push(pts[i]);
            continue;
        }
        let mut acc = pts[i];
        let mut count = 1.0;
        let mut d = 0.0;
        let mut j = i;
        while j + 1 < n {
            d += edge[j];
            if d > half {
                break;
            }
            j += 1;
            acc = add(acc, pts[j]);
            count += 1.0;
        }
        d = 0.0;
        j = i;
        while j > 0 {
            d += edge[j - 1];
            if d > half {
                break;
            }
            j -= 1;
            acc = add(acc, pts[j]);
            count += 1.0;
        }
        out.push(mul(acc, 1.0 / count));
    }
    out
}

/// Whether a stretch fits at the seam-slack tolerance: uniform over the
/// stretch, agreed by every ring through it, from the member shapes' colors
/// alone. The per-shape mask flags are symmetric only in principle; this
/// test is guaranteed identical on both sides.
fn run_slack(colors: &[Srgb], gate: bool, thresh: f32) -> bool {
    gate
        && colors.iter().enumerate().all(|(i, &a)| {
            colors[i + 1..].iter().all(|&b| a.dist(b) < thresh)
        })
}

/// The per-vertex slack flag of a free (unshared) vertex at scaled-space
/// integer `(x, y)`, read from the shape's mask-local slack mask.
fn free_flag(slack: Option<&GrayImage>, t: (i64, i64), x: f64, y: f64) -> bool {
    slack.is_some_and(|sm| {
        trace::corner_touches(sm, x.round() as i64 - t.0, y.round() as i64 - t.1)
    })
}

/// A ring with no shared segments inside a stitched shape, processed like
/// [`trace::smoothed_contours`] but in scaled space.
fn scaled_free_ring(
    verts: &[(f64, f64)],
    t: (i64, i64),
    slack: Option<&GrayImage>,
    cfg: &ContourParams,
) -> SmoothedContour {
    let pts: Vec<V> = verts
        .iter()
        .map(|&(x, y)| (x + t.0 as f64, y + t.1 as f64))
        .collect();
    let corners = fit::find_corners(&pts, cfg.corner_threshold(), cfg.corner_arm());
    let flags = pts.iter().map(|&(x, y)| free_flag(slack, t, x, y)).collect();
    let smoothed = fit::smooth_pinned(&pts, &corners, cfg.smooth_radius());
    SmoothedContour { pts: smoothed, corners, flags, seams: Vec::new() }
}

/// A ring entirely covered by one stretch: the ring is the canonical cycle,
/// oriented to the ring's winding, with a single whole-ring span.
fn full_ring(st: &Stretch) -> Option<SmoothedContour> {
    let v = st.raw.len() - 1;
    if v < 3 {
        return None;
    }
    let src = |j: usize| if st.forward { j } else { (v - j) % v };
    let pts: Vec<V> = (0..v).map(|j| st.smoothed[src(j)]).collect();
    let mut corners = vec![0];
    for &c in &st.corners {
        corners.push(if st.forward { c } else { v - c });
    }
    corners.sort_unstable();
    corners.dedup();
    Some(SmoothedContour {
        pts,
        corners,
        flags: vec![st.slack; v],
        seams: vec![SeamSpan { start: 0, end: 0, forward: st.forward, slack: st.slack }],
    })
}

/// Assembles a ring holding both free intervals and shared stretches: the
/// ring starts at the first run's start junction, spans splice the stretch
/// vertices oriented to the ring, free intervals keep the walked vertices
/// plus any junction the walk ran straight through.
fn assemble_ring(
    d: &DenseRing,
    runs: &[(Run, Stretch)],
    slack: Option<&GrayImage>,
    t: (i64, i64),
    cfg: &ContourParams,
) -> Option<SmoothedContour> {
    let p = d.pts.len();
    let mut raw: Vec<V> = Vec::new();
    let mut flags: Vec<bool> = Vec::new();
    let mut corners: Vec<usize> = Vec::new();
    // (ring offset of the span's first vertex, its stretch)
    let mut placed: Vec<(usize, &Stretch)> = Vec::new();

    for (ri, (run, st)) in runs.iter().enumerate() {
        let offset = raw.len();
        let m = st.raw.len();
        for j in 0..m - 1 {
            let src = if st.forward { j } else { m - 1 - j };
            raw.push(st.raw[src]);
            flags.push(st.slack);
        }
        corners.push(offset);
        for &c in &st.corners {
            corners.push(offset + if st.forward { c } else { m - 1 - c });
        }
        placed.push((offset, st));

        // The free interval to the next run; its first point is this span's
        // end junction, kept even where the walk ran straight through it.
        let end = (run.start + run.len) % p;
        let next = runs[(ri + 1) % runs.len()].0.start;
        let free = (next + p - end) % p;
        if free > 0 {
            corners.push(raw.len());
        }
        for j in 0..free {
            let di = (end + j) % p;
            if j == 0 || d.orig[di] {
                let pt = d.pts[di];
                let fpt = (pt.0 as f64, pt.1 as f64);
                raw.push(fpt);
                flags.push(free_flag(slack, t, fpt.0, fpt.1));
            }
        }
    }

    let total = raw.len();
    if total < 3 {
        return None;
    }

    let seams: Vec<SeamSpan> = placed
        .iter()
        .map(|&(offset, st)| SeamSpan {
            start: offset,
            end: (offset + st.raw.len() - 1) % total,
            forward: st.forward,
            slack: st.slack,
        })
        .collect();

    // Free corners detect over the whole assembled ring, then any that fell
    // strictly inside a span yield to the stretch's own corner set.
    let inside = |i: usize| {
        placed.iter().any(|&(offset, st)| {
            let rel = (i + total - offset) % total;
            rel > 0 && rel < st.raw.len() - 1
        })
    };
    for c in fit::find_corners(&raw, cfg.corner_threshold(), cfg.corner_arm()) {
        if !inside(c) {
            corners.push(c);
        }
    }
    corners.sort_unstable();
    corners.dedup();

    // Junctions are corners, so the ring smoothing never reaches across one;
    // span ranges then take the stretch's smoothed points verbatim.
    let mut pts = fit::smooth_pinned(&raw, &corners, cfg.smooth_radius());
    for &(offset, st) in &placed {
        let m = st.raw.len();
        for j in 0..m - 1 {
            pts[offset + j] = st.smoothed[if st.forward { j } else { m - 1 - j }];
        }
    }

    Some(SmoothedContour { pts, corners, flags, seams })
}

/// Walks every shape's rings and returns each shape's smoothed contours,
/// shared-stretch spans embedded, paired with the translation from the
/// contours' coordinate space to scaled space.
///
/// A shape with no shared stretch (and every shape when `seam_stitch` is
/// off) walks, smooths, and flags mask-local exactly as
/// [`trace::smoothed_contours`], with the mask origin's translation. A shape
/// with shared stretches is assembled in a frame anchored at the minimal
/// shape origin, so the stretch bytes it embeds are the very bytes its
/// sibling embeds, and cropping the layer (which shifts every origin
/// equally) cannot move them.
pub(crate) fn stitched_contours(
    shapes: &[Shape],
    contour: &ContourParams,
    stitch: &StitchParams,
) -> Vec<(Vec<SmoothedContour>, (f64, f64))> {
    let plain = |mask: &GrayImage, slack: Option<&GrayImage>, origin: (u32, u32)| {
        (
            trace::smoothed_contours(mask, contour, slack),
            (origin.0 as f64 - 1.0, origin.1 as f64 - 1.0),
        )
    };

    if !stitch.seam_stitch {
        return shapes
            .par_iter()
            .map(|(_, mask, slack, origin)| plain(mask, slack.as_ref(), *origin))
            .collect();
    }

    let walked: Vec<Vec<Vec<(f64, f64)>>> = shapes
        .par_iter()
        .map(|(_, mask, _, _)| trace::walk_rings(mask))
        .collect();

    let anchor = shapes.iter().fold((i64::MAX, i64::MAX), |a, s| {
        let t = translation(s.3);
        (a.0.min(t.0), a.1.min(t.1))
    });
    // Mask-local -> anchor frame, per shape.
    let frame = |origin: (u32, u32)| {
        let t = translation(origin);
        (t.0 - anchor.0, t.1 - anchor.1)
    };

    let mut dense: Vec<DenseRing> = Vec::new();
    let mut gids_by_shape: Vec<Vec<usize>> = vec![Vec::new(); shapes.len()];
    for (si, rings) in walked.iter().enumerate() {
        let t = frame(shapes[si].3);
        for (ri, ring) in rings.iter().enumerate() {
            gids_by_shape[si].push(dense.len());
            dense.push(densify(si, ri, ring, t));
        }
    }

    // A unit segment borders exactly two pixels, so it lies on a shape's
    // boundary exactly when the shape's mask holds one side: interior
    // parent-child seams are covered by the parent and never match, sibling
    // seams match, and a subtree-union boundary along a descendant's
    // silhouette matches too (which also pins the stacked outlines
    // together). No containment-tree knowledge enters here.
    let mut occ: HashMap<UnitSeg, Vec<(u32, u32)>> = HashMap::new();
    for (gid, d) in dense.iter().enumerate() {
        let p = d.pts.len();
        for k in 0..p {
            occ.entry(UnitSeg::of(d.pts[k], d.pts[(k + 1) % p]))
                .or_default()
                .push((gid as u32, k as u32));
        }
    }

    let mut partners: Vec<Vec<Vec<RingId>>> = dense
        .iter()
        .map(|d| vec![Vec::new(); d.pts.len()])
        .collect();
    for group in occ.into_values() {
        if group.len() < 2 {
            continue;
        }
        for &(g, k) in &group {
            let mut others: Vec<RingId> = group
                .iter()
                .filter(|&&(og, _)| og != g)
                .map(|&(og, _)| RingId(og))
                .collect();
            others.sort_unstable();
            others.dedup();
            partners[g as usize][k as usize] = others;
        }
    }

    let runs: Vec<Vec<Run>> = partners.par_iter().map(|p| shared_runs(p)).collect();

    let gate = stitch.seam_slack > 1.0 && stitch.stroke_merge_dist > 0.0;
    let thresh = 2.0 * stitch.stroke_merge_dist;

    shapes
        .par_iter()
        .enumerate()
        .map(|(si, (color, mask, slack, origin))| {
            if gids_by_shape[si].iter().all(|&g| runs[g].is_empty()) {
                return plain(mask, slack.as_ref(), *origin);
            }

            let t = frame(*origin);
            let contours = gids_by_shape[si]
                .iter()
                .filter_map(|&gid| {
                    let d = &dense[gid];
                    if runs[gid].is_empty() {
                        return Some(scaled_free_ring(
                            &walked[si][d.ring],
                            t,
                            slack.as_ref(),
                            contour,
                        ));
                    }

                    let stretches: Vec<(bool, Stretch)> = runs[gid]
                        .iter()
                        .map(|r| {
                            let full = r.len == d.pts.len();
                            let (canon, forward) = if full {
                                canonical_cycle(&cycle_vertices(&d.pts))
                            } else {
                                canonical_open(run_chain(&d.pts, r.start, r.len))
                            };
                            let mut colors = vec![*color];
                            colors.extend(
                                partners[gid][r.start]
                                    .iter()
                                    .map(|&RingId(og)| shapes[dense[og as usize].shape].0),
                            );
                            let sl = run_slack(&colors, gate, thresh);
                            (full, stretch_of(&canon, forward, sl, contour))
                        })
                        .collect();

                    if let Some((true, st)) = stretches.first().filter(|_| stretches.len() == 1) {
                        return full_ring(st);
                    }
                    let paired: Vec<(Run, Stretch)> = runs[gid]
                        .iter()
                        .zip(stretches)
                        .map(|(r, (_, st))| (Run { start: r.start, len: r.len }, st))
                        .collect();
                    assemble_ring(d, &paired, slack.as_ref(), t, contour)
                })
                .collect();

            (contours, (anchor.0 as f64, anchor.1 as f64))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fit::AnchorSpan;
    use crate::trace::{FitParams, TracedPath};
    use image::Luma;

    type Cubic = (V, V, V);

    /// A shape whose art covers the scaled pixels `[x, x+w) x [y, y+h)`:
    /// bbox origin `(x, y)`, mask with the pipeline's 1px border.
    fn rect_shape(color: Srgb, (x, y): (u32, u32), w: u32, h: u32) -> Shape {
        shape_from(color, (x, y), w, h, |_, _| true)
    }

    /// A shape over the same bbox whose art is the scaled pixels of
    /// `[x, x+w) x [y, y+h)` selected by `pred` (scaled coordinates).
    fn shape_from(
        color: Srgb,
        (x, y): (u32, u32),
        w: u32,
        h: u32,
        pred: impl Fn(u32, u32) -> bool,
    ) -> Shape {
        let mut mask = GrayImage::new(w + 2, h + 2);
        for py in 0..h {
            for px in 0..w {
                if pred(x + px, y + py) {
                    mask.put_pixel(px + 1, py + 1, Luma([255]));
                }
            }
        }
        (color, mask, None, (x, y))
    }

    fn params() -> (ContourParams, FitParams, StitchParams) {
        let cfg = Config {
            scale: 1,
            ..Default::default()
        };
        (
            ContourParams::of(&cfg),
            FitParams::of(&cfg),
            StitchParams::of(&cfg),
        )
    }

    fn reverse_run(start: V, run: &[Cubic]) -> (V, Vec<Cubic>) {
        let k = run.len();
        let out = (0..k)
            .map(|j| {
                let i = k - 1 - j;
                let end = if i == 0 { start } else { run[i - 1].2 };
                (run[i].1, run[i].0, end)
            })
            .collect();
        (run[k - 1].2, out)
    }

    /// A span's cubic run in canonical direction: (start anchor, cubics).
    /// Two siblings' runs over one stretch must compare exactly equal.
    fn canonical_run(path: &TracedPath, s: &AnchorSpan) -> (V, Vec<Cubic>) {
        let n = path.cubics.len();
        let count = if s.start == s.end { n } else { (s.end + n - s.start) % n };
        let anchor = |i: usize| if i == 0 { path.start } else { path.cubics[i - 1].2 };
        let run: Vec<Cubic> = (0..count).map(|t| path.cubics[(s.start + t) % n]).collect();
        if s.forward {
            (anchor(s.start), run)
        } else {
            reverse_run(anchor(s.start), &run)
        }
    }

    fn anchors(path: &TracedPath) -> Vec<V> {
        let mut out = vec![path.start];
        out.extend(path.cubics.iter().map(|c| c.2).take(path.cubics.len() - 1));
        out
    }

    #[test]
    fn abutting_siblings_embed_bitwise_equal_spans() {
        // Two rectangles on transparency meeting on a staircased seam: the
        // seam bulges one pixel right for y in 5..9, so the stretch carries
        // real interior vertices, not one straight run.
        let left = |px: u32, py: u32| px < 9 || (px < 10 && (5..9).contains(&py));
        let shapes = vec![
            shape_from(Srgb([200, 30, 30]), (1, 1), 16, 12, left),
            shape_from(Srgb([30, 30, 200]), (1, 1), 16, 12, |px, py| !left(px, py)),
        ];
        let (cp, fp, sp) = params();
        let stitched = stitched_contours(&shapes, &cp, &sp);

        for (contours, t) in &stitched {
            assert_eq!(*t, (0.0, 0.0), "the anchor frame is scaled space here");
            assert_eq!(contours.len(), 1);
            assert_eq!(contours[0].seams.len(), 1);
        }
        assert_ne!(
            stitched[0].0[0].seams[0].forward,
            stitched[1].0[0].seams[0].forward,
            "the two rings traverse the stretch in opposite directions"
        );

        let fitted: Vec<Vec<(TracedPath, Vec<AnchorSpan>)>> = stitched
            .iter()
            .map(|(c, _)| trace::fit_contours(c, &fp))
            .collect();
        let (pa, sa) = &fitted[0][0];
        let (pb, sb) = &fitted[1][0];
        assert_eq!(canonical_run(pa, &sa[0]), canonical_run(pb, &sb[0]));

        let ct = crate::config::corner_threshold(Config::default().alphamax);
        let (qa, ta) = fit::simplify_closed_seamed(pa, 1.0, ct, sa);
        let (qb, tb) = fit::simplify_closed_seamed(pb, 1.0, ct, sb);
        assert_eq!(canonical_run(&qa, &ta[0]), canonical_run(&qb, &tb[0]));
    }

    #[test]
    fn triple_point_junction_anchors_every_ring_through_it() {
        // A fills the left half; B and C stack on the right, so the three
        // meet at (11, 11) and every pair shares a stretch. A's boundary runs
        // straight down x = 11 through the triple point: its walk emits no
        // vertex there, so the junction must be inserted.
        let a = rect_shape(Srgb([200, 30, 30]), (1, 1), 10, 20);
        let b = rect_shape(Srgb([30, 200, 30]), (11, 1), 10, 10);
        let c = rect_shape(Srgb([30, 30, 200]), (11, 11), 10, 10);
        let shapes = vec![a, b, c];
        let (cp, fp, sp) = params();

        let j = (11.0, 11.0);
        let walked: Vec<(f64, f64)> = trace::walk_rings(&shapes[0].1)[0]
            .iter()
            .map(|&(x, y)| (x, y))
            .collect();
        assert!(!walked.contains(&(11.0, 11.0)), "A's raw walk has no vertex at the mask-local junction");

        let stitched = stitched_contours(&shapes, &cp, &sp);
        for (contours, _) in &stitched {
            assert_eq!(contours[0].seams.len(), 2);
        }
        let idx = stitched[0].0[0].pts.iter().position(|&p| p == j).expect("junction inserted into A's ring");
        assert!(stitched[0].0[0].corners.contains(&idx));

        let fitted: Vec<Vec<(TracedPath, Vec<AnchorSpan>)>> = stitched
            .iter()
            .map(|(cs, _)| trace::fit_contours(cs, &fp))
            .collect();
        for paths in &fitted {
            assert!(
                anchors(&paths[0].0).contains(&j),
                "the junction is an anchor in every ring through it"
            );
        }

        // Every span run pairs with exactly one bitwise-equal partner.
        let mut runs: Vec<(V, Vec<Cubic>)> = fitted
            .iter()
            .flat_map(|paths| {
                let (p, ss) = &paths[0];
                ss.iter().map(|s| canonical_run(p, s)).collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(runs.len(), 6);
        while let Some(r) = runs.pop() {
            let i = runs
                .iter()
                .position(|o| *o == r)
                .expect("a span run pairs with its sibling's");
            runs.remove(i);
        }
    }

    #[test]
    fn closed_loop_stretch_canonicalizes_stably() {
        // A block seated exactly in a non-parent shape's hole: the hole ring
        // and the block's outer ring share their entire boundary.
        let hole = |px: u32, py: u32| (5..9).contains(&px) && (5..9).contains(&py);
        let donut = shape_from(Srgb([200, 30, 30]), (1, 1), 12, 12, |px, py| !hole(px, py));
        let plug = rect_shape(Srgb([30, 30, 200]), (5, 5), 4, 4);
        let (cp, fp, sp) = params();

        let run_of = |shapes: &[Shape]| -> Vec<(V, Vec<Cubic>)> {
            stitched_contours(shapes, &cp, &sp)
                .iter()
                .flat_map(|(cs, _)| trace::fit_contours(cs, &fp))
                .filter(|(_, ss)| !ss.is_empty())
                .map(|(p, ss)| {
                    assert_eq!(ss, vec![ss[0]]);
                    assert_eq!((ss[0].start, ss[0].end), (0, 0), "the stretch covers the whole ring");
                    canonical_run(&p, &ss[0])
                })
                .collect()
        };

        let runs = run_of(&[donut.clone(), plug.clone()]);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0], runs[1]);

        // The canonical orientation is a function of geometry alone, so the
        // shape order cannot move it.
        assert_eq!(run_of(&[plug, donut]), runs);
    }

    #[test]
    fn stitch_is_identity_without_shared_stretches() {
        // Detached shapes share nothing: with the match on they must walk,
        // smooth, and translate exactly as the plain per-shape path.
        let shapes = vec![
            rect_shape(Srgb([200, 30, 30]), (1, 1), 6, 6),
            rect_shape(Srgb([30, 30, 200]), (12, 1), 6, 6),
        ];
        let (cp, fp, sp) = params();
        let on = stitched_contours(&shapes, &cp, &sp);

        for (si, (_, mask, slack, origin)) in shapes.iter().enumerate() {
            let plain = trace::smoothed_contours(mask, &cp, slack.as_ref());
            assert_eq!(on[si].0, plain);
            assert_eq!(on[si].1, (origin.0 as f64 - 1.0, origin.1 as f64 - 1.0));
        }

        let fits: Vec<_> = on.iter().map(|(c, _)| trace::fit_contours(c, &fp)).collect();
        for (fitted, (_, mask, slack, _)) in fits.iter().zip(&shapes) {
            let base = trace::trace_mask(mask, &cp, &fp, slack.as_ref());
            assert_eq!(fitted.len(), base.len());
            for ((p, ss), b) in fitted.iter().zip(&base) {
                assert!(ss.is_empty());
                assert_eq!(p.start, b.start);
                assert_eq!(p.cubics, b.cubics);
            }
        }
    }

    #[test]
    fn stitch_off_skips_the_match() {
        // Abutting siblings with the switch off: every ring is the plain
        // per-shape walk, with no spans anywhere.
        let shapes = vec![
            rect_shape(Srgb([200, 30, 30]), (1, 1), 8, 6),
            rect_shape(Srgb([30, 30, 200]), (9, 1), 8, 6),
        ];
        let (cp, _, sp) = params();
        let off = StitchParams { seam_stitch: false, ..sp };

        for (si, (_, mask, slack, origin)) in shapes.iter().enumerate() {
            let out = &stitched_contours(&shapes, &cp, &off)[si];
            assert_eq!(out.0, trace::smoothed_contours(mask, &cp, slack.as_ref()));
            assert_eq!(out.1, (origin.0 as f64 - 1.0, origin.1 as f64 - 1.0));
        }
    }
}
