//! Stage 5 (Trace) settings: smoothing, fit, simplify, and the sticker
//! stroke, with the anchor readouts.

use super::setting::setting;
use crate::gui::app::App;
use crate::gui::fields::Field;
use crate::gui::msg::{EditMsg, Msg};
use crate::gui::view::{theme, widgets};
use iced::widget::{column, row, slider, text, text_input};
use iced::{Alignment, Element};

pub fn stage5(app: &App) -> Element<'_, Msg> {
    let Some(sess) = app.session() else {
        return column![].into();
    };
    let cfg = &sess.cfg;
    column![
        setting(
            app,
            "Corner angle",
            format!("{:.0}\u{b0}", cfg.alphamax * 90.0),
            "A vertex is a corner (fit segments meet with independent \
             tangents) only where the path bends by at least this much. \
             Higher keeps fewer corners and smooths more: 0\u{b0} makes \
             every vertex a corner, 180\u{b0} keeps none. Fur tips \
             survive at the 104\u{b0} default.",
            Field::Alphamax,
            slider(0.0..=180.0, cfg.alphamax * 90.0, |deg| {
                Msg::Edit(EditMsg::Set(Field::Alphamax, deg / 90.0))
            })
            .step(1.0),
        ),
        setting(
            app,
            "Edge smoothing",
            format!("{:.1}x", cfg.smoothing),
            "Averages pixel-boundary vertices before fitting, as a \
             multiple of the supersample scale (corners stay pinned). \
             Higher rounds off more of the pixel staircase; too high \
             softens intended detail. 1.0 is the calibrated default, 0 \
             = none.",
            Field::Smoothing,
            slider(0.0..=4.0, cfg.smoothing as f64, |v| {
                Msg::Edit(EditMsg::Set(Field::Smoothing, v))
            })
            .step(0.1),
        ),
        setting(
            app,
            "Fit tolerance",
            format!("{:.2} px", cfg.opttolerance),
            "Max curve deviation in supersampled pixels (source px times \
             the supersample scale). Higher = fewer anchors, looser fit; \
             line widths start to wobble past ~1, and the high end \
             trades shape fidelity for anchor count.",
            Field::Opttolerance,
            slider(0.05..=20.0, cfg.opttolerance, |v| {
                Msg::Edit(EditMsg::Set(Field::Opttolerance, v))
            })
            .step(0.05),
        ),
        setting(
            app,
            "Seam slack",
            format!("{:.1}x", cfg.seam_slack),
            "Loosens the fit only along seams against a near-identical \
             color (within twice the stroke merge distance): such seams \
             are invisible, so they can carry fewer anchors. It multiplies \
             the fit tolerance there; the silhouette and high-contrast \
             edges keep the base tolerance. 1.0 disables.",
            Field::SeamSlack,
            slider(1.0..=4.0, cfg.seam_slack, |v| {
                Msg::Edit(EditMsg::Set(Field::SeamSlack, v))
            })
            .step(0.1),
        ),
        setting(
            app,
            "Simplify tolerance",
            format!("{:.2} px", cfg.simplify),
            "Max deviation in supersampled pixels (source px times the \
             supersample scale) allowed when dropping an anchor. Higher \
             removes more. 0 disables the pass.",
            Field::Simplify,
            slider(0.0..=20.0, cfg.simplify, |v| Msg::Edit(EditMsg::Set(Field::Simplify, v)))
                .step(0.1),
        ),
        setting(
            app,
            "Sticker stroke",
            format!("{:.1} px", cfg.stroke_width),
            "Centered stroke on every path of the layer, in source \
             pixels. 0 = none. Fill layers host the white sticker \
             outline: set it on a \"* Fill\" profile (the Illustrator \
             flow used 11).",
            Field::StrokeWidth,
            slider(0.0..=30.0, cfg.stroke_width as f64, |v| {
                Msg::Edit(EditMsg::Set(Field::StrokeWidth, v))
            })
            .step(0.5),
        ),
        row![
            widgets::help(
                text("Stroke color").size(12).width(130).color(theme::TEXT),
                "\"#rrggbb\"; applies once the hex is valid.",
            ),
            text_input("#ffffff", &sess.stroke_hex)
                .on_input(|s| Msg::Edit(EditMsg::StrokeHex(s)))
                .size(13)
                .width(90),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        widgets::mono(format!(
            "{} anchors ({} after simplify)",
            sess.stages.anchor_count, sess.stages.simplify_anchor_count
        ))
        .size(11)
        .color(theme::MUTED),
    ]
    .spacing(10)
    .into()
}
