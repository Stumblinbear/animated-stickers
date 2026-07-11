//! The Paint phase: supersample and flatten the layer into a clean raster.
//! Owns the phase's sub-views and its inspector section (scale, alpha
//! threshold, mode filter).

use super::SubView;
use crate::gui::app::App;
use crate::gui::fields::Field;
use crate::gui::msg::{EditMsg, Msg};
use crate::gui::view::inspector::setting::setting;
use iced::widget::{column, slider};
use iced::Element;

pub const SUBVIEWS: &[SubView] = &[SubView::Flatten, SubView::Remap];
pub const DEFAULT_SUBVIEW: SubView = SubView::Remap;

/// The status-line detail: Paint has no headline count.
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
            "Scale",
            format!("{}x", cfg.scale),
            "Supersample factor. Boundary precision is 1/scale source \
             pixels; cost grows with its square. 3 is calibrated; above \
             4 rarely earns its cost.",
            Field::Scale,
            slider(1.0..=6.0, cfg.scale as f64, |v| Msg::Edit(EditMsg::Set(Field::Scale, v)))
                .step(1.0),
        ),
        setting(
            app,
            "Alpha threshold",
            format!("{}%", (cfg.alpha_threshold as f64 / 255.0 * 100.0).round()),
            "Opacity cutoff after upsampling: a pixel at or above this is \
             art, below is background.",
            Field::AlphaThreshold,
            slider(
                1.0..=100.0,
                cfg.alpha_threshold as f64 / 255.0 * 100.0,
                |pct| Msg::Edit(EditMsg::Set(Field::AlphaThreshold, pct / 100.0 * 255.0)),
            )
            .step(1.0),
        ),
        setting(
            app,
            "Mode filter",
            if cfg.mode_filter == 0 {
                "off".into()
            } else {
                format!("{} px", cfg.mode_filter)
            },
            "Majority-vote denoise before quantization: kernel width in \
             supersampled pixels (source px times the supersample \
             scale), odd, 0 = off. Off by default: the smooth upsample \
             plus perceptual remap already cover its job.",
            Field::ModeFilter,
            slider(0.0..=15.0, cfg.mode_filter as f64, |v| {
                Msg::Edit(EditMsg::Set(Field::ModeFilter, v))
            })
            .step(1.0),
        ),
    ]
    .spacing(10)
    .into()
}
