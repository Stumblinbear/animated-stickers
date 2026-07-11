//! Schneider least-squares cubic fitting (Graphics Gems, "An Algorithm for
//! Automatically Fitting Digitized Curves") over closed pixel-boundary
//! polygons: corner detection, corner-pinned smoothing, and error-bounded
//! recursive fitting.

use crate::trace::{SeamSpan, TracedPath};
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

/// A shared-stretch run over a fitted path's anchors: the segments from
/// anchor `start` to anchor `end` (wrapping past the last segment) trace the
/// stretch. `start == end` marks a stretch covering the whole path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnchorSpan {
    pub start: usize,
    pub end: usize,
    /// Whether the path traverses the stretch in its canonical direction.
    pub forward: bool,
}

/// [`fit_closed`] with shared-stretch awareness: the ring is cut at its
/// corners and span endpoints, free sections fit as usual, and each span is
/// fit once over its points in canonical direction (at a uniform
/// `tol * seam_slack` when the span is flagged), then spliced into the ring
/// forward or reversed. Returns the path and each span's anchor run. With no
/// spans this is exactly [`fit_closed`].
pub fn fit_closed_seamed(
    pts: &[V],
    corners: &[usize],
    tol: f64,
    slack: Option<&[f64]>,
    seams: &[SeamSpan],
    seam_slack: f64,
) -> Option<(TracedPath, Vec<AnchorSpan>)> {
    if seams.is_empty() {
        return fit_closed(pts, corners, tol, slack).map(|p| (p, Vec::new()));
    }
    let n = pts.len();
    if n < 3 {
        return None;
    }

    // A span fits at its own uniform slack, never the per-vertex flags: the
    // flags are computed per shape, the span's value is agreed by both sides.
    let span_slack = |s: &SeamSpan, len_pts: usize| -> Option<Vec<f64>> {
        (s.slack && seam_slack != 1.0).then(|| vec![seam_slack; len_pts])
    };

    // A whole-ring stretch: one open chain from the canonical start around to
    // itself.
    if seams.len() == 1 && seams[0].start == seams[0].end {
        let s = seams[0];
        let chain = circular_slice(pts, s.start, s.start);
        let mut rel: Vec<usize> = corners
            .iter()
            .filter(|&&c| c < n)
            .map(|&c| (c + n - s.start) % n)
            .filter(|&r| r != 0)
            .collect();
        rel.sort_unstable();
        rel.dedup();
        let sl = span_slack(&s, chain.len());
        let cubics = fit_span(&chain, &rel, tol, sl.as_deref(), s.forward);
        if cubics.is_empty() {
            return None;
        }
        return Some((
            TracedPath { start: pts[s.start], cubics },
            vec![AnchorSpan { start: 0, end: 0, forward: s.forward }],
        ));
    }

    let circ = |from: usize, to: usize| (to + n - from) % n;

    let mut cuts: Vec<usize> = corners.iter().copied().filter(|&i| i < n).collect();
    for s in seams {
        cuts.push(s.start);
        cuts.push(s.end);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let m = cuts.len();

    // Span endpoints are cuts, so each section between consecutive cuts lies
    // inside at most one span, and a span tiles a contiguous run of sections.
    let span_of: Vec<Option<usize>> = (0..m)
        .map(|k| {
            let a = cuts[k];
            let seg = circ(a, cuts[(k + 1) % m]);
            seams.iter().position(|s| {
                let len = circ(s.start, s.end);
                let off = circ(s.start, a);
                off < len && off + seg <= len
            })
        })
        .collect();

    // Start the path at a cut that opens a free section or a span, never
    // mid-span, so every span is met at its start and none wraps the seam
    // between the last anchor and the first.
    let k0 = (0..m)
        .find(|&k| match span_of[k] {
            None => true,
            Some(si) => cuts[k] == seams[si].start,
        })
        .unwrap_or(0);

    let start = pts[cuts[k0]];
    let mut cubics: Vec<(V, V, V)> = Vec::new();
    let mut runs: Vec<(usize, usize, bool)> = Vec::new();

    let mut i = 0;
    while i < m {
        let k = (k0 + i) % m;
        match span_of[k] {
            Some(si) => {
                let s = &seams[si];
                let span_pts = circular_slice(pts, s.start, s.end);
                let len = circ(s.start, s.end);
                let mut rel: Vec<usize> = cuts
                    .iter()
                    .map(|&c| circ(s.start, c))
                    .filter(|&o| o > 0 && o < len)
                    .collect();
                rel.sort_unstable();
                rel.dedup();
                let sl = span_slack(s, span_pts.len());
                let off = cubics.len();
                cubics.extend(fit_span(&span_pts, &rel, tol, sl.as_deref(), s.forward));
                runs.push((off, cubics.len() - off, s.forward));

                while i < m && span_of[(k0 + i) % m] == Some(si) {
                    i += 1;
                }
            }
            None => {
                let a = cuts[k];
                let b = cuts[(k + 1) % m];
                let seg = circular_slice(pts, a, b);
                let seg_slack = slack.map(|sl| circular_slice(sl, a, b));
                let t1 = norm(sub(seg[1], seg[0]));
                let t2 = norm(sub(seg[seg.len() - 2], seg[seg.len() - 1]));
                fit_cubic(&seg, t1, t2, tol * tol, seg_slack.as_deref(), &mut cubics, 0);
                i += 1;
            }
        }
    }

    let total = cubics.len();
    if total == 0 {
        return None;
    }
    let spans_out = runs
        .into_iter()
        .map(|(off, cnt, fw)| AnchorSpan {
            start: off,
            end: (off + cnt) % total,
            forward: fw,
        })
        .collect();
    Some((TracedPath { start, cubics }, spans_out))
}

/// Fits an open polyline with error-bounded cubics from `pts[0]` to its last
/// point, cutting at the interior `corners` (indices into `pts`). Endpoint
/// tangents come from the polyline's end edges, as a closed fit's corner
/// cuts do. `slack` is the per-point tolerance multiplier of [`fit_closed`].
pub fn fit_open(pts: &[V], corners: &[usize], tol: f64, slack: Option<&[f64]>) -> Vec<(V, V, V)> {
    let n = pts.len();
    if n < 2 {
        return Vec::new();
    }
    let mut cuts: Vec<usize> = vec![0];
    cuts.extend(corners.iter().copied().filter(|&c| c > 0 && c < n - 1));
    cuts.push(n - 1);
    cuts.sort_unstable();
    cuts.dedup();

    let mut out = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let seg = &pts[a..=b];
        let seg_slack = slack.map(|s| &s[a..=b]);
        let t1 = norm(sub(seg[1], seg[0]));
        let t2 = norm(sub(seg[seg.len() - 2], seg[seg.len() - 1]));
        fit_cubic(seg, t1, t2, tol * tol, seg_slack, &mut out, 0);
    }
    out
}

