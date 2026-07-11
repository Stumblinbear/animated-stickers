//! Mask -> cubic-bezier paths: visioncortex walks the pixel boundaries,
//! crate::fit turns them into error-bounded curves. visioncortex's own
//! spline mode fits one cubic per ~45-degree direction change with no
//! error-driven merging, which produced anchors every few pixels; Schneider
//! places anchors only where the error tolerance demands.

use crate::config::Config;
use crate::fit::{self, AnchorSpan};
use image::GrayImage;
use visioncortex::clusters::Cluster;
use visioncortex::{BinaryImage, PathSimplifyMode};

/// One cubic segment: (control 1, control 2, end), absolute coords.
pub type Cubic = ((f64, f64), (f64, f64), (f64, f64));

/// Every config value the contour walk ([`smoothed_contours`]: boundary walk,
/// corner detection, smoothing) reads.
#[derive(Debug, Clone, PartialEq)]
pub struct ContourParams {
    pub alphamax: f64,
    pub smoothing: f32,
    pub scale: u32,
}

impl ContourParams {
    pub fn of(cfg: &Config) -> Self {
        Self {
            alphamax: cfg.alphamax,
            smoothing: cfg.smoothing,
            scale: cfg.scale,
        }
    }

    /// [`crate::config::corner_threshold`] at this `alphamax`.
    pub(crate) fn corner_threshold(&self) -> f64 {
        crate::config::corner_threshold(self.alphamax)
    }

    /// Arclength window for corner detection; single-vertex turn angles on a
    /// pixel-derived path are quantization noise.
    pub(crate) fn corner_arm(&self) -> f64 {
        2.5 * self.scale as f64
    }

    /// [`crate::config::smooth_radius`] at this `smoothing` and `scale`.
    pub(crate) fn smooth_radius(&self) -> usize {
        crate::config::smooth_radius(self.smoothing, self.scale)
    }
}

/// Every config value the cubic fit ([`fit_contours`]) reads.
#[derive(Debug, Clone, PartialEq)]
pub struct FitParams {
    pub opttolerance: f64,
    pub seam_slack: f64,
}

impl FitParams {
    pub fn of(cfg: &Config) -> Self {
        Self {
            opttolerance: cfg.opttolerance,
            seam_slack: cfg.seam_slack,
        }
    }
}

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
/// pixel are fit at `fit.opttolerance * fit.seam_slack`. `None` fits every
/// vertex at the base tolerance.
pub fn trace_mask(
    mask: &GrayImage,
    contour: &ContourParams,
    fit: &FitParams,
    slack: Option<&GrayImage>,
) -> Vec<TracedPath> {
    fit_contours(&smoothed_contours(mask, contour, slack), fit)
        .into_iter()
        .map(|(p, _)| p)
        .collect()
}

/// One fitted path and the anchor runs of the shared stretches it embeds.
pub type FittedPath = (TracedPath, Vec<AnchorSpan>);

/// A shared-stretch run over a ring's vertices: the ring traverses the
/// stretch from vertex `start` to vertex `end` (wrapping past the ring's
/// last index). `start == end` marks a stretch covering the whole ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SeamSpan {
    pub start: usize,
    pub end: usize,
    /// Whether the ring traverses the stretch in its canonical direction.
    pub forward: bool,
    /// Whether the whole stretch fits at the seam-slack tolerance. Uniform
    /// across the stretch, so both sides fit the same bytes.
    pub slack: bool,
}

/// A smoothed boundary polyline ready for the cubic fit: its corner vertex
/// indices, a per-vertex seam-slack flag (same length as the polyline), and
/// the shared-stretch spans the ring embeds.
#[derive(Debug, Clone, PartialEq)]
pub struct SmoothedContour {
    pub pts: Vec<(f64, f64)>,
    pub corners: Vec<usize>,
    pub flags: Vec<bool>,
    pub seams: Vec<SeamSpan>,
}

/// The raw integer boundary rings of a connected binary mask, one per closed
/// contour, in mask coordinates: the closing duplicate vertex dropped and
/// rings shorter than three vertices discarded.
pub(crate) fn walk_rings(mask: &GrayImage) -> Vec<Vec<(f64, f64)>> {
    let (w, h) = (mask.width() as usize, mask.height() as usize);
    let mut bin = BinaryImage::new_w_h(w, h);

    // The mask and BinaryImage share row-major layout, so the raw index maps
    // straight through.
    for (i, &v) in mask.as_raw().iter().enumerate() {
        if v > 0 {
            bin.set_pixel_index(i, true);
        }
    }

    // The mask is one connected component by construction, so no clustering
    // pass is needed: image_to_paths walks the outer boundary and every hole
    // directly. Mode None places vertices at boundary direction changes, dense
    // on staircased curves and sparse on straight runs, which is why smoothing
    // windows by arclength rather than vertex count. Polygon mode's
    // simplification keeps wobble extrema as vertices, which smoothing can't
    // average.
    Cluster::image_to_paths(&bin, PathSimplifyMode::None)
        .into_iter()
        .filter_map(|path| {
            let mut pts: Vec<(f64, f64)> =
                path.path.iter().map(|p| (p.x as f64, p.y as f64)).collect();

            if pts.len() > 1 && pts.first() == pts.last() {
                pts.pop();
            }

            (pts.len() >= 3).then_some(pts)
        })
        .collect()
}

