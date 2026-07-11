//! The slim bar below the preview: the view-composition readout and the zoom
//! controls.

use super::{icons, theme, widgets};
use crate::gui::app::App;
use crate::gui::msg::{Msg, StripView, UiMsg};
use iced::widget::{button, container, row, space};
use iced::{Alignment, Element, Length};

pub fn zoom_bar(app: &App) -> Element<'_, Msg> {
    let readout = widgets::mono(composition(app)).size(11).color(theme::MUTED);

    let zoom_pct = app
        .session()
        .and_then(|s| s.zoom())
        .map(|z| format!("{}%", (z * 100.0).round() as i32))
        .unwrap_or_else(|| "fit".into());

    let zoom = row![
        button(icons::icon(icons::FIT).size(11).color(theme::MUTED))
            .on_press(Msg::Ui(UiMsg::ZoomFit))
            .style(theme::flat_button)
            .padding([3, 6]),
        button(icons::icon(icons::MINUS).size(11).color(theme::MUTED))
            .on_press(Msg::Ui(UiMsg::ZoomOut))
            .style(theme::flat_button)
            .padding([3, 6]),
        button(widgets::mono(zoom_pct).size(11))
            .on_press(Msg::Ui(UiMsg::ZoomFit))
            .style(theme::flat_button)
            .padding([3, 4]),
        button(icons::icon(icons::PLUS).size(11).color(theme::MUTED))
            .on_press(Msg::Ui(UiMsg::ZoomIn))
            .style(theme::flat_button)
            .padding([3, 6]),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    container(
        row![
            space().width(Length::Fill),
            readout,
            space().width(16),
            zoom
        ]
        .align_y(Alignment::Center)
        .padding([5, 10]),
    )
    .style(theme::panel)
    .width(Length::Fill)
    .into()
}

/// The current preview's composition line: document dimensions and counts on
/// Document, or the layer, phase, shown sub-view, and a phase-fitting detail
/// count on a phase view.
fn composition(app: &App) -> String {
    let Some(sess) = app.session() else {
        return String::new();
    };

    match sess.view {
        StripView::Document => {
            let Some(doc) = app.doc() else {
                return String::new();
            };

            let (w, h) = doc.size;

            let shown = doc
                .inputs
                .values()
                .filter(|i| i.visible && i.enabled)
                .count();

            let excluded = doc.inputs.values().filter(|i| !i.enabled).count();

            format!("{w} × {h} · {shown} shown · {excluded} excluded")
        }
        StripView::Phase(p) => {
            let layer = app.layer_name().unwrap_or_else(|| "-".into());

            let sub = app.active_subview().map(|sv| sv.label()).unwrap_or("");

            let detail = p
                .status_detail(app)
                .map(|d| format!(" · {d}"))
                .unwrap_or_default();

            format!("{layer} · {} → {sub}{detail}", p.label())
        }
    }
}
