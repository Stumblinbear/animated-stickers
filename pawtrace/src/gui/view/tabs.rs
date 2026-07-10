//! The document tab row: one tab per open file with a close control, and a
//! trailing button that opens more files.

use super::{icons, theme};
use crate::gui::app::App;
use crate::gui::doc;
use crate::gui::msg::{FileMsg, Msg};
use iced::widget::{button, container, row, text};
use iced::{Alignment, Element, Length};

pub fn tabs(app: &App) -> Element<'_, Msg> {
    let mut r = row![].spacing(2).align_y(Alignment::Center);
    for (i, d) in app.docs.iter().enumerate() {
        let active = i == app.selected_pos();
        // A red warning icon marks a document with a failed layer.
        let failed = d.session.trace_error.is_some();
        let (glyph, icon_color) = if failed {
            (icons::ALERT, theme::DANGER)
        } else {
            (icons::FILE, theme::MUTED)
        };
        let name_color = if failed {
            theme::DANGER
        } else if active {
            theme::TEXT
        } else {
            theme::MUTED
        };
        let label = row![
            icons::icon(glyph).size(11).color(icon_color),
            text(doc::doc_label(&d.path)).size(12).color(name_color),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        // One shared surface behind the name and close buttons, so the pair
        // reads as a single tab.
        let tab = container(
            row![
                button(label)
                    .on_press(Msg::File(FileMsg::SelectDoc(i)))
                    .style(theme::flat_button)
                    .padding([4, 8]),
                button(icons::icon(icons::CLOSE).size(10).color(theme::MUTED))
                    .on_press(Msg::File(FileMsg::CloseDoc(Some(d.id))))
                    .style(theme::flat_button)
                    .padding(3),
            ]
            .spacing(0)
            .align_y(Alignment::Center),
        )
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(if active {
                theme::SURFACE2
            } else {
                theme::SURFACE
            })),
            border: iced::border::rounded(5),
            ..Default::default()
        })
        .padding([1, 3]);
        r = r.push(tab);
    }
    r = r.push(
        button(icons::icon(icons::PLUS).size(12).color(theme::MUTED))
            .on_press(Msg::File(FileMsg::OpenFiles))
            .style(theme::flat_button)
            .padding([4, 8]),
    );
    container(r)
        .style(theme::panel)
        .width(Length::Fill)
        .padding([3, 6])
        .into()
}
