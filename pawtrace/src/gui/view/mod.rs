//! The widget tree: menu bar and tabs on top, the three resizable rails in a
//! pane grid, and the status bar beneath. Each area is built by its own
//! submodule; this module only lays them out.

pub(in crate::gui) mod anim;
mod canvas_toolbar;
mod checkerboard;
mod failure;
pub mod icons;
pub(in crate::gui) mod inspector;
mod layers;
mod library;
mod menu_bar;
mod preview;
mod spinner;
mod status_bar;
mod strip;
mod tabs;
pub mod theme;
mod tool_rail;
pub(in crate::gui) mod viewport;
mod welcome;
pub mod widgets;

use crate::gui::app::{App, PaneKind};
use crate::gui::msg::{Msg, UiMsg};
use iced::widget::pane_grid::PaneGrid;
use iced::widget::{column, container, pane_grid, stack};
use iced::{Element, Length};

pub use theme::theme;

pub fn view(app: &App) -> Element<'_, Msg> {
    let body: Element<'_, Msg> = if app.docs.is_empty() {
        welcome::welcome(app)
    } else {
        let grid = PaneGrid::new(&app.panes, |_pane, kind, _| {
            pane_grid::Content::new(pane_body(app, *kind))
        })
        .on_resize(8, |e| Msg::Ui(UiMsg::PaneResized(e)))
        .spacing(4);
        container(grid).width(Length::Fill).height(Length::Fill).into()
    };

    let mut base = column![menu_bar::menu_bar(app)];
    if !app.docs.is_empty() {
        base = base.push(tabs::tabs(app));
    }
    base = base
        .push(container(body).width(Length::Fill).height(Length::Fill))
        .push(status_bar::status_bar(app));

    if app.profile_ui.library_open {
        stack![base, library::modal(app)].into()
    } else {
        base.into()
    }
}

fn pane_body(app: &App, kind: PaneKind) -> Element<'_, Msg> {
    match kind {
        PaneKind::Layers => layers::layers(app),
        PaneKind::Center => center_pane(app),
        PaneKind::Inspector => inspector::inspector(app),
    }
}

fn center_pane(app: &App) -> Element<'_, Msg> {
    let preview_area: Element<'_, Msg> = match app.session().and_then(|s| s.trace_error.as_ref()) {
        Some(err) => failure::placeholder(app, err),
        None => stack![preview::preview(app), tool_rail::tool_rail(app)].into(),
    };
    column![
        strip::strip(app),
        container(preview_area).width(Length::Fill).height(Length::Fill),
        canvas_toolbar::zoom_bar(app),
    ]
    .into()
}
