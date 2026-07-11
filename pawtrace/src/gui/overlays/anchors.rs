//! The anchors overlay: the active view's finalized vector paths drawn as
//! canvas geometry over the Fit and Simplify views, so what's on screen is the
//! actual trace rather than a pre-fit approximation. On hover it draws the
//! topmost path under the cursor as its true bezier outline plus a dot on
//! every anchor; while the show-all modifier is held it draws every path the
//! same way.

use super::OverlayCtx;
use crate::gui::app::App;
use crate::gui::compute::{LayerTrace, TraceOutput};
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

/// The active subview's finalized trace output, read off-session from the fit
/// or simplify memo. `None` off the Fit and Simplify views, or before that
/// stage has run for the selected layer.
pub(super) fn read(app: &App, subview: Option<SubView>) -> Option<TraceOutput> {
    let sess = app.session()?;
    let stages = sess.stages.peek(sess.selected_layer)?;

    // Reading each memo's current value, never its key: the overlay shows
    // whatever trace the view is actually displaying, not what a key claims.
    match subview? {
        SubView::Fit => stages.fit.current(),
        SubView::Simplify => stages.simplify.current(),
        _ => None,
    }
}

/// The anchors over the Fit or Simplify view, or nothing on any other view or
/// before its trace has been produced.
pub fn overlay<'a>(ctx: &OverlayCtx<'a>) -> Option<Element<'a, Msg>> {
    if !matches!(ctx.subview, Some(SubView::Fit | SubView::Simplify)) {
        return None;
    }

    let out = ctx.active_trace.clone()?;
    let dims = ctx.dims?;

    let overlay = AnchorOverlay {
        trace: out.trace,
        scale: out.scale,
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
        } else if let Some((ci, pi)) = super::hit::hovered(&self.trace, scale, bounds, cursor, &vp)
        {
            draw_path(&mut frame, &self.trace[ci].1[pi], scale, &vp);
        }

        vec![frame.into_geometry()]
    }
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