/// One smoothed boundary polyline per closed contour of the mask, paired with
/// the indices of its corner vertices and per-vertex seam-slack flags. Walks
/// the boundary, detects corners, and smooths, stopping before the cubic fit.
/// Every ring is free-standing: no shared-stretch spans (the cross-shape
/// match is [`crate::seams::stitched_contours`]).
///
/// `slack`, when given, is a mask over the same grid marking shape pixels
/// abutting a low-contrast neighbor; a vertex touching a marked pixel gets a
/// set flag. Every flag is `false` when `slack` is `None`.
pub fn smoothed_contours(
    mask: &GrayImage,
    cfg: &ContourParams,
    slack: Option<&GrayImage>,
) -> Vec<SmoothedContour> {
    let corner_threshold = cfg.corner_threshold();
    let corner_arm = cfg.corner_arm();
    let smooth_radius = cfg.smooth_radius();

    walk_rings(mask)
        .into_iter()
        .map(|pts| {
            let corners = fit::find_corners(&pts, corner_threshold, corner_arm);

            // Sampled from the pre-smoothing integer vertices, which sit on the
            // slack mask's pixel grid; smoothing preserves vertex count and order.
            let flags = match slack {
                Some(sm) => vertex_touches(&pts, sm),
                None => vec![false; pts.len()],
            };

            let smoothed = fit::smooth_pinned(&pts, &corners, smooth_radius);

            SmoothedContour {
                pts: smoothed,
                corners,
                flags,
                seams: Vec::new(),
            }
        })
        .collect()
}

/// Fits each smoothed contour into an error-bounded cubic path, paired with
/// the anchor-index spans of the shared stretches it embeds. A flagged
/// vertex is fit at `cfg.opttolerance * cfg.seam_slack`, the rest at the base
/// tolerance; a contour with no flagged vertex is fit uniformly. A shared
/// stretch fits uniformly at its own slack flag.
pub fn fit_contours(contours: &[SmoothedContour], cfg: &FitParams) -> Vec<FittedPath> {
    // Max fit deviation in scaled px, matching potrace's opttolerance units:
    // potrace ran on the supersampled bitmap, so its tolerance is scaled px,
    // NOT source px. Multiplying by scale here made every boundary wander
    // independently by ~half a source px, visibly wobbling the width of thin
    // outlines (each side of a line is fit separately).
    let tolerance = cfg.opttolerance;

    contours
        .iter()
        .filter_map(|c| {
            // An all-`1.0` multiplier fits identically to `None`, so a contour
            // with no flagged vertex passes `None` and stays byte-identical to
            // an unslackened trace.
            let vslack: Option<Vec<f64>> = c.flags.iter().any(|&f| f).then(|| {
                c.flags
                    .iter()
                    .map(|&f| if f { cfg.seam_slack } else { 1.0 })
                    .collect()
            });

            fit::fit_closed_seamed(
                &c.pts,
                &c.corners,
                tolerance,
                vslack.as_deref(),
                &c.seams,
                cfg.seam_slack,
            )
        })
        .collect()
}

/// Per-vertex slack flag: set for a vertex whose pixel corner touches a set
/// pixel in `slack`. A boundary vertex at integer `(x, y)` sits at the shared
/// corner of the four pixels `(x-1, y-1)..(x, y)`.
fn vertex_touches(pts: &[(f64, f64)], slack: &GrayImage) -> Vec<bool> {
    pts.iter()
        .map(|&(x, y)| corner_touches(slack, x.round() as i64, y.round() as i64))
        .collect()
}

/// Whether the pixel corner at integer `(vx, vy)` touches a set pixel in
/// `slack`: the corner is shared by the four pixels `(vx-1, vy-1)..(vx, vy)`.
pub(crate) fn corner_touches(slack: &GrayImage, vx: i64, vy: i64) -> bool {
    let (w, h) = (slack.width() as i64, slack.height() as i64);

    [(vx - 1, vy - 1), (vx, vy - 1), (vx - 1, vy), (vx, vy)]
        .into_iter()
        .any(|(px, py)| {
            px >= 0 && py >= 0 && px < w && py < h && slack.get_pixel(px as u32, py as u32)[0] != 0
        })
}
