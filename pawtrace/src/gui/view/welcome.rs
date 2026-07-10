//! The welcome / first-run screen: the whole window body when no
//! document is open. Left column is app identity, the open actions, and a drop
//! hint; right column is the searchable recent files and folders.

use super::{icons, theme, widgets};
use crate::gui::app::App;
use crate::gui::msg::{FileMsg, Msg, RecentMsg, RecentTab};
use crate::gui::recents::RecentEntry;
use iced::widget::{button, column, container, row, space, text, text_input};
use iced::{Alignment, Background, Color, Element, Length};

pub fn welcome(app: &App) -> Element<'_, Msg> {
    let divider = container(space().width(1).height(Length::Fill)).style(|_: &iced::Theme| {
        container::Style {
            background: Some(Background::Color(theme::BORDER)),
            ..Default::default()
        }
    });
    row![
        container(identity()).width(Length::FillPortion(2)).height(Length::Fill),
        divider,
        container(recent(app)).width(Length::FillPortion(3)).height(Length::Fill),
    ]
    .into()
}

/// The left column: mark, name, version, tagline, the open actions, and the
/// drop zone.
fn identity<'a>() -> Element<'a, Msg> {
    let mark = container(space().width(56).height(56)).style(|_: &iced::Theme| container::Style {
        background: Some(Background::Color(theme::ACCENT)),
        border: iced::border::rounded(14),
        ..Default::default()
    });
    let heading = row![
        mark,
        column![
            text("Pawtrace").size(30).color(theme::TEXT),
            widgets::mono(format!("v{}", env!("CARGO_PKG_VERSION")))
                .size(12)
                .color(theme::MUTED),
        ]
        .spacing(2),
    ]
    .spacing(16)
    .align_y(Alignment::Center);

    let tagline = text("Trace layered character art into clean vector output.")
        .size(14)
        .color(theme::MUTED);

    let open_files = action(
        icons::FILE_PLUS,
        "Open files\u{2026}",
        "Ctrl+O",
        true,
        Msg::File(FileMsg::OpenFiles),
    );
    let open_folder = action(
        icons::FOLDER,
        "Open folder\u{2026}",
        "Ctrl+Shift+O",
        false,
        Msg::File(FileMsg::OpenFolder),
    );

    let drop = container(
        column![
            icons::icon(icons::UPLOAD).size(24).color(theme::MUTED),
            text("Drop PSD or PNG here").size(13).color(theme::MUTED),
        ]
        .spacing(10)
        .align_x(Alignment::Center),
    )
    .style(|_: &iced::Theme| container::Style {
        border: iced::border::rounded(10).width(1.0).color(theme::BORDER),
        ..Default::default()
    })
    .width(Length::Fill)
    .height(140)
    .center_x(Length::Fill)
    .center_y(Length::Fill);

    container(
        column![
            heading,
            tagline,
            column![open_files, open_folder].spacing(10),
            space().height(Length::Fill),
            drop,
        ]
        .spacing(24),
    )
    .padding(40)
    .into()
}

