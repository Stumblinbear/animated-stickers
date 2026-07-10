//! The floating tool rail: a translucent vertical strip in the preview's
//! top-left corner holding the canvas tools offered on the current view, with a
//! contextual fly-out beside it when the active tool has options.
//!
//! The rail overlays the artwork, so it is translucent and kept in the corner.
//! Which tools appear is a function of the view: the rail iterates
//! [`Tool::ALL`](crate::gui::tools::Tool::ALL) and keeps the tools that apply.

use super::{icons, theme};
use crate::gui::app::App;
use crate::gui::msg::{Msg, UiMsg};
use crate::gui::tools::{self, Tool};
use iced::widget::{button, column, container, row, space};
use iced::{Alignment, Background, Color, Element, Length};

/// Overlays the rail (and any active tool's fly-out) on the top-left of the
/// preview. Returns an empty overlay when no document is open.
pub fn tool_rail(app: &App) -> Element<'_, Msg> {
    let Some(view) = app.session().map(|s| s.view) else {
        return space().into();
    };
    let sub = app.active_subview();

    let mut rail = column![].spacing(4).padding(4).align_x(Alignment::Center);
    for t in Tool::ALL.into_iter().filter(|t| t.applies(view, sub)) {
        rail = rail.push(tool_button(t.icon(), t, app.tools.active));
    }
    let rail = container(rail).style(rail_style);

    let mut group = row![rail].spacing(8).align_y(Alignment::Start);
    if let Some(fly) = tools::flyout(&app.tools) {
        group = group.push(fly);
    }

    // Sit the rail a small margin into the top-left corner, out of the subject.
    container(group)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced::alignment::Horizontal::Left)
        .align_y(iced::alignment::Vertical::Top)
        .padding(12)
        .into()
}

fn tool_button<'a>(glyph: char, this: Tool, active: Tool) -> Element<'a, Msg> {
    button(icons::icon(glyph).size(14))
        .on_press(Msg::Ui(UiMsg::Tool(this)))
        .style(theme::tool_button(this == active))
        .padding([6, 7])
        .into()
}

/// The rail surface: translucent so the art stays visible beneath it.
fn rail_style(_: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color { a: 0.82, ..theme::SURFACE })),
        border: iced::border::rounded(8).width(1.0).color(theme::BORDER),
        ..Default::default()
    }
}
