//! A fixed 5x7 uppercase bitmap font for the contact-sheet stage labels.
//!
//! The goldens are byte-compared PNGs, so label pixels must be identical on
//! every platform and toolchain version. Hand-coded bitmaps guarantee that
//! where a rasterized TrueType font would not.

use pawtrace::color::Srgb;
use image::{Rgba, RgbaImage};

const GLYPH_W: u32 = 5;
const GLYPH_H: u32 = 7;
/// Integer upscale applied to every glyph pixel.
const SCALE: u32 = 3;
/// Blank columns between glyphs, in pre-scale pixels.
const TRACKING: u32 = 1;

/// Rendered height of one line of text, in pixels.
pub const TEXT_H: u32 = GLYPH_H * SCALE;

/// Rendered width of `text` in pixels: the width [`draw_text`] needs as its
/// `max_w` to draw the whole string without dropping a glyph. Zero for empty.
pub fn text_width(text: &str) -> u32 {
    match text.chars().count() as u32 {
        0 => 0,
        n => (n - 1) * (GLYPH_W + TRACKING) * SCALE + GLYPH_W * SCALE,
    }
}

/// Draws `text` in `color` with its top-left at `(x, y)`, left to right.
/// Glyphs that would extend past `x + max_w` are dropped, so a label never
/// spills out of its tile column.
pub fn draw_text(dst: &mut RgbaImage, text: &str, x: u32, y: u32, max_w: u32, color: Srgb) {
    let px: Rgba<u8> = color.into();
    let advance = (GLYPH_W + TRACKING) * SCALE;
    let mut cx = x;
    for ch in text.chars() {
        if cx + GLYPH_W * SCALE > x + max_w {
            break;
        }
        let g = glyph(ch);
        for (row, bits) in g.iter().enumerate() {
            for col in 0..GLYPH_W {
                // 0x10 is the leftmost of the five columns.
                if bits & (0x10 >> col) != 0 {
                    fill_block(dst, cx + col * SCALE, y + row as u32 * SCALE, px);
                }
            }
        }
        cx += advance;
    }
}

fn fill_block(dst: &mut RgbaImage, x: u32, y: u32, px: Rgba<u8>) {
    for dy in 0..SCALE {
        for dx in 0..SCALE {
            if x + dx < dst.width() && y + dy < dst.height() {
                dst.put_pixel(x + dx, y + dy, px);
            }
        }
    }
}

/// Each row's low 5 bits, `0x10` leftmost. Only the letters used by the stage
/// labels are defined; any other character renders blank.
fn glyph(c: char) -> [u8; 7] {
    match c {
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'G' => [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110],
        'I' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111],
        'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b11001, 0b10101, 0b10101, 0b10011, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01110, 0b10001, 0b10000, 0b01110, 0b00001, 0b10001, 0b01110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        _ => [0; 7],
    }
}
