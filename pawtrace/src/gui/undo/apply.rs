//! Interpreting commands back onto the tiers and flags: reading and writing a
//! [`Target`]'s override block, applying a whole command in either direction,
//! and the undo/redo entry points that refresh the UI and pipeline afterward.

use super::{Change, Command, FlagChange, FlagKind, Target};
use crate::gui::app::App;
use crate::gui::msg::Msg;
use crate::profiles::Overrides;
use iced::Task;

/// Which way a command runs: `Forward` applies its `new` values, `Revert` its
/// `old` values.
#[derive(Debug, Clone, Copy)]
pub enum Dir {
    Forward,
    Revert,
}

impl App {
    /// Writes `val` to the override block at `target`, removing the profile or
    /// override entirely on `None` so no bare empty block lingers.
    fn put_block(&mut self, target: &Target, val: Option<Overrides>) {
        let i = self.selected_doc;
        match target {
            Target::Override(l) => {
                if let Some(tier) = self.project_tier_mut(i) {
                    match val {
                        Some(v) => {
                            tier.overrides.insert(l.clone(), v);
                        }
                        None => {
                            tier.overrides.remove(l);
                        }
                    }
                }
            }
            Target::Profile(scope, k) => {
                if let Some(tier) = self.tier_mut(*scope) {
                    match val {
                        Some(v) => {
                            tier.profiles.insert(k.clone(), v);
                        }
                        None => {
                            tier.profiles.remove(k);
                        }
                    }
                }
            }
            Target::Default(scope) => {
                if let Some(tier) = self.tier_mut(*scope) {
                    tier.default = val.unwrap_or_default();
                }
            }
        }
    }

    fn put_assign(&mut self, layer: &str, val: Option<String>) {
        let i = self.selected_doc;
        if let Some(tier) = self.project_tier_mut(i) {
            match val {
                Some(v) => {
                    tier.assign.insert(layer.to_string(), v);
                }
                None => {
                    tier.assign.remove(layer);
                }
            }
        }
    }

    fn set_flag(&mut self, change: &FlagChange, on: bool) {
        if let Some(f) = self.doc_mut().and_then(|d| d.flags.get_mut(change.layer.index())) {
            match change.kind {
                FlagKind::Visible => f.visible = on,
                FlagKind::Enabled => f.enabled = on,
            }
        }
    }

    fn apply_change(&mut self, change: &Change, dir: Dir) {
        match (change, dir) {
            (Change::Ov(c), Dir::Forward) => self.put_block(&c.target, c.new.clone()),
            (Change::Ov(c), Dir::Revert) => self.put_block(&c.target, c.old.clone()),
            (Change::Assign { layer, new, .. }, Dir::Forward) => self.put_assign(layer, new.clone()),
            (Change::Assign { layer, old, .. }, Dir::Revert) => self.put_assign(layer, old.clone()),
        }
    }

    fn apply_command(&mut self, cmd: &Command, dir: Dir) {
        match cmd {
            Command::Edit(e) => {
                for c in &e.changes {
                    self.apply_change(c, dir);
                }
            }
            Command::Flags(changes) => {
                for f in changes {
                    let on = match dir {
                        Dir::Forward => f.new,
                        Dir::Revert => f.old,
                    };
                    self.set_flag(f, on);
                }
            }
        }
    }

    /// Reverts the last command on the selected document, moving it to the redo
    /// stack, then reloads the controls and re-runs the pipeline so the UI and
    /// preview reflect the restored state.
    pub fn undo(&mut self) -> Task<Msg> {
        let Some(cmd) = self.session_mut().and_then(|s| s.undo.pop()) else {
            return Task::none();
        };
        self.apply_command(&cmd, Dir::Revert);
        if let Some(s) = self.session_mut() {
            s.redo.push(cmd);
        }
        self.load_layer_into_controls();
        self.preview_tasks()
    }

    /// Re-applies the last undone command, moving it back to the undo stack.
    pub fn redo(&mut self) -> Task<Msg> {
        let Some(cmd) = self.session_mut().and_then(|s| s.redo.pop()) else {
            return Task::none();
        };
        self.apply_command(&cmd, Dir::Forward);
        if let Some(s) = self.session_mut() {
            s.undo.push(cmd);
        }
        self.load_layer_into_controls();
        self.preview_tasks()
    }
}
