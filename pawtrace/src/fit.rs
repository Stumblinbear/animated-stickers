//! Schneider least-squares cubic fitting (Graphics Gems, "An Algorithm for
//! Automatically Fitting Digitized Curves") over closed pixel-boundary
//! polygons: corner detection, corner-pinned smoothing, and error-bounded
//! recursive fitting.

use crate::trace::TracedPath;
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

/// Fits a closed polygon with error-bounded cubics, cutting it at the
/// given corner indices. Returns `None` for degenerate inputs.
///
/// `slack`, when given, is a per-point tolerance multiplier the same length
/// as `pts`: point `i` is fit to `(tol * slack[i])` px rather than `tol`, so
/// a stretch flagged for seam slack tolerates a looser fit. `None` fits every
/// point to `tol`.
pub fn fit_closed(
    pts: &[V],
    corners: &[usize],
    tol: f64,
    slack: Option<&[f64]>,
) -> Option<TracedPath> {
    let n = pts.len();
    if n < 3 {
        return None;
    }
    let mut cuts: Vec<usize> = corners.iter().copied().filter(|&i| i < n).collect();
    let smooth_joints = cuts.is_empty();
    if smooth_joints {
        // No corners: a closed curve still needs two cut points to fit as
        // open segments. Central-difference tangents keep the joints G1.
        cuts = vec![0, n / 2];
    } else if cuts.len() == 1 {
        cuts.push((cuts[0] + n / 2) % n);
    }
    cuts.sort_unstable();
    cuts.dedup();

    let m = cuts.len();
    let start = pts[cuts[0]];
    let mut cubics = Vec::new();
    for k in 0..m {
        let a = cuts[k];
        let b = cuts[(k + 1) % m];
        let seg = circular_slice(pts, a, b);
        let seg_slack = slack.map(|s| circular_slice(s, a, b));
        let t1 = if smooth_joints {
            central_tangent(pts, a)
        } else {
            norm(sub(seg[1], seg[0]))
        };
        let t2 = if smooth_joints {
            mul(central_tangent(pts, b), -1.0)
        } else {
            norm(sub(seg[seg.len() - 2], seg[seg.len() - 1]))
        };
        fit_cubic(&seg, t1, t2, tol * tol, seg_slack.as_deref(), &mut cubics, 0);
    }
    Some(TracedPath { start, cubics })
}

/// Corner vertices: turn angle at or above `threshold`, measured between
/// directions over at least `arm` of arclength on each side to ride over
/// pixel quantization noise. Runs of adjacent above-threshold vertices
/// collapse to their sharpest member (on a dense pixel path every vertex
/// near a corner clears the threshold).
pub fn find_corners(pts: &[V], threshold: f64, arm: f64) -> Vec<usize> {
    let n = pts.len();
    let mut turns = vec![0.0f64; n];
    for i in 0..n {
        let mut back = (0.0, 0.0);
        let mut d = 0.0;
        let mut j = i;
        while d < arm {
            let pj = (j + n - 1) % n;
            back = sub(pts[pj], pts[i]);
            d = len(back);
            j = pj;
            if pj == i {
                break;
            }
        }
        let mut fwd = (0.0, 0.0);
        d = 0.0;
        j = i;
        while d < arm {
            let nj = (j + 1) % n;
            fwd = sub(pts[nj], pts[i]);
            d = len(fwd);
            j = nj;
            if nj == i {
                break;
            }
        }
        let (vin, vout) = (norm(mul(back, -1.0)), norm(fwd));
        turns[i] = dot(vin, vout).clamp(-1.0, 1.0).acos();
    }

    let mut corners = Vec::new();
    let mut i = 0;
    while i < n {
        if turns[i] < threshold {
            i += 1;
            continue;
        }
        // Extend the run of above-threshold vertices, keep the sharpest.
        let mut best = i;
        let mut j = i;
        while j + 1 < n && turns[j + 1] >= threshold {
            j += 1;
            if turns[j] > turns[best] {
                best = j;
            }
        }
        corners.push(best);
        i = j + 1;
    }
    // A run may wrap the seam of the ring; merge first and last.
    if corners.len() >= 2 && turns[0] >= threshold && turns[n - 1] >= threshold {
        let (first, last) = (corners[0], *corners.last().unwrap());
        if turns[first] >= turns[last] {
            corners.pop();
        } else {
            corners.remove(0);
        }
    }
    corners
}

