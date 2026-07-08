//! The editor's dark, amber-accented palette and the shared widget styles
//! built from it. Colors are monochrome plus one warm accent that carries
//! every kind of state: selection, activity, processing, and protection.

use iced::widget::{button, container, slider};
use iced::{border, Background, Color, Theme};

const fn rgb8(r: u8, g: u8, b: u8) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}

pub const BG: Color = rgb8(0x14, 0x14, 0x17);
pub const SURFACE: Color = rgb8(0x1b, 0x1b, 0x20);
pub const SURFACE2: Color = rgb8(0x22, 0x22, 0x28);
pub const BORDER: Color = rgb8(0x2c, 0x2c, 0x34);
pub const TEXT: Color = rgb8(0xd8, 0xd8, 0xde);
pub const MUTED: Color = rgb8(0x8a, 0x8a, 0x94);
pub const ACCENT: Color = rgb8(0xe8, 0xa3, 0x3d);
pub const ACCENT_DIM: Color = rgb8(0x8f, 0x6a, 0x2f);

/// The window theme: a custom dark palette with the amber accent as primary.
pub fn theme() -> Theme {
    Theme::custom(
        "Pawtrace",
        iced::theme::Palette {
            background: BG,
            text: TEXT,
            primary: ACCENT,
            success: ACCENT,
            warning: ACCENT,
            danger: rgb8(0xd0, 0x50, 0x40),
        },
    )
}

/// A raised surface: rails, toolbar, and status bar.
pub fn panel(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE)),
        text_color: Some(TEXT),
        border: border::width(1.0).color(BORDER),
        ..Default::default()
    }
}

/// An inset card or input surface.
pub fn card(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE2)),
        text_color: Some(TEXT),
        border: border::rounded(4).width(1.0).color(BORDER),
        ..Default::default()
    }
}

/// A floating menu or modal panel, lifted off the artwork by a soft shadow.
pub fn menu(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE)),
        text_color: Some(TEXT),
        border: border::rounded(10).width(1.0).color(BORDER),
        shadow: iced::Shadow {
            color: Color { a: 0.5, ..Color::BLACK },
            offset: iced::Vector::new(0.0, 6.0),
            blur_radius: 18.0,
        },
        ..Default::default()
    }
}

/// A small bordered square holding a single icon control, like a per-row
/// reset.
pub fn icon_box(_: &Theme, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered);
    button::Style {
        background: Some(Background::Color(if hovered { BORDER } else { SURFACE2 })),
        text_color: ACCENT,
        border: border::rounded(4).width(1.0).color(BORDER),
        ..Default::default()
    }
}

/// A small pill labeling a profile or badge.
pub fn badge(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE2)),
        text_color: Some(MUTED),
        border: border::rounded(4).width(1.0).color(BORDER),
        ..Default::default()
    }
}

/// A pipeline chip: a dark pill whose border and text turn amber when its
/// view is active.
pub fn chip(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered);
        button::Style {
            background: Some(Background::Color(if hovered { BORDER } else { SURFACE2 })),
            text_color: if active { ACCENT } else { TEXT },
            border: border::rounded(6)
                .width(1.0)
                .color(if active { ACCENT_DIM } else { BORDER }),
            ..Default::default()
        }
    }
}

/// A tool button: amber-filled while its tool is active, flat otherwise.
pub fn tool_button(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered);
        let bg = if active {
            Some(Background::Color(ACCENT))
        } else if hovered {
            Some(Background::Color(SURFACE2))
        } else {
            None
        };
        button::Style {
            background: bg,
            text_color: if active { BG } else { MUTED },
            border: border::rounded(5),
            ..Default::default()
        }
    }
}

/// A layer row. `selected` highlights it in accent, `dimmed` fades a hidden
/// or excluded layer.
pub fn layer_row(selected: bool, dimmed: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered);
        let bg = if selected {
            SURFACE2
        } else if hovered {
            SURFACE
        } else {
            Color::TRANSPARENT
        };
        let text = if dimmed { MUTED } else { TEXT };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: text,
            border: if selected {
                border::rounded(3).width(1.0).color(ACCENT_DIM)
            } else {
                border::rounded(3)
            },
            ..Default::default()
        }
    }
}

/// A thin setting slider: the filled side and handle carry the accent when
/// the field is customized at the edit target, neutral gray otherwise.
pub fn setting_slider(modified: bool) -> impl Fn(&Theme, slider::Status) -> slider::Style {
    move |_, _| {
        let fill = if modified { ACCENT } else { MUTED };
        slider::Style {
            rail: slider::Rail {
                backgrounds: (Background::Color(fill), Background::Color(BORDER)),
                width: 3.0,
                border: border::rounded(2),
            },
            handle: slider::Handle {
                shape: slider::HandleShape::Circle { radius: 6.0 },
                background: Background::Color(if modified { ACCENT } else { TEXT }),
                border_width: 0.0,
                border_color: Color::TRANSPARENT,
            },
        }
    }
}

/// A borderless text button that reads as a plain clickable label.
pub fn flat_button(_: &Theme, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered);
    button::Style {
        background: hovered.then_some(Background::Color(SURFACE2)),
        text_color: TEXT,
        border: border::rounded(3),
        ..Default::default()
    }
}
