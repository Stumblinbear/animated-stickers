//! The inspector rail: a per-stage accordion of settings with a header naming
//! the selected layer and a footer holding the override toggle, the profile
//! target, and a reset.

mod section;
mod setting;
mod stage2;
mod stage3;
mod stage4;
mod stage5;

use crate::gui::app::{App, DocState};
use crate::gui::msg::{EditMsg, Msg, StripView};
use crate::gui::view::{icons, theme, widgets};
use iced::widget::{button, checkbox, column, container, row, scrollable, space, text, text_input};
use iced::{Alignment, Element, Length};

const PATTERN_HELP: &str = "A case-sensitive glob against the whole layer name: \
    \"*\" matches any characters, everything else is literal. Add spaces \
    yourself for word boundaries: \"Deer *\" is a prefix, \"* Fill\" a suffix, \
    \"Deer * Fill\" anchors both ends. The most specific pattern wins.";

pub fn inspector(app: &App) -> Element<'_, Msg> {
    let Some(sess) = app.session() else {
        return container(text("No document open").size(12).color(theme::MUTED))
            .style(theme::panel)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(10)
            .into();
    };
    if sess.selection.is_empty() {
        return no_selection();
    }
    let layer_name = app.layer_name().unwrap_or_else(|| "-".into());

    let mut header = row![
        text("INSPECTOR").size(11).color(theme::MUTED),
        space().width(Length::Fill),
        text(layer_name.clone()).size(13),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if app.override_count() > 0 {
        header = header.push(
            container(text("OVERRIDE").size(9).color(theme::BG))
                .style(|_| container::Style {
                    background: Some(iced::Background::Color(theme::ACCENT)),
                    border: iced::border::rounded(3),
                    ..Default::default()
                })
                .padding([2, 5]),
        );
    }

    let e = sess.expanded;
    let now = app.anim_now;
    let sections = column![
        section::section(1, "Source", busy(app, 1), now, e == 1, source_body()),
        section::section(2, "Supersample & flatten", busy(app, 2), now, e == 2, stage2::stage2(app)),
        section::section(3, "Palette & remap", busy(app, 3), now, e == 3, stage3::stage3(app)),
        section::section(4, "Regions & absorption", busy(app, 4), now, e == 4, stage4::stage4(app)),
        section::section(5, "Trace", busy(app, 5), now, e == 5, stage5::stage5(app)),
    ]
    .spacing(0);

    let body = column![
        header,
        scrollable(sections).height(Length::Fill),
        footer(app, sess, &layer_name),
    ]
    .spacing(10)
    .padding(10);

    container(body)
        .style(theme::panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// The inspector with no layer selected: the rail heading over a muted hint,
/// no stage sections, and no override footer, since there is nothing to edit.
fn no_selection<'a>() -> Element<'a, Msg> {
    let header = row![
        text("INSPECTOR").size(11).color(theme::MUTED),
        space().width(Length::Fill),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let hint = container(
        text("No layer selected. Click a layer to edit its settings.")
            .size(12)
            .color(theme::MUTED),
    )
    .height(Length::Fill)
    .center_x(Length::Fill)
    .padding(10);
    let body = column![header, hint].spacing(10).padding(10);
    container(body)
        .style(theme::panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// Whether the stage feeding inspector section `n` is recomputing.
fn busy(app: &App, n: usize) -> bool {
    app.view_busy(StripView::Stage(n - 1))
}

fn source_body<'a>() -> Element<'a, Msg> {
    text("The layer as painted, cropped to its art. Nothing to configure here.")
        .size(12)
        .color(theme::MUTED)
        .into()
}

fn footer<'a>(app: &'a App, sess: &'a DocState, layer_name: &str) -> Element<'a, Msg> {
    let matched = app
        .stack_sel()
        .match_name(layer_name)
        .unwrap_or_else(|| "default".into());
    let n = app.override_count();
    let sub = text(if n == 1 {
        format!("1 setting differs from {matched}")
    } else {
        format!("{n} settings differ from {matched}")
    })
    .size(10)
    .color(theme::MUTED);
    let reset = button(
        row![
            icons::icon(icons::RESET).size(10).color(theme::ACCENT),
            text("Reset all").size(11).color(theme::ACCENT),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .on_press(Msg::Edit(EditMsg::ResetLayer))
    .style(theme::flat_button)
    .padding([3, 8]);
    let toggle = row![
        column![text("Override this layer").size(12), sub].spacing(2),
        space().width(Length::Fill),
        reset,
        iced::widget::toggler(sess.override_layer)
            .on_toggle(|b| Msg::Edit(EditMsg::OverrideLayer(b)))
            .size(20),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let profile: Element<'a, Msg> = if sess.override_layer {
        space().height(0).into()
    } else {
        let hint = if sess.profile_input.trim().is_empty() {
            ("writes to [default] (all layers)", theme::MUTED)
        } else if app.profile_input_matches_layer() {
            ("matches this layer", theme::ACCENT)
        } else {
            ("does not match this layer", theme::ACCENT_DIM)
        };
        column![
            widgets::help(
                text("Profile pattern").size(11).color(theme::MUTED),
                PATTERN_HELP,
            ),
            text_input("Deer *  ·  * Fill", &sess.profile_input)
                .on_input(|s| Msg::Edit(EditMsg::ProfileInput(s)))
                .size(12),
            text(hint.0).size(10).color(hint.1),
            checkbox(app.edit_global)
                .label("Save to global library")
                .on_toggle(|b| Msg::Edit(EditMsg::EditGlobal(b)))
                .size(14)
                .text_size(11),
        ]
        .spacing(4)
        .into()
    };

    column![toggle, profile].spacing(8).into()
}
