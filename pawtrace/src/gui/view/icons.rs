//! Lucide glyphs (assets/lucide.ttf, ISC license) drawn through the bundled
//! icon font. The default font has no glyphs for arrows or symbols, so every
//! icon in the UI comes from here.

use iced::widget::{text, Text};
use iced::Font;

const FONT: Font = Font::with_name("lucide");

// Codepoints read out of the shipped font's cmap and post tables.
pub const LOCK: char = '\u{e10b}'; // lock
pub const CHEVRON_RIGHT: char = '\u{e06f}'; // chevron-right
pub const CHEVRON_DOWN: char = '\u{e06d}'; // chevron-down
pub const EYE: char = '\u{e0ba}'; // eye
pub const EYE_OFF: char = '\u{e0bb}'; // eye-off
pub const POINTER: char = '\u{e11f}'; // mouse-pointer
pub const PIN: char = '\u{e259}'; // pin
pub const RESET: char = '\u{e148}'; // rotate-ccw
pub const CLOSE: char = '\u{e1b2}'; // x
pub const PLUS: char = '\u{e13d}'; // plus
pub const MINUS: char = '\u{e11c}'; // minus
pub const FIT: char = '\u{e113}'; // maximize-2
pub const FILE: char = '\u{e0c0}'; // file
pub const TAG: char = '\u{e17f}'; // tag
pub const BAN: char = '\u{e051}'; // ban
pub const COPY: char = '\u{e09e}'; // copy
pub const PENCIL: char = '\u{e1f9}'; // pencil
pub const TRASH: char = '\u{e18d}'; // trash
pub const CIRCLE: char = '\u{e076}'; // circle
pub const CIRCLE_DOT: char = '\u{e345}'; // circle-dot
pub const LIBRARY: char = '\u{e100}'; // library

/// An icon glyph as a `Text` widget, ready to size and color.
pub fn icon<'a>(code: char) -> Text<'a> {
    text(code.to_string()).font(FONT)
}
