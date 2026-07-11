//! The seams overlay: the stitched shared-boundary spans of the active view's
//! trace, drawn as a thick highlight over the art on the Fit and Simplify
//! views. A span's cubic run is a stretch a pair of sibling shapes fit once and
//! spliced into both, so the highlight marks where two shapes share an
//! identical edge. It follows the anchors overlay path for path: the hovered
//! path's spans by default, every path's while the show-all modifier is held,
//! drawn under the anchors so the seams read against the outline on top.

use super::OverlayCtx;
use crate::fit::AnchorSpan;
use crate::gui::compute::LayerTrace;
use crate::gui::msg::Msg;
use crate::gui::phases::SubView;
use crate::gui::view::theme;
use crate::gui::view::viewport::Viewport;
use crate::pipeline::TraceSeams;
use crate::trace::TracedPath;
use iced::mouse;
use iced::widget::canvas::{Action, Event, Frame, Geometry, Path, Program, Stroke};
use iced::{Element, Length, Rectangle, Vector};
use std::sync::Arc;

/// Screen-px width of the seam highlight, wider than the anchor outline so the
/// stitched stretch reads as a band beneath it. Constant in screen space, so it
/// stays visually fixed as the preview zooms.
const SEAM_W: f32 = 3.5;

/// The stitched-span highlight over the Fit or Simplify view, or nothing on any
/// other view or before its trace has been produced. Gated exactly like the
/// anchors overlay, off the same active trace output.
pub fn overlay<'a>(ctx: &OverlayCtx<'a>) -> Option<Element<'a, Msg>> {
    if !matches!(ctx.subview, Some(SubView::Fit | SubView::Simplify)) {
        return None;
    }

    let out = ctx.active_trace.clone()?;
    let dims = ctx.dims?;

    let overlay = SeamOverlay {
        trace: out.trace,
        seams: out.seams,
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

struct SeamOverlay {
    trace: Arc<LayerTrace>,
    /// The shared-stretch sidecar of `trace`, grouped run for run: `seams[ci]`
    /// pairs with the paths of `trace`'s `ci`th color run.
    seams: Arc<TraceSeams>,
    scale: u32,
    /// The shown art's crop-space dimensions, matching the preview so the seams
    /// land on the same rectangle as the trace they mark.
    dims: (f32, f32),
    zoom: Option<f32>,
    pan: Vector,
    show_all: bool,
}

impl SeamOverlay {
    /// Strokes every span of path `(ci, pi)`.
    fn draw_path_spans(&self, frame: &mut Frame, ci: usize, pi: usize, scale: f32, vp: &Viewport) {
        let Some(spans) = self.seams.get(ci).and_then(|c| c.get(pi)) else {
            return;
        };

        for span in spans {
            draw_span(frame, &self.trace[ci].1[pi], span, scale, vp);
        }
    }
}

impl Program<Msg> for SeamOverlay {
    type State = ();

    fn update(
        &self,
        _state: &mut (),
        event: &Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Option<Action<Msg>> {
        // Repaint so the hovered path's seams follow the cursor, exactly like
        // the anchors overlay. Returning no message leaves the event
        // uncaptured, so the preview beneath still pans and zooms.
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
            for (ci, (_, paths)) in self.trace.iter().enumerate() {
                for pi in 0..paths.len() {
                    self.draw_path_spans(&mut frame, ci, pi, scale, &vp);
                }
            }
        } else if let Some((ci, pi)) = super::hit::hovered(&self.trace, scale, bounds, cursor, &vp)
        {
            self.draw_path_spans(&mut frame, ci, pi, scale, &vp);
        }

        vec![frame.into_geometry()]
    }
}

/// Strokes `span`'s cubic run over `p`: the segments from anchor `span.start` to
/// anchor `span.end`, wrapping past the last segment, with `start == end`
/// covering the whole ring. Path coordinates are scaled space; dividing by
/// `scale` gives crop px, which `vp` maps to screen.
fn draw_span(frame: &mut Frame, p: &TracedPath, span: &AnchorSpan, scale: f32, vp: &Viewport) {
    let n = p.cubics.len();
    if n == 0 {
        return;
    }

    let pt = |(x, y): (f64, f64)| vp.to_screen(x as f32 / scale, y as f32 / scale);
    let anchor = |i: usize| if i == 0 { p.start } else { p.cubics[i - 1].2 };
    let count = if span.start == span.end {
        n
    } else {
        (span.end + n - span.start) % n
    };

    let run = Path::new(|b| {
        b.move_to(pt(anchor(span.start)));

        for t in 0..count {
            let (c1, c2, end) = p.cubics[(span.start + t) % n];
            b.bezier_curve_to(pt(c1), pt(c2), pt(end));
        }
    });

    frame.stroke(
        &run,
        Stroke::default().with_color(theme::ACCENT).with_width(SEAM_W),
    );
}
