//! The in-window menu bar. Each root opens a drop-down of actions that map to
//! the same messages as the toolbar and keyboard shortcuts.

use super::{theme, widgets};
use crate::gui::app::App;
use crate::gui::msg::{EditMsg, FileMsg, Msg, UiMsg};
use iced::widget::{button, row, space, text};
use iced::{Element, Length, Renderer, Theme};
use iced_aw::menu::{Item, Menu, MenuBar};

type MenuItem<'a> = Item<'a, Msg, Theme, Renderer>;
type MenuTree<'a> = Menu<'a, Msg, Theme, Renderer>;

pub fn menu_bar(app: &App) -> Element<'_, Msg> {
    let file = drop_down(vec![
        leaf("Open files", "Ctrl+O", Msg::File(FileMsg::OpenFiles)),
        leaf("Open folder", "Ctrl+Shift+O", Msg::File(FileMsg::OpenFolder)),
        leaf("Export all (JSON)", "Ctrl+E", Msg::File(FileMsg::ExportAll)),
        leaf("Save profiles", "Ctrl+S", Msg::File(FileMsg::SaveProfiles)),
        leaf("Close document", "Ctrl+W", Msg::File(FileMsg::CloseDoc(app.selected_doc))),
    ]);
    let undo = if app.can_undo() {
        leaf("Undo", "Ctrl+Z", Msg::Edit(EditMsg::Undo))
    } else {
        disabled("Undo")
    };
    let redo = if app.can_redo() {
        leaf("Redo", "Ctrl+Y", Msg::Edit(EditMsg::Redo))
    } else {
        disabled("Redo")
    };
    let edit = drop_down(vec![
        undo,
        redo,
        leaf("Reset layer overrides", "", Msg::Edit(EditMsg::ResetLayer)),
    ]);
    let view = drop_down(vec![
        leaf("Fit to window", "Ctrl+0", Msg::Ui(UiMsg::ZoomFit)),
        leaf("Zoom in", "Ctrl+=", Msg::Ui(UiMsg::ZoomIn)),
        leaf("Zoom out", "Ctrl+-", Msg::Ui(UiMsg::ZoomOut)),
    ]);
    let profiles = drop_down(vec![leaf("Save profiles", "Ctrl+S", Msg::File(FileMsg::SaveProfiles))]);
    let help = drop_down(vec![disabled("Pawtrace editor")]);

    let bar = MenuBar::new(vec![
        root("File", file),
        root("Edit", edit),
        root("View", view),
        root("Profiles", profiles),
        root("Help", help),
    ])
    .spacing(4.0);
    let dot = iced::widget::container(iced::widget::space().width(14).height(14)).style(|_| {
        iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::ACCENT)),
            border: iced::border::rounded(4),
            ..Default::default()
        }
    });
    iced::widget::row![dot, bar]
        .spacing(8)
        .align_y(iced::Alignment::Center)
        .padding([2, 8])
        .into()
}

fn drop_down(items: Vec<MenuItem<'_>>) -> MenuTree<'_> {
    Menu::new(items).max_width(220.0).offset(4.0).spacing(2.0)
}

fn root<'a>(label: &'a str, menu: MenuTree<'a>) -> MenuItem<'a> {
    Item::with_menu(
        button(text(label).size(13))
            .style(theme::flat_button)
            .padding([4, 10]),
        menu,
    )
}

fn leaf<'a>(label: &'a str, shortcut: &'a str, msg: Msg) -> MenuItem<'a> {
    let body = row![
        text(label).size(13),
        space().width(Length::Fill),
        widgets::mono(shortcut).size(11).color(theme::MUTED),
    ]
    .spacing(18)
    .align_y(iced::Alignment::Center);
    Item::new(
        button(body)
            .on_press(msg)
            .style(theme::flat_button)
            .width(Length::Fill)
            .padding([4, 10]),
    )
}

fn disabled<'a>(label: &'a str) -> MenuItem<'a> {
    Item::new(
        button(text(label).size(13).color(theme::MUTED))
            .style(theme::flat_button)
            .width(Length::Fill)
            .padding([4, 10]),
    )
}
