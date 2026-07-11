//! The pin overlay: the selected layer's pins as accent rings, each with a
//! wider protected-neighborhood ring that brightens while the cursor is inside
//! it. Drawn on every view where the selected layer has pins.

use super::OverlayCtx;
use crate::gui::msg::{Msg, StripView};
use crate::gui::view::theme;
use crate::gui::view::viewport::Viewport;
use iced::mouse;
use iced::widget::canvas::{Action, Event, Frame, Geometry, Path, Program, Stroke};
use iced::{Color, Element, Length, Point, Rectangle};

const RING_R: f32 = 3.75;
const RING_W: f32 = 2.5;
const OUTER_R: f32 = 9.5;
const OUTER_W: f32 = 2.0;
const OUTER_A: f32 = 0.30;
const OUTER_HOVER_A: f32 = 0.55;

/// The pin overlay for the selected layer, or nothing when it has no pins to
/// draw or nothing is rendered to align them against.
pub fn overlay<'a>(ctx: &OverlayCtx<'a>) -> Option<Element<'a, Msg>> {
    let dims = ctx.dims?;

    if ctx.pins.is_empty() {
        return None;
    }

    let overlay = PinOverlay {
        dims,
        zoom: ctx.zoom,
        pan: ctx.pan,
        pins: ctx.pins,
        offset: ctx.offset,
        view: ctx.view,
    };

    Some(
        iced::widget::canvas(overlay)
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
    )
}

struct PinOverlay<'a> {
    /// The shown art's crop-space dimensions, matching the preview's viewport.
    dims: (f32, f32),
    zoom: Option<f32>,
    pan: iced::Vector,
    pins: &'a [[u32; 2]],
    offset: (u32, u32),
    view: StripView,
}

impl PinOverlay<'_> {
    /// Screen position of a pin, or `None` when it maps outside the shown
    /// image. Pins are stored in document source px; the crop-space coordinate
    /// is the pin itself on Document and the pin minus the layer offset on a
    /// stage view. `dims` are the shown image's crop-space dimensions.
    fn screen(&self, pin: [u32; 2], vp: &Viewport, dims: (f32, f32)) -> Option<Point> {
        let (px, py) = (pin[0] as f32, pin[1] as f32);
        let (ox, oy) = (self.offset.0 as f32, self.offset.1 as f32);

        let (ix, iy) = match self.view {
            StripView::Document => (px, py),
            StripView::Phase(_) => (px - ox, py - oy),
        };

        if ix < 0.0 || iy < 0.0 || ix >= dims.0 || iy >= dims.1 {
            return None;
        }

        Some(vp.to_screen(ix, iy))
    }
}

impl Program<Msg> for PinOverlay<'_> {
    type State = ();

    fn update(
        &self,
        _state: &mut (),
        event: &Event,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Option<Action<Msg>> {
        // Repaint so the hover neighborhood ring follows the cursor. Returning
        // no message leaves the event uncaptured, so the preview beneath still
        // receives pan and zoom gestures.
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
        let cur = cursor.position_in(bounds);

        for &pin in self.pins {
            let Some(c) = self.screen(pin, &vp, self.dims) else {
                continue;
            };

            // The protected neighborhood: brighter while the cursor is inside it.
            let hovered = cur.is_some_and(|cur| {
                let (dx, dy) = (cur.x - c.x, cur.y - c.y);
                dx * dx + dy * dy <= OUTER_R * OUTER_R
            });

            let a = if hovered { OUTER_HOVER_A } else { OUTER_A };

            frame.stroke(
                &Path::circle(c, OUTER_R),
                Stroke::default()
                    .with_color(Color { a, ..theme::ACCENT })
                    .with_width(OUTER_W),
            );

            frame.stroke(
                &Path::circle(c, RING_R),
                Stroke::default()
                    .with_color(theme::ACCENT)
                    .with_width(RING_W),
            );
        }

        vec![frame.into_geometry()]
    }
}
