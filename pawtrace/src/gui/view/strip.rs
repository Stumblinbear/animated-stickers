//! The pipeline strip: a `◆ Document` chip set apart by a divider, then the
//! four phase pills. Beneath the strip a darker inset sub-panel exposes the
//! active phase's intermediate renders as a breadcrumb of steps; Document has
//! no sub-views, so it hides the panel.
//!
//! A pill carries the accent when its phase is viewed and a small eye cue so
//! "what I'm viewing" stays distinct from "what I'm editing". While a
//! phase recomputes, an indeterminate bar sweeps the pill's bottom edge.

use super::{icons, theme, widgets};
use crate::gui::app::App;
use crate::gui::msg::{Msg, Phase, StripView, UiMsg};
use crate::gui::phases::SubView;
use iced::mouse;
use iced::widget::canvas::{Frame, Geometry, Path, Program};
use iced::widget::{button, canvas, column, container, row, space, stack, text};
use iced::{Alignment, Color, Element, Length, Point, Rectangle, Size};
use std::time::Instant;

pub fn strip(app: &App) -> Element<'_, Msg> {
    let view = app.session().map(|s| s.view).unwrap_or_default();
    let doc_active = view == StripView::Document;

    let doc_label = row![
        icons::icon(icons::LAYERS).size(13),
        text("Document").size(12),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    let doc_chip = chip(app, doc_label.into(), StripView::Document, doc_active, false);

    let divider = container(space().width(1).height(18)).style(|_| container::Style {
        background: Some(iced::Background::Color(theme::BORDER)),
        ..Default::default()
    });
    let mut pills = row![doc_chip, divider]
        .spacing(10)
        .align_y(Alignment::Center);

    let failed = app.session().and_then(|s| s.trace_error.as_ref()).map(|e| e.phase);
    for p in crate::gui::phases::PHASES {
        let sv = StripView::Phase(p);
        let active = view == sv;
        let label = text(p.label()).size(12).into();
        pills = pills.push(chip(app, label, sv, active, failed == Some(p)));
    }

    let pill_bar = container(pills)
        .style(theme::panel)
        .width(Length::Fill)
        .padding([6, 10]);

    match view {
        StripView::Document => pill_bar.into(),
        StripView::Phase(p) => column![pill_bar, sub_panel(app, p)].into(),
    }
}

/// The sub-view panel: the phase's tag then its intermediate renders as
/// breadcrumb steps separated by arrows. The selected step is accented; a step
/// with no render yet is dimmed and disabled with an explanatory tooltip.
fn sub_panel(app: &App, phase: Phase) -> Element<'_, Msg> {
    let selected = app.active_subview();
    let tag = widgets::mono(phase.label().to_uppercase()).size(9).color(theme::MUTED);

    let mut steps = row![tag].spacing(10).align_y(Alignment::Center);
    for (i, &sv) in phase.subviews().iter().enumerate() {
        if i > 0 {
            steps = steps.push(
                icons::icon(icons::ARROW_RIGHT).size(9).color(theme::BORDER),
            );
        }
        steps = steps.push(step(sv, Some(sv) == selected));
    }

    container(steps)
        .style(theme::inset)
        .width(Length::Fill)
        .padding([5, 12])
        .into()
}

/// One breadcrumb step. A sub-view with no render yet is dimmed and unclickable.
fn step<'a>(sv: SubView, selected: bool) -> Element<'a, Msg> {
    if sv.stage().is_none() {
        let dim = button(text(sv.label()).size(11).color(theme::BORDER))
            .style(theme::flat_button)
            .padding([2, 6]);
        return widgets::help(dim, "This intermediate render is not available yet.");
    }
    let color = if selected { theme::ACCENT } else { theme::MUTED };
    button(text(sv.label()).size(11).color(color))
        .on_press(Msg::Ui(UiMsg::SubView(sv)))
        .style(theme::flat_button)
        .padding([2, 6])
        .into()
}

/// One strip chip: `label` clickable to show `view`, accented while `active`,
/// red while `failed`, with an eye cue on the viewed phase and the processing
/// bottom bar while its phase recomputes.
fn chip<'a>(
    app: &App,
    label: Element<'a, Msg>,
    view: StripView,
    active: bool,
    failed: bool,
) -> Element<'a, Msg> {
    let busy = app.view_busy(view);
    let mut content = row![label].spacing(6).align_y(Alignment::Center);
    // The viewing cue distinguishes the viewed phase (strip) from the edited
    // section (inspector), which can differ.
    if active && matches!(view, StripView::Phase(_)) {
        content = content.push(icons::icon(icons::EYE).size(10).color(theme::ACCENT));
    }
    let base = button(content)
        .on_press(Msg::Ui(UiMsg::View(view)))
        .padding([4, 10]);
    // The danger style is a plain fn item and the active style an opaque
    // closure, so pick the concrete Button in each branch rather than unifying
    // the style values.
    let chip = if failed {
        base.style(theme::chip_danger)
    } else {
        base.style(theme::chip(active))
    };
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
