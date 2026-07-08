//! Per-layer orchestration. The stages, in order: crop to the art's bbox,
//! supersample + flatten (raster), palette extract + remap (palette),
//! transition-band absorption + region segmentation (regions), then one
//! traced shape per region (trace + fit), painted as an outside-in stack.

use crate::trace::TracedPath;
use crate::{config::Config, palette, raster, regions, timing, trace};
use anyhow::Result;
use image::{GrayImage, RgbaImage};

/// One layer's traced result: color hex -> paths, in bottom-first paint
/// order. Colors may repeat: paint order is per region, not per color.
///
/// `doc_dim` is max(document width, height). Pass the document's dimension,
/// never the layer's own. `doc_offset` is where `src`'s origin sits in the
/// document, in source px: (0, 0) for a document-sized layer, the crop
/// origin for a pre-cropped one. It anchors `cfg.pins`, which are document
/// coordinates.
pub fn run(
    src: &RgbaImage,
    cfg: &Config,
    doc_dim: u32,
    doc_offset: (u32, u32),
) -> Result<Vec<(String, Vec<TracedPath>)>> {
    // PSD layers arrive document-sized with tight art, so tracing the full
    // canvas wastes nearly all the work on transparent pixels. Trace the crop
    // and translate the paths back by the offset below.
    let Some((src, ox, oy)) = crop_to_alpha(src, cfg) else {
        return Ok(Vec::new()); // fully transparent layer
    };
    let pins = scale_pins(
        &cfg.pins,
        (doc_offset.0 + ox, doc_offset.1 + oy),
        cfg.scale,
        (src.width(), src.height()),
    );

    // Uniform-color layers (solid fill and border mattes are ~half a
    // production PSD) need no palette, remap, absorption, or quantization:
    // the scaled alpha already determines their regions, and resizing one
    // plane costs a quarter of resizing four.
    let (alpha, regs) = if let Some(color) = raster::uniform_color(&src, cfg.alpha_threshold) {
        let alpha = timing::PREP.time(|| raster::scale_alpha(&src, cfg));
        let regs = timing::SEGMENT.time(|| regions::from_mask(&alpha, color));
        (alpha, regs)
    } else {
        let prep = timing::PREP.time(|| raster::prepare(&src, cfg));
        // detail normalizes against the document, not the crop: a tiny layer
        // would otherwise derive a palette floor larger than itself (README).
        let pal = timing::PALETTE
            .time(|| palette::extract_palette(&prep.flat, &prep.alpha, cfg, doc_dim));
        let quant = timing::REMAP.time(|| {
            let quant = palette::remap(&prep.flat, &prep.alpha, &pal);
            if cfg.color_cleanup > 0 {
                palette::label_smooth(&quant, &prep.alpha, cfg.color_cleanup)
            } else {
                quant
            }
        });
        // One solid shape per connected same-color region (transition bands
        // absorbed), painted as a containment forest: a shape covers its
        // subtree of neighbors, so the heaviest seams are fit once, by the
        // shape on top, and a fit wobble shifts a seam instead of opening a
        // crack to the background.
        let regs = regions::segment_absorbed(&quant, &prep.alpha, cfg);
        (prep.alpha, regs)
    };
    let mut out = simplify_paths(trace_regions(&regs, &alpha, cfg, doc_dim, &pins), cfg);

    // Crop offset in scaled (traced) coordinate space.
    let (sx, sy) = ((ox * cfg.scale) as f64, (oy * cfg.scale) as f64);
    for (_, paths) in &mut out {
        for p in paths {
            p.translate(sx, sy);
        }
    }
    Ok(out)
}

/// Runs the final anchor-reduction pass over every traced path when
/// `cfg.simplify > 0`; a no-op (returned unchanged) otherwise. Corner
/// threshold matches the tracer's, so a corner kept at fit time is kept
/// here.
pub fn simplify_paths(
    mut colors: Vec<(String, Vec<TracedPath>)>,
    cfg: &Config,
) -> Vec<(String, Vec<TracedPath>)> {
    if cfg.simplify <= 0.0 {
        return colors;
    }
    let corner_threshold = cfg.corner_threshold();
    use rayon::prelude::*;
    colors.par_iter_mut().for_each(|(_, paths)| {
        for p in paths.iter_mut() {
            *p = crate::fit::simplify_closed(p, cfg.simplify, corner_threshold);
        }
    });
    colors
}

