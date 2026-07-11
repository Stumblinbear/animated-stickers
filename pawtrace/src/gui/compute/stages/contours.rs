//! Contour-view stage: the trace's smoothing and corner detection, stopped
//! before the fit, rendered as a debug overlay.

use super::super::artifact::Artifact;
use super::super::render::render_debug;
use super::super::Img;
use crate::pipeline::{self, Shape};
use crate::trace::TraceParams;

/// Contour-view inputs: the shapes and the trace params (the contour walk is
/// the trace's smoothing and corner detection, stopped before the fit).
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gui) struct ContoursInputs {
    pub shapes: Artifact<Vec<Shape>>,
    pub params: TraceParams,
}

pub(super) fn compute_contours(k: &ContoursInputs, dims: (u32, u32)) -> Option<Img> {
    let contours = pipeline::debug_from_shapes(&k.shapes, &k.params);
    render_debug(&contours, dims.0, dims.1, k.params.scale)
}
