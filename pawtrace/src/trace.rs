//! Mask -> cubic-bezier paths: visioncortex walks the pixel boundaries,
//! crate::fit turns them into error-bounded curves. visioncortex's own
//! spline mode fits one cubic per ~45-degree direction change with no
//! error-driven merging, which produced anchors every few pixels; Schneider
//! places anchors only where the error tolerance demands.

use crate::config::Config;
use crate::fit;
use image::GrayImage;
use visioncortex::clusters::Cluster;
use visioncortex::{BinaryImage, PathSimplifyMode};

/// One cubic segment: (control 1, control 2, end), absolute coords.
pub type Cubic = ((f64, f64), (f64, f64), (f64, f64));

/// A closed filled path: cubic bezier in absolute coords (start + segments).
#[derive(Debug, Clone)]
pub struct TracedPath {
    pub start: (f64, f64),
    pub cubics: Vec<Cubic>,
}

impl TracedPath {
    /// Multiplies every coordinate by `s`, converting the path between
    /// scale spaces.
    pub fn scale(&mut self, s: f64) {
        self.start.0 *= s;
        self.start.1 *= s;
        for (c1, c2, end) in &mut self.cubics {
            c1.0 *= s;
            c1.1 *= s;
            c2.0 *= s;
            c2.1 *= s;
            end.0 *= s;
            end.1 *= s;
        }
    }

    pub fn translate(&mut self, dx: f64, dy: f64) {
        self.start.0 += dx;
        self.start.1 += dy;
        for (c1, c2, end) in &mut self.cubics {
            c1.0 += dx;
            c1.1 += dy;
            c2.0 += dx;
            c2.1 += dy;
            end.0 += dx;
            end.1 += dy;
        }
    }
}

/// Traces a connected binary mask (a region shape: one component, holes
/// allowed) into filled paths, one per boundary contour, in mask
/// coordinates. Hole contours carry opposite winding for nonzero fills.
///
/// `slack`, when given, is a mask over the same grid marking shape pixels
/// abutting a low-contrast neighbor; boundary vertices touching a marked
/// pixel are fit at `cfg.opttolerance * cfg.seam_slack`. `None` fits every
/// vertex at the base tolerance.
pub fn trace_mask(mask: &GrayImage, cfg: &Config, slack: Option<&GrayImage>) -> Vec<TracedPath> {
    let (w, h) = (mask.width() as usize, mask.height() as usize);
    let mut bin = BinaryImage::new_w_h(w, h);
    // The mask and BinaryImage share row-major layout, so the raw index maps
    // straight through.
    for (i, &v) in mask.as_raw().iter().enumerate() {
        if v > 0 {
            bin.set_pixel_index(i, true);
        }
    }

    let corner_threshold = cfg.corner_threshold();
    // Max fit deviation in scaled px, matching potrace's opttolerance units:
    // potrace ran on the supersampled bitmap, so its tolerance is scaled px,
    // NOT source px. Multiplying by scale here made every boundary wander
    // independently by ~half a source px, visibly wobbling the width of thin
    // outlines (each side of a line is fit separately).
    let tolerance = cfg.opttolerance;
    // Arclength window for corner detection; single-vertex turn angles on a
    // pixel-derived path are quantization noise.
    let corner_arm = 2.5 * cfg.scale as f64;
    let smooth_radius = cfg.smooth_radius();

    // The mask is one connected component by construction, so no clustering
    // pass is needed: image_to_paths walks the outer boundary and every hole
    // directly. Raw pixel boundaries (mode None) have the uniform ~1px
    // vertex spacing the windowed smoothing needs; the Polygon mode's
    // simplification keeps wobble extrema as vertices, which smoothing can't
    // average.
    let mut out = Vec::new();
    for path in Cluster::image_to_paths(&bin, PathSimplifyMode::None) {
        let mut pts: Vec<(f64, f64)> = path.path.iter().map(|p| (p.x as f64, p.y as f64)).collect();
        if pts.len() > 1 && pts.first() == pts.last() {
            pts.pop();
        }
        if pts.len() < 3 {
            continue;
        }
        let corners = fit::find_corners(&pts, corner_threshold, corner_arm);
        // Sampled from the pre-smoothing integer vertices, which sit on the
        // slack mask's pixel grid; smoothing preserves vertex count and order.
        let vslack = slack.map(|sm| vertex_slack(&pts, sm, cfg.seam_slack));
        let pts = fit::smooth_pinned(&pts, &corners, smooth_radius);
        if let Some(tp) = fit::fit_closed(&pts, &corners, tolerance, vslack.as_deref()) {
            out.push(tp);
        }
    }
    out
}

