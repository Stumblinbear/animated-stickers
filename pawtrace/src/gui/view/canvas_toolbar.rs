//! The strip beneath the preview: tool group, a Trace sub-view switch, the
//! view composition readout, and zoom controls.

use super::{icons, theme, widgets};
use crate::gui::app::App;
use crate::gui::msg::{Msg, StripView, Tool, TraceView, UiMsg};
use iced::widget::{button, container, row, space, text};
use iced::{Alignment, Element, Length};

pub fn canvas_toolbar(app: &App) -> Element<'_, Msg> {
    let tool = app.tool;
    let tools = row![
        tool_button(icons::POINTER, Tool::Select, tool),
        tool_button(icons::PIN, Tool::Pin, tool),
    ]
    .spacing(4);

    let mut left = row![tools].spacing(14).align_y(Alignment::Center);
    if app.session().is_some_and(|s| matches!(s.view, StripView::Stage(4))) {
        left = left.push(trace_switch(app));
    }

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
        row![left, space().width(Length::Fill), readout, space().width(16), zoom]
            .align_y(Alignment::Center)
            .padding([5, 10]),
    )
    .style(theme::panel)
    .width(Length::Fill)
    .into()
}

fn tool_button<'a>(glyph: char, this: Tool, active: Tool) -> Element<'a, Msg> {
    button(icons::icon(glyph).size(13))
        .on_press(Msg::Ui(UiMsg::Tool(this)))
        .style(theme::tool_button(this == active))
        .padding([4, 7])
        .into()
}

fn trace_switch(app: &App) -> Element<'_, Msg> {
    let tv = app.session().map(|s| s.trace_view).unwrap_or_default();
    let seg = |label: &'static str, this: TraceView| {
        button(text(label).size(11))
            .on_press(Msg::Ui(UiMsg::TraceView(this)))
            .style(theme::chip(this == tv))
            .padding([3, 8])
    };
    row![
        seg("Smooth", TraceView::Smooth),
        seg("Fit", TraceView::Fit),
        seg("Final", TraceView::Final),
    ]
    .spacing(2)
    .align_y(Alignment::Center)
    .into()
}

fn composition(app: &App) -> String {
    let Some(doc) = app.doc() else {
        return String::new();
    };
    let (w, h) = doc.size;
    let shown = doc.flags.iter().filter(|f| f.visible && f.enabled).count();
    let excluded = doc.flags.iter().filter(|f| !f.enabled).count();
    format!("{w} × {h} · {shown} shown · {excluded} excluded")
}
