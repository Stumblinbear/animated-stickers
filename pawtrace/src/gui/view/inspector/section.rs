//! One accordion section: a clickable header with the stage number, name, and
//! a spinner while its stage computes, revealing its settings body only while
//! expanded.

use crate::gui::msg::{Msg, UiMsg};
use crate::gui::view::{icons, spinner, theme, widgets};
use iced::widget::{button, column, container, row, space, text};
use iced::{Alignment, Element, Length};
use std::time::Instant;

pub fn section<'a>(
    number: usize,
    name: &'a str,
    busy: bool,
    now: Instant,
    expanded: bool,
    body: Element<'a, Msg>,
) -> Element<'a, Msg> {
    let chevron = icons::icon(if expanded {
        icons::CHEVRON_DOWN
    } else {
        icons::CHEVRON_RIGHT
    })
    .size(12)
    .color(theme::MUTED);
    let badge = container(
        widgets::mono(format!("{number}"))
            .size(10)
            .color(if expanded { theme::BG } else { theme::MUTED }),
    )
    .style(move |_| container::Style {
        background: Some(iced::Background::Color(if expanded {
            theme::ACCENT
        } else {
            theme::BORDER
        })),
        border: iced::border::rounded(3),
        ..Default::default()
    })
    .padding([1, 5]);
    let status: Element<'a, Msg> = if busy {
        spinner::spinner(now, 12.0)
    } else {
        space().width(6).into()
    };
    let header = button(
        row![
            badge,
            text(name)
                .size(13)
                .color(if expanded { theme::TEXT } else { theme::MUTED })
                .width(Length::Fill),
            status,
            chevron,
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .on_press(Msg::Ui(UiMsg::ExpandStage(number)))
    .style(theme::flat_button)
    .width(Length::Fill)
    .padding([6, 8]);

    let mut col = column![header];
    if expanded {
        col = col.push(container(body).padding([6, 8]));
    }
    col = col.push(iced::widget::rule::horizontal(1));
    container(col).width(Length::Fill).into()
}
