//! The anchors overlay: the active view's finalized vector paths drawn as
//! canvas geometry over the Fit and Simplify views, so what's on screen is the
//! actual trace rather than a pre-fit approximation. On hover it draws the
//! topmost path under the cursor as its true bezier outline plus a dot on
//! every anchor; while the show-all modifier is held it draws every path the
//! same way.

use super::OverlayCtx;
use crate::gui::app::App;
use crate::gui::compute::LayerTrace;
use crate::gui::msg::Msg;
use crate::gui::phases::SubView;
use crate::gui::view::viewport::Viewport;
use crate::trace::TracedPath;
use iced::mouse;
use iced::widget::canvas::{Action, Event, Frame, Geometry, Path, Program, Stroke};
use iced::{Color, Element, Length, Rectangle, Vector};
use std::sync::Arc;

/// The outline and anchor-dot colors.
const LINE: Color = rgb(0x6e, 0xa8, 0xff);
const ANCHOR: Color = rgb(0xff, 0x9d, 0x3c);

/// Screen-px line width and anchor-dot radius. Constant in screen space, so
/// they stay visually fixed as the preview zooms, matching the pins overlay.
const LINE_W: f32 = 1.25;
const DOT_R: f32 = 2.25;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}

/// The active subview's finalized trace and the supersample scale its
/// coordinates are expressed at, read off-session from the fit or simplify
/// memo. `None` off the Fit and Simplify views, or before that stage has run
/// for the selected layer.
pub(super) fn read(app: &App, subview: Option<SubView>) -> Option<(Arc<LayerTrace>, u32)> {
    let sess = app.session()?;
    let stages = sess.stages.peek(sess.selected_layer)?;

    // Reading each memo's current value, never its key: the overlay shows
    // whatever trace the view is actually displaying, not what a key claims.
    let out = match subview? {
        SubView::Fit => stages.fit.current()?,
        SubView::Simplify => stages.simplify.current()?,
        _ => return None,
    };

    Some((out.trace, out.scale))
}

/// The anchors over the Fit or Simplify view, or nothing on any other view or
/// before its trace has been produced.
pub fn overlay<'a>(ctx: &OverlayCtx<'a>) -> Option<Element<'a, Msg>> {
    if !matches!(ctx.subview, Some(SubView::Fit | SubView::Simplify)) {
        return None;
    }

    let (trace, scale) = ctx.active_trace.clone()?;
    let dims = ctx.dims?;

    let overlay = AnchorOverlay {
        trace,
        scale,
        dims,
        zoom: ctx.zoom,
        pan: ctx.pan,
        show_all: ctx.show_all_anchors,
    };

    Some(
        iced::widget::canvas(overlay)
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
    )
}

struct AnchorOverlay {
    trace: Arc<LayerTrace>,
    scale: u32,
    /// The shown art's crop-space dimensions, matching the preview so the trace
    /// lands on the same rectangle as the fill it outlines.
    dims: (f32, f32),
    zoom: Option<f32>,
    pan: Vector,
    show_all: bool,
}

impl Program<Msg> for AnchorOverlay {
    type State = ();

    fn update(
        &self,
        _state: &mut (),
        event: &Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Option<Action<Msg>> {
        // Repaint so the hovered path follows the cursor. Returning no message
        // leaves the event uncaptured, so the preview beneath still pans and
        // zooms.
        let moved = matches!(event, Event::Mouse(mouse::Event::CursorMoved { .. }));

        moved.then(Action::request_redraw)
    }

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let vp = Viewport::resolve(bounds.size(), self.dims, self.zoom, self.pan);
        let scale = self.scale as f32;

        if self.show_all {
            for (_, paths) in self.trace.iter() {
                for p in paths {
                    draw_path(&mut frame, p, scale, &vp);
                }
            }
        } else if let Some(cur) = cursor.position_in(bounds) {
            let ci = vp.to_image(cur);
            let (sx, sy) = (ci.x as f64 * scale as f64, ci.y as f64 * scale as f64);

            if let Some(p) = hit_path(&self.trace, sx, sy) {
                draw_path(&mut frame, p, scale, &vp);
            }
        }

        vec![frame.into_geometry()]
    }
}

/// The last path in paint order (iterating color groups, then their paths, in
/// order) whose flattened outline contains scaled-space point `(sx, sy)`, or
/// `None` when no path does. Paths paint bottom-first, so the last containing
/// path is the topmost.
fn hit_path(trace: &LayerTrace, sx: f64, sy: f64) -> Option<&TracedPath> {
    let mut hit = None;

    for (_, paths) in trace {
        for p in paths {
            if contains(p, sx, sy) {
                hit = Some(p);
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

/// Draws a dot on every anchor of `p`: its start point and each cubic's end
/// point.
fn draw_anchors(frame: &mut Frame, p: &TracedPath, scale: f32, vp: &Viewport) {
    let pt = |(x, y): (f64, f64)| vp.to_screen(x as f32 / scale, y as f32 / scale);

    frame.fill(&Path::circle(pt(p.start), DOT_R), ANCHOR);

    for &(_, _, end) in &p.cubics {
        frame.fill(&Path::circle(pt(end), DOT_R), ANCHOR);
    }
}

/// Draws `p`'s true bezier outline plus its anchor dots. Path coordinates are
/// scaled space; dividing by `scale` gives crop px, which `vp` maps to screen.
fn draw_path(frame: &mut Frame, p: &TracedPath, scale: f32, vp: &Viewport) {
    let pt = |(x, y): (f64, f64)| vp.to_screen(x as f32 / scale, y as f32 / scale);

    let outline = Path::new(|b| {
        b.move_to(pt(p.start));

        for &(c1, c2, end) in &p.cubics {
            b.bezier_curve_to(pt(c1), pt(c2), pt(end));
        }

        b.close();
    });

    frame.stroke(
        &outline,
        Stroke::default().with_color(LINE).with_width(LINE_W),
    );

    draw_anchors(frame, p, scale, vp);
}
