//! The pipeline strip: a `Document` chip then the five numbered stage chips.
//! A chip carries the accent when its view is active; while its stage
//! recomputes an indeterminate bar sweeps its bottom edge.

use super::{theme, widgets};
use crate::gui::app::App;
use crate::gui::msg::{Msg, StripView, UiMsg};
use iced::mouse;
use iced::widget::canvas::{Frame, Geometry, Path, Program};
use iced::widget::{button, canvas, container, row, space, stack, text};
use iced::{Alignment, Color, Element, Length, Point, Rectangle, Size};
use std::time::Instant;

const STAGES: [&str; 5] = ["Source", "Flatten", "Palette", "Regions", "Trace"];

pub fn strip(app: &App) -> Element<'_, Msg> {
    let view = app.session().map(|s| s.view).unwrap_or_default();

    let doc_active = view == StripView::Document;
    let doc_chip = chip(
        app,
        text("Document").size(12).into(),
        StripView::Document,
        doc_active,
    );
    // A fixed-height divider: a bare vertical rule stretches the row to fill
    // the whole pane.
    let divider = container(space().width(1).height(18)).style(|_| container::Style {
        background: Some(iced::Background::Color(theme::BORDER)),
        ..Default::default()
    });
    let mut r = row![doc_chip, divider]
        .spacing(10)
        .align_y(Alignment::Center);

    for (i, name) in STAGES.iter().enumerate() {
        let sv = StripView::Stage(i);
        let active = view == sv;
        let number = container(
            widgets::mono(format!("{}", i + 1))
                .size(10)
                .color(if active { theme::BG } else { theme::MUTED }),
        )
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(if active {
                theme::ACCENT
            } else {
                theme::BORDER
            })),
            border: iced::border::rounded(3),
            ..Default::default()
        })
        .padding([1, 5]);
        let label = row![number, text(*name).size(12)]
            .spacing(6)
            .align_y(Alignment::Center);
        r = r.push(chip(app, label.into(), sv, active));
    }

    container(r)
        .style(theme::panel)
        .width(Length::Fill)
        .padding([6, 10])
        .into()
}

/// One strip chip: `label` clickable to show `view`, accented while `active`,
/// carrying the processing bottom bar while its stage recomputes.
fn chip<'a>(app: &App, label: Element<'a, Msg>, view: StripView, active: bool) -> Element<'a, Msg> {
    let busy = app.view_busy(view);
    let content = row![label].spacing(6).align_y(Alignment::Center);
    let chip = button(content)
        .on_press(Msg::Ui(UiMsg::View(view)))
        .style(theme::chip(active))
        .padding([4, 10]);
    if !busy {
        return chip.into();
    }
    let bar = container(chip_bar(app.anim_now))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(iced::alignment::Vertical::Bottom);
    stack![chip, bar].into()
}

const BAR_H: f32 = 2.0;
/// Horizontal inset of the bottom bar, matching the chip's corner radius so
/// the segment reads as clipped to the rounded corners.
const BAR_INSET: f32 = 6.0;
const BAR_FRAC: f32 = 0.35;
const BAR_SECS: f32 = 1.2;

/// The indeterminate bottom bar: a thin accent segment sweeping the chip width.
fn chip_bar<'a>(now: Instant) -> Element<'a, Msg> {
    canvas(ChipBar { now })
        .width(Length::Fill)
        .height(Length::Fixed(BAR_H))
        .into()
}

struct ChipBar {
    now: Instant,
}

impl Program<Msg> for ChipBar {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let track = (bounds.width - 2.0 * BAR_INSET).max(0.0);
        let segw = track * BAR_FRAC;
        // A triangle wave bounces the segment edge to edge with no seam.
        let p = super::anim::phase(self.now, BAR_SECS);
        let bounce = 1.0 - (2.0 * p - 1.0).abs();
        let x = BAR_INSET + bounce * (track - segw);
        frame.fill(
            &Path::rectangle(Point::new(x, 0.0), Size::new(segw, BAR_H)),
            Color { a: 0.9, ..theme::ACCENT },
        );
        vec![frame.into_geometry()]
    }
}