/// Moving-average smoothing with corners pinned: each non-corner vertex
/// becomes the mean of vertices within `radius` steps, the window
/// shrinking symmetrically near corners so averages never reach across
/// one.
pub fn smooth_pinned(pts: &[V], corners: &[usize], radius: usize) -> Vec<V> {
    let n = pts.len();
    if radius == 0 || n < 3 {
        return pts.to_vec();
    }
    let is_corner: Vec<bool> = {
        let mut v = vec![false; n];
        for &c in corners {
            if c < n {
                v[c] = true;
            }
        }
        v
    };
    // Distance (in steps) to the nearest corner, so windows shrink as
    // they approach one. No corners: full radius everywhere.
    let mut room = vec![radius; n];
    if !corners.is_empty() {
        let inf = usize::MAX;
        let mut dist = vec![inf; n];
        let mut q: std::collections::VecDeque<usize> = Default::default();
        for &c in corners {
            if c < n {
                dist[c] = 0;
                q.push_back(c);
            }
        }
        while let Some(i) = q.pop_front() {
            for j in [(i + 1) % n, (i + n - 1) % n] {
                if dist[j] == inf {
                    dist[j] = dist[i] + 1;
                    q.push_back(j);
                }
            }
        }
        for i in 0..n {
            room[i] = radius.min(dist[i]);
        }
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        if is_corner[i] || room[i] == 0 {
            out.push(pts[i]);
            continue;
        }
        // Cap the window at half the ring so it never wraps onto itself:
        // a radius wider than the contour would underflow the index below
        // and double-count vertices.
        let r = room[i].min((n - 1) / 2);
        if r == 0 {
            out.push(pts[i]);
            continue;
        }
        let mut acc = (0.0, 0.0);
        let mut count = 0.0;
        for k in 0..=2 * r {
            let j = (i + n + k - r) % n;
            acc = add(acc, pts[j]);
            count += 1.0;
        }
        out.push(mul(acc, 1.0 / count));
    }
    out
}

/// Greedily removes anchors from a closed cubic path whose deletion keeps
/// the curve within `tol` px, merging the two incident segments into one
/// least-squares cubic that preserves the surviving endpoints' tangents.
/// An anchor whose tangents turn by `corner_threshold` or more is kept, so
/// corners survive. Never drops below three anchors.
pub fn simplify_closed(path: &TracedPath, tol: f64, corner_threshold: f64) -> TracedPath {
    let n = path.cubics.len();
    if n < 4 || tol <= 0.0 {
        return path.clone();
    }
    // Anchor positions with their incoming and outgoing control handles.
    // Segment i runs a[i] -> a[i+1] with controls out_h[i] and in_h[i+1].
    let mut a: Vec<V> = Vec::with_capacity(n);
    a.push(path.start);
    for k in 0..n - 1 {
        a.push(path.cubics[k].2);
    }
    let mut out_h: Vec<V> = (0..n).map(|k| path.cubics[k].0).collect();
    let mut in_h: Vec<V> = (0..n).map(|k| path.cubics[(k + n - 1) % n].1).collect();

    let mut prev: Vec<usize> = (0..n).map(|i| (i + n - 1) % n).collect();
    let mut next: Vec<usize> = (0..n).map(|i| (i + 1) % n).collect();
    let mut alive = vec![true; n];
    let mut count = n;

    let is_corner = |i: usize, a: &[V], in_h: &[V], out_h: &[V]| -> bool {
        let incoming = norm(sub(a[i], in_h[i]));
        let outgoing = norm(sub(out_h[i], a[i]));
        if len(incoming) < 0.5 || len(outgoing) < 0.5 {
            return false;
        }
        dot(incoming, outgoing).clamp(-1.0, 1.0).acos() >= corner_threshold
    };

    while count > 3 {
        // Removable anchor with the smallest merge error under tol.
        let mut best: Option<(usize, f64, [V; 4])> = None;
        for j in 0..n {
            if !alive[j] || is_corner(j, &a, &in_h, &out_h) {
                continue;
            }
            let (p, q) = (prev[j], next[j]);
            let pts = sample_pair(
                &[a[p], out_h[p], in_h[j], a[j]],
                &[a[j], out_h[j], in_h[q], a[q]],
            );
            if pts.len() < 3 {
                continue;
            }
            let t1 = norm(sub(out_h[p], a[p]));
            let t2 = norm(sub(in_h[q], a[q]));
            if len(t1) < 1e-9 || len(t2) < 1e-9 {
                continue;
            }
            let u = chord_length_param(&pts);
            let bez = generate_bezier(&pts, &u, t1, t2);
            let (err2, _) = max_error(&pts, &bez, &u, None);
            let err = err2.sqrt();
            if err <= tol && best.is_none_or(|(_, be, _)| err < be) {
                best = Some((j, err, bez));
            }
        }
        let Some((j, _, bez)) = best else { break };
        let (p, q) = (prev[j], next[j]);
        out_h[p] = bez[1];
        in_h[q] = bez[2];
        alive[j] = false;
        next[p] = q;
        prev[q] = p;
        count -= 1;
    }

    let start = (0..n).find(|&i| alive[i]).unwrap();
    let mut cubics = Vec::with_capacity(count);
    let mut i = start;
    loop {
        let q = next[i];
        cubics.push((out_h[i], in_h[q], a[q]));
        i = q;
        if i == start {
            break;
        }
    }
    TracedPath { start: a[start], cubics }
}

