//! The Pin tool: click a region to pin or unpin it, drag to pin across
//! regions. Pins protect a region's color through re-segmentation, so they are
//! a document property the tool writes, never tool state. Offered where regions
//! exist: the Document view and the Shapes and Curves phases.

use crate::gui::app::App;
use crate::gui::msg::{Msg, Phase, StripView};
use crate::gui::phases::SubView;
use crate::gui::view::icons;
use iced::{Point, Task};

pub const ICON: char = icons::PIN;
pub const CAPTURES_PRESS: bool = true;
pub const CURSOR: iced::mouse::Interaction = iced::mouse::Interaction::Crosshair;

/// Offered on the Document view and the Shapes and Curves phases, the views
/// where regions exist to pin.
pub fn applies(view: StripView, _sub: Option<SubView>) -> bool {
    matches!(
        view,
        StripView::Document | StripView::Phase(Phase::Shapes | Phase::Curves)
    )
}

/// A press toggles the pin under `p`: it removes a pin already in that region,
/// else adds one.
pub fn press(app: &mut App, p: Point) -> Task<Msg> {
    let Some(view) = app.session().map(|s| s.view) else {
        return Task::none();
    };

    pin_at(app, p, view, true)
}

/// A drag adds only, so painting across pinned regions leaves them pinned.
pub fn drag(app: &mut App, p: Point) -> Task<Msg> {
    let Some(view) = app
        .session()
        .filter(|s| !s.selection.is_empty())
        .map(|s| s.view)
    else {
        return Task::none();
    };

    pin_at(app, p, view, false)
}

/// Pins the region under `p`. With `toggle`, a press removes a pin already in
/// that region; without it, a drag only adds. Pins are per-layer document
/// state ([`LayerInputs::pins`](crate::gui::doc)) in document source px, so they
/// survive re-segmentation and follow the layer through exports. `p` is in
/// document px on the Document view and source-crop px on a stage view.
fn pin_at(app: &mut App, p: Point, view: StripView, toggle: bool) -> Task<Msg> {
    let Some(sess) = app.session() else {
        return Task::none();
    };

    let sel = sess.selected_layer;

    let Some(offset) = app.doc().and_then(|doc| doc.layer(sel)).map(|l| l.offset) else {
        return Task::none();
    };

    // The layer's current pins: the hit test reads them and the recorded edit
    // diffs against them. The segmentation raster is pin-independent, so the
    // regions the strip last produced for this layer are the ones to hit-test.
    let old: Vec<[u32; 2]> = app
        .doc()
        .and_then(|d| d.inputs.get(&sel))
        .map(|i| i.pins.clone())
        .unwrap_or_default();
    let Some(regs) = sess.stages.peek(sel).and_then(|s| s.regions.current()) else {
        return Task::none();
    };

    let s = sess.cfg.scale;

    // Regions live in scaled remap space (crop px times `scale`). Both incoming
    // spaces reduce to crop px first: document px minus the layer offset, or
    // source-crop px directly.
    let crop = match view {
        StripView::Document => (
            (p.x as i64 - offset.0 as i64).max(0) as u32,
            (p.y as i64 - offset.1 as i64).max(0) as u32,
        ),
        StripView::Phase(_) => (p.x as u32, p.y as u32),
    };

    let (sx, sy) = (crop.0 * s, crop.1 * s);

    let Some(region) = regs.iter().find(|r| r.contains(sx, sy)) else {
        return Task::none();
    };

    let existing = old.iter().position(|pin| {
        let Some(x) = pin[0].checked_sub(offset.0) else {
            return false;
        };

        let Some(y) = pin[1].checked_sub(offset.1) else {
            return false;
        };

        region.contains(x * s + s / 2, y * s + s / 2)
    });

    let mut new = old.clone();

    match existing {
        // A drag adds only; an already-pinned region stays pinned.
        Some(_) if !toggle => return Task::none(),
        Some(i) => {
            new.remove(i);
        }
        None => {
            new.push([sx / s + offset.0, sy / s + offset.1]);
        }
    }

    if let Some(inp) = app.doc_mut().and_then(|d| d.inputs_mut(sel)) {
        inp.pins = new.clone();
    }

    app.record_pins(sel, old, new);
    app.preview_tasks()
}
