//! Per-document undo/redo as a command stack. Each undoable action becomes a
//! [`Command`] carrying the pre- and post-edit values of exactly what it
//! changed, so reverting restores prior state (including a field's unset-ness)
//! and redoing replays it. Commands are captured at the [`App`] edit
//! chokepoints (see `record`) and interpreted back onto the tiers and flags
//! (see `apply`).

mod apply;
mod record;

use super::fields::Field;
use super::ids::LayerId;
use crate::gui::app::App;
use crate::profiles::{Overrides, Scope};

/// The deepest undo history kept per document; older commands drop off.
const UNDO_CAP: usize = 100;

/// Which override block, in which tier, a change addresses.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A tier's `[default]` section.
    Default(Scope),
    /// A named profile in a tier.
    Profile(Scope, String),
    /// A layer's own override, always in the project tier.
    Override(String),
}

/// One layer flag, so a captured toggle names which flag it flipped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FlagKind {
    Visible,
    Enabled,
}

/// One flag's value before and after a toggle, so a bulk set (which may leave
/// some layers unchanged) reverts each layer to exactly its prior state.
#[derive(Debug, Clone, Copy)]
pub struct FlagChange {
    pub layer: LayerId,
    pub kind: FlagKind,
    pub old: bool,
    pub new: bool,
}

/// An override block replaced (or created/removed, via `None`): the target and
/// its before/after values. Boxed within [`Change`] so the large [`Overrides`]
/// does not inflate every variant.
#[derive(Debug, Clone)]
pub struct OvChange {
    pub target: Target,
    pub old: Option<Overrides>,
    pub new: Option<Overrides>,
}

/// One tier-level effect: an override block change or a layer's profile
/// assignment changed (`None` = no explicit assignment).
#[derive(Debug, Clone)]
pub enum Change {
    Ov(Box<OvChange>),
    Assign { layer: String, old: Option<String>, new: Option<String> },
}

/// A change's identity for coalescing: two changes with equal keys address the
/// same block, so a merge keeps one entry that spans both.
#[derive(Debug, Clone, PartialEq)]
enum ChangeKey {
    Ov(Target),
    Assign(String),
}

impl Change {
    /// An override-block change.
    pub fn ov(target: Target, old: Option<Overrides>, new: Option<Overrides>) -> Change {
        Change::Ov(Box::new(OvChange { target, old, new }))
    }

    fn key(&self) -> ChangeKey {
        match self {
            Change::Ov(c) => ChangeKey::Ov(c.target.clone()),
            Change::Assign { layer, .. } => ChangeKey::Assign(layer.clone()),
        }
    }

    /// Adopts `other`'s post-edit value, extending this change to end where a
    /// later same-block change ends. A no-op if the two address different
    /// kinds of block, which the equal-key precondition rules out.
    fn take_new_from(&mut self, other: Change) {
        match (self, other) {
            (Change::Ov(c), Change::Ov(o)) => c.new = o.new,
            (Change::Assign { new, .. }, Change::Assign { new: n, .. }) => *new = n,
            _ => {}
        }
    }
}

/// What kind of edit produced an [`Edit`], keying which consecutive edits
/// merge into one undo step. `None` never merges. Merging is scoped to one
/// gesture: releasing the slider or tool seals the step (see [`seal`]), so
/// repeating the same gesture starts a new step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Coalesce {
    None,
    Field(Field),
    StrokeColor,
    Pins,
}

impl Coalesce {
    /// Whether an edit tagged `self` absorbs a following edit tagged `other`:
    /// the same non-`None` kind, so one slider drag or one run of hex typing
    /// collapses while a different edit seals the step.
    fn mergeable(self, other: Coalesce) -> bool {
        self != Coalesce::None && self == other
    }
}

/// One logical settings edit: the tier changes it made (a profile write may
/// also clear the promoted field from a layer's override, so more than one)
/// with the tag that governs coalescing.
#[derive(Debug, Clone)]
pub struct Edit {
    pub changes: Vec<Change>,
    pub coalesce: Coalesce,
}

impl Edit {
    /// Extends this edit to also cover `other`, a later edit of the same kind:
    /// each block keeps this edit's earlier `old` and takes `other`'s later
    /// `new`, and a block only `other` touched is appended whole.
    fn absorb(&mut self, other: Edit) {
        for oc in other.changes {
            match self.changes.iter_mut().find(|c| c.key() == oc.key()) {
                Some(sc) => sc.take_new_from(oc),
                None => self.changes.push(oc),
            }
        }
    }
}

/// One undoable action. A composite action (a profile op that touches several
/// blocks and assignments) is a single [`Edit`] carrying every change, since
/// the changes address distinct blocks and revert independently.
#[derive(Debug, Clone)]
pub enum Command {
    Edit(Edit),
    Flags(Vec<FlagChange>),
}

impl Command {
    /// Folds `other` into this command when both are same-kind coalescing
    /// edits, returning `Ok` once absorbed or handing `other` back untouched.
    fn merge(&mut self, other: Command) -> Result<(), Command> {
        let compatible = matches!(
            (&*self, &other),
            (Command::Edit(a), Command::Edit(b)) if a.coalesce.mergeable(b.coalesce)
        );
        if !compatible {
            return Err(other);
        }
        let (Command::Edit(a), Command::Edit(b)) = (self, other) else {
            unreachable!("compatibility checked above");
        };
        a.absorb(b);
        Ok(())
    }
}

/// Records `cmd` on `undo`, clearing `redo` (a new edit forks history) and
/// merging into the top command when both coalesce. Depth is capped at `cap`,
/// dropping the oldest command.
fn push(undo: &mut Vec<Command>, redo: &mut Vec<Command>, cmd: Command, cap: usize) {
    redo.clear();
    let cmd = match undo.last_mut() {
        Some(top) => match top.merge(cmd) {
            Ok(()) => return,
            Err(cmd) => cmd,
        },
        None => cmd,
    };
    undo.push(cmd);
    if undo.len() > cap {
        undo.remove(0);
    }
}

/// Makes the top command of `undo` final: no later edit merges into it, so
/// the gesture it accumulated stays one undo step.
fn seal(undo: &mut [Command]) {
    // Merging requires matching non-`None` tags, so retagging is enough.
    if let Some(Command::Edit(e)) = undo.last_mut() {
        e.coalesce = Coalesce::None;
    }
}

impl App {
    pub(in crate::gui) fn push_command(&mut self, cmd: Command) {
        if let Some(s) = self.session_mut() {
            push(&mut s.undo, &mut s.redo, cmd, UNDO_CAP);
        }
    }

    /// Ends the in-progress gesture on the selected document: the next edit,
    /// even of the same kind, starts a new undo step.
    pub(in crate::gui) fn seal_undo(&mut self) {
        if let Some(s) = self.session_mut() {
            seal(&mut s.undo);
        }
    }

    pub fn can_undo(&self) -> bool {
        self.session().is_some_and(|s| !s.undo.is_empty())
    }

    pub fn can_redo(&self) -> bool {
        self.session().is_some_and(|s| !s.redo.is_empty())
    }
}

#[cfg(test)]
mod tests;