/// Samples two adjacent cubic segments into a single polyline, dropping the
/// duplicated shared endpoint. Sample count follows each segment's length.
fn sample_pair(s1: &[V; 4], s2: &[V; 4]) -> Vec<V> {
    let mut pts = sample_cubic(s1);
    pts.pop();
    pts.extend(sample_cubic(s2));
    pts
}

fn sample_cubic(b: &[V; 4]) -> Vec<V> {
    let hull = len(sub(b[1], b[0])) + len(sub(b[2], b[1])) + len(sub(b[3], b[2]));
    let k = (hull.ceil() as usize).clamp(4, 64);
    (0..=k).map(|i| bezier_point(b, i as f64 / k as f64)).collect()
}

fn central_tangent(pts: &[V], i: usize) -> V {
    let n = pts.len();
    norm(sub(pts[(i + 1) % n], pts[(i + n - 1) % n]))
}

/// Vertices from index `a` to `b` inclusive, wrapping; `a == b` yields the
/// full ring closed back onto `a`.
fn circular_slice<T: Copy>(pts: &[T], a: usize, b: usize) -> Vec<T> {
    let n = pts.len();
    let mut out = Vec::new();
    let mut i = a;
    loop {
        out.push(pts[i]);
        if i == b && out.len() > 1 {
            break;
        }
        i = (i + 1) % n;
        if i == a {
            out.push(pts[a]);
            break;
        }
    }
    out
}

fn bezier_point(b: &[V; 4], t: f64) -> V {
    let u = 1.0 - t;
    add(
        add(
            mul(b[0], u * u * u),
            mul(b[1], 3.0 * u * u * t),
        ),
        add(mul(b[2], 3.0 * u * t * t), mul(b[3], t * t * t)),
    )
}

fn fit_cubic(
    pts: &[V],
    t_hat1: V,
    t_hat2: V,
    tol2: f64,
    slack: Option<&[f64]>,
    out: &mut Vec<(V, V, V)>,
    depth: u32,
) {
    let n = pts.len();
    if n == 2 {
        let d = len(sub(pts[1], pts[0])) / 3.0;
        out.push((
            add(pts[0], mul(t_hat1, d)),
            add(pts[1], mul(t_hat2, d)),
            pts[1],
        ));
        return;
    }

    let mut u = chord_length_param(pts);
    let mut bez = generate_bezier(pts, &u, t_hat1, t_hat2);
    let (max_err, mut split) = max_error(pts, &bez, &u, slack);
    if max_err <= tol2 {
        out.push((bez[1], bez[2], bez[3]));
        return;
    }

    // Close misses are usually parameterization error, not shape error.
    if max_err <= tol2 * 16.0 {
        for _ in 0..4 {
            u = reparameterize(pts, &u, &bez);
            bez = generate_bezier(pts, &u, t_hat1, t_hat2);
            let (e, s) = max_error(pts, &bez, &u, slack);
            if e <= tol2 {
                out.push((bez[1], bez[2], bez[3]));
                return;
            }
            split = s;
        }
    }

    if depth > 32 {
        out.push((bez[1], bez[2], bez[3]));
        return;
    }
    let center = norm(add(
        sub(pts[split - 1], pts[split]),
        sub(pts[split], pts[split + 1]),
    ));
    let (s1, s2) = match slack {
        Some(s) => (Some(&s[..=split]), Some(&s[split..])),
        None => (None, None),
    };
    fit_cubic(&pts[..=split], t_hat1, center, tol2, s1, out, depth + 1);
    fit_cubic(&pts[split..], mul(center, -1.0), t_hat2, tol2, s2, out, depth + 1);
}