/// Converts document-space pin points into the scaled space of a crop at
/// `origin`, dropping pins outside it. `(w, h)` is the crop size in source
/// px. The +scale/2 lands each pin on its source pixel's center.
pub fn scale_pins(
    pins: &[[u32; 2]],
    origin: (u32, u32),
    scale: u32,
    (w, h): (u32, u32),
) -> Vec<(u32, u32)> {
    pins.iter()
        .filter_map(|p| {
            let x = p[0].checked_sub(origin.0).filter(|&x| x < w)?;
            let y = p[1].checked_sub(origin.1).filter(|&y| y < h)?;
            Some((x * scale + scale / 2, y * scale + scale / 2))
        })
        .collect()
}

/// Traces segmented regions into color-grouped filled paths, in bottom-first
/// paint order (outermost shape first), in the alpha mask's coordinate space.
/// Shared by run() and the GUI's cached stage pipeline. `pins` are
/// speckle-floor exemption points in the same scaled space as the regions.
pub fn trace_regions(
    regs: &[regions::Region],
    alpha: &GrayImage,
    cfg: &Config,
    doc_dim: u32,
    pins: &[(u32, u32)],
) -> Vec<(String, Vec<TracedPath>)> {
    trace_planned(&regions::merge_plan(regs, alpha, cfg, doc_dim, pins), alpha, cfg)
}

/// [`trace_regions`] from a prebuilt merge plan, so a caller that also needs
/// the region report or the debug contours runs the speckle merge and shape
/// build once.
pub fn trace_planned(
    plan: &regions::MergePlan,
    alpha: &GrayImage,
    cfg: &Config,
) -> Vec<(String, Vec<TracedPath>)> {
    // Shapes and traces run in parallel (nested rayon inside the per-layer
    // parallelism is fine: it's one shared thread pool). Assembly stays
    // sequential, so paint order and output are unchanged.
    use rayon::prelude::*;
    let shapes = surviving_shapes(plan, alpha, cfg);
    let traced: Vec<([u8; 3], Vec<TracedPath>)> = timing::TRACE.time(|| {
        shapes
            .par_iter()
            .map(|(color, mask, slack, (bx, by))| {
                let mut paths = trace::trace_mask(mask, cfg, slack.as_ref());
                // -1.0: the shape mask's origin sits one border pixel above
                // and left of the region bbox (see region_shape).
                for p in &mut paths {
                    p.translate(*bx as f64 - 1.0, *by as f64 - 1.0);
                }
                (*color, paths)
            })
            .collect()
    });
    group_traced(traced)
}

/// Groups per-shape traces, in paint order, into the color-hex runs the
/// output format wants: adjacent shapes of one color join a single entry,
/// empty traces drop out.
pub(crate) fn group_traced(
    traced: Vec<([u8; 3], Vec<TracedPath>)>,
) -> Vec<(String, Vec<TracedPath>)> {
    let mut out: Vec<(String, Vec<TracedPath>)> = Vec::new();
    for (c, mut paths) in traced {
        if paths.is_empty() {
            continue;
        }
        let hex = format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
        match out.last_mut() {
            Some((last_hex, last_paths)) if *last_hex == hex => last_paths.append(&mut paths),
            _ => out.push((hex, paths)),
        }
    }
    out
}

/// A smoothed boundary polyline and its corner points, in the alpha mask's
/// scaled coordinate space, for the debug view.
pub struct DebugContour {
    pub points: Vec<(f64, f64)>,
    pub corners: Vec<(f64, f64)>,
    /// Per-vertex seam-slack flag, aligned with `points`: a set flag marks a
    /// vertex fit at the slackened tolerance.
    pub slack: Vec<bool>,
}

/// The smoothed boundary and detected corners for every shape that
/// [`trace_regions`] would trace, in the same coordinate space as its paths.
/// Shows what the fit is about to run on.
pub fn debug_contours(
    regs: &[regions::Region],
    alpha: &GrayImage,
    cfg: &Config,
    doc_dim: u32,
    pins: &[(u32, u32)],
) -> Vec<DebugContour> {
    debug_planned(&regions::merge_plan(regs, alpha, cfg, doc_dim, pins), alpha, cfg)
}

