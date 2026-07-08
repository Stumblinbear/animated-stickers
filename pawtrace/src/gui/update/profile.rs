//! Profile assignment and library-management messages. The mutations live on
//! [`App`] (see `profile_ops`); this routes them and refreshes the affected
//! renders.

use crate::gui::app::{App, LibraryRename};
use crate::gui::msg::{Msg, ProfileMsg};
use crate::gui::undo::Coalesce;
use iced::Task;

pub(super) fn update(app: &mut App, msg: ProfileMsg) -> Task<Msg> {
    match msg {
        ProfileMsg::ToggleChip(i) => {
            let ui = &mut app.profile_ui;
            ui.chip_open = (ui.chip_open != Some(i)).then_some(i);
            Task::none()
        }
        ProfileMsg::CloseChip => {
            app.profile_ui.chip_open = None;
            Task::none()
        }
        ProfileMsg::Assign(i, key) => {
            app.profile_ui.chip_open = None;
            app.record(Coalesce::None, move |app| {
                if let Some(name) = app.layer_name_of(app.selected_doc, i) {
                    app.assign_layer(name, key);
                }
            });
            reassess(app)
        }
        ProfileMsg::AssignSelection(key) => {
            app.record(Coalesce::None, move |app| app.assign_selection(key));
            reassess(app)
        }
        ProfileMsg::NewFromLayer(i) => {
            app.profile_ui.chip_open = None;
            app.record(Coalesce::None, move |app| {
                if let Some(name) = app.layer_name_of(app.selected_doc, i) {
                    app.new_profile_from_layer(name);
                }
            });
            reassess(app)
        }
        ProfileMsg::GroupNew => {
            app.record(Coalesce::None, |app| {
                app.group_into_new_profile();
            });
            reassess(app)
        }
        ProfileMsg::OpenLibrary => {
            app.profile_ui.chip_open = None;
            app.profile_ui.library_open = true;
            Task::none()
        }
        ProfileMsg::CloseLibrary => {
            app.profile_ui.library_open = false;
            app.profile_ui.rename = None;
            Task::none()
        }
        ProfileMsg::RenameStart(scope, key) => {
            app.profile_ui.rename = Some(LibraryRename { scope, text: key.clone(), key });
            Task::none()
        }
        ProfileMsg::RenameInput(text) => {
            if let Some(r) = &mut app.profile_ui.rename {
                r.text = text;
            }
            Task::none()
        }
        ProfileMsg::RenameCommit => {
            if let Some(r) = app.profile_ui.rename.take() {
                app.record(Coalesce::None, move |app| {
                    app.rename_profile(r.scope, &r.key, r.text.trim());
                });
            }
            reassess(app)
        }
        ProfileMsg::Duplicate(scope, key) => {
            app.record(Coalesce::None, move |app| app.duplicate_profile(scope, &key));
            Task::none()
        }
        ProfileMsg::Delete(scope, key) => {
            if app.profile_ui.rename.as_ref().is_some_and(|r| r.key == key) {
                app.profile_ui.rename = None;
            }
            app.record(Coalesce::None, move |app| app.delete_profile(scope, &key));
            reassess(app)
        }
    }
}

/// Reloads the selected layer's resolved controls and re-runs the pipeline: a
/// profile change can shift how any layer resolves.
fn reassess(app: &mut App) -> Task<Msg> {
    app.load_layer_into_controls();
    app.preview_tasks()
}
