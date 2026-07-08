//! The transparency checkerboard drawn behind the preview art.

use iced::widget::canvas::Frame;
use iced::{Color, Point, Size};

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

/// Fills the display rect with the two-tone alpha checker, iterating only the
/// tiles visible within `size` so a deep zoom stays cheap.
pub(super) fn checkerboard(frame: &mut Frame, origin: Point, dw: f32, dh: f32, size: Size) {
    let x0 = origin.x.max(0.0);
    let y0 = origin.y.max(0.0);
    let x1 = (origin.x + dw).min(size.width);
    let y1 = (origin.y + dh).min(size.height);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    frame.fill_rectangle(Point::new(x0, y0), Size::new(x1 - x0, y1 - y0), CHECK_LIGHT);
    let col0 = ((x0 - origin.x) / TILE).floor() as i32;
    let col1 = ((x1 - origin.x) / TILE).ceil() as i32;
    let row0 = ((y0 - origin.y) / TILE).floor() as i32;
    let row1 = ((y1 - origin.y) / TILE).ceil() as i32;
    for row in row0..row1 {
        for col in col0..col1 {
            if (row + col) % 2 == 0 {
                continue;
            }
            let px = (origin.x + col as f32 * TILE).max(x0);
            let py = (origin.y + row as f32 * TILE).max(y0);
            let pw = (origin.x + (col + 1) as f32 * TILE).min(x1) - px;
            let ph = (origin.y + (row + 1) as f32 * TILE).min(y1) - py;
            if pw > 0.0 && ph > 0.0 {
                frame.fill_rectangle(Point::new(px, py), Size::new(pw, ph), CHECK_DARK);
            }
        }
    }
}
