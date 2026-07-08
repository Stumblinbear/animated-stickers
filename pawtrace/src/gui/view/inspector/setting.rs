//! One setting row: label, slider, value readout, and hover help. A field
//! customized at the current write target gains an accent marker and a per-row
//! reset. In override mode, rows the layer does not override are dimmed as
//! inherited.

use crate::gui::app::App;
use crate::gui::fields::Field;
use crate::gui::msg::{EditMsg, Msg};
use crate::gui::view::{icons, theme, widgets};
use iced::widget::{button, row, slider::Slider, space, text};
use iced::{Alignment, Element, Length};

pub fn setting<'a>(
    app: &App,
    label: &'a str,
    value: String,
    help: &'a str,
    field: Field,
    control: Slider<'a, f64, Msg>,
) -> Element<'a, Msg> {
    let override_mode = app.session().is_some_and(|s| s.override_layer);
    // In override mode a row reads as changed when the layer itself overrides
    // the field; otherwise it reflects the profile write target.
    let modified = if override_mode {
        app.field_overridden(field)
    } else {
        app.field_is_set(field)
    };
    let inherited = override_mode && !app.field_overridden(field);

    let color = if modified {
        theme::ACCENT
    } else if inherited {
        theme::MUTED
    } else {
        theme::TEXT
    };
    let marker: Element<'a, Msg> = if modified {
        widgets::dot(theme::ACCENT, 5.0)
    } else {
        space().width(5).into()
    };
    let label_text = row![marker, text(label).size(12).color(color)]
        .spacing(6)
        .align_y(Alignment::Center)
        .width(130);

    let reset: Element<'a, Msg> = if modified {
        button(icons::icon(icons::RESET).size(10).color(theme::ACCENT))
            .on_press(Msg::Edit(EditMsg::ResetField(field)))
            .style(theme::icon_box)
            .padding(4)
            .into()
    } else {
        // Sized to the reset box so rows stay column-aligned without one.
        space().width(20).into()
    };

    row![
        widgets::help(label_text, help),
        control
            .on_release(Msg::Edit(EditMsg::Seal))
            .style(theme::setting_slider(modified))
            .width(Length::Fill),
        widgets::mono(value).size(11).width(64),
        reset,
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}
