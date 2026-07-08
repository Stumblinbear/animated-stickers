//! The bottom bar: whole-document totals and transient status on the left,
//! the active document's name and position on the right.

use super::{theme, widgets};
use crate::gui::app::App;
use crate::gui::doc;
use crate::gui::msg::Msg;
use iced::widget::{container, row, space, text};
use iced::{Alignment, Element, Length};

pub fn status_bar(app: &App) -> Element<'_, Msg> {
    let stats = app
        .session()
        .and_then(|s| s.full_stats)
        .map(|s| {
            format!(
                "{} shapes · {} anchors",
                widgets::thousands(s.shapes),
                widgets::thousands(s.anchors)
            )
        })
        .unwrap_or_default();
    let left = row![
        widgets::mono(stats).size(11).color(theme::MUTED),
        text(app.status.clone()).size(11).color(theme::MUTED),
    ]
    .spacing(16)
    .align_y(Alignment::Center);

    let name = app.doc().map(|d| doc::doc_label(&d.path)).unwrap_or_default();
    let position = if app.docs.is_empty() {
        String::new()
    } else {
        format!("{} / {} docs", app.selected_doc + 1, app.docs.len())
    };
    let right = row![
        text(name).size(12).color(theme::TEXT),
        widgets::mono(position).size(11).color(theme::MUTED),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    container(
        row![left, space().width(Length::Fill), right]
            .align_y(Alignment::Center)
            .padding([4, 10]),
    )
    .style(theme::panel)
    .width(Length::Fill)
    .into()
}
