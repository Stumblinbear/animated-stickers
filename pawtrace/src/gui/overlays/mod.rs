//! Canvas overlays as pure functions of state. Each overlay reads an
//! [`OverlayCtx`] and decides for itself whether it has anything to draw,
//! returning a sibling canvas the preview stacks over the art. Registration is
//! the flat [`ALL`] list; its order is the composite order, bottom-first.
//!
//! Overlays derive from state, never from the active tool: a tool writes state
//! and an overlay reads it, so two edits that reach the same state draw the
//! same thing. They render into sibling canvases rather than the preview's own
//! frame because iced draws a canvas's images above its meshes within one
//! layer, so an overlay drawn in the preview frame would fall under the art.

mod anchors;
mod fates;
mod hit;
mod pins;
mod seams;

use super::app::App;
use super::compute::{Img, TraceOutput};
use super::msg::{Msg, StripView};
use super::phases::SubView;
use iced::widget::canvas::{Frame, LineDash, Path, Stroke};
use iced::{Element, Vector};
use std::time::Instant;

/// The read-only view of app state an overlay draws from. Every field is here
/// because a registered overlay consumes it; the viewport fields resolve the
/// same image-to-screen transform the preview uses, so overlays stay aligned
/// with the art.
pub struct OverlayCtx<'a> {
    /// The active view, selecting the whole-document composite or a phase.
    view: StripView,
    /// The active phase's sub-view, or `None` on the Document view.
    subview: Option<SubView>,
    /// The shown art's crop-space dimensions, the size overlays resolve their
    /// viewport against so they align with the preview. `None` when nothing is
    /// rendered yet.
    dims: Option<(f32, f32)>,
    /// The active view's zoom, `None` to fit.
    zoom: Option<f32>,
    /// The active view's pan, a screen-px offset from centered.
    pan: Vector,
    /// The selected layer's source-px offset, for mapping pins on a stage view.
    offset: (u32, u32),
    /// The selected layer's pins in document source px, empty when nothing is
    /// selected.
    pins: &'a [[u32; 2]],
    /// The trace-fate tint over the segmentation, when the Regions stage has
    /// produced one.
    fate_tint: Option<&'a Img>,
    /// The active subview's finalized trace output, read off the fit or
    /// simplify memo, `None` off the Fit and Simplify views or before that
    /// stage has produced one. The anchors and seams overlays both derive from
    /// it, so they draw the trace the view is actually showing.
    active_trace: Option<TraceOutput>,
    /// Whether the show-all modifier is held, drawing every path's anchors and
    /// seams rather than only the hovered path's.
    show_all_anchors: bool,
}

impl OverlayCtx<'_> {
    /// Reads the overlay-relevant slice of `app` for the selected document.
    pub fn from_app(app: &App) -> OverlayCtx<'_> {
        let session = app.session();
        let has_selection = session.is_some_and(|s| !s.selection.is_empty());
        let offset = session
            .zip(app.doc())
            .and_then(|(s, doc)| doc.layer(s.selected_layer))
            .map(|l| l.offset)
            .unwrap_or((0, 0));
        let subview = app.active_subview();

        OverlayCtx {
            view: session.map(|s| s.view).unwrap_or_default(),
            subview,
            dims: app.active_art().map(|a| a.dims()),
            zoom: session.and_then(|s| s.zoom()),
            pan: session.map(|s| s.pan()).unwrap_or(Vector::ZERO),
            offset,
            pins: if has_selection {
                session
                    .zip(app.doc())
                    .and_then(|(s, doc)| doc.inputs.get(&s.selected_layer))
                    .map(|i| i.pins.as_slice())
                    .unwrap_or(&[])
            } else {
                &[]
            },
            fate_tint: session.and_then(|s| s.preview.fate_tint.as_ref()),
            active_trace: anchors::read(app, subview),
            show_all_anchors: app.modifiers.alt(),
        }
    }
}

/// One overlay: given the state, either a sibling canvas to composite or
/// nothing.
type Overlay = for<'a> fn(&OverlayCtx<'a>) -> Option<Element<'a, Msg>>;

/// Every overlay, in composite order: earlier entries draw under later ones.
/// The fate tint sits over the art, the seam highlights over the tint, the
/// anchors over the seams so their outline stays legible, and the pins on top.
pub const ALL: &[Overlay] =
    &[fates::overlay, seams::overlay, anchors::overlay, pins::overlay];

/// Strokes `path` as a marching-ants outline: a thin dashed accent line whose
/// dash phase advances from `now`, so the dashes crawl along the path over a
/// half-second cycle. Widths are in the frame's coordinate space.
// Reserved for the spec §8 brushed-region treatment; nothing draws it yet.
#[allow(dead_code)]
pub fn marching_ants(frame: &mut Frame, path: &Path, now: Instant) {
    const SEGMENTS: [f32; 2] = [4.0, 4.0];
    const CYCLE_SECS: f32 = 0.5;

    let cycle: f32 = SEGMENTS.iter().sum();
    let offset = (super::view::anim::phase(now, CYCLE_SECS) * cycle) as usize;

    frame.stroke(
        path,
        Stroke {
            line_dash: LineDash {
                segments: &SEGMENTS,
                offset,
            },
            ..Stroke::default()
                .with_color(super::view::theme::ACCENT)
                .with_width(1.0)
        },
    );
}
