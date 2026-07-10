//! The preview error placeholder for a failed trace: a red warning
//! mark, the human-readable cause, the raw pipeline message, and one-click
//! fixes (Retry plus an optional suggested setting change).

use super::{icons, theme, widgets};
use crate::gui::app::{App, ErrorFix, LayerError};
use crate::gui::msg::{EditMsg, Msg, UiMsg};
use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Background, Color, Element, Length};

pub fn placeholder<'a>(app: &'a App, err: &'a LayerError) -> Element<'a, Msg> {
    let layer = app
        .layer_name_of(app.selected_pos(), err.layer)
        .unwrap_or_else(|| "layer".into());

    let mark = container(icons::icon(icons::ALERT).size(30).color(theme::DANGER))
        .style(|_: &iced::Theme| container::Style {
            background: Some(Background::Color(Color { a: 0.12, ..theme::DANGER })),
            border: iced::border::rounded(14).width(1.0).color(theme::DANGER_DIM),
            ..Default::default()
        })
        .padding(18);

    let title = text(format!("Couldn't trace \u{201c}{layer}\u{201d}"))
        .size(20)
        .color(theme::TEXT);
    let cause = text(err.human.clone())
        .size(13)
        .color(theme::MUTED)
        .align_x(iced::alignment::Horizontal::Center);
    let raw = container(widgets::mono(err.raw.clone()).size(12).color(theme::MUTED))
        .style(theme::card)
        .padding([8, 12]);

    let retry = button(
        row![
            icons::icon(icons::RESET).size(12).color(theme::BG),
            text("Retry").size(13).color(theme::BG),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .on_press(Msg::Ui(UiMsg::Retry))
    .style(|_: &iced::Theme, status| theme::accent_button_style(status, 6.0))
    .padding([8, 16]);

    let mut actions = row![retry].spacing(10).align_y(Alignment::Center);
    if let Some(fix) = &err.fix {
        actions = actions.push(fix_button(fix));
    }

    let body = column![mark, title, cause, raw, actions]
        .spacing(16)
        .align_x(Alignment::Center)
        .max_width(560);

    container(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .padding(24)
        .into()
}

fn fix_button(fix: &ErrorFix) -> Element<'_, Msg> {
    button(text(fix.label.clone()).size(13).color(theme::TEXT))
        .on_press(Msg::Edit(EditMsg::Set(fix.field, fix.value)))
        .style(theme::flat_button)
        .padding([8, 16])
        .into()
}
