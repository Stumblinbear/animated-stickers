//! Small shared builders used across the view modules.

use super::theme;
use crate::gui::msg::Msg;
use iced::widget::{container, text, tooltip, Text};
use iced::{Background, Color, Element, Font};

/// Text in the monospace face, for numeric readouts.
pub fn mono<'a>(s: impl text::IntoFragment<'a>) -> Text<'a> {
    text(s).font(Font::MONOSPACE)
}

/// `n` with comma thousands separators, for the status readouts.
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A filled dot of `size` px in `color`, for status and anchor indicators.
pub fn dot<'a>(color: Color, size: f32) -> Element<'a, Msg> {
    container(iced::widget::space().width(size).height(size))
        .style(move |_| container::Style {
            background: Some(Background::Color(color)),
            border: iced::border::rounded(size / 2.0),
            ..Default::default()
        })
        .into()
}

/// Wraps `content` with a hover tooltip carrying `help`, styled as a card.
pub fn help<'a>(
    content: impl Into<Element<'a, Msg>>,
    help: &'a str,
) -> Element<'a, Msg> {
    tooltip(
        content,
        container(text(help).size(12))
            .padding(6)
            .max_width(320)
            .style(theme::card),
        tooltip::Position::Bottom,
    )
    .into()
}
