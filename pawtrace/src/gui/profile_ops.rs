//! Profile-library mutations: pinning layers to profiles, promoting a layer's
//! deviations into a new profile, and renaming, duplicating, or deleting
//! library profiles. Every profile write here is a single observable point so
//! a later undo layer can capture the pre-edit state.

use super::app::App;
use super::fields;
use crate::profiles::{self, Overrides, Profiles, Scope};
use std::collections::BTreeMap;

impl App {
    /// Pins layer `name` in the selected document to profile `key`.
    pub(super) fn assign_layer(&mut self, name: String, key: String) {
        let i = self.selected_pos();
        if let Some(tier) = self.project_tier_mut(i) {
            tier.assign.insert(name, key);
        }
    }

    /// Pins every layer in the current selection to profile `key`.
    pub(super) fn assign_selection(&mut self, key: String) {
        let names = self.selected_layer_names();
        let i = self.selected_pos();
        if let Some(tier) = self.project_tier_mut(i) {
            for n in names {
                tier.assign.insert(n, key.clone());
            }
        }
    }

    /// Promotes layer `name`'s deviations into a fresh project profile, pins
    /// the layer to it, and clears the promoted fields from the layer's
    /// override. Returns the new profile key. The profile captures
    /// `diff(profile_base, resolved)`, the layer's full deviation from the
    /// tier defaults, so the layer resolves identically afterward.
    pub(super) fn new_profile_from_layer(&mut self, name: String) -> Option<String> {
        let i = self.selected_pos();
        let stack = self.stack(i);
        // Diff against the tier defaults, not override_base: the assignment
        // replaces the layer's old glob profile, so the new profile must also
        // carry that profile's fields or they stop applying.
        let base = stack.profile_base();
        let (resolved, _) = stack.resolve(&name);
        let ov = profiles::diff(&base, &resolved);
        let tier = self.project_tier_mut(i)?;
        let key = unique_key(&first_word_glob(&name), &tier.profiles);
        tier.profiles.insert(key.clone(), ov.clone());
        tier.assign.insert(name.clone(), key.clone());
        if let Some(o) = tier.overrides.get_mut(&name) {
            for f in fields::ALL {
                if fields::is_set(&ov, f) {
                    fields::clear(o, f);
                }
            }
            if *o == Overrides::default() {
                tier.overrides.remove(&name);
            }
        }
        Some(key)
    }

    /// Promotes the primary layer's deviations into a new project profile and
    /// pins the whole selection to it.
    pub(super) fn group_into_new_profile(&mut self) -> Option<String> {
        let primary = self.layer_name()?;
        let key = self.new_profile_from_layer(primary)?;
        self.assign_selection(key.clone());
        Some(key)
    }

    /// Renames a library profile in `scope`, repointing the assignments that
    /// resolve to it. A no-op when `new` is empty, unchanged, or already
    /// taken.
    pub(super) fn rename_profile(&mut self, scope: Scope, old: &str, new: &str) {
        if new.is_empty() || new == old {
            return;
        }
        let Some(tier) = self.tier_mut(scope) else {
            return;
        };
        if tier.profiles.contains_key(new) {
            return;
        }
        let Some(ov) = tier.profiles.remove(old) else {
            return;
        };
        tier.profiles.insert(new.to_string(), ov);
        match scope {
            // Only this project's assignments name this tier's key; another
            // project's same-named key is a different profile.
            Scope::Project => {
                if let Some(tier) = self.project_tier_mut(self.selected_pos()) {
                    repoint(tier, old, new);
                }
            }
            Scope::Global => {
                for p in self.projects.values_mut() {
                    // A project with its own same-named profile shadows the
                    // global one; its assignments still resolve there.
                    if !p.profiles.contains_key(old) {
                        repoint(p, old, new);
                    }
                }
            }
        }
    }

    /// Copies a library profile in `scope` under a fresh "… copy" key.
    pub(super) fn duplicate_profile(&mut self, scope: Scope, key: &str) {
        let Some(tier) = self.tier_mut(scope) else {
            return;
        };
        if let Some(ov) = tier.profiles.get(key).cloned() {
            let copy = unique_key(&format!("{key} copy"), &tier.profiles);
            tier.profiles.insert(copy, ov);
        }
    }

    /// Removes a library profile in `scope`, along with the assignments that
    /// resolved to it; those layers fall back to glob matching.
    pub(super) fn delete_profile(&mut self, scope: Scope, key: &str) {
        match scope {
            Scope::Project => {
                // An assignment whose key the global tier also holds still
                // resolves there, so it survives the project deletion.
                let global_has = self.global_profiles.profiles.contains_key(key);
                if let Some(tier) = self.project_tier_mut(self.selected_pos()) {
                    tier.profiles.remove(key);
                    if !global_has {
                        tier.assign.retain(|_, v| v != key);
                    }
                }
            }
            Scope::Global => {
                self.global_profiles.profiles.remove(key);
                for p in self.projects.values_mut() {
                    // A same-named project profile shadows the global one; its
                    // assignments keep resolving to it.
                    if !p.profiles.contains_key(key) {
                        p.assign.retain(|_, v| v != key);
                    }
                }
            }
        }
    }

    /// The tier a library edit at `scope` writes into: the global library, or
    /// the selected document's project tier.
    pub(super) fn tier_mut(&mut self, scope: Scope) -> Option<&mut Profiles> {
        match scope {
            Scope::Global => Some(&mut self.global_profiles),
            Scope::Project => self.project_tier_mut(self.selected_pos()),
        }
    }

    /// The names of every layer in the current selection.
    fn selected_layer_names(&self) -> Vec<String> {
        let d = self.selected_pos();
        self.session()
            .map(|s| s.selection.iter().filter_map(|&i| self.layer_name_of(d, i)).collect())
            .unwrap_or_default()
    }
}

/// Rewrites every assignment in `tier` naming `old` to `new`.
fn repoint(tier: &mut Profiles, old: &str, new: &str) {
    for v in tier.assign.values_mut() {
        if v == old {
            *v = new.to_string();
        }
    }
}

/// A profile key seeded from a layer name: its first word plus " *", the
/// convention for a word-prefix class.
fn first_word_glob(name: &str) -> String {
    let word = name.split_whitespace().next().unwrap_or("");
    format!("{word} *")
}

/// `base`, or `base` with the least " N" suffix that is free in `map`.
fn unique_key(base: &str, map: &BTreeMap<String, Overrides>) -> String {
    if !map.contains_key(base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base} {n}")).find(|k| !map.contains_key(k)).expect("integers are unbounded")
}
