//! View-only interactions: strip phase, sub-view, active tool, brush controls,
//! inspector expansion with edit-gating, zoom, and rail resizing. None
//! recompute the pipeline.

use crate::gui::app::App;
use crate::gui::msg::{Msg, StripView, UiMsg};
use crate::gui::tools;
use iced::Task;

const ZOOM_STEP: f32 = 1.25;
const ZOOM_MIN: f32 = 0.05;
const ZOOM_MAX: f32 = 32.0;

pub(super) fn update(app: &mut App, msg: UiMsg) -> Task<Msg> {
    match msg {
        UiMsg::View(v) => {
            if let Some(s) = app.session_mut() {
                // Phase views show the selected layer; with nothing selected
                // there is nothing to show, so the chips stay on Document.
                if v == StripView::Document || !s.selection.is_empty() {
                    s.view = v;
                }
            }
            app.reconcile_tool();
        }
        UiMsg::SubView(sv) => {
            if let Some(s) = app.session_mut() {
                s.phase_sub[sv.phase()] = sv;
            }
            app.reconcile_tool();
        }
        UiMsg::Tool(t) => {
            // A shortcut can name a tool the current view does not offer;
            // ignore it rather than activating a tool with no target.
            if app.tool_applicable(t) {
                app.tools.active = t;
            }
        }
        UiMsg::ExpandSection(p) => {
            if app.section_locked(p) {
                // A locked section can't be edited in place: clicking it jumps
                // the view to that phase (which unlocks it) and expands it.
                if let Some(s) = app.session_mut() {
                    if !s.selection.is_empty() {
                        s.view = StripView::Phase(p);
                        s.expanded = Some(p);
                    }
                }
                app.reconcile_tool();
            } else if let Some(s) = app.session_mut() {
                // Toggle: clicking the open section collapses the accordion.
                s.expanded = if s.expanded == Some(p) { None } else { Some(p) };
            }
        }
        UiMsg::ToolMsg(m) => tools::update(&mut app.tools, m),
        UiMsg::Retry => {
            if let Some(s) = app.session_mut() {
                s.trace_error = None;
            }
            return app.preview_tasks();
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
