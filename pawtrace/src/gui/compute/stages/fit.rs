//! Fit stage: the boundary walk, the cross-shape seam match, and the cubic
//! fit of every shape into the pre-simplify trace. Shapes are independent at
//! the fit: each ring embeds its shared-span bytes during the (uncached) walk
//! preamble, so each shape is cached by its own contour content and the fit
//! params, and a recompute re-fits only the shapes whose contours changed: a
//! pin toggle re-fits one shape, an absorb tweak only the shapes it moved.
//! The cache holds a seam-free shape's paths mask-local, so one entry serves
//! it wherever its bbox sits.

use crate::color::Srgb;
use super::super::artifact::Artifact;
use super::super::cache::ShapeCache;
use super::super::{layer_bboxes, LayerBboxes, LayerTrace};
use crate::pipeline::{self, Shape, TraceSeams};
use crate::seams::{self, StitchParams};
use crate::trace::{ContourParams, FitParams, FittedPath, SmoothedContour};
use rayon::prelude::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// One shape's contours: its color, its smoothed boundary polylines with
/// corners, seam-slack flags, and seam spans, and the translation from the
/// contours' coordinate space to scaled space. Internal to the
/// walk-then-fit step; nothing outside this module reads pre-fit geometry.
#[derive(Debug)]
struct ShapeContours {
    color: Srgb,
    contours: Vec<SmoothedContour>,
    translate: (f64, f64),
}

/// Pre-simplify trace inputs: the shapes to walk and fit, the contour-walk
/// params, the fit params, and the seam-match params. The pins and every
/// upstream edit ride in through the chained shapes artifact.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gui) struct FitInputs {
    pub shapes: Artifact<Vec<Shape>>,
    pub contour: ContourParams,
    pub fit: FitParams,
    pub stitch: StitchParams,
}

/// A finalized trace and the supersample scale its coordinates are expressed
/// at: dividing a coordinate by `scale` gives its position in crop px. Both
/// the fit and the simplify stage produce this shape, so the anchors overlay
/// reads either's current value without caring which stage it came from.
#[derive(Clone, Debug)]
pub(in crate::gui) struct TraceOutput {
    pub trace: Arc<LayerTrace>,
    /// Per-path culling boxes for `trace`, in its supersample space, grouped to
    /// match `trace` run for run. A consumer pairing them reads both from the
    /// same output so the grouping lines up.
    pub bboxes: Arc<LayerBboxes>,
    /// The shared-stretch sidecar of `trace`, same grouping. Both stages carry
    /// it: the simplify stage remaps its spans onto the post-simplify anchors,
    /// so the seams overlay reads it off whichever stage's output is current.
    pub seams: Arc<TraceSeams>,
    pub scale: u32,
}

pub(super) fn compute_fit(k: &FitInputs, cache: ShapeCache) -> TraceOutput {
    // The cross-shape seam match runs uncached: shapes are independent only
    // from here on, once each ring carries its shared-span bytes.
    let stitched = seams::stitched_contours(&k.shapes, &k.contour, &k.stitch);

    let contours: Vec<ShapeContours> = k
        .shapes
        .iter()
        .zip(stitched)
        .map(|((color, ..), (contours, translate))| ShapeContours {
            color: *color,
            contours,
            translate,
        })
        .collect();

    let (trace, seams) = fit_contours(&cache, &contours, &k.fit);
    let bboxes = layer_bboxes(&trace);

    TraceOutput {
        trace: Arc::new(trace),
        bboxes: Arc::new(bboxes),
        seams: Arc::new(seams),
        scale: k.contour.scale,
    }
}

/// Key of one shape's fitted paths: the fit params plus the shape's contour
/// content, seam spans included. Color and translation are excluded: the
/// paths are fit in the contours' own coordinate space, so two shapes with
/// identical contours fit identically wherever they sit.
fn contour_key(cfg: &FitParams, shape: &ShapeContours) -> u64 {
    let mut h = DefaultHasher::new();

    cfg.opttolerance.to_bits().hash(&mut h);
    cfg.seam_slack.to_bits().hash(&mut h);

    shape.contours.len().hash(&mut h);
    for c in &shape.contours {
        c.pts.len().hash(&mut h);
        for &(x, y) in &c.pts {
            x.to_bits().hash(&mut h);
            y.to_bits().hash(&mut h);
        }
        c.corners.hash(&mut h);
        c.flags.hash(&mut h);
        c.seams.hash(&mut h);
    }

    h.finish()
}

