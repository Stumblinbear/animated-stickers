//! Capturing commands at the edit chokepoints. Each recorded edit snapshots
//! the profile tiers before and after the mutation and diffs them, so a
//! command carries exactly the blocks and assignments that changed however the
//! underlying helper reached them (a profile write that also clears a promoted
//! field from a layer override records both).

use super::{Change, Coalesce, Command, Edit, FlagChange, PinChange, Target};
use crate::gui::app::App;
use crate::gui::ids::LayerId;
use crate::profiles::{Overrides, Scope};
use std::collections::{BTreeMap, BTreeSet};

/// A copy of both tiers' editable state, diffed against a later copy to derive
/// a command's changes.
struct Snapshot {
    global_default: Overrides,
    global_profiles: BTreeMap<String, Overrides>,
    project_default: Overrides,
    project_profiles: BTreeMap<String, Overrides>,
    project_overrides: BTreeMap<String, Overrides>,
    project_assign: BTreeMap<String, String>,
}

impl App {
    fn tier_snapshot(&self) -> Snapshot {
        let g = &self.global_profiles;
        let p = self.stack(self.selected_pos()).project;
        Snapshot {
            global_default: g.default.clone(),
            global_profiles: g.profiles.clone(),
            project_default: p.default.clone(),
            project_profiles: p.profiles.clone(),
            project_overrides: p.overrides.clone(),
            project_assign: p.assign.clone(),
        }
    }

    /// Runs `body`, then records the resulting tier changes as one undoable
    /// [`Edit`] tagged `coalesce`. Nothing is pushed when `body` leaves the
    /// tiers unchanged.
    pub(in crate::gui) fn record<R>(&mut self, coalesce: Coalesce, body: impl FnOnce(&mut App) -> R) -> R {
        let before = self.tier_snapshot();
        let r = body(self);
        let after = self.tier_snapshot();
        let changes = diff(&before, &after);
        if !changes.is_empty() {
            self.push_command(Command::Edit(Edit { changes, coalesce }));
        }
        r
    }

    /// Records a set of flag toggles as one command, dropping the layers a bulk
    /// set left unchanged so redo/undo touch only what actually moved.
    pub(in crate::gui) fn record_flags(&mut self, changes: Vec<FlagChange>) {
        let changes: Vec<FlagChange> = changes.into_iter().filter(|c| c.old != c.new).collect();
        if !changes.is_empty() {
            self.push_command(Command::Flags(changes));
        }
    }

    /// Records a pin edit on `layer` from `old` to `new`. Consecutive edits
    /// within one paint-drag coalesce into a single undo step; a no-op when the
    /// set is unchanged.
    pub(in crate::gui) fn record_pins(
        &mut self,
        layer: LayerId,
        old: Vec<[u32; 2]>,
        new: Vec<[u32; 2]>,
    ) {
        if old != new {
            self.push_command(Command::Pins(PinChange { layer, old, new, sealed: false }));
        }
    }
}

fn diff(before: &Snapshot, after: &Snapshot) -> Vec<Change> {
    let mut out = Vec::new();
    if before.global_default != after.global_default {
        out.push(Change::ov(
            Target::Default(Scope::Global),
            Some(before.global_default.clone()),
            Some(after.global_default.clone()),
        ));
    }
    if before.project_default != after.project_default {
        out.push(Change::ov(
            Target::Default(Scope::Project),
            Some(before.project_default.clone()),
            Some(after.project_default.clone()),
        ));
    }
    diff_ov_map(&before.global_profiles, &after.global_profiles, &mut out, |k| {
        Target::Profile(Scope::Global, k)
    });
    diff_ov_map(&before.project_profiles, &after.project_profiles, &mut out, |k| {
        Target::Profile(Scope::Project, k)
    });
    diff_ov_map(&before.project_overrides, &after.project_overrides, &mut out, Target::Override);
    diff_assign(&before.project_assign, &after.project_assign, &mut out);
    out
}

fn diff_ov_map(
    before: &BTreeMap<String, Overrides>,
    after: &BTreeMap<String, Overrides>,
    out: &mut Vec<Change>,
    target: impl Fn(String) -> Target,
) {
    for key in keys(before, after) {
        let (old, new) = (before.get(&key), after.get(&key));
        if old != new {
            out.push(Change::ov(target(key), old.cloned(), new.cloned()));
        }
    }
}

fn diff_assign(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
    out: &mut Vec<Change>,
) {
    for key in keys(before, after) {
        let (old, new) = (before.get(&key), after.get(&key));
        if old != new {
            out.push(Change::Assign { layer: key, old: old.cloned(), new: new.cloned() });
        }
    }
}

/// The union of both maps' keys.
fn keys<V>(before: &BTreeMap<String, V>, after: &BTreeMap<String, V>) -> BTreeSet<String> {
    before.keys().chain(after.keys()).cloned().collect()
}
