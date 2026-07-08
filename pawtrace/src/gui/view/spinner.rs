//! The shared processing spinner: a small rotating accent arc, used wherever
//! a single stage or layer is computing (strip chips, inspector headers, layer
//! rows). One implementation, sized per call site.

use super::{anim, theme};
use crate::gui::msg::Msg;
use iced::mouse;
use iced::widget::canvas::{path::Arc, Frame, Geometry, Path, Program, Stroke};
use iced::widget::canvas;
use iced::{Element, Length, Radians, Rectangle, Renderer, Theme};
use std::f32::consts::TAU;
use std::time::Instant;

/// A spinner of `size` px whose arc rotates from the frame clock `now`.
pub fn spinner<'a>(now: Instant, size: f32) -> Element<'a, Msg> {
    canvas(Spinner { now, size })
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .into()
}

struct Spinner {
    now: Instant,
    size: f32,
}

// A three-quarter arc reads as a spinner. A full ring would look static.
const SWEEP: f32 = 0.75 * TAU;
const ROTATION_SECS: f32 = 0.9;

impl Program<Msg> for Spinner {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let width = (self.size * 0.14).max(1.2);
        let radius = self.size / 2.0 - width;
        let center = frame.center();
        let start = anim::phase(self.now, ROTATION_SECS) * TAU;
        let arc = Path::new(|b| {
            b.arc(Arc {
                center,
                radius,
                start_angle: Radians(start),
                end_angle: Radians(start + SWEEP),
            });
        });
        frame.stroke(
            &arc,
            Stroke::default()
                .with_color(theme::ACCENT)
                .with_width(width),
        );
        vec![frame.into_geometry()]
    }
}