/// Fits one span's polyline (ring order, both endpoints included) in the
/// stretch's canonical direction and returns the run in ring order. A
/// non-`forward` span reverses its inputs before the fit and the resulting
/// run after it, by index order and control-point swap alone, so both
/// siblings emit bitwise-equal coordinates.
fn fit_span(pts: &[V], corners: &[usize], tol: f64, slack: Option<&[f64]>, forward: bool) -> Vec<(V, V, V)> {
    if pts.len() < 2 {
        return Vec::new();
    }
    if forward {
        return fit_open(pts, corners, tol, slack);
    }
    let m = pts.len();
    let rev: Vec<V> = pts.iter().rev().copied().collect();
    let rev_slack: Option<Vec<f64>> = slack.map(|s| s.iter().rev().copied().collect());
    let mut rev_corners: Vec<usize> = corners.iter().map(|&c| m - 1 - c).collect();
    rev_corners.sort_unstable();
    let cubics = fit_open(&rev, &rev_corners, tol, rev_slack.as_deref());
    reverse_cubics(rev[0], &cubics)
}

/// Reverses a cubic run that starts at `start`: segment order flips and each
/// segment's control points swap; every coordinate is reused untouched.
fn reverse_cubics(start: V, cubics: &[(V, V, V)]) -> Vec<(V, V, V)> {
    let k = cubics.len();
    (0..k)
        .map(|j| {
            let i = k - 1 - j;
            let end = if i == 0 { start } else { cubics[i - 1].2 };
            (cubics[i].1, cubics[i].0, end)
        })
        .collect()
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

/// Moving-average smoothing with corners pinned. Each non-corner vertex
/// becomes the mean of the vertices lying within `radius` of arclength along
/// the contour on each side, the vertex itself always included. The window
/// shrinks symmetrically as it approaches a corner so an average never reaches
/// across one, and corners themselves stay fixed.
///
/// `radius` is in the same scaled px as the contour coordinates. On a densely
/// sampled boundary this is an ordinary moving average; on a long straight run
/// carrying only its two endpoint vertices the window spans no interior vertex,
/// so the run stays put.
pub fn smooth_pinned(pts: &[V], corners: &[usize], radius: usize) -> Vec<V> {
    let n = pts.len();
    if radius == 0 || n < 3 {
        return pts.to_vec();
    }
    let radius = radius as f64;
    let is_corner: Vec<bool> = {
        let mut v = vec![false; n];
        for &c in corners {
            if c < n {
                v[c] = true;
            }
        }
        v
    };
    // edge[i] is the arclength from pts[i] to pts[i+1].
    let edge: Vec<f64> = (0..n).map(|i| len(sub(pts[(i + 1) % n], pts[i]))).collect();
    let perimeter: f64 = edge.iter().sum();

    // Arclength to the nearest corner in each direction, so the window shrinks
    // as it approaches one. A corner-free chain is shorter than the whole ring,
    // so two relaxation passes settle the single seam wrap. No corners: the full
    // radius everywhere.
    let mut room = vec![radius; n];
    if !corners.is_empty() {
        let mut fwd = vec![f64::INFINITY; n];
        let mut bwd = vec![f64::INFINITY; n];
        for _ in 0..2 {
            for k in (0..n).rev() {
                fwd[k] = if is_corner[k] { 0.0 } else { fwd[k].min(edge[k] + fwd[(k + 1) % n]) };
            }
            for k in 0..n {
                let pv = (k + n - 1) % n;
                bwd[k] = if is_corner[k] { 0.0 } else { bwd[k].min(edge[pv] + bwd[pv]) };
            }
        }
        for i in 0..n {
            room[i] = radius.min(fwd[i].min(bwd[i]));
        }
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        // Cap at half the perimeter so the two walks can't wrap past each other
        // and double-count on a ring with no nearby corner.
        let half = room[i].min(perimeter * 0.5);
        if is_corner[i] || half <= 0.0 {
            out.push(pts[i]);
            continue;
        }
        let mut acc = pts[i];
        let mut count = 1.0;
        let mut d = 0.0;
        let mut j = i;
        loop {
            d += edge[j];
            if d > half {
                break;
            }
            j = (j + 1) % n;
            if j == i {
                break;
            }
            acc = add(acc, pts[j]);
            count += 1.0;
        }
        d = 0.0;
        j = i;
        loop {
            let pv = (j + n - 1) % n;
            d += edge[pv];
            if d > half {
                break;
            }
            j = pv;
            if j == i {
                break;
            }
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
    merge_ring(path, tol, corner_threshold, None).0
}

/// [`simplify_closed`] with shared-stretch awareness: anchors inside a span
/// merge by an open-chain pass over the span's cubics in canonical
/// direction, junction anchors never merge away, and free anchors merge as
/// usual without crossing a junction. Returns the simplified path and the
/// spans' anchor runs within it. With no spans this is exactly
/// [`simplify_closed`].
pub fn simplify_closed_seamed(
    path: &TracedPath,
    tol: f64,
    corner_threshold: f64,
    spans: &[AnchorSpan],
) -> (TracedPath, Vec<AnchorSpan>) {
    if spans.is_empty() {
        return (simplify_closed(path, tol, corner_threshold), Vec::new());
    }
    let n = path.cubics.len();
    if tol <= 0.0 || n == 0 {
        return (path.clone(), spans.to_vec());
    }
    let anchor = |i: usize| if i == 0 { path.start } else { path.cubics[i - 1].2 };

    if spans.len() == 1 && spans[0].start == spans[0].end {
        let s = spans[0];
        let start = anchor(s.start);
        let chain: Vec<(V, V, V)> = (0..n).map(|j| path.cubics[(s.start + j) % n]).collect();
        let cubics = simplify_span(start, &chain, tol, corner_threshold, s.forward);
        return (
            TracedPath { start, cubics },
            vec![AnchorSpan { start: 0, end: 0, forward: s.forward }],
        );
    }

    // Spans never straddle anchor 0 (the fit starts the path at a span
    // boundary), so one ring-order walk splices each span in place.
    let mut span_at: Vec<Option<usize>> = vec![None; n];
    for (si, s) in spans.iter().enumerate() {
        span_at[s.start] = Some(si);
    }

    let mut cubics: Vec<(V, V, V)> = Vec::new();
    let mut runs: Vec<(usize, usize, bool)> = Vec::new();
    let mut j = 0;
    while j < n {
        match span_at[j] {
            Some(si) => {
                let s = &spans[si];
                let count = (s.end + n - s.start) % n;
                let chain: Vec<(V, V, V)> =
                    (0..count).map(|t| path.cubics[(s.start + t) % n]).collect();
                let run = simplify_span(anchor(s.start), &chain, tol, corner_threshold, s.forward);
                runs.push((cubics.len(), run.len(), s.forward));
                cubics.extend(run);
                j += count;
            }
            None => {
                cubics.push(path.cubics[j]);
                j += 1;
            }
        }
    }

    let mid = TracedPath { start: path.start, cubics };
    let m = mid.cubics.len();
    let mut locked = vec![false; m];
    for &(off, cnt, _) in &runs {
        for t in 0..=cnt {
            locked[(off + t) % m] = true;
        }
    }

    let (out, order) = merge_ring(&mid, tol, corner_threshold, Some(&locked));
    let mut new_idx = vec![usize::MAX; m];
    for (ni, &oi) in order.iter().enumerate() {
        new_idx[oi] = ni;
    }
    let spans_out = runs
        .into_iter()
        .map(|(off, cnt, fw)| AnchorSpan {
            start: new_idx[off],
            end: new_idx[(off + cnt) % m],
            forward: fw,
        })
        .collect();
    (out, spans_out)
}

/// Simplifies one span's cubic run in the stretch's canonical direction and
/// splices it back in ring order, mirroring `fit_span`'s reversal discipline.
fn simplify_span(
    start: V,
    cubics: &[(V, V, V)],
    tol: f64,
    corner_threshold: f64,
    forward: bool,
) -> Vec<(V, V, V)> {
    if forward {
        return simplify_open(start, cubics, tol, corner_threshold);
    }
    if cubics.is_empty() {
        return Vec::new();
    }
    let end = cubics[cubics.len() - 1].2;
    let rev = reverse_cubics(start, cubics);
    let simplified = simplify_open(end, &rev, tol, corner_threshold);
    reverse_cubics(end, &simplified)
}

/// Greedy anchor removal on an open cubic chain, the open-chain counterpart
/// of [`simplify_closed`]: the two chain endpoints always survive, interior
/// non-corner anchors whose merge stays within `tol` px merge away.
fn simplify_open(start: V, cubics: &[(V, V, V)], tol: f64, corner_threshold: f64) -> Vec<(V, V, V)> {
    let k = cubics.len();
    if k < 2 || tol <= 0.0 {
        return cubics.to_vec();
    }
    let mut a: Vec<V> = Vec::with_capacity(k + 1);
    a.push(start);
    for c in cubics {
        a.push(c.2);
    }
    let mut out_h = vec![(0.0, 0.0); k + 1];
    let mut in_h = vec![(0.0, 0.0); k + 1];
    for i in 0..k {
        out_h[i] = cubics[i].0;
        in_h[i + 1] = cubics[i].1;
    }
    let mut prev: Vec<usize> = (0..=k).map(|i| i.saturating_sub(1)).collect();
    let mut next: Vec<usize> = (0..=k).map(|i| (i + 1).min(k)).collect();
    let mut alive = vec![true; k + 1];

    let is_corner = |i: usize, a: &[V], in_h: &[V], out_h: &[V]| -> bool {
        let incoming = norm(sub(a[i], in_h[i]));
        let outgoing = norm(sub(out_h[i], a[i]));
        if len(incoming) < 0.5 || len(outgoing) < 0.5 {
            return false;
        }
        dot(incoming, outgoing).clamp(-1.0, 1.0).acos() >= corner_threshold
    };

    loop {
        let mut best: Option<(usize, f64, [V; 4])> = None;
        for j in 1..k {
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
    }

    let mut out = Vec::new();
    let mut i = 0;
    while i != k {
        let q = next[i];
        out.push((out_h[i], in_h[q], a[q]));
        i = q;
    }
    out
}

/// The greedy closed-ring merge behind [`simplify_closed`]: `locked` anchors
/// (when given) are never removal candidates. Returns the merged path and
/// the surviving anchors' original indices in emission order.
fn merge_ring(
    path: &TracedPath,
    tol: f64,
    corner_threshold: f64,
    locked: Option<&[bool]>,
) -> (TracedPath, Vec<usize>) {
    let n = path.cubics.len();
    if n < 4 || tol <= 0.0 {
        return (path.clone(), (0..n).collect());
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
            if !alive[j]
                || locked.is_some_and(|l| l[j])
                || is_corner(j, &a, &in_h, &out_h)
            {
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
    let mut order = Vec::with_capacity(count);
    let mut i = start;
    loop {
        order.push(i);
        let q = next[i];
        cubics.push((out_h[i], in_h[q], a[q]));
        i = q;
        if i == start {
            break;
        }
    }
    (TracedPath { start: a[start], cubics }, order)
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
        let chord = sub(pts[1], pts[0]);
        let dir = norm(chord);
        // A two-point segment has no interior points to check, so the usual
        // heuristic handle of a third of the chord goes unbounded: with a
        // tangent inherited from a split next to a corner it bows the curve
        // by up to a third of the chord. The bow is at most 3/4 of the
        // handles' perpendicular offset, so cap the handle length to keep it
        // within tolerance.
        let bow = (t_hat1.0 * dir.1 - t_hat1.1 * dir.0)
            .abs()
            .max((t_hat2.0 * dir.1 - t_hat2.1 * dir.0).abs());
        let allow = tol2.sqrt() * slack.map_or(1.0, |s| s[0].min(s[1]));
        let mut d = len(chord) / 3.0;
        if 0.75 * d * bow > allow {
            d = allow / (0.75 * bow);
        }
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

/// Worst squared fit error and a split index in `1..len-1`, measured at the
/// interior vertices and at curve samples between consecutive vertices. With
/// `slack`, each error is divided by its squared multiplier, so
/// `err_i <= (tol * slack_i)^2` becomes a single comparison of the returned
/// max against `tol^2`. A `None` slack (or all-1.0 multipliers) leaves every
/// error unweighted.
fn max_error(pts: &[V], bez: &[V; 4], u: &[f64], slack: Option<&[f64]>) -> (f64, usize) {
    let n = pts.len();
    let mut max_d = 0.0;
    let mut split = n / 2;
    for i in 1..n - 1 {
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
    // Vertices alone under-sample the curve: across a sparse span the two
    // free control points can thread every interior vertex exactly while
    // bowing far off the polyline in between. Sample the curve inside each
    // parameter span against the chord it should hug.
    for i in 0..n - 1 {
        let (a, b) = (pts[i], pts[i + 1]);
        let k = ((len(sub(b, a)) * 0.5).ceil() as usize).clamp(1, 32);
        for j in 1..=k {
            let t = u[i] + (u[i + 1] - u[i]) * j as f64 / (k + 1) as f64;
            let mut d2 = dist2_to_segment(bezier_point(bez, t), a, b);
            if let Some(s) = slack {
                let sm = s[i].min(s[i + 1]);
                d2 /= sm * sm;
            }
            if d2 > max_d {
                max_d = d2;
                split = (i + 1).min(n - 2);
            }
        }
    }
    (max_d, split)
}

fn dist2_to_segment(p: V, a: V, b: V) -> f64 {
    let ab = sub(b, a);
    let t = (dot(sub(p, a), ab) / dot(ab, ab).max(1e-12)).clamp(0.0, 1.0);
    let d = sub(p, add(a, mul(ab, t)));
    dot(d, d)
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

    /// Max distance from any dense sample of `path` to the closed polygon
    /// `pts`, measured against the polygon's segments.
    fn fit_deviation(path: &TracedPath, pts: &[V]) -> f64 {
        let n = pts.len();
        let seg_d = |q: V, a: V, b: V| -> f64 {
            let ab = sub(b, a);
            let t = (dot(sub(q, a), ab) / dot(ab, ab).max(1e-12)).clamp(0.0, 1.0);
            len(sub(q, add(a, mul(ab, t))))
        };
        let mut cur = path.start;
        let mut worst = 0.0f64;
        for &(c1, c2, e) in &path.cubics {
            let b = [cur, c1, c2, e];
            for k in 0..=64 {
                let q = bezier_point(&b, k as f64 / 64.0);
                let d = (0..n)
                    .map(|i| seg_d(q, pts[i], pts[(i + 1) % n]))
                    .fold(f64::MAX, f64::min);
                worst = worst.max(d);
            }
            cur = e;
        }
        worst
    }

    #[test]
    fn fit_honors_tolerance_on_sparse_straight_edges() {
        // A trapezoid ring in the shape the boundary tracer emits: the slanted
        // top carries a vertex every 5px, the other three straight edges only
        // their endpoints. No corners passed, as corner detection misses
        // shallow ones. The bound must hold everywhere on the curve, not just
        // at the vertices (regression for the bulge across sparse edges).
        let mut pts: Vec<V> = (0..=60).map(|k| (5.0 * k as f64, 0.5 * k as f64)).collect();
        pts.push((300.0, 150.0));
        pts.push((0.0, 150.0));
        let tol = 3.0;
        let fit = fit_closed(&pts, &[], tol, None).unwrap();
        let dev = fit_deviation(&fit, &pts);
        assert!(dev <= tol + 0.5, "max deviation {dev} > tol {tol}");
    }

    #[test]
    fn fit_honors_tolerance_with_corners_cut() {
        // The same sparse ring with the true corners given as cuts.
        let mut pts: Vec<V> = (0..=60).map(|k| (5.0 * k as f64, 0.5 * k as f64)).collect();
        pts.push((300.0, 150.0));
        pts.push((0.0, 150.0));
        let corners = vec![0, 60, 61, 62];
        let tol = 3.0;
        let fit = fit_closed(&pts, &corners, tol, None).unwrap();
        let dev = fit_deviation(&fit, &pts);
        assert!(dev <= tol + 0.5, "max deviation {dev} > tol {tol}");
    }

    #[test]
    fn seamed_fit_without_spans_is_fit_closed() {
        // The seamed entry with no spans must reproduce fit_closed exactly,
        // so a shape with no shared stretch stays byte-identical.
        let (pts, corners, _) = bumped_ring();
        let base = fit_closed(&pts, &corners, 0.5, None).unwrap();
        let (same, spans) = fit_closed_seamed(&pts, &corners, 0.5, None, &[], 1.0).unwrap();
        assert!(spans.is_empty());
        assert_eq!(base.start, same.start);
        assert_eq!(base.cubics, same.cubics);
    }

    #[test]
    fn seamed_simplify_without_spans_is_simplify_closed() {
        let h = hexagon();
        let (same, spans) = simplify_closed_seamed(&h, 1000.0, PI, &[]);
        assert!(spans.is_empty());
        assert_eq!(simplify_closed(&h, 1000.0, PI).cubics, same.cubics);
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
