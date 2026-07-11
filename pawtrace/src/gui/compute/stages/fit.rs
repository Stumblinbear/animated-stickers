//! Fit stage: the boundary walk and the cubic fit of every shape into the
//! pre-simplify trace. Shapes are independent, so each is cached by its own
//! contour content and the fit params, and a recompute re-fits only the shapes
//! that changed: a pin toggle re-fits one shape, an absorb tweak only the
//! shapes it moved. The cache holds the paths mask-local, so one entry serves a
//! shape wherever its bbox sits.

use crate::color::Srgb;
use super::super::artifact::Artifact;
use super::super::cache::ShapeCache;
use super::super::LayerTrace;
use crate::pipeline::{self, Shape};
use crate::trace::{self, ContourParams, FitParams, SmoothedContour, TracedPath};
use rayon::prelude::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// One shape's mask-local contours: its color, its smoothed boundary polylines
/// with corners and seam-slack flags, and the mask bbox origin a consumer
/// translates by to reach scaled space. Internal to the walk-then-fit step;
/// nothing outside this module reads pre-fit geometry.
#[derive(Debug)]
struct ShapeContours {
    color: Srgb,
    contours: Vec<SmoothedContour>,
    origin: (u32, u32),
}

/// Pre-simplify trace inputs: the shapes to walk and fit, the contour-walk
/// params, and the fit params. The pins and every upstream edit ride in through
/// the chained shapes artifact.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gui) struct FitInputs {
    pub shapes: Artifact<Vec<Shape>>,
    pub contour: ContourParams,
    pub fit: FitParams,
}

/// A finalized trace and the supersample scale its coordinates are expressed
/// at: dividing a coordinate by `scale` gives its position in crop px. Both
/// the fit and the simplify stage produce this shape, so the anchors overlay
/// reads either's current value without caring which stage it came from.
#[derive(Clone, Debug)]
pub(in crate::gui) struct TraceOutput {
    pub trace: Arc<LayerTrace>,
    pub scale: u32,
}

pub(super) fn compute_fit(k: &FitInputs, cache: ShapeCache) -> TraceOutput {
    let contours: Vec<ShapeContours> = k
        .shapes
        .par_iter()
        .map(|(color, mask, slack, origin)| ShapeContours {
            color: *color,
            contours: trace::smoothed_contours(mask, &k.contour, slack.as_ref()),
            origin: *origin,
        })
        .collect();

    TraceOutput {
        trace: Arc::new(fit_contours(&cache, &contours, &k.fit)),
        scale: k.contour.scale,
    }
}

/// Key of one shape's fitted paths: the fit params plus the shape's contour
/// content. Color and origin are excluded: the paths are fit mask-local, so
/// two shapes with identical contours fit identically wherever they sit.
fn contour_key(cfg: &FitParams, shape: &ShapeContours) -> u64 {
    let mut h = DefaultHasher::new();

    cfg.opttolerance.to_bits().hash(&mut h);
    cfg.seam_slack.to_bits().hash(&mut h);

    shape.contours.len().hash(&mut h);
    for (pts, corners, flags) in &shape.contours {
        pts.len().hash(&mut h);
        for &(x, y) in pts {
            x.to_bits().hash(&mut h);
            y.to_bits().hash(&mut h);
        }
        corners.hash(&mut h);
        flags.hash(&mut h);
    }

    h.finish()
}

/// Fits every shape's contours with per-shape reuse: cached shapes skip the
/// fit, misses are fitted in parallel and stored. The paths match an uncached
/// trace; each shape's paths are translated from mask-local to scaled space and
/// grouped into the color runs the output wants.
fn fit_contours(cache: &ShapeCache, shapes: &[ShapeContours], cfg: &FitParams) -> LayerTrace {
    let keys: Vec<u64> = shapes.par_iter().map(|s| contour_key(cfg, s)).collect();

    let mut fitted: Vec<Option<Arc<Vec<TracedPath>>>> = {
        let mut c = cache.lock().unwrap();
        keys.iter().map(|k| c.get(k).cloned()).collect()
    };

    let fresh: Vec<(usize, Arc<Vec<TracedPath>>)> = shapes
        .par_iter()
        .enumerate()
        .filter(|&(i, _)| fitted[i].is_none())
        .map(|(i, s)| (i, Arc::new(trace::fit_contours(&s.contours, cfg))))
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
            let mut paths: Vec<TracedPath> = t.unwrap().as_ref().clone();

            // -1.0: the shape mask's origin sits one border pixel above and
            // left of the region bbox (see regions::region_shape).
            for p in &mut paths {
                p.translate(s.origin.0 as f64 - 1.0, s.origin.1 as f64 - 1.0);
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

    fn contours_of(shapes: &[Shape], cp: &ContourParams) -> Vec<ShapeContours> {
        shapes
            .iter()
            .map(|(color, mask, slack, origin)| ShapeContours {
                color: *color,
                contours: trace::smoothed_contours(mask, cp, slack.as_ref()),
                origin: *origin,
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
        let plain = pipeline::trace_planned(&plan, &alpha, &cfg);

        let contours = contours_of(&shapes, &ContourParams::of(&cfg));
        let fp = FitParams::of(&cfg);

        let c = cache();
        let cold = fit_contours(&c, &contours, &fp);
        let warm = fit_contours(&c, &contours, &fp);

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

        let plan = regions::merge_plan(&regs, &alpha, &PlanParams::of(&cfg), 512, &[]);
        let shapes = pipeline::planned_shapes(&plan, &alpha, &ShapeParams::of(&cfg));
        fit_contours(&c, &contours_of(&shapes, &cp), &fp);
        assert_eq!(shapes.len(), 1, "the speck is culled without a pin");

        let before = c.lock().unwrap().len();

        // Pinning the speck adds its shape; the block's contours are untouched,
        // so its fit is a cache hit and only the speck enters the cache.
        let pins = [(15u32, 0u32)];
        let plan = regions::merge_plan(&regs, &alpha, &PlanParams::of(&cfg), 512, &pins);
        let shapes = pipeline::planned_shapes(&plan, &alpha, &ShapeParams::of(&cfg));
        assert_eq!(shapes.len(), 2);

        let memoed = fit_contours(&c, &contours_of(&shapes, &cp), &fp);
        assert_eq!(c.lock().unwrap().len(), before + 1);
        assert_same(&pipeline::trace_planned(&plan, &alpha, &cfg), &memoed);
    }
}
