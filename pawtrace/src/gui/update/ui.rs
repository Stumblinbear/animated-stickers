//! View-only interactions: strip view, active tool, trace sub-view, inspector
//! expansion, zoom, and rail resizing. None recompute the pipeline.

use crate::gui::app::App;
use crate::gui::msg::{Msg, UiMsg};
use iced::Task;

const ZOOM_STEP: f32 = 1.25;
const ZOOM_MIN: f32 = 0.05;
const ZOOM_MAX: f32 = 32.0;

pub(super) fn update(app: &mut App, msg: UiMsg) -> Task<Msg> {
    match msg {
        UiMsg::View(v) => {
            if let Some(s) = app.session_mut() {
                s.view = v;
            }
        }
        UiMsg::Tool(t) => app.tool = t,
        UiMsg::TraceView(tv) => {
            if let Some(s) = app.session_mut() {
                s.trace_view = tv;
            }
        }
        UiMsg::ExpandStage(n) => {
            if let Some(s) = app.session_mut() {
                // Toggle: clicking the open section collapses the accordion.
                s.expanded = if s.expanded == n { 0 } else { n };
            }
        }
        UiMsg::ZoomIn => zoom_by(app, ZOOM_STEP),
        UiMsg::ZoomOut => zoom_by(app, 1.0 / ZOOM_STEP),
        UiMsg::ZoomFit => {
            if let Some(s) = app.session_mut() {
                s.set_zoom(None);
            }
        }
        UiMsg::PaneResized(e) => app.panes.resize(e.split, e.ratio),
    }
    Task::none()
}

/// Scales the current zoom, seeding from 1:1 when the view was fit-to-window
/// since the precise fit factor is only known inside the canvas.
fn zoom_by(app: &mut App, factor: f32) {
    if let Some(s) = app.session_mut() {
        let base = s.zoom().unwrap_or(1.0);
        s.set_zoom(Some((base * factor).clamp(ZOOM_MIN, ZOOM_MAX)));
    }
}
