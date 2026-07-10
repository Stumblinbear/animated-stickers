//! One layer row: visibility eye, thumbnail, name, profile chip, and anchor
//! count, with selected / dimmed / excluded states.

use super::chip;
use crate::gui::app::{App, DocState};
use crate::gui::ids::LayerId;
use crate::gui::msg::{LayerMsg, Msg};
use crate::gui::view::{icons, spinner, theme, widgets};
use iced::widget::{button, container, row, space, text};
use iced::{Alignment, Color, Element, Length};
use std::time::Instant;

const SHOW_TIP: &str = "Shown in the preview composite. Click to hide it there; \
    it is still traced and exported.";
const HIDE_TIP: &str = "Hidden from the preview composite, still traced and \
    exported. Click to show it.";
const EXCLUDED_TIP: &str = "Excluded from processing and export. Click to \
    include it again.";

/// Builds the row for layer `i`. The caller guarantees a document is open.
pub fn layer_row(app: &App, i: LayerId) -> Element<'_, Msg> {
    let doc = app.doc().expect("layer row needs an open document");
    let layer = doc.layer(i).expect("layer row is built from a live layer id");
    let flags = &doc.inputs[&i];
    let name = &layer.name;
    let sess = app.session().expect("layer row needs a session");
    let selected = sess.selection.contains(&i);
    // The primary is meaningless with an empty selection, so no row is primary.
    let primary = !sess.selection.is_empty() && sess.selected_layer == i;
    let dimmed = !flags.visible || !flags.enabled;
    let failed = sess.trace_error.as_ref().is_some_and(|e| e.layer == i);

    let eye_glyph = if flags.visible { icons::EYE } else { icons::EYE_OFF };
    let eye = widgets::help(
        button(
            icons::icon(eye_glyph)
                .size(13)
                .color(if flags.visible { theme::TEXT } else { theme::MUTED }),
        )
        .on_press(Msg::Layer(LayerMsg::ToggleVisible(i)))
        .style(theme::flat_button)
        .padding(3),
        if flags.visible { SHOW_TIP } else { HIDE_TIP },
    );

    let thumb = container(space().width(22).height(22)).style(|_| container::Style {
        background: Some(iced::Background::Color(Color {
            a: 0.30,
            ..theme::ACCENT_DIM
        })),
        border: iced::border::rounded(4),
        ..Default::default()
    });
    let name_color = if failed {
        theme::DANGER
    } else if dimmed {
        theme::MUTED
    } else {
        theme::TEXT
    };
    let name_text = text(name.as_str()).size(13).color(name_color).width(Length::Fill);
    let tail = if failed {
        // A warning icon replaces the anchor count on a failed layer.
        icons::icon(icons::ALERT).size(13).color(theme::DANGER).into()
    } else {
        tail(sess, app.anim_now, flags.enabled, i, primary)
    };

    let inner = button(
        row![thumb, name_text, chip::chip(app, i, name), tail]
            .spacing(8)
            .align_y(Alignment::Center),
    )
    .on_press(Msg::Layer(LayerMsg::Click(i)))
    .style(theme::layer_row(selected, dimmed))
    .width(Length::Fill)
    .padding([3, 6]);

    row![eye, inner].spacing(2).align_y(Alignment::Center).into()
}

/// The trailing indicator: an `EXCLUDED` badge for a disabled layer, a
/// spinner while this layer is being traced, otherwise the anchor count with
/// a ramp dot.
fn tail<'a>(
    sess: &DocState,
    now: Instant,
    enabled: bool,
    i: LayerId,
    primary: bool,
) -> Element<'a, Msg> {
    if !enabled {
        // The badge doubles as the re-include control; excluding goes
        // through the context menu.
        return widgets::help(
            button(text("EXCLUDED").size(9).color(theme::MUTED))
                .on_press(Msg::Layer(LayerMsg::ToggleEnabled(i)))
                .style(theme::flat_button)
                .padding([1, 4]),
            EXCLUDED_TIP,
        );
    }
    // The strip traces only the selected layer. A full render with no counts
    // yet is tracing every enabled layer for the first time.
    let tracing = (primary && sess.stages_running) || (sess.full_busy && sess.layer_outputs.is_empty());
    if tracing {
        return spinner::spinner(now, 14.0);
    }
    let anchors = sess.layer_outputs.get(&i).map(|o| o.anchors).unwrap_or(0);
    if anchors == 0 {
        return space().width(0).into();
    }
    let max = sess.layer_outputs.values().map(|o| o.anchors).max().unwrap_or(1).max(1);
    let color = ramp(anchors as f32 / max as f32);
    row![
        widgets::dot(color, 6.0),
        widgets::mono(format!("{anchors}")).size(11).color(theme::MUTED),
    ]
    .spacing(4)
    .align_y(Alignment::Center)
    .into()
}

/// Linear ramp from muted to accent over `t` in 0..=1.
fn ramp(t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let (a, b) = (theme::MUTED, theme::ACCENT);
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: 1.0,
    }
}