fn chord_length_param(pts: &[V]) -> Vec<f64> {
    let mut u = vec![0.0; pts.len()];
    for i in 1..pts.len() {
        u[i] = u[i - 1] + len(sub(pts[i], pts[i - 1]));
    }
    let last = u[pts.len() - 1].max(1e-12);
    for v in &mut u {
        *v /= last;
    }
    u
}

fn generate_bezier(pts: &[V], u: &[f64], t_hat1: V, t_hat2: V) -> [V; 4] {
    let n = pts.len();
    let (first, last) = (pts[0], pts[n - 1]);
    let mut c = [[0.0f64; 2]; 2];
    let mut x = [0.0f64; 2];
    for (i, &ui) in u.iter().enumerate() {
        let b0 = (1.0 - ui).powi(3);
        let b1 = 3.0 * ui * (1.0 - ui).powi(2);
        let b2 = 3.0 * ui * ui * (1.0 - ui);
        let b3 = ui.powi(3);
        let a1 = mul(t_hat1, b1);
        let a2 = mul(t_hat2, b2);
        c[0][0] += dot(a1, a1);
        c[0][1] += dot(a1, a2);
        c[1][1] += dot(a2, a2);
        let tmp = sub(
            pts[i],
            add(
                add(mul(first, b0), mul(first, b1)),
                add(mul(last, b2), mul(last, b3)),
            ),
        );
        x[0] += dot(a1, tmp);
        x[1] += dot(a2, tmp);
    }
    c[1][0] = c[0][1];

    let det_c = c[0][0] * c[1][1] - c[1][0] * c[0][1];
    let det_x1 = x[0] * c[1][1] - x[1] * c[0][1];
    let det_x2 = c[0][0] * x[1] - c[1][0] * x[0];
    let mut alpha1 = if det_c.abs() > 1e-12 { det_x1 / det_c } else { 0.0 };
    let mut alpha2 = if det_c.abs() > 1e-12 { det_x2 / det_c } else { 0.0 };

    // Degenerate, inverted, or exploded alphas (near-singular systems can
    // shoot control points across the canvas, rendering as hairline
    // slivers): use the Wu/Barsky heuristic, a third of the chord along
    // each tangent, rather than emit a spike.
    let seg_len = len(sub(last, first));
    let eps = 1e-6 * seg_len;
    let cap = 10.0 * seg_len;
    if !alpha1.is_finite() || !alpha2.is_finite()
        || alpha1 < eps || alpha2 < eps
        || alpha1 > cap || alpha2 > cap
    {
        alpha1 = seg_len / 3.0;
        alpha2 = alpha1;
    }
    [
        first,
        add(first, mul(t_hat1, alpha1)),
        add(last, mul(t_hat2, alpha2)),
        last,
    ]
}

fn reparameterize(pts: &[V], u: &[f64], bez: &[V; 4]) -> Vec<f64> {
    u.iter()
        .zip(pts.iter())
        .map(|(&ui, &p)| newton_raphson(bez, p, ui))
        .collect()
}

fn newton_raphson(bez: &[V; 4], p: V, u: f64) -> f64 {
    // Q'(u) and Q''(u) as bezier polynomials of degree 2 and 1.
    let q1: [V; 3] = [
        mul(sub(bez[1], bez[0]), 3.0),
        mul(sub(bez[2], bez[1]), 3.0),
        mul(sub(bez[3], bez[2]), 3.0),
    ];
    let q2: [V; 2] = [mul(sub(q1[1], q1[0]), 2.0), mul(sub(q1[2], q1[1]), 2.0)];
    let qu = bezier_point(bez, u);
    let v = 1.0 - u;
    let q1u = add(add(mul(q1[0], v * v), mul(q1[1], 2.0 * v * u)), mul(q1[2], u * u));
    let q2u = add(mul(q2[0], v), mul(q2[1], u));
    let num = dot(sub(qu, p), q1u);
    let den = dot(q1u, q1u) + dot(sub(qu, p), q2u);
    if den.abs() < 1e-12 {
        u
    } else {
        (u - num / den).clamp(0.0, 1.0)
    }
}

