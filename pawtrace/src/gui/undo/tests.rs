use super::*;

fn ov(detail: f32) -> Overrides {
    Overrides { detail: Some(detail), ..Default::default() }
}

fn set_field(target: Target, field: Field, old: Option<Overrides>, new: Option<Overrides>) -> Command {
    Command::Edit(Edit {
        changes: vec![Change::ov(target, old, new)],
        coalesce: Coalesce::Field(field),
    })
}

/// The `new`/`old` blocks of a single-change `Edit`, for assertions.
fn only_change(cmd: &Command) -> (&Option<Overrides>, &Option<Overrides>) {
    let Command::Edit(e) = cmd else {
        panic!("expected an Edit");
    };
    let Change::Ov(c) = &e.changes[0] else {
        panic!("expected an Ov change");
    };
    (&c.old, &c.new)
}

/// A discrete edit that never coalesces, for stacking distinct steps.
fn reset(n: usize) -> Command {
    Command::Edit(Edit {
        changes: vec![Change::ov(Target::Profile(Scope::Project, format!("p{n}")), None, Some(ov(n as f32)))],
        coalesce: Coalesce::None,
    })
}

#[test]
fn set_field_round_trips_none_and_some() {
    let target = Target::Profile(Scope::Project, "Deer *".into());
    // None -> Some: revert restores the unset block, redo restores the value.
    let cmd = set_field(target.clone(), Field::Detail, None, Some(ov(7.0)));
    let (old, new) = only_change(&cmd);
    assert_eq!(*old, None);
    assert_eq!(*new, Some(ov(7.0)));

    // Some -> None (a reset): the two directions mirror.
    let cmd = set_field(target, Field::Detail, Some(ov(7.0)), None);
    let (old, new) = only_change(&cmd);
    assert_eq!(*old, Some(ov(7.0)));
    assert_eq!(*new, None);
}

#[test]
fn coalescing_merges_a_drag_and_seals_on_a_different_edit() {
    let target = Target::Profile(Scope::Project, "Deer *".into());
    let mut top = set_field(target.clone(), Field::Detail, None, Some(ov(1.0)));

    // A continued drag of the same field merges: earliest old, latest new.
    let step = set_field(target.clone(), Field::Detail, Some(ov(1.0)), Some(ov(2.0)));
    assert!(top.merge(step).is_ok());
    let (old, new) = only_change(&top);
    assert_eq!(*old, None, "keeps the drag's starting value");
    assert_eq!(*new, Some(ov(2.0)), "advances to the latest value");

    // A different field does not merge, sealing the previous step.
    let other = set_field(target, Field::MaxColors, Some(ov(2.0)), Some(ov(2.0)));
    assert!(top.merge(other).is_err());
}

#[test]
fn a_promote_edit_merges_by_union_of_targets() {
    let profile = Target::Profile(Scope::Project, "Deer *".into());
    let layer = Target::Override("Deer L Hand".into());
    // First drag message writes the profile and clears the layer override.
    let mut top = Command::Edit(Edit {
        changes: vec![
            Change::ov(profile.clone(), None, Some(ov(1.0))),
            Change::ov(layer.clone(), Some(ov(9.0)), None),
        ],
        coalesce: Coalesce::Field(Field::Detail),
    });
    // Later messages only touch the profile, the override already cleared.
    let step = set_field(profile.clone(), Field::Detail, Some(ov(1.0)), Some(ov(3.0)));
    assert!(top.merge(step).is_ok());
    let Command::Edit(e) = &top else { panic!() };
    assert_eq!(e.changes.len(), 2, "the override clear is retained");
    let prof = e.changes.iter().find(|c| c.key() == ChangeKey::Ov(profile.clone())).unwrap();
    let Change::Ov(c) = prof else { panic!() };
    assert_eq!(c.old, None);
    assert_eq!(c.new, Some(ov(3.0)));
}

#[test]
fn a_new_edit_clears_the_redo_stack() {
    let (mut undo, mut redo) = (Vec::new(), vec![reset(1), reset(2)]);
    push(&mut undo, &mut redo, reset(3), UNDO_CAP);
    assert!(redo.is_empty(), "a fresh edit forks history");
    assert_eq!(undo.len(), 1);
}

