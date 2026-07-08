//! The widget tree: menu bar and tabs on top, the three resizable rails in a
//! pane grid, and the status bar beneath. Each area is built by its own
//! submodule; this module only lays them out.

mod anim;
mod canvas_toolbar;
mod icons;
mod inspector;
mod layers;
mod library;
mod menu_bar;
mod overlays;
mod preview;
mod spinner;
mod status_bar;
mod strip;
mod tabs;
pub mod theme;
mod viewport;
mod widgets;

use crate::gui::app::{App, PaneKind};
use crate::gui::msg::{Msg, UiMsg};
use iced::widget::{center, column, container, pane_grid, stack, text};
use iced::widget::pane_grid::PaneGrid;
use iced::{Element, Length};

pub use theme::theme;

pub fn view(app: &App) -> Element<'_, Msg> {
    let grid = PaneGrid::new(&app.panes, |_pane, kind, _| {
        pane_grid::Content::new(pane_body(app, *kind))
    })
    .on_resize(8, |e| Msg::Ui(UiMsg::PaneResized(e)))
    .spacing(4);

    let base = column![
        menu_bar::menu_bar(app),
        tabs::tabs(app),
        container(grid).width(Length::Fill).height(Length::Fill),
        status_bar::status_bar(app),
    ];
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
    if app.docs.is_empty() {
        let hint = center(text("Open files  ·  Ctrl+O").size(14).color(theme::MUTED));
        return stack![preview::preview(app), hint].into();
    }
    column![
        strip::strip(app),
        container(preview::preview(app))
            .width(Length::Fill)
            .height(Length::Fill),
        canvas_toolbar::canvas_toolbar(app),
    ]
    .into()
}
