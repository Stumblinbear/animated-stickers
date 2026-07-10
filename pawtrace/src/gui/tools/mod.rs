//! Canvas tools as self-contained input policies. Each tool module owns its
//! parameter state, its parameter-edit messages, its fly-out, its
//! applicability, its icon, and its canvas interaction. The shell composes
//! them: it iterates [`Tool::ALL`], asks each whether it applies to the current
//! view, and delegates the active tool's edits and fly-out here.

pub mod heat;
pub mod lock;
pub mod pin;
pub mod protect;
pub mod select;

use super::app::App;
use super::msg::{Msg, StripView, UiMsg};
use super::phases::SubView;
use super::view::theme;
use super::view::widgets;
use iced::widget::{button, container, row, slider, text};
use iced::{Alignment, Color, Element, Length};

/// The active canvas tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Select,
    Pin,
    Lock,
    Protect,
    Heat,
}

impl Tool {
    /// Every tool, in rail order.
    pub const ALL: [Tool; 5] = [Tool::Select, Tool::Pin, Tool::Lock, Tool::Protect, Tool::Heat];

    /// The rail button glyph for this tool.
    pub fn icon(self) -> char {
        match self {
            Tool::Select => select::ICON,
            Tool::Pin => pin::ICON,
            Tool::Lock => lock::ICON,
            Tool::Protect => protect::ICON,
            Tool::Heat => heat::ICON,
        }
    }

    /// Whether this tool is offered on `view`, whose active sub-view is `sub`
    /// (`None` on the Document view).
    pub fn applies(self, view: StripView, sub: Option<SubView>) -> bool {
        match self {
            Tool::Select => select::applies(view, sub),
            Tool::Pin => pin::applies(view, sub),
            Tool::Lock => lock::applies(view, sub),
            Tool::Protect => protect::applies(view, sub),
            Tool::Heat => heat::applies(view, sub),
        }
    }
}

/// The active tool and the per-tool parameter state of the stateful tools.
#[derive(Default)]
pub struct Tools {
    pub active: Tool,
    pub protect: protect::State,
    pub heat: heat::State,
}

/// A parameter edit addressed to one tool's fly-out.
#[derive(Debug, Clone)]
pub enum ToolMsg {
    Protect(protect::Msg),
    Heat(heat::Msg),
}

/// Applies a fly-out parameter edit to its owning tool's state.
pub fn update(tools: &mut Tools, msg: ToolMsg) {
    match msg {
        ToolMsg::Protect(m) => protect::update(&mut tools.protect, m),
        ToolMsg::Heat(m) => heat::update(&mut tools.heat, m),
    }
}

/// The active tool's option fly-out, or `None` when it has no options.
pub fn flyout(tools: &Tools) -> Option<Element<'_, Msg>> {
    match tools.active {
        Tool::Protect => Some(protect::flyout(&tools.protect)),
        Tool::Heat => Some(heat::flyout(&tools.heat)),
        _ => None,
    }
}

/// Routes a canvas press to the active tool, in the shown view's coordinates.
/// A tool acts on the primary layer, so a press with nothing selected is a
/// no-op.
pub fn press(app: &mut App, p: iced::Point) -> iced::Task<Msg> {
    if app.session().is_none_or(|s| s.selection.is_empty()) {
        return iced::Task::none();
    }
    match app.tools.active {
        Tool::Pin => pin::press(app, p),
        Tool::Lock => lock::press(app, p),
        _ => iced::Task::none(),
    }
}

/// Routes a canvas drag to the active tool. Only the pin tool paints on drag.
pub fn drag(app: &mut App, p: iced::Point) -> iced::Task<Msg> {
    match app.tools.active {
        Tool::Pin => pin::drag(app, p),
        _ => iced::Task::none(),
    }
}

/// Wraps a tool's parameter edit as an app message.
fn edit(msg: ToolMsg) -> Msg {
    Msg::Ui(UiMsg::ToolMsg(msg))
}

// Shared fly-out chrome. The protect and heat cards read as one control style,
// so their surface, section title, slider rows, and clear button live here.

fn title<'a>(label: &'a str) -> Element<'a, Msg> {
    text(label).size(9).color(theme::MUTED).into()
}

fn slider_row<'a>(
    label: &'a str,
    value: String,
    control: slider::Slider<'a, f32, Msg>,
) -> Element<'a, Msg> {
    row![
        text(label).size(11).color(theme::MUTED).width(38),
        control.style(theme::setting_slider(true)).width(Length::Fill),
        widgets::mono(value).size(10).width(38),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

fn clear_button<'a>(msg: Msg) -> Element<'a, Msg> {
    button(
        row![
            super::view::icons::icon(super::view::icons::RESET).size(10).color(theme::MUTED),
            text("Clear").size(11).color(theme::MUTED),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .on_press(msg)
    .style(theme::flat_button)
    .padding([3, 6])
    .into()
}

fn card(body: Element<'_, Msg>) -> Element<'_, Msg> {
    container(body)
        .style(flyout_style)
        .width(Length::Fixed(190.0))
        .padding(10)
        .into()
}

/// The fly-out surface: translucent so the art stays visible, lifted by a soft
/// shadow so it reads as floating beside the rail.
fn flyout_style(_: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(Color { a: 0.92, ..theme::SURFACE })),
        text_color: Some(theme::TEXT),
        border: iced::border::rounded(8).width(1.0).color(theme::BORDER),
        shadow: iced::Shadow {
            color: Color { a: 0.5, ..Color::BLACK },
            offset: iced::Vector::new(0.0, 4.0),
            blur_radius: 14.0,
        },
        ..Default::default()
    }
}
