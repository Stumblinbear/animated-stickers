//! The Select tool: the default pointer. On the Document view a click
//! hit-tests the layer stack (handled by the canvas as a document interaction,
//! not a tool press); elsewhere it only pans. It is offered on every view.

use crate::gui::msg::StripView;
use crate::gui::phases::SubView;
use crate::gui::view::icons;

pub const ICON: char = icons::POINTER;
/// A Select left-press pans (and may click-select on release), so the canvas
/// keeps it rather than routing it to the tool.
pub const CAPTURES_PRESS: bool = false;
pub const CURSOR: iced::mouse::Interaction = iced::mouse::Interaction::Grab;

/// Select is always available.
pub fn applies(_view: StripView, _sub: Option<SubView>) -> bool {
    true
}
