//! The profile-library modal: a centered panel with the library icon, its file
//! path and template count, a template search, and one swatched row per profile
//! with rename, duplicate, delete, and an Import action. It carries no
//! "used in N" counts, since copy-on-use leaves no live link to count.

use crate::gui::app::App;
use crate::gui::msg::{Msg, ProfileMsg};
use crate::gui::view::{icons, theme, widgets};
use crate::profiles::{self, Overrides, Profiles, Scope};
use iced::widget::{
    button, center, column, container, mouse_area, opaque, row, scrollable, space, stack, text,
    text_input,
};
use iced::{Alignment, Background, Color, Element, Length};

/// The modal as a full-window overlay: a dimmed backdrop that closes on an
/// outside click, with the library panel centered over it.
pub fn modal(app: &App) -> Element<'_, Msg> {
    let backdrop = mouse_area(
        container(space())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_: &iced::Theme| container::Style {
                background: Some(Background::Color(Color { a: 0.55, ..Color::BLACK })),
                ..Default::default()
            }),
    )
    .on_press(Msg::Profile(ProfileMsg::CloseLibrary));
    opaque(stack![backdrop, center(panel(app))])
}

fn panel(app: &App) -> Element<'_, Msg> {
    let stack_ref = app.stack_sel();
    let count = stack_ref.global.profiles.len() + stack_ref.project.profiles.len();
    let path = profiles::global_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "profiles.toml".into());

    let mark = container(icons::icon(icons::LIBRARY).size(16).color(theme::ACCENT))
        .style(|_: &iced::Theme| container::Style {
            background: Some(Background::Color(theme::SURFACE2)),
            border: iced::border::rounded(8),
            ..Default::default()
        })
        .padding(8);
    let close = button(icons::icon(icons::CLOSE).size(14).color(theme::MUTED))
        .on_press(Msg::Profile(ProfileMsg::CloseLibrary))
        .style(theme::flat_button)
        .padding(4);
    let header = row![
        mark,
        column![
            text("Profile library").size(16).color(theme::TEXT),
            widgets::mono(format!("{path}  ·  {count} templates"))
                .size(11)
                .color(theme::MUTED),
        ]
        .spacing(2)
        .width(Length::Fill),
        close,
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    let search = container(
        row![
            icons::icon(icons::SEARCH).size(13).color(theme::MUTED),
            text_input("Search templates\u{2026}", &app.profile_ui.library_search)
                .on_input(|s| Msg::Profile(ProfileMsg::LibrarySearch(s)))
                .size(13)
                .style(theme::transparent_input),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .style(theme::card)
    .padding([8, 12]);

    let body = column![
        tier(app, Scope::Project, "PROJECT", stack_ref.project),
        tier(app, Scope::Global, "GLOBAL", stack_ref.global),
    ]
    .spacing(14);

    let import = button(
        row![
            icons::icon(icons::DOWNLOAD).size(13).color(theme::TEXT),
            text("Import\u{2026}").size(13).color(theme::TEXT),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .on_press(Msg::Profile(ProfileMsg::ImportLibrary))
    .style(|_: &iced::Theme, status| theme::bordered_button_style(status, 6.0))
    .padding([8, 14]);
    let export = button(text("Export\u{2026}").size(13).color(theme::MUTED))
        .on_press(Msg::Profile(ProfileMsg::ExportLibrary))
        .style(theme::flat_button)
        .padding([8, 14]);
    let footer = row![import, export, space().width(Length::Fill)].spacing(10);

    container(
        column![header, search, scrollable(body).height(Length::Fill), footer]
            .spacing(14)
            .padding(18),
    )
    .style(theme::menu)
    .width(Length::Fixed(520.0))
    .max_height(580.0)
    .into()
}

fn tier<'a>(app: &'a App, scope: Scope, label: &'a str, profiles: &'a Profiles) -> Element<'a, Msg> {
    let q = app.profile_ui.library_search.trim().to_lowercase();
    let keys: Vec<&String> = profiles
        .profiles
        .keys()
        .filter(|k| q.is_empty() || k.to_lowercase().contains(&q))
        .collect();
    let mut list = column![text(label).size(9).color(theme::MUTED)].spacing(4);
    if keys.is_empty() {
        list = list.push(text("No templates").size(11).color(theme::MUTED));
    }
    for key in keys {
        list = list.push(profile_row(app, scope, key, &profiles.profiles[key]));
    }
    list.into()
}

/// A swatch color for a template: its first locked color, else its stroke
/// color, else a neutral gray so the row still reads as a labeled swatch.
fn swatch_color(ov: &Overrides) -> Color {
    let hex = ov
        .locked
        .as_ref()
        .and_then(|v| v.first())
        .or(ov.stroke_color.as_ref());
    match hex.and_then(|s| crate::color::Srgb::from_hex(s)) {
        Some(c) => c.into(),
        None => theme::SURFACE2,
    }
}

fn profile_row<'a>(app: &'a App, scope: Scope, key: &'a str, ov: &'a Overrides) -> Element<'a, Msg> {
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

    let color = swatch_color(ov);
    let swatch = container(space().width(28).height(28)).style(move |_: &iced::Theme| container::Style {
        background: Some(Background::Color(color)),
        border: iced::border::rounded(6),
        ..Default::default()
    });
    let controls = row![
        icon_button(icons::PENCIL, theme::MUTED, Msg::Profile(ProfileMsg::RenameStart(scope, key.to_string()))),
        icon_button(icons::COPY, theme::MUTED, Msg::Profile(ProfileMsg::Duplicate(scope, key.to_string()))),
        icon_button(icons::TRASH, theme::MUTED, Msg::Profile(ProfileMsg::Delete(scope, key.to_string()))),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    container(
        row![
            swatch,
            text(key.to_string()).size(13).color(theme::TEXT).width(Length::Fill),
            controls,
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .style(theme::badge)
    .padding([6, 10])
    .into()
}

fn icon_button<'a>(glyph: char, tint: Color, msg: Msg) -> Element<'a, Msg> {
    button(icons::icon(glyph).size(13).color(tint))
        .on_press(msg)
        .style(theme::flat_button)
        .padding(3)
        .into()
}