/// [`debug_contours`] from a prebuilt merge plan.
pub fn debug_planned(
    plan: &regions::MergePlan,
    alpha: &GrayImage,
    cfg: &Config,
) -> Vec<DebugContour> {
    debug_from_shapes(&surviving_shapes(plan, alpha, cfg), cfg)
}

/// [`debug_contours`] from prebuilt shapes, so the GUI shares one shape build
/// between the contour view and the trace.
pub(crate) fn debug_from_shapes(shapes: &[Shape], cfg: &Config) -> Vec<DebugContour> {
    use rayon::prelude::*;
    shapes
        .par_iter()
        .flat_map(|(_, mask, slack, (bx, by))| {
            let (dx, dy) = (*bx as f64 - 1.0, *by as f64 - 1.0);
            trace::smoothed_contours(mask, cfg, slack.as_ref())
                .into_iter()
                .map(|(pts, corners, flags)| {
                    let shift = |&(x, y): &(f64, f64)| (x + dx, y + dy);
                    let corners = corners.iter().map(|&i| shift(&pts[i])).collect();
                    DebugContour {
                        points: pts.iter().map(shift).collect(),
                        corners,
                        slack: flags,
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A paintable shape in paint order: `(region color, shape mask, seam-slack
/// mask, mask bbox origin)`. The seam-slack mask is `None` when seam slack is
/// off.
pub(crate) type Shape = ([u8; 3], GrayImage, Option<GrayImage>, (u32, u32));

/// The shapes [`trace_planned`] traces, in paint order, for callers that fit
/// them shape by shape (the GUI's per-shape trace memo).
#[cfg(feature = "gui")]
pub(crate) fn planned_shapes(
    plan: &regions::MergePlan,
    alpha: &GrayImage,
    cfg: &Config,
) -> Vec<Shape> {
    surviving_shapes(plan, alpha, cfg)
}

/// The paintable shapes for the regions that clear the speckle floor (or
/// hold a pin), in paint order: `(region color, shape mask, seam-slack mask,
/// mask bbox origin)`. Shapes stack as a containment forest: each mask is the
/// union of its subtree in a seam-weighted spanning tree, so a parent's shape
/// covers its children and is painted before them. A pinned region is
/// exempt from the floor, since the pin marks a small feature (a tooth, a
/// glint) as deliberate.
///
/// The seam-slack mask, over the shape mask's grid, marks shape pixels
/// 4-adjacent to a neighbor region whose color is within `2 *
/// stroke_merge_dist` of the shape's own, so the fit can loosen there. It is
/// `None` when `seam_slack` is off.
fn surviving_shapes(plan: &regions::MergePlan, alpha: &GrayImage, cfg: &Config) -> Vec<Shape> {
    use rayon::prelude::*;
    let turdsize = plan.floor;
    let regs = &plan.regs;
    let areas = &plan.areas;
    let (w, h) = alpha.dimensions();

    let survivors: Vec<usize> = (0..regs.len()).filter(|&i| plan.survives[i]).collect();
    if survivors.is_empty() {
        return Vec::new();
    }

    timing::SHAPES.time(|| {
        let m = survivors.len();
        // Survivor id per pixel. Culled regions stay unlabeled and read as
        // background here; their pixels return via hole-filling below.
        let mut label = vec![u32::MAX; (w * h) as usize];
        for (si, &ri) in survivors.iter().enumerate() {
            let r = &regs[ri];
            for &(px, py) in &r.pixels {
                label[((r.y0 + py) * w + (r.x0 + px)) as usize] = si as u32;
            }
        }
        let araw = alpha.as_raw();
        // Dense-id scratch counter, as in regions::census(): adjacency order
        // is unobservable because the root pick and Prim's edge pick compare
        // with total-order tie-breaks.
        let mut adj: Vec<Vec<(u32, u64)>> = vec![Vec::new(); m];
        let mut seam_len = vec![0u64; m];
        let mut touched: Vec<u32> = Vec::new();
        let mut open_len = vec![0u64; m]; // frontier against transparency/canvas edge
        for (si, &ri) in survivors.iter().enumerate() {
            let r = &regs[ri];
            for &(px, py) in &r.pixels {
                let (x, y) = (r.x0 + px, r.y0 + py);
                for (nx, ny) in [
                    (x.wrapping_sub(1), y),
                    (x + 1, y),
                    (x, y.wrapping_sub(1)),
                    (x, y + 1),
                ] {
                    if nx >= w || ny >= h {
                        open_len[si] += 1;
                    } else {
                        let o = label[(ny * w + nx) as usize];
                        if o == u32::MAX {
                            if araw[(ny * w + nx) as usize] == 0 {
                                open_len[si] += 1;
                            }
                        } else if o != si as u32 {
                            if seam_len[o as usize] == 0 {
                                touched.push(o);
                            }
                            seam_len[o as usize] += 1;
                        }
                    }
                }
            }
            adj[si] = touched
                .drain(..)
                .map(|o| {
                    let e = (o, seam_len[o as usize]);
                    seam_len[o as usize] = 0;
                    e
                })
                .collect();
        }

        // A maximum spanning tree per connected component, edges weighted by
        // seam length, rooted at the longest transparency frontier. Each
        // shape is its subtree's union, so parents cover children: the
        // longest seams become tree edges and are fit once, by the child,
        // and a fit crack along any seam exposes an ancestor's color, never
        // the background. Not a single total-order stack: that re-traces a
        // seam once per shape between its two regions' paint positions, and
        // no total order keeps every neighbor pair adjacent.
        let mut comp_of = vec![u32::MAX; m];
        let mut comps: Vec<Vec<u32>> = Vec::new();
        for s in 0..m {
            if comp_of[s] != u32::MAX {
                continue;
            }
            let cid = comps.len() as u32;
            comp_of[s] = cid;
            let mut queue = vec![s as u32];
            let mut members = Vec::new();
            while let Some(i) = queue.pop() {
                members.push(i);
                for &(nb, _) in &adj[i as usize] {
                    if comp_of[nb as usize] == u32::MAX {
                        comp_of[nb as usize] = cid;
                        queue.push(nb);
                    }
                }
            }
            comps.push(members);
        }
        let mut parent_of = vec![u32::MAX; m];
        let mut in_tree = vec![false; m];
        let mut order: Vec<u32> = Vec::with_capacity(m); // paint order, parents first
        for comp in &comps {
            let root = *comp
                .iter()
                .max_by_key(|&&i| {
                    let i = i as usize;
                    (open_len[i], areas[survivors[i]], std::cmp::Reverse(i))
                })
                .unwrap();
            in_tree[root as usize] = true;
            order.push(root);
            // Prim's, maximizing: attach the heaviest tree-to-outside seam.
            for _ in 1..comp.len() {
                let mut best: Option<(u64, u32, u32)> = None; // (seam, child, parent)
                for &t in comp {
                    if !in_tree[t as usize] {
                        continue;
                    }
                    for &(nb, seam) in &adj[t as usize] {
                        if in_tree[nb as usize] {
                            continue;
                        }
                        let cand = (seam, nb, t);
                        if best.is_none_or(|(bs, bc, bp)| {
                            (cand.0, std::cmp::Reverse(cand.1), std::cmp::Reverse(cand.2))
                                > (bs, std::cmp::Reverse(bc), std::cmp::Reverse(bp))
                        }) {
                            best = Some(cand);
                        }
                    }
                }
                let Some((_, c, p)) = best else { break };
                in_tree[c as usize] = true;
                parent_of[c as usize] = p;
                order.push(c);
            }
        }

        // Every node's mask is the union of its whole subtree. Fold each
        // bbox into its parent children-first (order runs parents-first, so
        // its reverse finishes a child before its parent needs it), then
        // build the masks in parallel: each is independent given the tree.
        let mut children: Vec<Vec<u32>> = vec![Vec::new(); m];
        for (si, &p) in parent_of.iter().enumerate() {
            if p != u32::MAX {
                children[p as usize].push(si as u32);
            }
        }
        let mut bbox: Vec<(u32, u32, u32, u32)> = survivors
            .iter()
            .map(|&ri| {
                let r = &regs[ri];
                (r.x0, r.y0, r.x1, r.y1)
            })
            .collect();
        for &si in order.iter().rev() {
            let p = parent_of[si as usize];
            if p != u32::MAX {
                let (sb, pb) = (bbox[si as usize], bbox[p as usize]);
                bbox[p as usize] = (
                    pb.0.min(sb.0),
                    pb.1.min(sb.1),
                    pb.2.max(sb.2),
                    pb.3.max(sb.3),
                );
            }
        }
        // A neighbor is "low contrast" when its color sits within this OKLab
        // distance of the shape's own, piggybacking on the stroke-merge scale:
        // colors that close are already treated as interchangeable linework.
        let slack_thresh = 2.0 * cfg.stroke_merge_dist;
        let want_slack = cfg.seam_slack > 1.0 && slack_thresh > 0.0;
        order
            .par_iter()
            .map(|&si| {
                let si = si as usize;
                let (x0, y0, x1, y1) = bbox[si];
                let mask = if children[si].is_empty() {
                    // A leaf's shape is exactly its plan mask (its bbox never
                    // folds), so cloning it skips a mask build and hole fill.
                    plan.masks[survivors[si]].clone()
                } else {
                    let bw = x1 - x0 + 3;
                    let mut mask = GrayImage::new(bw, y1 - y0 + 3);
                    {
                        let mraw: &mut [u8] = &mut mask;
                        let mut stack = vec![si as u32];
                        while let Some(mi) = stack.pop() {
                            stack.extend_from_slice(&children[mi as usize]);
                            let r = &regs[survivors[mi as usize]];
                            for &(px, py) in &r.pixels {
                                mraw[((r.y0 + py - y0 + 1) * bw + (r.x0 + px - x0 + 1))
                                    as usize] = 255;
                            }
                        }
                    }
                    regions::fill_holes(&mut mask, (x0, y0), alpha, turdsize);
                    mask
                };
                let slack = want_slack.then(|| {
                    slack_mask(&mask, (x0, y0), regs[survivors[si]].color, &label, &survivors, regs, (w, h), slack_thresh)
                });
                (regs[survivors[si]].color, mask, slack, (x0, y0))
            })
            .collect()
    })
}

/// Builds a shape's seam-slack mask over its `mask` grid: a shape pixel is
/// set when a 4-neighbor lies outside the shape in a survivor region whose
/// color is within `thresh` (OKLab ΔE) of `own`. Transparency and
/// higher-contrast neighbors stay unset. `origin` is the bbox origin, so mask
/// pixel `(mx, my)` is scaled-space `(origin.0 - 1 + mx, origin.1 - 1 + my)`;
/// `label` maps a scaled-space pixel to its survivor id, or `u32::MAX`.
#[allow(clippy::too_many_arguments)]
fn slack_mask(
    mask: &GrayImage,
    origin: (u32, u32),
    own: [u8; 3],
    label: &[u32],
    survivors: &[usize],
    regs: &[regions::Region],
    (w, h): (u32, u32),
    thresh: f32,
) -> GrayImage {
    let (bw, bh) = mask.dimensions();
    let (ox, oy) = (origin.0 as i64 - 1, origin.1 as i64 - 1);
    let mut out = GrayImage::new(bw, bh);
    let mraw = mask.as_raw();
    let oraw: &mut [u8] = &mut out;
    let lc: Vec<bool> = survivors
        .iter()
        .map(|&ri| crate::config::color_dist(own, regs[ri].color) < thresh)
        .collect();
    let low_contrast = |o: u32| -> bool { o != u32::MAX && lc[o as usize] };
    for my in 0..bh {
        for mx in 0..bw {
            if mraw[(my * bw + mx) as usize] == 0 {
                continue;
            }
            let touches = [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)].into_iter().any(|(dx, dy)| {
                let (nmx, nmy) = (mx as i64 + dx, my as i64 + dy);
                // A neighbor still inside the shape is not a seam.
                if nmx >= 0 && nmy >= 0 && nmx < bw as i64 && nmy < bh as i64
                    && mraw[(nmy as u32 * bw + nmx as u32) as usize] != 0
                {
                    return false;
                }
                let (sx, sy) = (ox + nmx, oy + nmy);
                sx >= 0 && sy >= 0 && sx < w as i64 && sy < h as i64
                    && low_contrast(label[(sy as u32 * w + sx as u32) as usize])
            });
            if touches {
                oraw[(my * bw + mx) as usize] = 255;
            }
        }
    }
    out
}

/// Crops to the bounding box of pixels at or above the alpha threshold.
/// Returns the crop and its (x, y) offset in the source image, or `None` if
/// nothing is opaque.
pub fn crop_to_alpha(src: &RgbaImage, cfg: &Config) -> Option<(RgbaImage, u32, u32)> {
    let (w, h) = src.dimensions();
    let raw = src.as_raw();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    for y in 0..h {
        let row = &raw[(y as usize) * (w as usize) * 4..(y as usize + 1) * (w as usize) * 4];
        let mut first = None;
        let mut last = 0u32;
        for (x, px) in row.chunks_exact(4).enumerate() {
            if px[3] >= cfg.alpha_threshold {
                if first.is_none() {
                    first = Some(x as u32);
                }
                last = x as u32;
            }
        }
        if let Some(f) = first {
            x0 = x0.min(f);
            x1 = x1.max(last);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
    }
    if x0 > x1 {
        return None;
    }
    // Pad so the mode filter has room at the crop edges. It operates in
    // scaled space, hence the division by scale. The +1 keeps a transparent
    // ring around the silhouette.
    let pad = 1 + (cfg.mode_filter / 2).div_ceil(cfg.scale.max(1));
    let x0 = x0.saturating_sub(pad);
    let y0 = y0.saturating_sub(pad);
    let x1 = (x1 + pad).min(w - 1);
    let y1 = (y1 + pad).min(h - 1);
    let crop = image::imageops::crop_imm(src, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image();
    Some((crop, x0, y0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{palette, raster, regions};

    /// The GUI stage strip traces the whole uncropped layer while `run` (and
    /// the full render) traces the alpha crop and translates back. The memo
    /// shares one trace between them, so the two must produce identical
    /// geometry: integer-scale bilinear supersampling is shift-equivariant and
    /// the crop pad covers the filter's reach, so an interior art crop resolves
    /// to the same scaled pixels either way.
    #[test]
    fn uncropped_and_cropped_traces_are_identical() {
        // Art at (8..32) inside a 40x40 layer, so the crop bbox is a strict,
        // grid-aligned subset. Two colors give two regions.
        let mut img = RgbaImage::new(40, 40);
        for y in 8..32u32 {
            for x in 8..32u32 {
                let c = if x < 20 { [200, 50, 50, 255] } else { [50, 60, 200, 255] };
                img.put_pixel(x, y, image::Rgba(c));
            }
        }
        let cfg = Config { scale: 3, detail: 1.0, ..Default::default() };
        let doc_dim = 40;

        let cropped = run(&img, &cfg, doc_dim, (0, 0)).unwrap();

        let prep = raster::prepare(&img, &cfg);
        let pal = palette::extract_palette(&prep.flat, &prep.alpha, &cfg, doc_dim);
        let mut quant = palette::remap(&prep.flat, &prep.alpha, &pal);
        if cfg.color_cleanup > 0 {
            quant = palette::label_smooth(&quant, &prep.alpha, cfg.color_cleanup);
        }
        let pins = scale_pins(&cfg.pins, (0, 0), cfg.scale, img.dimensions());
        let regs = regions::segment_absorbed(&quant, &prep.alpha, &cfg);
        let uncropped = simplify_paths(trace_regions(&regs, &prep.alpha, &cfg, doc_dim, &pins), &cfg);

        assert_eq!(cropped.len(), uncropped.len());
        assert!(!cropped.is_empty());
        for ((h1, p1), (h2, p2)) in cropped.iter().zip(&uncropped) {
            assert_eq!(h1, h2);
            assert_eq!(p1.len(), p2.len());
            for (a, b) in p1.iter().zip(p2) {
                assert_eq!(a.start, b.start);
                assert_eq!(a.cubics, b.cubics);
            }
        }
    }

    #[test]
    fn pinned_regions_survive_the_speckle_floor() {
        // 16x16 dark field with a 2x2 white patch: area 4, far below the
        // turdsize this config produces (50).
        let dark = [10u8, 10, 10];
        let white = [255u8, 255, 255];
        let mut quant = image::RgbImage::from_pixel(16, 16, image::Rgb(dark));
        let alpha = GrayImage::from_pixel(16, 16, image::Luma([255]));
        for y in 7..9 {
            for x in 7..9 {
                quant.put_pixel(x, y, image::Rgb(white));
            }
        }
        let cfg = Config { scale: 1, detail: 10.0, ..Default::default() };
        let regs = regions::segment(&quant, &alpha);

        let hexes = |out: Vec<(String, Vec<TracedPath>)>| -> Vec<String> {
            out.into_iter().map(|(h, _)| h).collect()
        };
        let plain = hexes(trace_regions(&regs, &alpha, &cfg, 512, &[]));
        assert!(!plain.contains(&"#ffffff".to_string()), "{plain:?}");

        // A pin inside the patch keeps it; one outside does not.
        let pinned = hexes(trace_regions(&regs, &alpha, &cfg, 512, &[(7, 8)]));
        assert!(pinned.contains(&"#ffffff".to_string()), "{pinned:?}");
        let missed = hexes(trace_regions(&regs, &alpha, &cfg, 512, &[(2, 2)]));
        assert!(!missed.contains(&"#ffffff".to_string()), "{missed:?}");
    }

    #[test]
    fn shapes_stack_outside_in() {
        // 12x12, three vertical bands, light to dark left to right. Every
        // band touches the canvas edge (depth 0 for all), so luminance
        // orders the stack.
        let colors = [[240u8, 240, 240], [128, 128, 128], [16, 16, 16]];
        let quant = image::RgbImage::from_fn(12, 12, |x, _| image::Rgb(colors[(x / 4) as usize]));
        let alpha = GrayImage::from_pixel(12, 12, image::Luma([255]));
        let cfg = Config { scale: 1, detail: 5.0, ..Default::default() };
        let regs = regions::segment(&quant, &alpha);

        let plan = regions::merge_plan(&regs, &alpha, &cfg, 512, &[]);
        let shapes = surviving_shapes(&plan, &alpha, &cfg);
        let on = |mask: &GrayImage| mask.pixels().filter(|p| p[0] != 0).count();
        // Lightest first, each shape covering everything painted after it.
        assert_eq!(shapes.len(), 3);
        assert_eq!(shapes[0].0, colors[0]);
        assert_eq!(shapes[1].0, colors[1]);
        assert_eq!(shapes[2].0, colors[2]);
        assert_eq!(on(&shapes[0].1), 144);
        assert_eq!(on(&shapes[1].1), 96);
        assert_eq!(on(&shapes[2].1), 48);
    }

    #[test]
    fn seam_slack_never_adds_cubics_on_a_low_contrast_seam() {
        // A near-identical disk enclosed by a fill: the disk's whole boundary
        // is a low-contrast seam, so slackening the fit there can only drop
        // anchors, never add them. The fill's silhouette is unaffected.
        let fill = [96u8, 96, 96];
        let disk = [120u8, 120, 120];
        let thresh = 2.0 * Config::default().stroke_merge_dist;
        assert!(
            crate::config::color_dist(fill, disk) < thresh,
            "colors must read as a low-contrast seam"
        );
        let mut quant = image::RgbImage::from_pixel(32, 32, image::Rgb(fill));
        for y in 0..32i32 {
            for x in 0..32i32 {
                if (x - 16).pow(2) + (y - 16).pow(2) <= 49 {
                    quant.put_pixel(x as u32, y as u32, image::Rgb(disk));
                }
            }
        }
        let alpha = GrayImage::from_pixel(32, 32, image::Luma([255]));
        let regs = regions::segment(&quant, &alpha);

        let cubics = |slack: f64| -> usize {
            let cfg = Config { scale: 1, detail: 1.0, seam_slack: slack, ..Default::default() };
            trace_regions(&regs, &alpha, &cfg, 512, &[])
                .iter()
                .flat_map(|(_, ps)| ps.iter())
                .map(|p| p.cubics.len())
                .sum()
        };
        let base = cubics(1.0);
        let slack = cubics(3.0);
        assert!(base > 0);
        assert!(slack <= base, "slack {slack} > base {base}");
    }

    #[test]
    fn scale_pins_maps_document_points_into_the_crop() {
        let pins = [[10u32, 20], [3, 3], [100, 100]];
        // Crop at (8, 16), 8x8 source px, scale 3: only the first pin lands
        // inside; it maps to the center of its scaled source pixel.
        let scaled = scale_pins(&pins, (8, 16), 3, (8, 8));
        assert_eq!(scaled, vec![(2 * 3 + 1, 4 * 3 + 1)]);
    }
}
