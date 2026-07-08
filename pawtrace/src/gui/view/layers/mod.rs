//! The layers rail: the layer list top-to-bottom, wrapped in a right-click
//! context menu of bulk actions over the current selection.

mod chip;
#[path = "row.rs"]
mod rows;

use crate::gui::app::App;
use crate::gui::ids::LayerId;
use crate::gui::msg::{LayerMsg, Msg, ProfileMsg};
use crate::gui::view::{icons, theme};
use iced::widget::{button, column, container, row, rule, scrollable, text};
use iced::{Alignment, Element, Length};
use iced_aw::ContextMenu;

pub fn layers(app: &App) -> Element<'_, Msg> {
    let header = row![
        text("LAYERS").size(11).color(theme::MUTED),
        text("top to bottom").size(10).color(theme::MUTED),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    // Right padding keeps the anchor counts clear of the overlay scrollbar.
    let mut list = column![].spacing(2).padding(iced::Padding {
        right: 10.0,
        ..iced::Padding::ZERO
    });
    if let Some(doc) = app.doc() {
        // Topmost layer first; storage is bottom-first paint order.
        for i in (0..doc.layers.len()).rev() {
            list = list.push(rows::layer_row(app, LayerId(i)));
        }
    }
    let body = column![
        header,
        scrollable(list).id(crate::gui::ids::layers_scrollable()).height(Length::Fill)
    ]
    .spacing(8)
    .padding(8);
    let underlay = container(body)
        .style(theme::panel)
        .width(Length::Fill)
        .height(Length::Fill);
    ContextMenu::new(underlay, move || context_menu(app)).into()
}

fn context_menu(app: &App) -> Element<'_, Msg> {
    let count = app.session().map(|s| s.selection.len()).unwrap_or(0);
    let head = text(format!("{count} LAYERS SELECTED")).size(9).color(theme::MUTED);

    let mut body = column![container(head).padding([8, 14])].spacing(2);
    body = body
        .push(item(icons::EYE_OFF, "Hide from preview", false, Msg::Layer(LayerMsg::BulkVisible(false))))
        .push(item(icons::BAN, "Exclude from export", false, Msg::Layer(LayerMsg::BulkEnabled(false))))
        .push(assign_group(app))
        .push(container(rule::horizontal(1)).padding([4, 12]))
        .push(item(icons::PLUS, "Group into new profile\u{2026}", true, Msg::Profile(ProfileMsg::GroupNew)))
        .push(item(icons::CLOSE, "Clear selection", false, Msg::Layer(LayerMsg::ClearSelection)));

    container(body).style(theme::menu).width(240).padding([6, 0]).into()
}

/// The "Assign profile" heading and, indented beneath it, one entry per
/// profile key that pins the whole selection to it.
fn assign_group(app: &App) -> Element<'_, Msg> {
    let stack = app.stack(app.selected_doc);
    let mut keys: Vec<&str> = stack.project.profiles.keys().map(String::as_str).collect();
    for k in stack.global.profiles.keys() {
        if !keys.contains(&k.as_str()) {
            keys.push(k);
        }
    }

    let heading = row![
        icons::icon(icons::TAG).size(13).color(theme::MUTED),
        text("Assign profile").size(12).color(theme::TEXT).width(Length::Fill),
        icons::icon(icons::CHEVRON_RIGHT).size(11).color(theme::MUTED),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let mut group = column![container(heading).padding([6, 14])].spacing(2);
    if keys.is_empty() {
        group = group.push(
            container(text("No profiles yet").size(11).color(theme::MUTED)).padding([2, 38]),
        );
    }
    for k in keys {
        group = group.push(
            button(text(k.to_string()).size(12).color(theme::MUTED))
                .on_press(Msg::Profile(ProfileMsg::AssignSelection(k.to_string())))
                .style(theme::flat_button)
                .width(Length::Fill)
                .padding([4, 38]),
        );
    }
    group.into()
}

fn item(glyph: char, label: &str, accent: bool, msg: Msg) -> Element<'_, Msg> {
    let (ic, tc) = if accent {
        (theme::ACCENT, theme::ACCENT)
    } else {
        (theme::MUTED, theme::TEXT)
    };
    button(
        row![
            icons::icon(glyph).size(13).color(ic),
            text(label.to_string()).size(12).color(tc),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .on_press(msg)
    .style(theme::flat_button)
    .width(Length::Fill)
    .padding([6, 14])
    .into()
}
