//! The Shapes phase: segment the remap into regions and absorb thin bands.
//! Owns the phase's sub-views and its inspector section (absorption, the
//! stroke-merge settings, and the region count readout).

use super::SubView;
use crate::gui::app::App;
use crate::gui::fields::Field;
use crate::gui::msg::{EditMsg, Msg};
use crate::gui::view::inspector::setting::setting;
use crate::gui::view::{theme, widgets};
use iced::widget::{column, slider};
use iced::Element;

pub const SUBVIEWS: &[SubView] = &[SubView::Regions, SubView::Fates, SubView::Stack];
pub const DEFAULT_SUBVIEW: SubView = SubView::Regions;

/// The status-line detail: Shapes has no headline count.
pub fn status_detail(_app: &App) -> Option<String> {
    None
}

pub fn inspector(app: &App) -> Element<'_, Msg> {
    let Some(sess) = app.session() else {
        return column![].into();
    };

    let cfg = &sess.cfg;

    column![
        setting(
            app,
            "Absorb distance",
            format!("{:.3} \u{394}E", cfg.absorb_dist),
            "Bands within this perceptual distance (OKLab \u{394}E) of an \
             adjacent region merge into it. 0 disables absorption.",
            Field::AbsorbDist,
            slider(0.0..=0.30, cfg.absorb_dist as f64, |v| {
                Msg::Edit(EditMsg::Set(Field::AbsorbDist, v))
            })
            .step(0.005),
        ),
        setting(
            app,
            "Absorb aggressiveness",
            format!("{:.2}x", cfg.absorb_aggr),
            "Scales how thick a band may be and still absorb. 1.0 is the \
             baseline; the right value depends on the artwork, so raise \
             it to swallow chunkier transitions and lower it to keep \
             only the thinnest. It raises the ceiling for deliberate \
             features too, so pin any it erases.",
            Field::AbsorbAggr,
            slider(0.0..=3.0, cfg.absorb_aggr as f64, |v| {
                Msg::Edit(EditMsg::Set(Field::AbsorbAggr, v))
            })
            .step(0.05),
        ),
        setting(
            app,
            "Stroke merge distance",
            format!("{:.3} \u{394}E", cfg.stroke_merge_dist),
            "Adjacent thin regions within this perceptual distance \
             (OKLab \u{394}E) fuse as segments of one stroke, reuniting \
             linework that quantization cut into pieces. Wide regions \
             never fuse, so gradient banding keeps its structure. 0 \
             disables the merge.",
            Field::StrokeMergeDist,
            slider(0.0..=0.30, cfg.stroke_merge_dist as f64, |v| {
                Msg::Edit(EditMsg::Set(Field::StrokeMergeDist, v))
            })
            .step(0.005),
        ),
        setting(
            app,
            "Stroke merge width",
            format!("{:.1} px", cfg.stroke_merge_width),
            "How wide (source px) a region may be and still count as a \
             stroke segment. Set it just above the artwork's line \
             weight: lower keeps close-colored shapes apart, higher \
             fuses chunkier linework.",
            Field::StrokeMergeWidth,
            slider(0.0..=12.0, cfg.stroke_merge_width as f64, |v| {
                Msg::Edit(EditMsg::Set(Field::StrokeMergeWidth, v))
            })
            .step(0.5),
        ),
        widgets::mono(format!(
            "{} regions, {} pinned",
            sess.preview.region_count,
            app.doc()
                .and_then(|d| d.inputs.get(&sess.selected_layer))
                .map_or(0, |i| i.pins.len())
        ))
        .size(11)
        .color(theme::MUTED),
    ]
    .spacing(10)
    .into()
}