/// Fits every shape's contours with per-shape reuse: cached shapes skip the
/// fit, misses are fitted in parallel and stored. The paths match an uncached
/// trace; each shape's paths are translated to scaled space and grouped into
/// the color runs the output wants, the seam sidecar alongside.
fn fit_contours(
    cache: &ShapeCache,
    shapes: &[ShapeContours],
    cfg: &FitParams,
) -> (LayerTrace, TraceSeams) {
    let keys: Vec<u64> = shapes.par_iter().map(|s| contour_key(cfg, s)).collect();

    let mut fitted: Vec<Option<Arc<Vec<FittedPath>>>> = {
        let mut c = cache.lock().unwrap();
        keys.iter().map(|k| c.get(k).cloned()).collect()
    };

    let fresh: Vec<(usize, Arc<Vec<FittedPath>>)> = shapes
        .par_iter()
        .enumerate()
        .filter(|&(i, _)| fitted[i].is_none())
        .map(|(i, s)| (i, Arc::new(crate::trace::fit_contours(&s.contours, cfg))))
        .collect();
    {
        let mut c = cache.lock().unwrap();

        for (i, t) in fresh {
            c.put(keys[i], t.clone());
            fitted[i] = Some(t);
        }
    }

    let traced = shapes
        .iter()
        .zip(fitted)
        .map(|(s, t)| {
            let mut paths: Vec<FittedPath> = t.unwrap().as_ref().clone();

            for (p, _) in &mut paths {
                p.translate(s.translate.0, s.translate.1);
            }

            (s.color, paths)
        })
        .collect();

    pipeline::group_traced(traced)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::pipeline::{Shape, ShapeParams};
    use crate::regions::{self, PlanParams};
    use crate::trace::ContourParams;
    use image::{GrayImage, Luma, RgbImage};
    use lru::LruCache;
    use std::num::NonZeroUsize;
    use std::sync::Mutex;

    fn cache() -> ShapeCache {
        Arc::new(Mutex::new(LruCache::new(NonZeroUsize::new(64).unwrap())))
    }

    fn contours_of(shapes: &[Shape], cp: &ContourParams, sp: &StitchParams) -> Vec<ShapeContours> {
        seams::stitched_contours(shapes, cp, sp)
            .into_iter()
            .zip(shapes)
            .map(|((contours, translate), (color, ..))| ShapeContours {
                color: *color,
                contours,
                translate,
            })
            .collect()
    }

    /// A 36px block that clears the floor plus an isolated 2px speck that
    /// does not, on transparency.
    fn fixture() -> (RgbImage, GrayImage) {
        let mut quant = RgbImage::from_pixel(24, 8, image::Rgb([0, 0, 0]));
        let mut alpha = GrayImage::new(24, 8);
        let mut opaque = |q: &mut RgbImage, x: u32, y: u32, c: Srgb| {
            q.put_pixel(x, y, c.into());
            alpha.put_pixel(x, y, Luma([255]));
        };

        for y in 0..6 {
            for x in 0..6 {
                opaque(&mut quant, x, y, Srgb([200, 30, 30]));
            }
        }

        opaque(&mut quant, 15, 0, Srgb([200, 200, 40]));
        opaque(&mut quant, 16, 0, Srgb([200, 200, 40]));

        (quant, alpha)
    }

    fn assert_same(a: &LayerTrace, b: &LayerTrace) {
        assert_eq!(a.len(), b.len());

        for ((h1, p1), (h2, p2)) in a.iter().zip(b) {
            assert_eq!(h1, h2);
            assert_eq!(p1.len(), p2.len());

            for (x, y) in p1.iter().zip(p2) {
                assert_eq!(x.start, y.start);
                assert_eq!(x.cubics, y.cubics);
            }
        }
    }

    #[test]
    fn memoed_trace_is_identical_cold_and_warm() {
        let (quant, alpha) = fixture();

        let cfg = Config {
            scale: 1,
            detail: 5.0,
            ..Default::default()
        };

        let regs = regions::segment(&quant, &alpha);
        let plan = regions::merge_plan(&regs, &alpha, &PlanParams::of(&cfg), 512, &[]);
        let shapes = pipeline::planned_shapes(&plan, &alpha, &ShapeParams::of(&cfg));
        let (plain, _) = pipeline::trace_planned(&plan, &alpha, &cfg);

        let contours = contours_of(&shapes, &ContourParams::of(&cfg), &StitchParams::of(&cfg));
        let fp = FitParams::of(&cfg);

        let c = cache();
        let (cold, _) = fit_contours(&c, &contours, &fp);
        let (warm, _) = fit_contours(&c, &contours, &fp);

        assert_same(&plain, &cold);
        assert_same(&plain, &warm);
    }

    #[test]
    fn pin_toggle_refits_only_the_pinned_shape() {
        let (quant, alpha) = fixture();
        let cfg = Config {
            scale: 1,
            detail: 5.0,
            ..Default::default()
        };
        let regs = regions::segment(&quant, &alpha);
        let c = cache();
        let cp = ContourParams::of(&cfg);
        let fp = FitParams::of(&cfg);
        let sp = StitchParams::of(&cfg);

        let plan = regions::merge_plan(&regs, &alpha, &PlanParams::of(&cfg), 512, &[]);
        let shapes = pipeline::planned_shapes(&plan, &alpha, &ShapeParams::of(&cfg));
        fit_contours(&c, &contours_of(&shapes, &cp, &sp), &fp);
        assert_eq!(shapes.len(), 1, "the speck is culled without a pin");

        let before = c.lock().unwrap().len();

        // Pinning the speck adds its shape; the block's contours are untouched,
        // so its fit is a cache hit and only the speck enters the cache.
        let pins = [(15u32, 0u32)];
        let plan = regions::merge_plan(&regs, &alpha, &PlanParams::of(&cfg), 512, &pins);
        let shapes = pipeline::planned_shapes(&plan, &alpha, &ShapeParams::of(&cfg));
        assert_eq!(shapes.len(), 2);

        let (memoed, _) = fit_contours(&c, &contours_of(&shapes, &cp, &sp), &fp);
        assert_eq!(c.lock().unwrap().len(), before + 1);
        assert_same(&pipeline::trace_planned(&plan, &alpha, &cfg).0, &memoed);
    }

    /// A shape whose art covers the scaled pixels `[x, x+w) x [y, y+h)`:
    /// bbox origin `(x, y)`, mask with the pipeline's 1px border.
    fn rect_shape(color: Srgb, (x, y): (u32, u32), w: u32, h: u32) -> Shape {
        let mut mask = GrayImage::new(w + 2, h + 2);
        for my in 1..=h {
            for mx in 1..=w {
                mask.put_pixel(mx, my, Luma([255]));
            }
        }
        (color, mask, None, (x, y))
    }

    #[test]
    fn sibling_fixture_is_identical_cold_and_warm() {
        // Two abutting siblings plus a detached block: the memoed fit must
        // reproduce itself warm (spans included), and an edit to the detached
        // shape must re-fit it alone while the siblings hit the cache.
        let shapes = vec![
            rect_shape(Srgb([200, 30, 30]), (1, 1), 8, 6),
            rect_shape(Srgb([30, 30, 200]), (9, 1), 8, 6),
            rect_shape(Srgb([30, 200, 30]), (24, 1), 4, 4),
        ];
        let cfg = Config {
            scale: 1,
            ..Default::default()
        };
        let cp = ContourParams::of(&cfg);
        let fp = FitParams::of(&cfg);
        let sp = StitchParams::of(&cfg);
        let c = cache();

        let contours = contours_of(&shapes, &cp, &sp);
        let (cold, cold_seams) = fit_contours(&c, &contours, &fp);
        let entries = c.lock().unwrap().len();
        let (warm, warm_seams) = fit_contours(&c, &contours, &fp);

        assert_same(&cold, &warm);
        assert_eq!(cold_seams, warm_seams);
        assert!(cold_seams.iter().flatten().any(|s| !s.is_empty()));
        assert_eq!(c.lock().unwrap().len(), entries, "a warm run adds nothing");

        let mut grown = shapes.clone();
        grown[2] = rect_shape(Srgb([30, 200, 30]), (24, 1), 4, 5);
        let (regrown, _) = fit_contours(&c, &contours_of(&grown, &cp, &sp), &fp);
        assert_eq!(
            c.lock().unwrap().len(),
            entries + 1,
            "only the edited shape's contour content changed"
        );
        assert_same(&cold, &fit_contours(&c, &contours, &fp).0);
        assert_ne!(regrown.len(), 0);
    }
}
