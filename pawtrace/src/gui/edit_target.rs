//! Edit-target routing: where a settings edit lands (a profile, a tier
//! default, or the selected layer's own override) and the [`App`] helpers
//! that read and write through that target.

use crate::color::Srgb;
use super::app::App;
use super::fields::{self, Field};
use super::msg::Msg;
use super::undo::Coalesce;
use crate::profiles::{self, Overrides};
use iced::Task;

/// What the profile controls currently write to.
pub enum EditTarget {
    /// The tier `[default]` section.
    Default,
    /// A named profile (a class of layers).
    Profile(String),
    /// A single layer's override, keyed on its exact name.
    Override(String),
}

impl App {
    /// Where a setting change lands: the selected layer's own override when
    /// the override toggle is on, otherwise the profile named in
    /// `profile_input` (its tier `[default]` when that box is empty).
    pub fn edit_target(&self) -> EditTarget {
        let Some(sess) = self.session() else {
            return EditTarget::Default;
        };
        if sess.override_layer {
            match self.layer_name() {
                Some(l) => EditTarget::Override(l),
                None => EditTarget::Default,
            }
        } else {
            let key = sess.profile_input.trim();
            if key.is_empty() {
                EditTarget::Default
            } else {
                EditTarget::Profile(key.to_string())
            }
        }
    }

    /// Whether the typed profile pattern matches the selected layer, so its
    /// edits actually affect this layer's preview.
    pub fn profile_input_matches_layer(&self) -> bool {
        let Some(sess) = self.session() else {
            return false;
        };
        let key = sess.profile_input.trim();
        !key.is_empty()
            && self
                .layer_name()
                .is_some_and(|l| profiles::key_matches(key, &l))
    }

    /// Whether a profile edit writes to the global tier.
    fn edits_global(&self) -> bool {
        !self.session().is_some_and(|s| s.override_layer) && self.edit_global
    }

    /// The overrides map the current target writes into, created if absent.
    fn target_ov(&mut self) -> Option<&mut Overrides> {
        let i = self.selected_pos();
        match self.edit_target() {
            EditTarget::Override(layer) => Some(
                self.project_tier_mut(i)?
                    .overrides
                    .entry(layer)
                    .or_default(),
            ),
            EditTarget::Profile(key) => {
                let tier = if self.edits_global() {
                    &mut self.global_profiles
                } else {
                    self.project_tier_mut(i)?
                };
                Some(tier.profiles.entry(key).or_default())
            }
            EditTarget::Default => {
                let tier = if self.edits_global() {
                    &mut self.global_profiles
                } else {
                    self.project_tier_mut(i)?
                };
                Some(&mut tier.default)
            }
        }
    }

    /// The current target's overrides, if it exists, without creating it.
    fn target_ov_ref(&self) -> Option<&Overrides> {
        let stack = self.stack_sel();
        match self.edit_target() {
            EditTarget::Override(layer) => stack.project.overrides.get(&layer),
            EditTarget::Profile(key) => {
                let tier = if self.edits_global() {
                    stack.global
                } else {
                    stack.project
                };
                tier.profiles.get(&key)
            }
            EditTarget::Default => {
                let tier = if self.edits_global() {
                    stack.global
                } else {
                    stack.project
                };
                Some(&tier.default)
            }
        }
    }

    /// Whether `field` is set at the current edit target.
    pub fn field_is_set(&self, field: Field) -> bool {
        self.target_ov_ref()
            .is_some_and(|ov| fields::is_set(ov, field))
    }

    /// Whether `field` is overridden on the selected layer itself.
    pub fn field_overridden(&self, field: Field) -> bool {
        let i = self.selected_pos();
        self.layer_name()
            .and_then(|l| {
                self.stack(i)
                    .project
                    .overrides
                    .get(&l)
                    .map(|ov| fields::is_set(ov, field))
            })
            .unwrap_or(false)
    }

    /// How many fields the selected layer's override sets.
    pub fn override_count(&self) -> usize {
        let i = self.selected_pos();
        self.layer_name()
            .and_then(|l| {
                self.stack(i).project.overrides.get(&l).map(|ov| {
                    fields::ALL
                        .iter()
                        .filter(|&&f| fields::is_set(ov, f))
                        .count()
                })
            })
            .unwrap_or(0)
    }

    /// Clears the current layer's override of `field` (profile mode only), so
    /// a profile edit is not shadowed by an existing per-layer override.
    fn clear_layer_field(&mut self, field: Field) {
        let i = self.selected_pos();
        if let Some(layer) = self.layer_name() {
            if let Some(tier) = self.project_tier_mut(i) {
                if let Some(o) = tier.overrides.get_mut(&layer) {
                    fields::clear(o, field);
                }
            }
        }
    }

    /// Writes one setting's current value to the edit target. In profile mode
    /// it also clears that setting from the layer's override, promoting it to
    /// the profile without touching the layer's other overrides.
    pub(super) fn write_field(&mut self, field: Field) {
        self.record(Coalesce::Field(field), |app| {
            let cfg = app.session().map(|s| s.cfg.clone()).unwrap_or_default();
            let override_layer = app.session().is_some_and(|s| s.override_layer);
            if let Some(ov) = app.target_ov() {
                fields::set(ov, field, &cfg);
            }
            if !override_layer {
                app.clear_layer_field(field);
            }
        });
    }

    /// Clears one setting from the edit target and re-resolves the controls.
    /// A layer override emptied of its last field is dropped, so no bare
    /// `[overrides."layer"]` lingers.
    pub(super) fn reset_field(&mut self, field: Field) {
        self.record(Coalesce::None, |app| {
            if let Some(ov) = app.target_ov() {
                fields::clear(ov, field);
            }
            if let EditTarget::Override(layer) = app.edit_target() {
                let i = app.selected_pos();
                if let Some(tier) = app.project_tier_mut(i) {
                    if tier.overrides.get(&layer) == Some(&Overrides::default()) {
                        tier.overrides.remove(&layer);
                    }
                }
            }
        });
        self.load_layer_into_controls();
    }

    pub(super) fn write_stroke_color(&mut self) {
        self.record(Coalesce::StrokeColor, |app| {
            let c = app
                .session()
                .map(|s| s.cfg.stroke_color)
                .unwrap_or_default();
            if let Some(ov) = app.target_ov() {
                ov.stroke_color = Some(c.to_hex());
            }
        });
    }

    /// Writes the current locked-color set to the edit target.
    fn write_locked(&mut self) {
        self.record(Coalesce::None, |app| {
            let hexes: Vec<String> = app
                .session()
                .map(|s| {
                    s.cfg
                        .locked
                        .iter()
                        .map(|c| c.to_hex())
                        .collect()
                })
                .unwrap_or_default();
            if let Some(ov) = app.target_ov() {
                ov.locked = Some(hexes);
            }
        });
    }

    pub(super) fn toggle_lock(&mut self, c: Srgb) -> Task<Msg> {
        if let Some(sess) = self.session_mut() {
            if let Some(i) = sess.cfg.locked.iter().position(|&l| l == c) {
                sess.cfg.locked.remove(i);
            } else {
                sess.cfg.locked.push(c);
            }
        }
        self.write_locked();
        self.preview_tasks()
    }
}
