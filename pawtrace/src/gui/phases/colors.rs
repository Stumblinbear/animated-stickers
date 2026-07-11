//! The Colors phase: source pixels through feature merge to the extracted
//! palette and its remap. Owns the phase's sub-views and its inspector section
//! (detail, palette cap, color cleanup, and the palette swatches with their
//! nearest-neighbor distance and click-to-lock).

use super::SubView;
use crate::gui::app::App;
use crate::gui::fields::Field;
use crate::gui::msg::{EditMsg, Msg};
use crate::gui::view::icons;
use crate::gui::view::inspector::setting::setting;
use iced::widget::{button, column, container, row, slider, text};
use iced::{Alignment, Background, Color, Element};

pub const SUBVIEWS: &[SubView] = &[
    SubView::Source,
    SubView::Features,
    SubView::Merged,
    SubView::Palette,
];
pub const DEFAULT_SUBVIEW: SubView = SubView::Palette;

/// The status-line detail: the extracted palette's color count.
pub fn status_detail(app: &App) -> Option<String> {
    app.session().map(|s| format!("{} colors", s.palette().len()))
}

pub fn inspector(app: &App) -> Element<'_, Msg> {
    let Some(sess) = app.session() else {
        return column![].into();
    };

    let cfg = &sess.cfg;

    column![
        setting(
            app,
            "Detail",
            format!("{:.1}px", cfg.detail),
            "Smallest feature worth keeping, in pixels at 512-canvas \
             scale. Drives the palette floor and speckle removal.",
            Field::Detail,
            slider(0.5..=24.0, cfg.detail as f64, |v| Msg::Edit(EditMsg::Set(
                Field::Detail,
                v
            )))
            .step(0.5),
        ),
        setting(
            app,
            "Max colors",
            format!("{}", cfg.max_colors),
            "Palette safety cap; extraction usually self-terminates \
             below it.",
            Field::MaxColors,
            slider(2.0..=64.0, cfg.max_colors as f64, |v| {
                Msg::Edit(EditMsg::Set(Field::MaxColors, v))
            })
            .step(1.0),
        ),
        setting(
            app,
            "Color cleanup",
            if cfg.color_cleanup == 0 {
                "off".into()
            } else {
                format!("{} px", cfg.color_cleanup)
            },
            "Reassigns each pixel to the majority color in a window \
             (kernel width in supersampled px, 0 = off). Cleans jagged \
             or speckled edges where two similar palette colors (a dark \
             line on dark fur) got assigned noisily. Larger kernels also \
             swallow 1px detail strokes, so raise it only when a \
             boundary looks ragged.",
            Field::ColorCleanup,
            slider(0.0..=9.0, cfg.color_cleanup as f64, |v| {
                Msg::Edit(EditMsg::Set(Field::ColorCleanup, v))
            })
            .step(1.0),
        ),
        text("PALETTE · CLICK TO LOCK")
            .size(9)
            .color(crate::gui::view::theme::MUTED),
        swatches(app),
    ]
    .spacing(10)
    .into()
}

/// The extracted palette as clickable swatches. Each shows its hex, a lock
/// marker when locked, and its OKLab distance to the nearest other slot.
fn swatches(app: &App) -> Element<'_, Msg> {
    let Some(sess) = app.session() else {
        return row![].into();
    };

    let pal = sess.palette();
    let locked = &sess.cfg.locked;
    let nearest = |i: usize| -> f32 {
        pal.iter()
            .enumerate()
            .filter(|&(j, _)| j != i)
            .map(|(_, o)| pal[i].dist(*o))
            .fold(f32::INFINITY, f32::min)
    };

    pal.iter()
        .enumerate()
        .fold(row![].spacing(4), |r, (i, c)| {
            let is_locked = locked.contains(c);
            let hex = c.to_hex();

            let de = nearest(i);
            let de_text = if de.is_finite() {
                format!("{de:.3}")
            } else {
                "-".into()
            };

            let color = *c;

            let top = if is_locked {
                row![icons::icon(icons::LOCK).size(10), text(hex).size(11)]
                    .spacing(3)
                    .align_y(Alignment::Center)
            } else {
                row![text(hex).size(11)]
            };
            let swatch = container(column![top, text(de_text).size(9)])
                .style(move |_: &iced::Theme| container::Style {
                    background: Some(Background::Color(color.into())),
                    text_color: Some(
                        if color.r() as u32 + color.g() as u32 + color.b() as u32 > 380 {
                            Color::BLACK
                        } else {
                            Color::WHITE
                        },
                    ),
                    ..Default::default()
                })
                .padding(4);

            r.push(
                button(swatch)
                    .padding(if is_locked { 3 } else { 0 })
                    .on_press(Msg::Edit(EditMsg::ToggleLock(*c))),
            )
        })
        .into()
}
