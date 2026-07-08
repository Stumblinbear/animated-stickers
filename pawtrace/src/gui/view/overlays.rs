//! Overlays drawn atop the preview art: the pin overlay, a transparent canvas
//! that draws the selected layer's pins, and [`marching_ants`], a reusable
//! dashed-outline primitive. The pins live in a separate canvas because iced
//! draws a canvas's images above its meshes within one layer, so pins drawn in
//! the preview's own frame fall under the art; a sibling canvas is a later
//! layer and renders over it.

use super::viewport::Viewport;
use super::{anim, theme};
use crate::gui::app::App;
use crate::gui::msg::{Msg, StripView};
use iced::mouse;
use iced::widget::canvas::{Action, Event, Frame, Geometry, LineDash, Path, Program, Stroke};
use iced::{Color, Element, Length, Point, Rectangle, Vector};
use std::time::Instant;

const RING_R: f32 = 3.75;
const RING_W: f32 = 2.5;
const OUTER_R: f32 = 9.5;
const OUTER_W: f32 = 2.0;
const OUTER_A: f32 = 0.30;
const OUTER_HOVER_A: f32 = 0.55;

/// Strokes `path` as a marching-ants outline: a thin dashed accent line whose
/// dash phase advances from `now`, so the dashes crawl along the path over a
/// half-second cycle. Widths are in the frame's coordinate space.
#[allow(dead_code)]
pub fn marching_ants(frame: &mut Frame, path: &Path, now: Instant) {
    // Spec §8's brushed-region treatment, kept as a reference implementation:
    // pins are the sole protection mechanism, so nothing draws this today.
    const SEGMENTS: [f32; 2] = [4.0, 4.0];
    const CYCLE_SECS: f32 = 0.5;
    let cycle: f32 = SEGMENTS.iter().sum();
    let offset = (anim::phase(now, CYCLE_SECS) * cycle) as usize;
    frame.stroke(
        path,
        Stroke {
            line_dash: LineDash { segments: &SEGMENTS, offset },
            ..Stroke::default().with_color(theme::ACCENT).with_width(1.0)
        },
    );
}

/// Builds the transparent pin overlay for the selected layer's pins. Draws
/// nothing when the selection is empty, since there is no layer to protect.
pub fn pin_overlay(app: &App) -> Element<'_, Msg> {
    let has_selection = app.session().is_some_and(|s| !s.selection.is_empty());
    let offset = app
        .session()
        .zip(app.doc())
        .and_then(|(s, doc)| doc.layers.get(s.selected_layer.index()))
        .map(|l| l.offset)
        .unwrap_or((0, 0));
    let overlay = PinOverlay {
        img: app.active_image().map(|i| i.size),
        zoom: app.session().and_then(|s| s.zoom()),
        pan: app.session().map(|s| s.pan()).unwrap_or(Vector::ZERO),
        pins: if has_selection {
            app.session().map(|s| s.cfg.pins.as_slice()).unwrap_or(&[])
        } else {
            &[]
        },
        offset,
        factor: app.view_density(),
        view: app.session().map(|s| s.view).unwrap_or_default(),
    };
    iced::widget::canvas(overlay)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

struct PinOverlay<'a> {
    img: Option<(u32, u32)>,
    zoom: Option<f32>,
    pan: Vector,
    pins: &'a [[u32; 2]],
    offset: (u32, u32),
    /// Screen-raster px per crop px for the shown view, matching the preview so
    /// pins resolve against the same crop-space viewport.
    factor: f32,
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
            StripView::Stage(_) => (px - ox, py - oy),
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
        (moved && !self.pins.is_empty()).then(Action::request_redraw)
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
        if let Some(img_px) = self.img {
            let dims = (img_px.0 as f32 / self.factor, img_px.1 as f32 / self.factor);
            let vp = Viewport::resolve(bounds.size(), dims, self.zoom, self.pan);
            let cur = cursor.position_in(bounds);
            for &pin in self.pins {
                let Some(c) = self.screen(pin, &vp, dims) else { continue };
                // The protected neighborhood: brighter while the cursor is
                // inside it.
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
                    Stroke::default().with_color(theme::ACCENT).with_width(RING_W),
                );
            }
        }
        vec![frame.into_geometry()]
    }
}