/// Worst squared fit error and its point index. With `slack`, each point's
/// error is divided by its squared multiplier, so `err_i <= (tol * slack_i)^2`
/// becomes a single comparison of the returned max against `tol^2`. A `None`
/// slack (or all-1.0 multipliers) leaves every error unweighted.
fn max_error(pts: &[V], bez: &[V; 4], u: &[f64], slack: Option<&[f64]>) -> (f64, usize) {
    let mut max_d = 0.0;
    let mut split = pts.len() / 2;
    for i in 1..pts.len() - 1 {
        let d = sub(bezier_point(bez, u[i]), pts[i]);
        let mut d2 = dot(d, d);
        if let Some(s) = slack {
            d2 /= s[i] * s[i];
        }
        if d2 > max_d {
            max_d = d2;
            split = i;
        }
    }
    (max_d, split)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// A regular hexagon as six cubic segments, each control handle a third
    /// of the way toward the neighbor (so joints are near-G1, not corners).
    fn hexagon() -> TracedPath {
        let pts: Vec<V> = (0..6)
            .map(|k| {
                let a = PI / 3.0 * k as f64;
                (200.0 + 100.0 * a.cos(), 200.0 + 100.0 * a.sin())
            })
            .collect();
        let n = pts.len();
        let cubics = (0..n)
            .map(|i| {
                let (p, q) = (pts[i], pts[(i + 1) % n]);
                let d = sub(q, p);
                (add(p, mul(d, 1.0 / 3.0)), sub(q, mul(d, 1.0 / 3.0)), q)
            })
            .collect();
        TracedPath { start: pts[0], cubics }
    }

    #[test]
    fn smooth_pinned_survives_radius_wider_than_the_ring() {
        // A radius far larger than the vertex count must not underflow the
        // window index (regression for the usize subtract overflow).
        let tri = vec![(0.0, 0.0), (10.0, 0.0), (5.0, 8.0)];
        let out = smooth_pinned(&tri, &[], 50);
        assert_eq!(out.len(), tri.len());
    }

    /// A thin closed rectangle sampled at 1px, with one top-edge vertex
    /// pushed 1.5px off the line. Returns the ring, the four rectangle corners
    /// as explicit cut indices, and the bumped vertex's index.
    fn bumped_ring() -> (Vec<V>, Vec<usize>, usize) {
        let (wd, ht) = (40i32, 8i32);
        let mut pts: Vec<V> = Vec::new();
        let mut corners = Vec::new();
        corners.push(pts.len());
        for x in 0..wd {
            pts.push((x as f64, 0.0));
        }
        corners.push(pts.len());
        for y in 0..ht {
            pts.push((wd as f64, y as f64));
        }
        corners.push(pts.len());
        for x in (1..=wd).rev() {
            pts.push((x as f64, ht as f64));
        }
        corners.push(pts.len());
        for y in (1..=ht).rev() {
            pts.push((0.0, y as f64));
        }
        let bump = (wd / 2) as usize;
        pts[bump].1 = -1.5;
        (pts, corners, bump)
    }

    #[test]
    fn slack_is_identity_at_unit_multiplier() {
        // A slack of 1.0 everywhere must reproduce the no-slack fit exactly,
        // so the default path stays byte-identical.
        let (pts, corners, _) = bumped_ring();
        let base = fit_closed(&pts, &corners, 0.5, None).unwrap();
        let ones = vec![1.0; pts.len()];
        let same = fit_closed(&pts, &corners, 0.5, Some(&ones)).unwrap();
        assert_eq!(base.start, same.start);
        assert_eq!(base.cubics, same.cubics);
    }

    #[test]
    fn slack_loosens_a_flagged_point() {
        // The 1.5px bump fails a 0.5px uniform fit and forces a subdivision;
        // slackening just that vertex to 4x (2.0px allowed) absorbs it.
        let (pts, corners, bump) = bumped_ring();
        let tight = fit_closed(&pts, &corners, 0.5, None).unwrap();
        let mut slack = vec![1.0; pts.len()];
        slack[bump] = 4.0;
        let loose = fit_closed(&pts, &corners, 0.5, Some(&slack)).unwrap();
        assert!(
            loose.cubics.len() < tight.cubics.len(),
            "loose {} !< tight {}",
            loose.cubics.len(),
            tight.cubics.len()
        );
    }

    #[test]
    fn simplify_off_is_identity() {
        let h = hexagon();
        assert_eq!(simplify_closed(&h, 0.0, PI).cubics.len(), h.cubics.len());
    }

    #[test]
    fn simplify_reduces_a_smooth_loop() {
        // corner_threshold = PI: no joint counts as a corner, so a generous
        // tolerance collapses toward the three-anchor floor.
        let out = simplify_closed(&hexagon(), 1000.0, PI);
        assert!(out.cubics.len() < 6);
        assert!(out.cubics.len() >= 3);
    }

    #[test]
    fn simplify_keeps_corners() {
        // At a tiny threshold every 60-degree vertex is a corner, so none
        // can be removed however large the tolerance.
        let out = simplify_closed(&hexagon(), 1000.0, 0.1);
        assert_eq!(out.cubics.len(), 6);
    }
}