fn action<'a>(glyph: char, label: &'a str, shortcut: &'a str, primary: bool, msg: Msg) -> Element<'a, Msg> {
    let (icon_c, text_c) = if primary {
        (theme::BG, theme::BG)
    } else {
        (theme::MUTED, theme::TEXT)
    };
    let body = row![
        icons::icon(glyph).size(15).color(icon_c),
        text(label).size(14).color(text_c).width(Length::Fill),
        widgets::mono(shortcut).size(11).color(if primary { theme::BG } else { theme::MUTED }),
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    button(body)
        .on_press(msg)
        .style(move |_: &iced::Theme, status| {
            if primary {
                theme::accent_button_style(status, 8.0)
            } else {
                theme::bordered_button_style(status, 8.0)
            }
        })
        .width(Length::Fill)
        .padding([12, 16])
        .into()
}

/// The right column: the Recent header with Files/Folders tabs, a search box,
/// and the filtered list.
fn recent(app: &App) -> Element<'_, Msg> {
    let tab = app.welcome.tab;
    let tabs = row![
        text("Recent").size(15).color(theme::TEXT),
        tab_button("Files", RecentTab::Files, tab),
        tab_button("Folders", RecentTab::Folders, tab),
    ]
    .spacing(16)
    .align_y(Alignment::Center);

    let search = container(
        row![
            icons::icon(icons::SEARCH).size(12).color(theme::MUTED),
            text_input("Search recent\u{2026}", &app.welcome.search)
                .on_input(|s| Msg::Recent(RecentMsg::Search(s)))
                .size(12)
                .style(theme::transparent_input),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .style(theme::card)
    .padding([6, 10])
    .width(Length::Fixed(260.0));

    let header = row![tabs, space().width(Length::Fill), search]
        .align_y(Alignment::Center);

    let entries = app.filtered_recents();
    let mut list = column![].spacing(6);
    if entries.is_empty() {
        list = list.push(
            container(text("No recent items yet.").size(12).color(theme::MUTED)).padding([10, 4]),
        );
    }
    for (i, e) in entries.iter().enumerate() {
        list = list.push(recent_row(i, e));
    }

    container(
        column![header, iced::widget::scrollable(list).height(Length::Fill)].spacing(20),
    )
    .padding(40)
    .into()
}

fn tab_button<'a>(label: &'a str, this: RecentTab, active: RecentTab) -> Element<'a, Msg> {
    let color = if this == active { theme::ACCENT } else { theme::MUTED };
    button(text(label).size(13).color(color))
        .on_press(Msg::Recent(RecentMsg::Tab(this)))
        .style(theme::flat_button)
        .padding([2, 4])
        .into()
}

fn recent_row(i: usize, e: &RecentEntry) -> Element<'_, Msg> {
    let name = e
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| e.path.display().to_string());
    let thumb: Element<'_, Msg> = if e.folder {
        container(icons::icon(icons::FOLDER_OPEN).size(18).color(theme::MUTED))
            .style(|_: &iced::Theme| container::Style {
                background: Some(Background::Color(theme::SURFACE2)),
                border: iced::border::rounded(6),
                ..Default::default()
            })
            .width(40)
            .height(40)
            .center_x(40)
            .center_y(40)
            .into()
    } else {
        container(space().width(40).height(40))
            .style(|_: &iced::Theme| container::Style {
                background: Some(Background::Color(Color { a: 0.3, ..theme::ACCENT_DIM })),
                border: iced::border::rounded(6),
                ..Default::default()
            })
            .into()
    };

    let mut title_row = row![text(name).size(14).color(theme::TEXT)]
        .spacing(8)
        .align_y(Alignment::Center);
    if e.folder {
        title_row = title_row.push(
            container(widgets::mono("FOLDER").size(9).color(theme::ACCENT))
                .style(theme::badge)
                .padding([1, 5]),
        );
    }
    let meta = column![
        title_row,
        widgets::mono(e.path.display().to_string()).size(11).color(theme::MUTED),
    ]
    .spacing(3);

    let pin_color = if e.pinned { theme::ACCENT } else { theme::MUTED };
    // The pin is a sibling of the open button, not nested inside it: iced
    // buttons capture presses, so a button within a button never fires.
    let pin = button(icons::icon(icons::STAR).size(12).color(pin_color))
        .on_press(Msg::Recent(RecentMsg::Pin(i)))
        .style(theme::flat_button)
        .padding(4);
    let when = widgets::mono(e.opened.ago()).size(11).color(theme::MUTED);

    let open = button(row![thumb, meta].spacing(14).align_y(Alignment::Center))
        .on_press(Msg::Recent(RecentMsg::Open(i)))
        .style(theme::flat_button)
        .width(Length::Fill)
        .padding([8, 10]);

    row![open, pin, when]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
}