/// Per-vertex fit-tolerance multiplier: `factor` for a vertex whose pixel
/// corner touches a set pixel in `slack`, `1.0` otherwise. A boundary vertex
/// at integer `(x, y)` sits at the shared corner of the four pixels
/// `(x-1, y-1)..(x, y)`.
fn vertex_slack(pts: &[(f64, f64)], slack: &GrayImage, factor: f64) -> Vec<f64> {
    let (w, h) = (slack.width() as i64, slack.height() as i64);
    pts.iter()
        .map(|&(x, y)| {
            let (vx, vy) = (x.round() as i64, y.round() as i64);
            let touches = [(vx - 1, vy - 1), (vx, vy - 1), (vx - 1, vy), (vx, vy)]
                .into_iter()
                .any(|(px, py)| {
                    px >= 0 && py >= 0 && px < w && py < h
                        && slack.get_pixel(px as u32, py as u32)[0] != 0
                });
            if touches { factor } else { 1.0 }
        })
        .collect()
}

/// A smoothed boundary polyline, the indices of its corner vertices, and a
/// per-vertex seam-slack flag (same length as the polyline).
pub type SmoothedContour = (Vec<(f64, f64)>, Vec<usize>, Vec<bool>);

/// One smoothed boundary polyline per closed contour of the mask, paired
/// with the indices of its corner vertices and per-vertex seam-slack flags.
/// Runs the same corner detection and smoothing as [`trace_mask`] but stops
/// before the cubic fit, for the debug view that shows what the fit is about
/// to run on. A vertex's slack flag is set when it would fit at the slackened
/// tolerance; every flag is `false` when `slack` is `None`.
pub fn smoothed_contours(
    mask: &GrayImage,
    cfg: &Config,
    slack: Option<&GrayImage>,
) -> Vec<SmoothedContour> {
    let (w, h) = (mask.width() as usize, mask.height() as usize);
    let mut bin = BinaryImage::new_w_h(w, h);
    // The mask and BinaryImage share row-major layout, so the raw index maps
    // straight through.
    for (i, &v) in mask.as_raw().iter().enumerate() {
        if v > 0 {
            bin.set_pixel_index(i, true);
        }
    }
    let corner_threshold = cfg.corner_threshold();
    let corner_arm = 2.5 * cfg.scale as f64;
    let smooth_radius = cfg.smooth_radius();

    let mut out = Vec::new();
    for path in Cluster::image_to_paths(&bin, PathSimplifyMode::None) {
        let mut pts: Vec<(f64, f64)> = path.path.iter().map(|p| (p.x as f64, p.y as f64)).collect();
        if pts.len() > 1 && pts.first() == pts.last() {
            pts.pop();
        }
        if pts.len() < 3 {
            continue;
        }
        let corners = fit::find_corners(&pts, corner_threshold, corner_arm);
        let flags = match slack {
            Some(sm) => vertex_slack(&pts, sm, cfg.seam_slack)
                .into_iter()
                .map(|s| s != 1.0)
                .collect(),
            None => vec![false; pts.len()],
        };
        let smoothed = fit::smooth_pinned(&pts, &corners, smooth_radius);
        out.push((smoothed, corners, flags));
    }
    out
}
