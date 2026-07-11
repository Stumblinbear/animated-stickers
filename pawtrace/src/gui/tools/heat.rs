//! The Heat tool: a brush that weights how much detail a region earns. The
//! painted heat field is a layer property, not tool state, so only the brush
//! parameters live here. Offered on every phase. Painting itself is a future
//! task, so its canvas presses do nothing yet.

use super::{edit, ToolMsg};
use crate::gui::msg::{Msg as AppMsg, StripView};
use crate::gui::phases::SubView;
use crate::gui::view::{icons, theme};
use iced::widget::{column, container, row, slider, text, toggler};
use iced::{Alignment, Color, Element, Length};

pub const ICON: char = icons::FLAME;
/// The brush press is absorbed with no effect yet, but capturing it keeps a
/// left-drag from panning.
pub const CAPTURES_PRESS: bool = true;
pub const CURSOR: iced::mouse::Interaction = iced::mouse::Interaction::Crosshair;

/// The heat brush parameters.
pub struct State {
    pub flow: f32,
    pub max: f32,
    pub size: f32,
    pub show: bool,
}

impl Default for State {
    fn default() -> Self {
        Self { flow: 0.5, max: 4.0, size: 64.0, show: true }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Flow(f32),
    Max(f32),
    Size(f32),
    Show(bool),
    /// Resets the painted heat field. Absorbed with no effect until painting lands.
    Reset,
}

pub fn update(state: &mut State, msg: Msg) {
    match msg {
        Msg::Flow(v) => state.flow = v,
        Msg::Max(v) => state.max = v,
        Msg::Size(v) => state.size = v,
        Msg::Show(v) => state.show = v,
        Msg::Reset => {}
    }
}

/// Offered on every phase's heat channel.
pub fn applies(view: StripView, _sub: Option<SubView>) -> bool {
    matches!(view, StripView::Phase(_))
}

pub fn flyout(state: &State) -> Element<'_, AppMsg> {
    let legend = column![
        legend_row(icons::FLAME, theme::ACCENT, "L", "warm"),
        legend_row(icons::SNOWFLAKE, theme::MUTED, "Alt+L", "cool"),
        legend_row(icons::MINUS, theme::MUTED, "R", "neutralize"),
    ]
    .spacing(3);
    let show = row![
        text("Show overlay").size(11).color(theme::MUTED).width(Length::Fill),
        toggler(state.show).on_toggle(|v| edit(ToolMsg::Heat(Msg::Show(v)))).size(16),
    ]
    .align_y(Alignment::Center);
    let body = column![
        super::title("HEAT BRUSH"),
        super::slider_row(
            "Flow",
            format!("{:.1}", state.flow),
            slider(0.05..=1.0, state.flow, |v| edit(ToolMsg::Heat(Msg::Flow(v)))),
        ),
        super::slider_row(
            "Max",
            format!("{:.1}×", state.max),
            slider(1.5..=8.0, state.max, |v| edit(ToolMsg::Heat(Msg::Max(v)))),
        ),
        super::slider_row(
            "Size",
            format!("{}", state.size as i32),
            slider(8.0..=256.0, state.size, |v| edit(ToolMsg::Heat(Msg::Size(v)))),
        ),
        container(legend).style(theme::card).padding(6).width(Length::Fill),
        show,
        super::clear_button(edit(ToolMsg::Heat(Msg::Reset))),
    ]
    .spacing(8);
    super::card(body.into())
}

fn legend_row<'a>(
    glyph: char,
    tint: Color,
    gesture: &'a str,
    meaning: &'a str,
) -> Element<'a, AppMsg> {
    row![
        icons::icon(glyph).size(11).color(tint),
        crate::gui::view::widgets::mono(gesture).size(10).color(theme::TEXT).width(46),
        text(meaning).size(10).color(theme::MUTED),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .into()
}
