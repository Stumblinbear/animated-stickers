//! The Protect tool: a brush that marks areas to keep through color merge and
//! region absorption. The painted mask is a layer property, not tool state, so
//! only the brush parameters live here. Offered on the Colors and Shapes
//! phases, where merge and absorption run. Painting itself is a future task, so
//! its canvas presses do nothing yet.

use super::{edit, ToolMsg};
use crate::gui::msg::{Msg as AppMsg, Phase, StripView};
use crate::gui::phases::SubView;
use crate::gui::view::{icons, theme};
use iced::widget::{button, column, row, slider, text};
use iced::{Alignment, Element};

pub const ICON: char = icons::PAINTBRUSH;

/// The protect brush parameters.
pub struct State {
    pub size: f32,
    pub erase: bool,
}

impl Default for State {
    fn default() -> Self {
        Self { size: 40.0, erase: false }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Size(f32),
    Erase(bool),
    /// Clears the painted mask. Absorbed with no effect until painting lands.
    Clear,
}

pub fn update(state: &mut State, msg: Msg) {
    match msg {
        Msg::Size(v) => state.size = v,
        Msg::Erase(v) => state.erase = v,
        Msg::Clear => {}
    }
}

/// Offered on the Colors and Shapes phases.
pub fn applies(view: StripView, _sub: Option<SubView>) -> bool {
    matches!(view, StripView::Phase(Phase::Colors | Phase::Shapes))
}

pub fn flyout(state: &State) -> Element<'_, AppMsg> {
    let mode = |glyph: char, label: &'static str, erase: bool| {
        button(
            row![icons::icon(glyph).size(11), text(label).size(11)]
                .spacing(5)
                .align_y(Alignment::Center),
        )
        .on_press(edit(ToolMsg::Protect(Msg::Erase(erase))))
        .style(theme::chip(state.erase == erase))
        .padding([3, 8])
    };
    let body = column![
        super::title("PROTECT BRUSH"),
        super::slider_row(
            "Size",
            format!("{}", state.size as i32),
            slider(4.0..=200.0, state.size, |v| edit(ToolMsg::Protect(Msg::Size(v)))),
        ),
        row![mode(icons::PAINTBRUSH, "Paint", false), mode(icons::ERASER, "Erase", true)].spacing(4),
        super::clear_button(edit(ToolMsg::Protect(Msg::Clear))),
    ]
    .spacing(8);
    super::card(body.into())
}