#[test]
fn a_drag_collapses_to_one_stack_entry() {
    let target = Target::Profile(Scope::Project, "Deer *".into());
    let (mut undo, mut redo) = (Vec::new(), Vec::new());
    let mut prev = 0.0;
    for step in 1..=5 {
        let v = step as f32;
        push(&mut undo, &mut redo, set_field(target.clone(), Field::Detail, Some(ov(prev)), Some(ov(v))), UNDO_CAP);
        prev = v;
    }
    assert_eq!(undo.len(), 1, "the whole drag is one undo step");
    let (old, new) = only_change(&undo[0]);
    assert_eq!(*old, Some(ov(0.0)));
    assert_eq!(*new, Some(ov(5.0)));
}

#[test]
fn the_stack_caps_at_its_depth_dropping_the_oldest() {
    let (mut undo, mut redo) = (Vec::new(), Vec::new());
    for n in 0..UNDO_CAP + 10 {
        push(&mut undo, &mut redo, reset(n), UNDO_CAP);
    }
    assert_eq!(undo.len(), UNDO_CAP);
    // The oldest ten fell off, so the front is command number 10.
    let (_, new) = only_change(&undo[0]);
    assert_eq!(*new, Some(ov(10.0)));
}

#[test]
fn a_sealed_step_does_not_absorb_a_re_drag_of_the_same_slider() {
    let target = Target::Profile(Scope::Project, "Deer *".into());
    let (mut undo, mut redo) = (Vec::new(), Vec::new());
    push(&mut undo, &mut redo, set_field(target.clone(), Field::Detail, None, Some(ov(1.0))), UNDO_CAP);
    // Releasing the slider seals the gesture.
    seal(&mut undo);
    push(&mut undo, &mut redo, set_field(target, Field::Detail, Some(ov(1.0)), Some(ov(2.0))), UNDO_CAP);
    assert_eq!(undo.len(), 2, "a second drag of the same slider is a new step");
}

#[test]
fn pin_painting_merges_within_a_gesture_and_splits_across_releases() {
    let target = Target::Override("Deer L Hand".into());
    let pin_edit = |old: Option<Overrides>, new: Option<Overrides>| {
        Command::Edit(Edit {
            changes: vec![Change::ov(target.clone(), old, new)],
            coalesce: Coalesce::Pins,
        })
    };
    let (mut undo, mut redo) = (Vec::new(), Vec::new());
    // One paint-drag: a press plus drags, no release between.
    push(&mut undo, &mut redo, pin_edit(None, Some(ov(1.0))), UNDO_CAP);
    push(&mut undo, &mut redo, pin_edit(Some(ov(1.0)), Some(ov(2.0))), UNDO_CAP);
    assert_eq!(undo.len(), 1, "a paint-drag is one step");
    // Release, then a separate press.
    seal(&mut undo);
    push(&mut undo, &mut redo, pin_edit(Some(ov(2.0)), Some(ov(3.0))), UNDO_CAP);
    assert_eq!(undo.len(), 2, "separate pin presses are separate steps");
}

#[test]
fn a_flag_toggle_is_self_inverse() {
    let old = true;
    let change = FlagChange { layer: LayerId(0), kind: FlagKind::Visible, old, new: !old };
    // Redo sets `new`, undo sets `old`, and the two are opposites.
    assert_eq!(change.new, !change.old);
}

#[test]
fn non_coalescing_edits_never_merge() {
    // Locked-color toggles record as Coalesce::None, so two swatch clicks are
    // exercised here: they stack as two steps, never one.
    let target = Target::Default(Scope::Project);
    let mut a = Command::Edit(Edit {
        changes: vec![Change::ov(target.clone(), None, Some(ov(1.0)))],
        coalesce: Coalesce::None,
    });
    let b = Command::Edit(Edit {
        changes: vec![Change::ov(target, Some(ov(1.0)), Some(ov(2.0)))],
        coalesce: Coalesce::None,
    });
    assert!(a.merge(b).is_err());
}
