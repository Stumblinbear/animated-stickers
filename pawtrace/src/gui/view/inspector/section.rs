//! One accordion phase section: a clickable header with the phase number and
//! name, its processing spinner, and its settings body while expanded. A
//! section downstream of the viewed phase is locked (a lock glyph, dimmed, no
//! body); clicking it jumps the view to that phase and unlocks it. A failed
//! phase carries the reserved red on its header.

use crate::gui::msg::{Msg, Phase, UiMsg};
use crate::gui::view::{icons, spinner, theme, widgets};
use iced::widget::{button, column, container, row, space, text};
use iced::{Alignment, Element, Length};
use std::time::Instant;

/// A locked section's hover hint, naming the phase whose view can show its edit.
const LOCK_HINT: &str = "This view can't show downstream changes. Click to view \
    this phase and edit it.";

pub struct Section<'a> {
    pub phase: Phase,
    pub name: &'a str,
    pub busy: bool,
    pub locked: bool,
    pub failed: bool,
    pub now: Instant,
    pub expanded: bool,
    pub body: Element<'a, Msg>,
}

pub fn section(s: Section<'_>) -> Element<'_, Msg> {
    let Section { phase, name, busy, locked, failed, now, expanded, body } = s;
    let number = phase.index() + 1;
    let name_color = if failed {
        theme::DANGER
    } else if locked {
        theme::MUTED
    } else if expanded {
        theme::TEXT
    } else {
        theme::MUTED
    };

    let lead: Element<'_, Msg> = if locked {
        icons::icon(icons::LOCK).size(12).color(theme::MUTED).into()
    } else {
        badge(number, expanded, failed)
    };

    let status: Element<'_, Msg> = if busy {
        spinner::spinner(now, 12.0)
    } else if failed {
        icons::icon(icons::ALERT).size(12).color(theme::DANGER).into()
    } else {
        space().width(6).into()
    };

    let chevron_glyph = if expanded && !locked {
        icons::CHEVRON_DOWN
    } else {
        icons::CHEVRON_RIGHT
    };

    let header = button(
        row![
            lead,
            text(name).size(13).color(name_color).width(Length::Fill),
            status,
            icons::icon(chevron_glyph).size(12).color(theme::MUTED),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .on_press(Msg::Ui(UiMsg::ExpandSection(phase)))
    .style(theme::flat_button)
    .width(Length::Fill)
    .padding([6, 8]);
    // A locked header reveals why on hover and jumps the view on click.
    let header: Element<'_, Msg> = if locked {
        widgets::help(header, LOCK_HINT)
    } else {
        header.into()
    };

    let mut col = column![header];
    // A locked section shows no settings, only the header.
    if expanded && !locked {
        col = col.push(container(body).padding([6, 8]));
    }
    col = col.push(iced::widget::rule::horizontal(1));
    container(col).width(Length::Fill).into()
}

fn badge<'a>(number: usize, expanded: bool, failed: bool) -> Element<'a, Msg> {
    let (bg, fg) = if failed {
        (theme::DANGER, theme::BG)
    } else if expanded {
        (theme::ACCENT, theme::BG)
    } else {
        (theme::BORDER, theme::MUTED)
    };
    container(widgets::mono(format!("{number}")).size(10).color(fg))
        .style(move |_: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(bg)),
            border: iced::border::rounded(3),
            ..Default::default()
        })
        .padding([1, 5])
        .into()
}
