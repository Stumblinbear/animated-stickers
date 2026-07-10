//! Layer selection (with modifier-driven multi-select) and the visible /
//! enabled flags, including bulk toggles across the whole selection.

use crate::gui::app::App;
use crate::gui::ids::LayerId;
use crate::gui::msg::{LayerMsg, Msg, StripView};
use crate::gui::undo::{FlagChange, FlagKind};
use iced::Task;
use std::collections::BTreeSet;

pub(super) fn update(app: &mut App, msg: LayerMsg) -> Task<Msg> {
    match msg {
        LayerMsg::Click(i) => click(app, i),
        LayerMsg::ToggleVisible(i) => {
            let mut changes = Vec::new();
            if let Some(d) = app.doc_mut() {
                if let Some(f) = d.inputs_mut(i) {
                    let old = f.visible;
                    f.visible = !old;
                    changes.push(FlagChange { layer: i, kind: FlagKind::Visible, old, new: f.visible });
                }
            }
            app.record_flags(changes);
            app.spawn_full()
        }
        LayerMsg::ToggleEnabled(i) => {
            let mut changes = Vec::new();
            if let Some(d) = app.doc_mut() {
                if let Some(f) = d.inputs_mut(i) {
                    let old = f.enabled;
                    f.enabled = !old;
                    changes.push(FlagChange { layer: i, kind: FlagKind::Enabled, old, new: f.enabled });
                }
            }
            app.record_flags(changes);
            app.spawn_full()
        }
        LayerMsg::BulkVisible(b) => {
            let sel = app.session().map(|s| s.selection.clone()).unwrap_or_default();
            let mut changes = Vec::new();
            if let Some(d) = app.doc_mut() {
                for i in &sel {
                    if let Some(f) = d.inputs_mut(*i) {
                        changes.push(FlagChange { layer: *i, kind: FlagKind::Visible, old: f.visible, new: b });
                        f.visible = b;
                    }
                }
            }
            app.record_flags(changes);
            app.spawn_full()
        }
        LayerMsg::BulkEnabled(b) => {
            let sel = app.session().map(|s| s.selection.clone()).unwrap_or_default();
            let mut changes = Vec::new();
            if let Some(d) = app.doc_mut() {
                for i in &sel {
                    if let Some(f) = d.inputs_mut(*i) {
                        changes.push(FlagChange { layer: *i, kind: FlagKind::Enabled, old: f.enabled, new: b });
                        f.enabled = b;
                    }
                }
            }
            app.record_flags(changes);
            app.spawn_full()
        }
        LayerMsg::ClearSelection => deselect(app),
    }
}

/// Empties the selection, a legal resting state with no primary layer. A stage
/// view then has no layer to show, so it falls back to the Document view.
pub(super) fn deselect(app: &mut App) -> Task<Msg> {
    if let Some(s) = app.session_mut() {
        s.selection = BTreeSet::new();
        if !s.is_doc_view() {
            s.view = StripView::Document;
        }
    }
    Task::none()
}

pub(super) fn click(app: &mut App, i: LayerId) -> Task<Msg> {
    let m = app.modifiers;
    let Some(sess) = app.session() else {
        return Task::none();
    };
    if m.control() || m.command() {
        let mut sel = sess.selection.clone();
        let removed = !sel.insert(i);
        if removed {
            sel.remove(&i);
        }
        if sel.is_empty() {
            sel.insert(i);
        }
        // Deselecting the clicked layer moves the primary onto a remaining
        // member; the inspector must never edit a deselected layer.
        let primary = if sel.contains(&i) {
            i
        } else {
            *sel.last().expect("selection is non-empty")
        };
        focus(app, primary, sel, primary)
    } else if m.shift() {
        let anchor = sess.select_anchor;
        // Range select is positional: resolve both ends to paint-order slots,
        // then store the ids that span them so the selection survives reorders.
        let sel: BTreeSet<LayerId> = app
            .doc()
            .and_then(|doc| {
                let a = doc.layer_pos(anchor)?;
                let b = doc.layer_pos(i)?;
                let (lo, hi) = (a.min(b), a.max(b));
                Some((lo..=hi).filter_map(|p| doc.layers.get(p).map(|l| l.id)).collect())
            })
            .unwrap_or_else(|| BTreeSet::from([i]));
        focus(app, i, sel, anchor)
    } else {
        app.select_layer(i)
    }
}

/// Makes layer `i` the primary of a multi-selection: reseeds the profile
/// target, reloads the controls to `i`'s config, and recomputes its strip,
/// while keeping `selection` and `anchor` as given.
fn focus(app: &mut App, i: LayerId, selection: BTreeSet<LayerId>, anchor: LayerId) -> Task<Msg> {
    app.clear_stages_on_switch(i);
    let d = app.selected_pos();
    let seed = app
        .layer_name_of(d, i)
        .as_deref()
        .and_then(|l| app.stack(d).match_name(l))
        .unwrap_or_default();
    if let Some(s) = app.session_mut() {
        s.selected_layer = i;
        s.select_anchor = anchor;
        s.selection = selection;
        s.profile_input = seed;
    }
    app.load_layer_into_controls();
    app.spawn_stages()
}
