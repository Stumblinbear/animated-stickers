//! The transparency checkerboard behind the preview art: the bottom canvas of
//! the preview stack, screen-anchored so it never pans or zooms with the art
//! drawn above it.

use crate::gui::msg::Msg;
use iced::mouse;
use iced::widget::canvas::{Cache, Frame, Geometry, Program};
use iced::{Color, Point, Rectangle, Size};

const CHECK_LIGHT: Color = Color {
    r: 0.16,
    g: 0.16,
    b: 0.18,
    a: 1.0,
};
const CHECK_DARK: Color = Color {
    r: 0.11,
    g: 0.11,
    b: 0.13,
    a: 1.0,
};
const TILE: f32 = 8.0;

/// The screen-anchored checkerboard canvas program, filling its whole canvas
/// with the two-tone alpha checker.
pub(super) struct Checkerboard;

impl Program<Msg> for Checkerboard {
    type State = Cache;

    fn draw(
        &self,
        cache: &Cache,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        // The pattern is a function of the size alone, and the cache
        // re-records itself on a size change, so no manual key is needed.
        vec![cache.draw(renderer, bounds.size(), |frame| {
            checkerboard(frame, bounds.size());
        })]
    }
}

/// Fills a `size`d frame with the checker tiles.
fn checkerboard(frame: &mut Frame, size: Size) {
    frame.fill_rectangle(Point::ORIGIN, size, CHECK_LIGHT);

    let cols = (size.width / TILE).ceil() as i32;
    let rows = (size.height / TILE).ceil() as i32;

    for row in 0..rows {
        for col in 0..cols {
            if (row + col) % 2 == 0 {
                continue;
            }

            let px = col as f32 * TILE;
            let py = row as f32 * TILE;
            let pw = ((col + 1) as f32 * TILE).min(size.width) - px;
            let ph = ((row + 1) as f32 * TILE).min(size.height) - py;

            frame.fill_rectangle(Point::new(px, py), Size::new(pw, ph), CHECK_DARK);
        }
    }
}
