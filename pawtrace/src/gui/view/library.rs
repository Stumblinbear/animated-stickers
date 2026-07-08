//! The profile-library modal: a centered panel listing both tiers' profiles
//! with a live count of the layers using each, and per-row rename, duplicate,
//! and delete.

use crate::gui::app::App;
use crate::gui::msg::{Msg, ProfileMsg};
use crate::gui::view::{icons, theme, widgets};
use crate::profiles::{Profiles, Scope};
use iced::widget::{
    button, center, column, container, mouse_area, opaque, row, scrollable, stack, text, text_input,
};
use iced::{Alignment, Background, Color, Element, Length};

/// The modal as a full-window overlay: a dimmed backdrop that closes on an
/// outside click, with the library panel centered over it.
pub fn modal(app: &App) -> Element<'_, Msg> {
    let backdrop = mouse_area(container(iced::widget::space()).width(Length::Fill).height(Length::Fill).style(
        |_: &iced::Theme| container::Style {
            background: Some(Background::Color(Color { a: 0.55, ..Color::BLACK })),
            ..Default::default()
        },
    ))
    .on_press(Msg::Profile(ProfileMsg::CloseLibrary));
    opaque(stack![backdrop, center(panel(app))])
}

fn panel(app: &App) -> Element<'_, Msg> {
    let close = button(icons::icon(icons::CLOSE).size(14).color(theme::MUTED))
        .on_press(Msg::Profile(ProfileMsg::CloseLibrary))
        .style(theme::flat_button)
        .padding(4);
    let header = row![
        text("PROFILE LIBRARY").size(11).color(theme::MUTED).width(Length::Fill),
        close,
    ]
    .align_y(Alignment::Center);

    let stack_ref = app.stack_sel();
    let body = column![
        tier(app, Scope::Project, "PROJECT", stack_ref.project),
        tier(app, Scope::Global, "GLOBAL", stack_ref.global),
    ]
    .spacing(14);

    container(
        column![header, scrollable(body).height(Length::Fill)]
            .spacing(12)
            .padding(16),
    )
    .style(theme::menu)
    .width(Length::Fixed(480.0))
    .max_height(560.0)
    .into()
}

fn tier<'a>(app: &'a App, scope: Scope, label: &'a str, profiles: &'a Profiles) -> Element<'a, Msg> {
    let mut list = column![text(label).size(9).color(theme::MUTED)].spacing(4);
    if profiles.profiles.is_empty() {
        list = list.push(text("No profiles").size(11).color(theme::MUTED));
    }
    for key in profiles.profiles.keys() {
        list = list.push(profile_row(app, scope, key));
    }
    list.into()
}

fn profile_row<'a>(app: &'a App, scope: Scope, key: &'a str) -> Element<'a, Msg> {
    let renaming = app
        .profile_ui
        .rename
        .as_ref()
        .filter(|r| r.scope == scope && r.key == key);
    if let Some(r) = renaming {
        let field = text_input("profile key", &r.text)
            .on_input(|s| Msg::Profile(ProfileMsg::RenameInput(s)))
            .on_submit(Msg::Profile(ProfileMsg::RenameCommit))
            .size(12);
        let confirm = icon_button(icons::CHEVRON_RIGHT, theme::ACCENT, Msg::Profile(ProfileMsg::RenameCommit));
        return container(row![field, confirm].spacing(6).align_y(Alignment::Center))
            .padding([4, 6])
            .into();
    }

    let used = app.profile_usage(key);
    let count = widgets::mono(format!("{used} layer{}", if used == 1 { "" } else { "s" }))
        .size(10)
        .color(theme::MUTED);
    let controls = row![
        count,
        icon_button(icons::PENCIL, theme::MUTED, Msg::Profile(ProfileMsg::RenameStart(scope, key.to_string()))),
        icon_button(icons::COPY, theme::MUTED, Msg::Profile(ProfileMsg::Duplicate(scope, key.to_string()))),
        icon_button(icons::TRASH, theme::MUTED, Msg::Profile(ProfileMsg::Delete(scope, key.to_string()))),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    container(
        row![text(key.to_string()).size(12).width(Length::Fill), controls]
            .spacing(8)
            .align_y(Alignment::Center),
    )
    .style(theme::badge)
    .padding([5, 8])
    .into()
}

fn icon_button<'a>(glyph: char, tint: Color, msg: Msg) -> Element<'a, Msg> {
    button(icons::icon(glyph).size(12).color(tint))
        .on_press(msg)
        .style(theme::flat_button)
        .padding(3)
        .into()
}
