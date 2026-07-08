//! Preview-canvas interactions: viewport gestures and the tool actions the
//! canvas publishes in the shown view's coordinates (document px on Document,
//! source-crop px on a stage view).

use crate::gui::app::App;
use crate::gui::compute::StageKeys;
use crate::gui::msg::{CanvasMsg, Msg, StripView, Tool};
use iced::{Point, Task};

pub(super) fn update(app: &mut App, msg: CanvasMsg) -> Task<Msg> {
    match msg {
        CanvasMsg::SetViewport { zoom, pan } => {
            if let Some(s) = app.session_mut() {
                s.set_viewport(Some(zoom), pan);
            }
            Task::none()
        }
        CanvasMsg::ToolPress(p) => press(app, p),
        // Dragging the pin tool paints pins.
        CanvasMsg::ToolDrag(p) if app.tool == Tool::Pin => {
            let view = match app.session() {
                Some(s) => s.view,
                None => return Task::none(),
            };
            pin_at(app, p, view, false)
        }
        CanvasMsg::ToolDrag(_) => Task::none(),
        // A release ends the gesture: one paint-drag stays one undo step and
        // the next press starts a new one.
        CanvasMsg::ToolRelease => {
            app.seal_undo();
            Task::none()
        }
    }
}

fn press(app: &mut App, p: Point) -> Task<Msg> {
    let (tool, view) = match app.session() {
        Some(s) => (app.tool, s.view),
        None => return Task::none(),
    };
    match tool {
        Tool::Pin => pin_at(app, p, view, true),
        Tool::Select if view == StripView::Stage(2) => pick_color(app, p),
        _ => Task::none(),
    }
}

/// Locks or unlocks the palette color at source-crop px `p` on the quantized
/// view.
fn pick_color(app: &mut App, p: Point) -> Task<Msg> {
    let Some(sess) = app.session() else {
        return Task::none();
    };
    let Some(q) = &sess.stages.quant_px else {
        return Task::none();
    };
    // The quant raster is the crop supersampled by `scale`; map crop px into it.
    let s = sess.cfg.scale as f32;
    let (x, y) = ((p.x * s) as u32, (p.y * s) as u32);
    if x >= q.width() || y >= q.height() {
        return Task::none();
    }
    let px = q.get_pixel(x, y).0;
    let c = [px[0], px[1], px[2]];
    if px[3] != 0 && sess.stages.palette.contains(&c) {
        app.toggle_lock(c)
    } else {
        Task::none()
    }
}

/// Pins the region under `p`. With `toggle`, a press removes a pin already in
/// that region; without it, a drag only adds, so painting across pinned
/// regions leaves them pinned. Pins are stored in document source px so they
/// survive re-segmentation and follow the layer through exports. `p` is in
/// document px on the Document view and source-crop px on a stage view.
fn pin_at(app: &mut App, p: Point, view: StripView, toggle: bool) -> Task<Msg> {
    let Some(sess) = app.session() else {
        return Task::none();
    };
    let layer = sess.selected_layer.index();
    let Some(offset) = app.doc().and_then(|doc| doc.layers.get(layer)).map(|l| l.offset) else {
        return Task::none();
    };
    let keys = StageKeys::of(&sess.cfg);
    let Some(regs) = sess.memo.peek_regions(sess.selected_layer, keys.regions) else {
        return Task::none();
    };
    let s = sess.cfg.scale;
    // Regions live in scaled quant space (crop px times `scale`). Both incoming
    // spaces reduce to crop px first: document px minus the layer offset, or
    // source-crop px directly.
    let crop = match view {
        StripView::Document => (
            (p.x as i64 - offset.0 as i64).max(0) as u32,
            (p.y as i64 - offset.1 as i64).max(0) as u32,
        ),
        StripView::Stage(_) => (p.x as u32, p.y as u32),
    };
    let (sx, sy) = (crop.0 * s, crop.1 * s);
    let Some(region) = regs.iter().find(|r| r.contains(sx, sy)) else {
        return Task::none();
    };
    let existing = sess.cfg.pins.iter().position(|pin| {
        let Some(x) = pin[0].checked_sub(offset.0) else {
            return false;
        };
        let Some(y) = pin[1].checked_sub(offset.1) else {
            return false;
        };
        region.contains(x * s + s / 2, y * s + s / 2)
    });
    match existing {
        // A drag adds only; an already-pinned region stays pinned.
        Some(_) if !toggle => return Task::none(),
        Some(i) => {
            if let Some(sess) = app.session_mut() {
                sess.cfg.pins.remove(i);
            }
        }
        None => {
            if let Some(sess) = app.session_mut() {
                sess.cfg.pins.push([sx / s + offset.0, sy / s + offset.1]);
            }
        }
    }
    app.write_pins();
    app.preview_tasks()
}
