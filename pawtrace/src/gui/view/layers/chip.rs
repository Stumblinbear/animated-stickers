//! The profile chip on a layer row: a drop-down that shows the governing
//! profile and reassigns the layer to another, promotes it into a new profile,
//! or opens the library.

use crate::gui::app::App;
use crate::gui::ids::LayerId;
use crate::gui::msg::{Msg, ProfileMsg};
use crate::gui::view::{icons, theme, widgets};
use iced::widget::{button, column, container, row, rule, text};
use iced::{Alignment, Element, Length};
use iced_aw::drop_down::{self, DropDown};

const CHIP_TIP: &str = "Governing profile. Click to reassign this layer, spin \
    off a new profile from it, or open the library.";

/// The chip and its drop-down for layer `i`, named `name`.
pub fn chip<'a>(app: &'a App, i: LayerId, name: &'a str) -> Element<'a, Msg> {
    let stack = app.stack(app.selected_doc);
    let matched = stack.match_name(name);
    let label = matched.clone().unwrap_or_else(|| "default".into());

    let mut chip_row = row![text(label).size(10).color(theme::MUTED)]
        .spacing(4)
        .align_y(Alignment::Center);
    if stack.has_override(name) {
        chip_row = chip_row.push(widgets::dot(theme::ACCENT, 4.0));
    }
    chip_row = chip_row.push(icons::icon(icons::CHEVRON_DOWN).size(8).color(theme::MUTED));

    let underlay = widgets::help(
        button(container(chip_row).style(theme::badge).padding([2, 6]))
            .on_press(Msg::Profile(ProfileMsg::ToggleChip(i)))
            .style(theme::flat_button)
            .padding(0),
        CHIP_TIP,
    );

    let open = app.profile_ui.chip_open == Some(i);
    DropDown::new(underlay, menu(app, i, matched.as_deref()), open)
        .on_dismiss(Msg::Profile(ProfileMsg::CloseChip))
        .alignment(drop_down::Alignment::BottomStart)
        .width(Length::Fixed(210.0))
        .into()
}

fn menu<'a>(app: &'a App, i: LayerId, current: Option<&str>) -> Element<'a, Msg> {
    let stack = app.stack(app.selected_doc);
    let mut keys: Vec<&str> = stack.project.profiles.keys().map(String::as_str).collect();
    for k in stack.global.profiles.keys() {
        if !keys.contains(&k.as_str()) {
            keys.push(k);
        }
    }

    let mut list = column![text("ASSIGN PROFILE").size(9).color(theme::MUTED)].spacing(2);
    if keys.is_empty() {
        list = list.push(text("No profiles yet").size(11).color(theme::MUTED));
    }
    for k in keys {
        let selected = current == Some(k);
        let glyph = if selected { icons::CIRCLE_DOT } else { icons::CIRCLE };
        let color = if selected { theme::ACCENT } else { theme::MUTED };
        list = list.push(
            button(
                row![icons::icon(glyph).size(11).color(color), text(k.to_string()).size(12)]
                    .spacing(6)
                    .align_y(Alignment::Center),
            )
            .on_press(Msg::Profile(ProfileMsg::Assign(i, k.to_string())))
            .style(theme::flat_button)
            .width(Length::Fill)
            .padding([3, 6]),
        );
    }

    let action = |glyph, label: &'a str, msg| {
        button(
            row![icons::icon(glyph).size(12).color(theme::ACCENT), text(label).size(12).color(theme::ACCENT)]
                .spacing(6)
                .align_y(Alignment::Center),
        )
        .on_press(msg)
        .style(theme::flat_button)
        .width(Length::Fill)
        .padding([3, 6])
    };

    container(
        column![
            list,
            rule::horizontal(1),
            action(icons::PLUS, "New profile from this layer\u{2026}", Msg::Profile(ProfileMsg::NewFromLayer(i))),
            action(icons::LIBRARY, "Manage library\u{2026}", Msg::Profile(ProfileMsg::OpenLibrary)),
        ]
        .spacing(3),
    )
    .style(theme::menu)
    .padding(8)
    .width(Length::Fixed(210.0))
    .into()
}
