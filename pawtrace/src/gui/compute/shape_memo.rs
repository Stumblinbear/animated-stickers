//! Per-shape fitted-path memo. Shapes are independent, so each is keyed by
//! its own mask and slack content plus the fit params the tracer reads, and
//! a recompute re-fits only the shapes that changed: a pin toggle re-fits
//! one shape, an absorb tweak only the shapes it moved.

use super::cache::ShapeCache;
use super::LayerTrace;
use crate::trace::{TraceParams, TracedPath};
use crate::{pipeline, trace};
use image::GrayImage;
use rayon::prelude::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Key of one shape's fitted paths: the mask and slack content plus every
/// value `trace::trace_mask` reads.
fn shape_key(cfg: &TraceParams, mask: &GrayImage, slack: Option<&GrayImage>) -> u64 {
    let mut h = DefaultHasher::new();

    cfg.alphamax.to_bits().hash(&mut h);
    cfg.opttolerance.to_bits().hash(&mut h);
    cfg.seam_slack.to_bits().hash(&mut h);
    cfg.smoothing.to_bits().hash(&mut h);
    cfg.scale.hash(&mut h);
    mask.dimensions().hash(&mut h);
    mask.as_raw().hash(&mut h);

    match slack {
        Some(s) => {
            true.hash(&mut h);
            s.as_raw().hash(&mut h);
        }
        None => false.hash(&mut h),
    }

    h.finish()
}

/// [`pipeline::trace_planned`]'s trace half with per-shape reuse: cached
/// shapes skip the fit, misses are fitted in parallel and stored. The paths
/// are identical to an uncached trace; the cache holds them untranslated
/// (mask-local), so one entry serves a shape wherever its bbox sits.
pub(super) fn trace_shapes_memo(
    cache: &ShapeCache,
    shapes: &[pipeline::Shape],
    cfg: &TraceParams,
) -> LayerTrace {
    let keys: Vec<u64> = shapes
        .par_iter()
        .map(|(_, mask, slack, _)| shape_key(cfg, mask, slack.as_ref()))
        .collect();

    let mut fitted: Vec<Option<Arc<Vec<TracedPath>>>> = {
        let mut c = cache.lock().unwrap();
        keys.iter().map(|k| c.get(k).cloned()).collect()
    };

    let fresh: Vec<(usize, Arc<Vec<TracedPath>>)> = shapes
        .par_iter()
        .enumerate()
        .filter(|&(i, _)| fitted[i].is_none())
        .map(|(i, (_, mask, slack, _))| (i, Arc::new(trace::trace_mask(mask, cfg, slack.as_ref()))))
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
        .map(|((color, _, _, (bx, by)), t)| {
            let mut paths: Vec<TracedPath> = t.unwrap().as_ref().clone();

            // -1.0: the shape mask's origin sits one border pixel above and
            // left of the region bbox (see regions::region_shape).
            for p in &mut paths {
                p.translate(*bx as f64 - 1.0, *by as f64 - 1.0);
            }

            (*color, paths)
        })
        .collect();

    pipeline::group_traced(traced)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::pipeline::ShapeParams;
    use crate::regions::{self, PlanParams};
    use image::{GrayImage, Luma, RgbImage};
    use lru::LruCache;
    use std::num::NonZeroUsize;
    use std::sync::Mutex;

    fn cache() -> ShapeCache {
        Arc::new(Mutex::new(LruCache::new(NonZeroUsize::new(64).unwrap())))
    }

    /// A 36px block that clears the floor plus an isolated 2px speck that
    /// does not, on transparency.
    fn fixture() -> (RgbImage, GrayImage) {
        let mut quant = RgbImage::from_pixel(24, 8, image::Rgb([0, 0, 0]));
        let mut alpha = GrayImage::new(24, 8);
        let mut opaque = |q: &mut RgbImage, x: u32, y: u32, c: [u8; 3]| {
            q.put_pixel(x, y, image::Rgb(c));
            alpha.put_pixel(x, y, Luma([255]));
        };

        for y in 0..6 {
            for x in 0..6 {
                opaque(&mut quant, x, y, [200, 30, 30]);
            }
        }

        opaque(&mut quant, 15, 0, [200, 200, 40]);
        opaque(&mut quant, 16, 0, [200, 200, 40]);

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

        let c = cache();
        let tp = TraceParams::of(&cfg);
        let cold = trace_shapes_memo(&c, &shapes, &tp);
        let warm = trace_shapes_memo(&c, &shapes, &tp);

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
        let tp = TraceParams::of(&cfg);

        let plan = regions::merge_plan(&regs, &alpha, &PlanParams::of(&cfg), 512, &[]);
        let shapes = pipeline::planned_shapes(&plan, &alpha, &ShapeParams::of(&cfg));
        trace_shapes_memo(&c, &shapes, &tp);
        assert_eq!(shapes.len(), 1, "the speck is culled without a pin");

        let before = c.lock().unwrap().len();

        // Pinning the speck adds its shape; the block's mask is untouched, so
        // its fit is a cache hit and only the speck enters the cache.
        let pins = [(15u32, 0u32)];
        let plan = regions::merge_plan(&regs, &alpha, &PlanParams::of(&cfg), 512, &pins);
        let shapes = pipeline::planned_shapes(&plan, &alpha, &ShapeParams::of(&cfg));
        assert_eq!(shapes.len(), 2);

        let memoed = trace_shapes_memo(&c, &shapes, &tp);
        assert_eq!(c.lock().unwrap().len(), before + 1);
        assert_same(&pipeline::trace_planned(&plan, &alpha, &cfg), &memoed);
    }
}
