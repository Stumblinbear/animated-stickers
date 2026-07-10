//! Preview-canvas interactions: viewport gestures, the Document-view click that
//! selects a layer, and the tool press/drag/release the canvas publishes. The
//! per-tool press and drag actions live in the owning tool modules; this
//! routes to them. Tool points are in the shown view's coordinates (document px
//! on Document, source-crop px on a stage view).

use crate::gui::app::App;
use crate::gui::ids::{layers_scrollable, LayerId};
use crate::gui::msg::{CanvasMsg, Msg};
use crate::gui::tools;
use iced::advanced::widget::operation::scrollable::{snap_to, RelativeOffset};
use iced::advanced::widget::operate;
use iced::{Point, Task};

pub(super) fn update(app: &mut App, msg: CanvasMsg) -> Task<Msg> {
    match msg {
        CanvasMsg::SetViewport { zoom, pan } => {
            if let Some(s) = app.session_mut() {
                s.set_viewport(Some(zoom), pan);
            }
            Task::none()
        }
        CanvasMsg::SelectAt(p) => select_at(app, p),
        CanvasMsg::ToolPress(p) => tools::press(app, p),
        CanvasMsg::ToolDrag(p) => tools::drag(app, p),
        // A release ends the gesture: one paint-drag stays one undo step and
        // the next press starts a new one.
        CanvasMsg::ToolRelease => {
            app.seal_undo();
            Task::none()
        }
    }
}

/// Resolves a Document-view click at document px `p`: selects the topmost
/// enabled, visible layer whose art covers the point, routing the hit through
/// the rail's click semantics so modifiers behave identically, or deselects
/// when the click lands on empty space. On a hit the rail scrolls the row into
/// view so both selection surfaces agree.
fn select_at(app: &mut App, p: Point) -> Task<Msg> {
    match hit_test(app, p) {
        Some(i) => {
            let selected = super::layer::click(app, i);
            Task::batch([selected, scroll_to_row(app, i)])
        }
        None => super::layer::deselect(app),
    }
}

/// The topmost layer covering document px `p`: walks the stack front-to-back,
/// skips preview-hidden and export-excluded layers, and takes the first whose
/// cropped art has source alpha at or above that layer's resolved threshold.
fn hit_test(app: &App, p: Point) -> Option<LayerId> {
    let doc_idx = app.selected_pos();
    let doc = app.doc()?;
    doc.layers.iter().rev().find_map(|layer| {
        let flags = &doc.inputs[&layer.id];
        if !flags.visible || !flags.enabled {
            return None;
        }
        let (lx, ly) = (p.x - layer.offset.0 as f32, p.y - layer.offset.1 as f32);
        if lx < 0.0 || ly < 0.0 {
            return None;
        }
        let (x, y) = (lx as u32, ly as u32);
        if x >= layer.img.width() || y >= layer.img.height() {
            return None;
        }
        let alpha = layer.img.get_pixel(x, y).0[3];
        let threshold = app.stack(doc_idx).resolve(&layer.name).0.alpha_threshold;
        (alpha >= threshold).then_some(layer.id)
    })
}

/// Scrolls the rail so layer `i`'s row is visible.
fn scroll_to_row(app: &App, i: LayerId) -> Task<Msg> {
    let Some(doc) = app.doc() else {
        return Task::none();
    };
    let n = doc.layers.len();
    let Some(pos) = doc.layer_pos(i) else {
        return Task::none();
    };
    if n <= 1 {
        return Task::none();
    }
    // The rail lists layers topmost-first, so a row's fraction down the list is
    // its distance from the top of the stack over the stack height.
    let from_top = (n - 1 - pos) as f32 / (n - 1) as f32;
    let offset = RelativeOffset { x: 0.0, y: from_top }.into();
    operate(snap_to(layers_scrollable(), offset))
}
