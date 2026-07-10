//! Setting edits and the write-mode toggles that redirect where they land.

use crate::gui::app::App;
use crate::gui::fields;
use crate::gui::msg::{EditMsg, Msg};
use crate::profiles;
use iced::Task;

pub(super) fn update(app: &mut App, msg: EditMsg) -> Task<Msg> {
    match msg {
        EditMsg::Set(field, v) => {
            if let Some(s) = app.session_mut() {
                fields::apply(&mut s.cfg, field, v);
            }
            app.write_field(field);
            app.preview_tasks()
        }
        EditMsg::ResetField(field) => {
            app.reset_field(field);
            app.preview_tasks()
        }
        EditMsg::StrokeHex(text) => {
            if let Some(s) = app.session_mut() {
                s.stroke_hex = text.clone();
            }
            let Some(c) = profiles::parse_hex(&text) else {
                return Task::none();
            };
            if app.session().map(|s| s.cfg.stroke_color) == Some(c) {
                return Task::none();
            }
            if let Some(s) = app.session_mut() {
                s.cfg.stroke_color = c;
            }
            app.write_stroke_color();
            app.preview_tasks()
        }
        EditMsg::ToggleLock(c) => app.toggle_lock(c),
        // The write-mode toggles never move a slider: the controls always show
        // the layer's resolved config, so only the next edit's target changes.
        EditMsg::OverrideLayer(b) => {
            if let Some(s) = app.session_mut() {
                s.override_layer = b;
            }
            Task::none()
        }
        EditMsg::EditGlobal(b) => {
            app.edit_global = b;
            Task::none()
        }
        EditMsg::ProfileInput(text) => {
            if let Some(s) = app.session_mut() {
                s.profile_input = text;
            }
            Task::none()
        }
        EditMsg::ResetLayer => {
            app.record(crate::gui::undo::Coalesce::None, |app| {
                let i = app.selected_pos();
                if let Some(layer) = app.layer_name() {
                    if let Some(tier) = app.project_tier_mut(i) {
                        tier.overrides.remove(&layer);
                    }
                }
            });
            app.load_layer_into_controls();
            app.preview_tasks()
        }
        EditMsg::Undo => app.undo(),
        EditMsg::Redo => app.redo(),
        EditMsg::Seal => {
            app.seal_undo();
            Task::none()
        }
    }
}
