//! The trace hit-test shared by the anchors and seams overlays: which path is
//! under the cursor. Both overlays highlight the same hovered path, so the
//! resolution lives here once and each reads the result against its own data.

use crate::gui::compute::LayerTrace;
use crate::gui::view::viewport::Viewport;
use crate::trace::TracedPath;
use iced::mouse;
use iced::Rectangle;

/// The topmost path of `trace` under the cursor, as its `(color run, path)`
/// indices, or `None` when the cursor is outside `bounds` or over no path.
/// `scale` is the trace's supersample scale and `vp` the overlay's resolved
/// viewport, so the cursor maps into the same scaled space the paths live in.
pub(super) fn hovered(
    trace: &LayerTrace,
    scale: f32,
    bounds: Rectangle,
    cursor: mouse::Cursor,
    vp: &Viewport,
) -> Option<(usize, usize)> {
    let cur = cursor.position_in(bounds)?;
    let ci = vp.to_image(cur);
    let (sx, sy) = (ci.x as f64 * scale as f64, ci.y as f64 * scale as f64);

    hit_path(trace, sx, sy)
}

/// The last path in paint order (iterating color groups, then their paths, in
/// order) whose flattened outline contains scaled-space point `(sx, sy)`,
/// returned as its `(color run, path)` indices into `trace`, or `None` when no
/// path does. Paths paint bottom-first, so the last containing path is the
/// topmost.
fn hit_path(trace: &LayerTrace, sx: f64, sy: f64) -> Option<(usize, usize)> {
    let mut hit = None;

    for (ci, (_, paths)) in trace.iter().enumerate() {
        for (pi, p) in paths.iter().enumerate() {
            if contains(p, sx, sy) {
                hit = Some((ci, pi));
            }
        }
    }

    hit
}

/// Whether scaled-space point `(sx, sy)` falls inside `p`'s flattened outline,
/// by the even-odd rule. Bbox-prefiltered against the control-point hull: a
/// point outside every control point's bounding box is outside the curve it
/// bounds, so the prefilter can only reject, never wrongly accept.
fn contains(p: &TracedPath, sx: f64, sy: f64) -> bool {
    let (mut x0, mut y0, mut x1, mut y1) = (p.start.0, p.start.1, p.start.0, p.start.1);

    for &(c1, c2, end) in &p.cubics {
        for &(x, y) in &[c1, c2, end] {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }

    if sx < x0 || sx > x1 || sy < y0 || sy > y1 {
        return false;
    }

    point_in_polygon(&flatten(p), sx, sy)
}

/// Fixed subdivisions per cubic segment for the hit-test polyline: enough to
/// track the curve closely without the cost of an adaptive walk.
const FLATTEN_STEPS: usize = 8;

/// `p`'s closed outline as a polyline, start point first.
fn flatten(p: &TracedPath) -> Vec<(f64, f64)> {
    let mut pts = Vec::with_capacity(p.cubics.len() * FLATTEN_STEPS + 1);
    pts.push(p.start);

    let mut cur = p.start;

    for &(c1, c2, end) in &p.cubics {
        for i in 1..=FLATTEN_STEPS {
            let t = i as f64 / FLATTEN_STEPS as f64;
            pts.push(cubic_point(cur, c1, c2, end, t));
        }

        cur = end;
    }

    pts
}

/// The point at parameter `t` on the cubic bezier from `p0` to `p3` via
/// control points `p1`, `p2`.
fn cubic_point(
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    t: f64,
) -> (f64, f64) {
    let mt = 1.0 - t;
    let (a, b, c, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);

    (
        a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
        a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
    )
}

/// Even-odd point-in-polygon over closed ring `pts`.
fn point_in_polygon(pts: &[(f64, f64)], sx: f64, sy: f64) -> bool {
    let n = pts.len();

    if n < 3 {
        return false;
    }

    let mut inside = false;
    let mut j = n - 1;

    for i in 0..n {
        let (xi, yi) = pts[i];
        let (xj, yj) = pts[j];

        if (yi > sy) != (yj > sy) && sx < (xj - xi) * (sy - yi) / (yj - yi) + xi {
            inside = !inside;
        }

        j = i;
    }

    inside
}
