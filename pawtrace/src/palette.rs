//! Region-first palette selection (NOT k-means, NOT histogram). Flat sticker
//! art authors color as spatial features: fills, stripes, highlights. Feature
//! detection over the 1x source crop finds those directly; a color-space-only
//! method cannot separate a deliberate low-contrast feature from anti-alias
//! fringe, since both live at the same OKLab distance. Selection is greedy by
//! feature salience with an OKLab dedup floor.

use image::{RgbImage, GrayImage, RgbaImage};
use std::collections::{HashMap, HashSet};
use crate::config::{srgb_to_oklab, Config};

/// One color-uniform connected component of the 1x source crop, the evidence
/// unit of region-first palette selection.
#[derive(Debug, Clone)]
pub struct Feature {
    /// Mean member color, sRGB.
    pub mean: [u8; 3],
    /// Member pixel count, source px.
    pub area: u32,
    /// Bbox in source-crop px, inclusive: (x0, y0, x1, y1).
    pub bbox: (u32, u32, u32, u32),
}

/// Union-find accumulator for one growing component.
struct Comp {
    parent: u32,
    count: u32,
    // f64: a component can span millions of pixels, and f32 stops absorbing
    // per-pixel increments around 4e6.
    sum_lab: [f64; 3],
    sum_rgb: [u64; 3],
    bbox: (u32, u32, u32, u32),
}

impl Comp {
    fn mean_lab(&self) -> [f32; 3] {
        let n = self.count as f64;
        [
            (self.sum_lab[0] / n) as f32,
            (self.sum_lab[1] / n) as f32,
            (self.sum_lab[2] / n) as f32,
        ]
    }

    fn add(&mut self, lab: [f32; 3], rgb: [u8; 3], x: u32, y: u32) {
        self.count += 1;
        for (s, v) in self.sum_lab.iter_mut().zip(lab) {
            *s += v as f64;
        }
        for (s, v) in self.sum_rgb.iter_mut().zip(rgb) {
            *s += v as u64;
        }
        self.bbox.0 = self.bbox.0.min(x);
        self.bbox.1 = self.bbox.1.min(y);
        self.bbox.2 = self.bbox.2.max(x);
        self.bbox.3 = self.bbox.3.max(y);
    }
}

fn comp_find(comps: &mut [Comp], mut id: u32) -> u32 {
    while comps[id as usize].parent != id {
        let gp = comps[comps[id as usize].parent as usize].parent;
        comps[id as usize].parent = gp;
        id = gp;
    }
    id
}

/// Feature index per pixel of the 1x source crop, [`u32::MAX`] where the
/// pixel is below the alpha threshold. Indexes into the [`Feature`] slice
/// [`detect_features`] returns alongside it, so callers can read which
/// feature owns a pixel and which features share a boundary.
#[derive(Debug, Clone)]
pub struct FeatureLabels {
    pub w: u32,
    pub h: u32,
    pub at: Vec<u32>,
}

/// Growing cap for detection (OKLab ΔE): a pixel joins a component only while
/// the component's mean color stays within this of it. Fine on purpose, so a
/// smooth ramp over-segments into thin bands that feature-level merging then
/// judges band by band. A coarse cap lets a component's mean drift through the
/// anti-alias blend far enough to swallow an adjacent soft highlight (measured
/// ΔE ~0.037 from base fur), which no later stage can then recover.
const DETECT_TOL: f32 = 0.015;

/// Weight of the smoothness term relative to the data term in the RTV system.
/// Larger flattens more.
const RTV_LAMBDA: f64 = 0.015;
/// Gaussian spatial scale (source px) of the windowed variation measures.
const RTV_SIGMA: f64 = 3.0;
/// Floor on the windowed inherent variation `L`, in normalized [0,1] channel
/// units. Sets the coherence a faint edge needs before RTV counts it structure
/// rather than texture.
const RTV_EPS_S: f64 = 0.02;
/// Floor on the per-pixel gradient magnitude in the reweighting, in normalized
/// [0,1] channel units, so a zero-gradient pixel yields a finite weight.
const RTV_EPS: f64 = 1e-3;
/// Outer iteratively-reweighted-least-squares passes.
const RTV_ITERS: usize = 4;
/// Conjugate-gradient iteration cap per outer pass.
const RTV_CG_ITERS: usize = 60;
/// Conjugate-gradient stop: residual norm fallen to this fraction of its start.
const RTV_CG_REL_TOL: f64 = 1e-3;

/// Structure-preserving smoothing by Relative Total Variation (Xu, Yan, Xia,
/// Jia, SIGGRAPH Asia 2012), the growing guide [`detect_features`] smooths with
/// before growing components. Runs per RGB channel over `src`'s opaque pixels
/// ([`Config::alpha_threshold`]); below-threshold pixels are copied through
/// untouched and never couple across the silhouette. RTV separates structure
/// from texture by local anisotropy, keeping a faint but coherent edge while
/// flattening incoherent texture at the same amplitude.
pub fn rtv_smooth(src: &RgbaImage, cfg: &Config) -> RgbaImage {
    use rayon::prelude::*;
    let (w, h) = src.dimensions();
    let (wu, hu) = (w as usize, h as usize);
    let n = wu * hu;
    let raw = src.as_raw();
    let opaque: Vec<bool> = (0..n).map(|i| raw[i * 4 + 3] >= cfg.alpha_threshold).collect();
    // Cover depends only on the silhouette, so the three channels and four outer
    // passes share this one windowed opaque count instead of reblurring it each.
    let maskf: Vec<f64> = opaque.iter().map(|&o| if o { 1.0 } else { 0.0 }).collect();
    let cover = gauss_blur(&maskf, wu, hu);
    let channels: Vec<Vec<f64>> = (0..3usize)
        .into_par_iter()
        .map(|c| {
            let s0: Vec<f64> = (0..n).map(|i| raw[i * 4 + c] as f64 / 255.0).collect();
            let mut s = s0.clone();
            for _ in 0..RTV_ITERS {
                let (wx, wy) = rtv_weights(&s, &opaque, &cover, wu, hu);
                rtv_cg_solve(&mut s, &s0, &wx, &wy, wu, hu);
            }
            s
        })
        .collect();
    let mut out = src.clone();
    let orw: &mut [u8] = &mut out;
    for (c, s) in channels.iter().enumerate() {
        for (i, &o) in opaque.iter().enumerate() {
            if o {
                orw[i * 4 + c] = (s[i].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }
    out
}

/// Per-edge RTV weights for the current channel estimate `s`: `wx[i]` on the
/// edge from pixel `i` to `i+1`, `wy[i]` on the edge to `i+w`, both zero unless
/// that edge joins two opaque pixels. Higher weight preserves the edge.
fn rtv_weights(s: &[f64], opaque: &[bool], cover: &[f64], w: usize, h: usize) -> (Vec<f64>, Vec<f64>) {
    let n = w * h;
    // Signed forward differences, zero where the edge would cross the
    // silhouette or the image border so the windowed measures below count only
    // real intra-art gradients.
    let mut gx = vec![0.0f64; n];
    let mut gy = vec![0.0f64; n];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if !opaque[i] {
                continue;
            }
            if x + 1 < w && opaque[i + 1] {
                gx[i] = s[i + 1] - s[i];
            }
            if y + 1 < h && opaque[i + w] {
                gy[i] = s[i + w] - s[i];
            }
        }
    }
    // `cover` is the windowed opaque count. Dividing the blurred signals by it
    // makes each a mean over just the opaque window, so a boundary window is
    // not diluted by the transparent surround.
    let agx: Vec<f64> = gx.iter().map(|v| v.abs()).collect();
    let agy: Vec<f64> = gy.iter().map(|v| v.abs()).collect();
    let d_x = mean_over(&gauss_blur(&agx, w, h), cover);
    let d_y = mean_over(&gauss_blur(&agy, w, h), cover);
    let l_x = mean_over(&gauss_blur(&gx, w, h), cover);
    let l_y = mean_over(&gauss_blur(&gy, w, h), cover);
    let mut wx = vec![0.0f64; n];
    let mut wy = vec![0.0f64; n];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if x + 1 < w && opaque[i] && opaque[i + 1] {
                wx[i] = rtv_weight(d_x[i], l_x[i], gx[i]);
            }
            if y + 1 < h && opaque[i] && opaque[i + w] {
                wy[i] = rtv_weight(d_y[i], l_y[i], gy[i]);
            }
        }
    }
    (wx, wy)
}

/// The RTV penalty weight `(D/(|L|+εs)) / (|∂S|+ε)` on this edge's gradient in
/// the smoothness term. Texture makes `D≫L`, so the weight is large and the
/// gradient is penalized into flatness. A coherent edge has `D≈L`, so the weight
/// is small and the edge survives.
fn rtv_weight(d: f64, l: f64, g: f64) -> f64 {
    (d / (l.abs() + RTV_EPS_S)) / (g.abs() + RTV_EPS)
}

/// Elementwise `num / den`, zero where `den` is negligible.
fn mean_over(num: &[f64], den: &[f64]) -> Vec<f64> {
    num.iter()
        .zip(den)
        .map(|(&a, &b)| if b > 1e-12 { a / b } else { 0.0 })
        .collect()
}

/// Separable Gaussian blur of `v` (`w`x`h`, row-major) at scale [`RTV_SIGMA`],
/// with taps beyond the border contributing zero.
fn gauss_blur(v: &[f64], w: usize, h: usize) -> Vec<f64> {
    use rayon::prelude::*;
    let r = (2.0 * RTV_SIGMA).ceil() as isize;
    let two_s2 = 2.0 * RTV_SIGMA * RTV_SIGMA;
    let kernel: Vec<f64> =
        (-r..=r).map(|t| (-(t * t) as f64 / two_s2).exp()).collect();
    let ksum: f64 = kernel.iter().sum();
    let kernel: Vec<f64> = kernel.iter().map(|k| k / ksum).collect();
    let mut tmp = vec![0.0f64; v.len()];
    tmp.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, out) in row.iter_mut().enumerate() {
            let mut acc = 0.0;
            for (t, &k) in (-r..=r).zip(&kernel) {
                let xx = x as isize + t;
                if xx >= 0 && (xx as usize) < w {
                    acc += k * v[y * w + xx as usize];
                }
            }
            *out = acc;
        }
    });
    let mut out = vec![0.0f64; v.len()];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, out) in row.iter_mut().enumerate() {
            let mut acc = 0.0;
            for (t, &k) in (-r..=r).zip(&kernel) {
                let yy = y as isize + t;
                if yy >= 0 && (yy as usize) < h {
                    acc += k * tmp[yy as usize * w + x];
                }
            }
            *out = acc;
        }
    });
    out
}

/// Solves `(I + λ(∂xᵀWx∂x + ∂yᵀWy∂y)) s = s0` in place by conjugate gradient,
/// warm-started from the incoming `s`. The system matrix is symmetric positive
/// definite (identity plus a weighted graph Laplacian), so CG converges.
fn rtv_cg_solve(s: &mut [f64], s0: &[f64], wx: &[f64], wy: &[f64], w: usize, h: usize) {
    let dot = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
    let mut ap = vec![0.0f64; s.len()];
    rtv_matvec(s, wx, wy, w, h, &mut ap);
    let mut r: Vec<f64> = s0.iter().zip(&ap).map(|(b, a)| b - a).collect();
    let mut p = r.clone();
    let mut rs = dot(&r, &r);
    let stop = RTV_CG_REL_TOL * RTV_CG_REL_TOL * rs;
    if rs <= stop {
        return;
    }
    for _ in 0..RTV_CG_ITERS {
        rtv_matvec(&p, wx, wy, w, h, &mut ap);
        let alpha = rs / dot(&p, &ap);
        for i in 0..s.len() {
            s[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        let rs_new = dot(&r, &r);
        if rs_new <= stop {
            break;
        }
        let beta = rs_new / rs;
        for i in 0..p.len() {
            p[i] = r[i] + beta * p[i];
        }
        rs = rs_new;
    }
}

/// `(I + λ·Lw) x`, where `Lw` is the weighted graph Laplacian of the opaque
/// pixel grid with edge weights `wx`/`wy` (zero on absent edges).
fn rtv_matvec(x: &[f64], wx: &[f64], wy: &[f64], w: usize, h: usize, out: &mut [f64]) {
    out.copy_from_slice(x);
    for y in 0..h {
        for xx in 0..w {
            let i = y * w + xx;
            if xx + 1 < w && wx[i] != 0.0 {
                let d = RTV_LAMBDA * wx[i] * (x[i] - x[i + 1]);
                out[i] += d;
                out[i + 1] -= d;
            }
            if y + 1 < h && wy[i] != 0.0 {
                let d = RTV_LAMBDA * wy[i] * (x[i] - x[i + w]);
                out[i] += d;
                out[i + w] -= d;
            }
        }
    }
}

/// The [`detect_features`] output: the detected features and the label raster
/// pinning each opaque pixel to its feature, the input pair the palette build
/// consumes.
pub type Detection = (Vec<Feature>, FeatureLabels);

/// Color-uniform connected components (4-connectivity) over the opaque pixels
/// of the 1x source crop `src`, grown under [`DETECT_TOL`], with the
/// feature-index label raster that records which feature owns each pixel.
/// Components come out in first-encounter scan order. Growing runs on an
/// RTV-smoothed copy of `src`, so compression and anti-alias fringe does not
/// spawn a speckle feature per artifact pixel; each feature's mean color is
/// taken from the original `src` pixels so authored fills stay exact, and
/// callers downstream of the palette keep operating on the original pixels.
pub fn detect_features(src: &RgbaImage, cfg: &Config) -> Detection {
    let smooth = rtv_smooth(src, cfg);
    grow_features(src, &smooth, cfg)
}

/// Grows the color-uniform components of [`detect_features`] over `src`'s
/// opaque pixels, taking `smooth` (same dimensions as `src`) as the guide whose
/// OKLab drives growing and each component's mean cap, while every feature's
/// reported color is read from the real `src` pixels. [`detect_features`] passes
/// the [`rtv_smooth`] output.
fn grow_features(src: &RgbaImage, smooth: &RgbaImage, cfg: &Config) -> (Vec<Feature>, FeatureLabels) {
    // Squared distances: the tolerance tests run up to three times per pixel,
    // and squaring the threshold once saves the sqrt in each.
    let tol2 = DETECT_TOL * DETECT_TOL;
    let dist2 = |a: [f32; 3], b: [f32; 3]| {
        let (d0, d1, d2) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
        d0 * d0 + d1 * d1 + d2 * d2
    };
    let (w, h) = src.dimensions();
    let (wu, hu) = (w as usize, h as usize);
    let raw = src.as_raw();
    let sraw = smooth.as_raw();
    let mut label: Vec<u32> = vec![u32::MAX; wu * hu];
    let mut comps: Vec<Comp> = Vec::new();
    for y in 0..hu {
        for x in 0..wu {
            let i = y * wu + x;
            if raw[i * 4 + 3] < cfg.alpha_threshold {
                continue;
            }
            // rgb is the real source pixel that fixes the feature's reported
            // color; lab comes from the smoothed pixel so growing and the mean
            // it caps against ignore edge speckle.
            let rgb = [raw[i * 4], raw[i * 4 + 1], raw[i * 4 + 2]];
            let lab = srgb_to_oklab([sraw[i * 4], sraw[i * 4 + 1], sraw[i * 4 + 2]]);
            let mut joined: Option<u32> = None;
            for ni in [x.checked_sub(1).map(|x| y * wu + x), y.checked_sub(1).map(|y| y * wu + x)]
                .into_iter()
                .flatten()
            {
                let nl = label[ni];
                if nl == u32::MAX {
                    continue;
                }
                let root = comp_find(&mut comps, nl);
                match joined {
                    None => {
                        // The cap tests the component mean against the pixel,
                        // not neighbor against neighbor: pairwise linkage
                        // would chain a smooth gradient dark to light, while
                        // the mean drifts out of tolerance after ~2 tolerances
                        // of gradient and cuts a new band.
                        if dist2(comps[root as usize].mean_lab(), lab) <= tol2 {
                            comps[root as usize].add(lab, rgb, x as u32, y as u32);
                            label[i] = root;
                            joined = Some(root);
                        }
                    }
                    Some(j) if root != j => {
                        // Both components accepted this pixel, but they only
                        // fuse when their means also agree: one boundary pixel
                        // must not bridge two adjacent gradient bands.
                        if dist2(comps[root as usize].mean_lab(), lab) <= tol2
                            && dist2(
                                comps[root as usize].mean_lab(),
                                comps[j as usize].mean_lab(),
                            ) <= tol2
                        {
                            // The smaller id stays root, keeping component
                            // order first-encounter.
                            let (lo, hi) = (j.min(root), j.max(root));
                            comps[hi as usize].parent = lo;
                            let (count, sum_lab, sum_rgb, bbox) = {
                                let c = &comps[hi as usize];
                                (c.count, c.sum_lab, c.sum_rgb, c.bbox)
                            };
                            let t = &mut comps[lo as usize];
                            t.count += count;
                            for (s, v) in t.sum_lab.iter_mut().zip(sum_lab) {
                                *s += v;
                            }
                            for (s, v) in t.sum_rgb.iter_mut().zip(sum_rgb) {
                                *s += v;
                            }
                            t.bbox.0 = t.bbox.0.min(bbox.0);
                            t.bbox.1 = t.bbox.1.min(bbox.1);
                            t.bbox.2 = t.bbox.2.max(bbox.2);
                            t.bbox.3 = t.bbox.3.max(bbox.3);
                            joined = Some(lo);
                        }
                    }
                    Some(_) => {}
                }
            }
            if joined.is_none() {
                let id = comps.len() as u32;
                let (x, y) = (x as u32, y as u32);
                comps.push(Comp {
                    parent: id,
                    count: 1,
                    sum_lab: [lab[0] as f64, lab[1] as f64, lab[2] as f64],
                    sum_rgb: [rgb[0] as u64, rgb[1] as u64, rgb[2] as u64],
                    bbox: (x, y, x, y),
                });
                label[i] = id;
            }
        }
    }
    let mut root_feat = vec![u32::MAX; comps.len()];
    let mut features = Vec::new();
    for id in 0..comps.len() as u32 {
        if comps[id as usize].parent != id {
            continue;
        }
        root_feat[id as usize] = features.len() as u32;
        let c = &comps[id as usize];
        let n = c.count as u64;
        features.push(Feature {
            mean: [
                (c.sum_rgb[0] / n) as u8,
                (c.sum_rgb[1] / n) as u8,
                (c.sum_rgb[2] / n) as u8,
            ],
            area: c.count,
            bbox: c.bbox,
        });
    }
    let mut at = vec![u32::MAX; wu * hu];
    for (i, slot) in at.iter_mut().enumerate() {
        if label[i] != u32::MAX {
            let root = comp_find(&mut comps, label[i]);
            *slot = root_feat[root as usize];
        }
    }
    (features, FeatureLabels { w, h, at })
}

/// OKLab distance under which feature means count as one authored color.
/// Independent detections of the same fill land a few thousandths apart
/// (mean jitter from the AA edge pixels each component absorbs); 0.015
/// covers that and stays under the closest deliberate pair measured on the
/// goldens (soft fur highlight vs base, 0.037).
const FEATURE_DEDUP: f32 = 0.015;

/// A group's aggregate area must reach this many detail areas to earn a
/// palette slot when no single member does.
const AGGREGATE_EVIDENCE: f32 = 4.0;

/// Features of one authored color, aggregated as a palette candidate.
#[derive(Debug, Clone)]
pub struct FeatureGroup {
    /// Group color: the mean of its largest member, sRGB.
    pub color: [u8; 3],
    /// Largest member area, source px.
    pub largest: u32,
    /// Total member area, source px.
    pub aggregate: u64,
}

/// Groups features whose means sit within [`FEATURE_DEDUP`] of each other,
/// so a color drawn as many features (spots, stripes) pools its evidence.
/// Groups come out in salience order: largest member desc, then aggregate
/// desc, then color.
pub fn group_features(features: &[Feature], cfg: &Config, dim: u32) -> Vec<FeatureGroup> {
    let scale2 = (cfg.scale * cfg.scale).max(1) as f32;
    let detail_src = cfg.detail_area_scaled(dim) / scale2;
    // Slivers of a few px are resample fringe. They cannot found a palette
    // color, and pooling tens of thousands of them would both fabricate
    // aggregate evidence for blend colors and blow up the O(F*G) grouping.
    let prune = (detail_src / 8.0).max(3.0);
    let mut feats: Vec<&Feature> = features.iter().filter(|f| f.area as f32 >= prune).collect();
    feats.sort_unstable_by_key(|f| (std::cmp::Reverse(f.area), f.mean));
    let mut groups: Vec<FeatureGroup> = Vec::new();
    let mut group_lab: Vec<[f32; 3]> = Vec::new();
    for f in feats {
        let l = srgb_to_oklab(f.mean);
        // The founding (largest) member fixes the group color and its lab
        // anchor: an exact fill stays exact instead of drifting with every
        // joining stripe.
        match group_lab.iter().position(|&g| lab_dist(g, l) <= FEATURE_DEDUP) {
            Some(gi) => groups[gi].aggregate += f.area as u64,
            None => {
                groups.push(FeatureGroup {
                    color: f.mean,
                    largest: f.area,
                    aggregate: f.area as u64,
                });
                group_lab.push(l);
            }
        }
    }
    groups.sort_unstable_by_key(|g| {
        (std::cmp::Reverse(g.largest), std::cmp::Reverse(g.aggregate), g.color)
    });
    groups
}

/// Palette slots from grouped features: `cfg.locked` first, unconditionally,
/// then groups in salience order until `cfg.max_colors`. A group earns a
/// slot when its largest member covers a detail area (source px) or its
/// aggregate covers [`AGGREGATE_EVIDENCE`] of them; kept colors dedup at
/// [`FEATURE_DEDUP`], with no merge radius beyond it.
pub fn select_features(groups: &[FeatureGroup], cfg: &Config, dim: u32) -> Vec<[u8; 3]> {
    let scale2 = (cfg.scale * cfg.scale).max(1) as f32;
    let detail_src = cfg.detail_area_scaled(dim) / scale2;
    let mut palette: Vec<[u8; 3]> = Vec::new();
    for &c in &cfg.locked {
        if !palette.contains(&c) {
            palette.push(c);
        }
    }
    let mut kept: Vec<[f32; 3]> = palette.iter().map(|&c| srgb_to_oklab(c)).collect();
    for g in groups {
        if palette.len() >= cfg.max_colors {
            break;
        }
        if (g.largest as f32) < detail_src
            && (g.aggregate as f32) < AGGREGATE_EVIDENCE * detail_src
        {
            continue;
        }
        let l = srgb_to_oklab(g.color);
        if kept.iter().any(|&k| lab_dist(l, k) < FEATURE_DEDUP) {
            continue;
        }
        palette.push(g.color);
        kept.push(l);
    }
    // A layer smaller than the detail floor still needs one color to remap
    // to, as the histogram path guarantees with its top entry.
    if palette.is_empty() {
        if let Some(g) = groups.first() {
            palette.push(g.color);
        }
    }
    palette
}

/// Outcome of [`merge_features`]: the post-merge feature set, the count of
/// gradient regions that were consolidated, and the feature-index raster
/// remapped onto that post-merge set.
pub struct MergeResult {
    pub features: Vec<Feature>,
    pub families: usize,
    /// Per-pixel index into [`Self::features`], [`u32::MAX`] outside the alpha.
    /// The label raster passed to [`merge_features`] with every pixel moved
    /// onto the merged feature it now belongs to.
    pub labels: FeatureLabels,
}

/// Merges the detected `features` at the feature level, the v2 refinement over
/// per-pixel absorption. `labels` is the raster [`detect_features`] returned
/// for these `features`.
///
/// A feature is *salient* when its mean sits brighter or darker than every
/// spatial neighbor by [`EXTREMUM_MARGIN`]: a highlight, a specular dot, a
/// stripe, a spot, or a distinctly shaded part is a local lightness extremum,
/// where a gradient band has a brighter neighbor up-ramp and a darker one
/// down-ramp. Salient features pass through untouched and always reach
/// selection. Everything else is smooth-ramp or flat-fill material; each
/// connected run of it is one region, quantized to at most `cfg.gradient_bands`
/// representative colors by area-weighted error, with every other feature
/// remapped to its nearest kept representative.
///
/// The extremum test separates distinct features from ramp material, and the
/// band budget is the consolidation lever, so no proximity radius is used.
pub fn merge_features(features: &[Feature], labels: &FeatureLabels, cfg: &Config) -> MergeResult {
    let n = features.len();
    if n == 0 {
        let empty = FeatureLabels { w: labels.w, h: labels.h, at: labels.at.clone() };
        return MergeResult { features: Vec::new(), families: 0, labels: empty };
    }
    // remap[old feature index] = its index in `out` after consolidation.
    let mut remap = vec![u32::MAX; n];
    let lab: Vec<[f32; 3]> = features.iter().map(|f| srgb_to_oklab(f.mean)).collect();
    let edges = boundary_edges(labels);
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); n];
    for &(a, b) in &edges {
        adj[a as usize].push(b);
        adj[b as usize].push(a);
    }

    let salient: Vec<bool> = (0..n)
        .map(|i| {
            let li = lab[i][0];
            let ns = &adj[i];
            if ns.is_empty() {
                return true;
            }
            let hi = ns.iter().map(|&j| lab[j as usize][0]).fold(f32::MIN, f32::max);
            let lo = ns.iter().map(|&j| lab[j as usize][0]).fold(f32::MAX, f32::min);
            li - hi > EXTREMUM_MARGIN || lo - li > EXTREMUM_MARGIN
        })
        .collect();

    // Connected runs of non-salient features: with the distinct features pulled
    // out, what stays adjacent is one smooth ramp or fill, so a plain component
    // is the gradient the budget applies to (no line fitting, no chaining).
    let mut parent: Vec<u32> = (0..n as u32).collect();
    for &(a, b) in &edges {
        if !salient[a as usize] && !salient[b as usize] {
            let (ra, rb) = (uf_find(&mut parent, a), uf_find(&mut parent, b));
            if ra != rb {
                parent[ra.max(rb) as usize] = ra.min(rb);
            }
        }
    }
    // Regions in ascending-root order, not HashMap iteration order: `out` and
    // its label raster carry that order, and a random one leaves the label
    // raster nondeterministic run to run. Selection sorts its input, so the
    // palette is unaffected either way.
    let mut region_at: HashMap<u32, usize> = HashMap::new();
    let mut regions: Vec<Vec<u32>> = Vec::new();
    for i in 0..n as u32 {
        if !salient[i as usize] {
            let root = uf_find(&mut parent, i);
            let idx = *region_at.entry(root).or_insert_with(|| {
                regions.push(Vec::new());
                regions.len() - 1
            });
            regions[idx].push(i);
        }
    }

    let bands = cfg.gradient_bands.max(2) as usize;
    let mut out: Vec<Feature> = Vec::new();
    let mut consolidated = 0;
    for members in &regions {
        let reps = family_reps(members, features, &lab, bands);
        if reps.len() < members.len() {
            consolidated += 1;
        }
        // A representative founds one output feature keeping its own color;
        // every other member folds its area and bbox into the nearest.
        let mut slot: HashMap<u32, u32> = HashMap::new();
        for &r in &reps {
            slot.insert(r, out.len() as u32);
            out.push(features[r as usize].clone());
        }
        for &m in members {
            let r = nearest_rep(m, &reps, &lab);
            remap[m as usize] = slot[&r];
            if m != r {
                absorb(&mut out[slot[&r] as usize], &features[m as usize]);
            }
        }
    }
    for (i, &s) in salient.iter().enumerate() {
        if s {
            remap[i] = out.len() as u32;
            out.push(features[i].clone());
        }
    }
    let at: Vec<u32> = labels
        .at
        .iter()
        .map(|&l| if l == u32::MAX { u32::MAX } else { remap[l as usize] })
        .collect();
    let labels = FeatureLabels { w: labels.w, h: labels.h, at };
    MergeResult { features: out, families: consolidated, labels }
}

/// Full 4-connected outline length of each feature in `labels`, in edge units:
/// every side of a feature pixel facing a different feature, the background, or
/// the image border counts one. Indexed by feature; `n` is the feature count.
///
/// This is the whole geometric perimeter, not just the shared-with-a-neighbor
/// part, so a thin ring hugging the silhouette counts both its outer (against
/// background) and inner (against the fill) edges. That is what makes
/// `area / (0.5 * perimeter)` read its mean thickness rather than double it.
fn feature_perimeters(labels: &FeatureLabels, n: usize) -> Vec<u32> {
    let (w, h) = (labels.w as usize, labels.h as usize);
    let mut per = vec![0u32; n];
    for y in 0..h {
        for x in 0..w {
            let a = labels.at[y * w + x];
            if a == u32::MAX {
                continue;
            }
            let ai = a as usize;
            let mut face = |nb: Option<u32>| {
                if nb != Some(a) {
                    per[ai] += 1;
                }
            };
            face(if x > 0 { Some(labels.at[y * w + x - 1]) } else { None });
            face(if x + 1 < w { Some(labels.at[y * w + x + 1]) } else { None });
            face(if y > 0 { Some(labels.at[(y - 1) * w + x]) } else { None });
            face(if y + 1 < h { Some(labels.at[(y + 1) * w + x]) } else { None });
        }
    }
    per
}

/// Mean-thickness ceiling for [`merge_indistinct`], in source px. A feature
/// thinner than this on average is a silhouette ring or transition sliver, so
/// it folds into its neighbor.
const INDISTINCT_THIN: f32 = 1.2;

/// Compact-size floor for [`merge_indistinct`]: the bbox diagonal in source px
/// below which a feature is a speck and folds into its neighbor.
const INDISTINCT_SPECK: f32 = 6.0;

/// Near-duplicate color ceiling for [`merge_indistinct`], OKLab ΔE. A feature
/// within this of its most-similar neighbor is an imperceptible color step; the
/// closest deliberate detail measured on the goldens (a fur-depth highlight at
/// ΔE ~0.037) stays well clear.
const INDISTINCT_COLOR_JND: f32 = 0.02;

/// Absorbs visually indistinct features left by [`merge_features`] into an
/// adjacent neighbor. A feature is absorbed when it is imperceptible at
/// phone-viewing scale by ANY of three signals (OR-combined):
///
/// - Too thin: mean thickness `area / (0.5 * perimeter)` at most `thin_max`
///   (source px). For a `t`-by-`L` strip this is `tL/(t+L)`, which is `t` for
///   `L >> t`, so it reads the width of a long thin sliver and of the ring
///   wrapping the whole silhouette by thickness rather than length, catching
///   the near-black silhouette ring and the 1px transition slivers a
///   bbox-diagonal measure passes.
/// - Too small: bbox diagonal below `speck_floor` (source px), a compact speck.
/// - Near-duplicate color: boundary OKLab ΔE to its most-similar adjacent
///   neighbor below `color_jnd`. Well under the deliberate-detail band (fur
///   stripes at ΔE ~0.06, the cheek fur-depth highlight ~0.037), so only true
///   near-duplicates fold and the thin and small axes carry the bulk.
///
/// A feature absorbs into its most-similar adjacent neighbor by boundary ΔE,
/// so a ring of near-black fragments collapses into ONE near-black region
/// rather than into the brighter fill it also borders. A component with no
/// surviving member takes its largest member's color. A feature with no opaque
/// neighbor is the layer's sole feature and is kept. `labels` is the raster
/// [`merge_features`] returned for these `features`. [`MergeResult::families`]
/// counts the survivors that absorbed at least one feature.
pub fn merge_indistinct(
    features: &[Feature],
    labels: &FeatureLabels,
    thin_max: f32,
    speck_floor: f32,
    color_jnd: f32,
) -> MergeResult {
    let n = features.len();
    if n == 0 {
        let empty = FeatureLabels { w: labels.w, h: labels.h, at: labels.at.clone() };
        return MergeResult { features: Vec::new(), families: 0, labels: empty };
    }
    let lab: Vec<[f32; 3]> = features.iter().map(|f| srgb_to_oklab(f.mean)).collect();
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (a, b) in boundary_edges(labels) {
        adj[a as usize].push(b);
        adj[b as usize].push(a);
    }
    let per = feature_perimeters(labels, n);

    // nearest[i] is the most-similar neighbor by mean-color ΔE, the feature i
    // absorbs toward. The union chain below carries it to a survivor.
    let mut keep = vec![false; n];
    let mut nearest = vec![u32::MAX; n];
    for i in 0..n {
        let ns = &adj[i];
        if ns.is_empty() {
            keep[i] = true;
            continue;
        }
        let (mut best, mut tgt) = (f32::MAX, u32::MAX);
        for &j in ns {
            let d = lab_dist(lab[i], lab[j as usize]);
            // Tie-break on the lower feature index: adjacency comes from a
            // hashed edge set, so a bare `<` would resolve equal-distance
            // neighbors by iteration order and desync the output run to run.
            if d < best || (d == best && j < tgt) {
                best = d;
                tgt = j;
            }
        }
        nearest[i] = tgt;
        let (x0, y0, x1, y1) = features[i].bbox;
        let bw = (x1 - x0 + 1) as f32;
        let bh = (y1 - y0 + 1) as f32;
        let diag = (bw * bw + bh * bh).sqrt();
        let thickness = features[i].area as f32 / (0.5 * per[i].max(1) as f32);
        let thin = thickness <= thin_max;
        let small = diag < speck_floor;
        let color_dup = best < color_jnd;
        keep[i] = !(thin || small || color_dup);
    }

    // A failing feature unions toward its nearest neighbor. The chain reaches a
    // survivor because a union only ever starts from a failing node.
    let mut parent: Vec<u32> = (0..n as u32).collect();
    for i in 0..n {
        if !keep[i] && nearest[i] != u32::MAX {
            let (ra, rb) = (uf_find(&mut parent, i as u32), uf_find(&mut parent, nearest[i]));
            if ra != rb {
                parent[ra.max(rb) as usize] = ra.min(rb);
            }
        }
    }
    // Ascending-root order keeps the label raster deterministic, as in
    // merge_features. A component with no survivor still needs a color, so its
    // largest member founds the output feature.
    let mut region_at: HashMap<u32, usize> = HashMap::new();
    let mut regions: Vec<Vec<u32>> = Vec::new();
    for i in 0..n as u32 {
        let root = uf_find(&mut parent, i);
        let idx = *region_at.entry(root).or_insert_with(|| {
            regions.push(Vec::new());
            regions.len() - 1
        });
        regions[idx].push(i);
    }

    let mut remap = vec![u32::MAX; n];
    let mut out: Vec<Feature> = Vec::new();
    let mut consolidated = 0;
    for members in &regions {
        let rep = *members
            .iter()
            .max_by(|&&a, &&b| {
                let ka = (keep[a as usize], features[a as usize].area);
                let kb = (keep[b as usize], features[b as usize].area);
                ka.cmp(&kb)
            })
            .unwrap();
        let slot = out.len() as u32;
        out.push(features[rep as usize].clone());
        if members.len() > 1 {
            consolidated += 1;
        }
        for &m in members {
            remap[m as usize] = slot;
            if m != rep {
                absorb(&mut out[slot as usize], &features[m as usize]);
            }
        }
    }
    let at: Vec<u32> = labels
        .at
        .iter()
        .map(|&l| if l == u32::MAX { u32::MAX } else { remap[l as usize] })
        .collect();
    let labels = FeatureLabels { w: labels.w, h: labels.h, at };
    MergeResult { features: out, families: consolidated, labels }
}

/// Unique unordered feature-index pairs that share a 4-connected boundary in
/// `labels`, background excluded.
fn boundary_edges(labels: &FeatureLabels) -> Vec<(u32, u32)> {
    let (w, h) = (labels.w as usize, labels.h as usize);
    let mut set: HashSet<(u32, u32)> = HashSet::new();
    for y in 0..h {
        for x in 0..w {
            let a = labels.at[y * w + x];
            if a == u32::MAX {
                continue;
            }
            let mut edge = |b: u32| {
                if b != u32::MAX && b != a {
                    set.insert((a.min(b), a.max(b)));
                }
            };
            if x + 1 < w {
                edge(labels.at[y * w + x + 1]);
            }
            if y + 1 < h {
                edge(labels.at[(y + 1) * w + x]);
            }
        }
    }
    set.into_iter().collect()
}

/// OKLab lightness margin by which a feature must beat every neighbor to count
/// as a local extremum (a distinct blob, not a ramp band). At detection's own
/// tolerance, so a real gradient step never reads as a peak while a highlight
/// clears it easily.
const EXTREMUM_MARGIN: f32 = DETECT_TOL;

/// At most `bands` representative members of a region, chosen greedily by
/// area-weighted error. The seed is the largest-area member, so the dominant
/// color is always a kept band that other pixels remap onto rather than
/// darkening toward a mid-band; each next band maximizes `area * ΔE²` to its
/// nearest kept band, which still reaches the color extremes because they are
/// farthest. Returns every member when there are at most `bands` of them.
/// Member order is irrelevant.
fn family_reps(members: &[u32], features: &[Feature], lab: &[[f32; 3]], bands: usize) -> Vec<u32> {
    if members.len() <= bands {
        return members.to_vec();
    }
    let seed = *members
        .iter()
        .max_by_key(|&&m| features[m as usize].area)
        .unwrap();
    let mut reps = vec![seed];
    let mut dist: Vec<f32> = members.iter().map(|&m| lab_dist(lab[m as usize], lab[seed as usize])).collect();
    while reps.len() < bands {
        let best = (0..members.len())
            .max_by(|&i, &j| {
                let ei = features[members[i] as usize].area as f32 * dist[i] * dist[i];
                let ej = features[members[j] as usize].area as f32 * dist[j] * dist[j];
                ei.partial_cmp(&ej).unwrap()
            })
            .unwrap();
        if dist[best] <= 0.0 {
            break;
        }
        let picked = members[best];
        reps.push(picked);
        for (d, &m) in dist.iter_mut().zip(members) {
            *d = d.min(lab_dist(lab[m as usize], lab[picked as usize]));
        }
    }
    reps
}

/// The representative in `reps` whose mean is nearest `m`'s in OKLab.
fn nearest_rep(m: u32, reps: &[u32], lab: &[[f32; 3]]) -> u32 {
    reps.iter()
        .copied()
        .min_by(|&a, &b| {
            lab_dist(lab[m as usize], lab[a as usize])
                .partial_cmp(&lab_dist(lab[m as usize], lab[b as usize]))
                .unwrap()
        })
        .unwrap()
}

/// Adds `src`'s area and bbox to `dst`, leaving `dst`'s color unchanged as the
/// color of the absorbing feature.
fn absorb(dst: &mut Feature, src: &Feature) {
    dst.area = dst.area.saturating_add(src.area);
    grow_bbox(&mut dst.bbox, src.bbox);
}

fn grow_bbox(dst: &mut (u32, u32, u32, u32), s: (u32, u32, u32, u32)) {
    dst.0 = dst.0.min(s.0);
    dst.1 = dst.1.min(s.1);
    dst.2 = dst.2.max(s.2);
    dst.3 = dst.3.max(s.3);
}

fn uf_find(parent: &mut [u32], mut i: u32) -> u32 {
    while parent[i as usize] != i {
        parent[i as usize] = parent[parent[i as usize] as usize];
        i = parent[i as usize];
    }
    i
}

/// The merged feature partition [`layer_palette`] selects colors from: fine
/// detection, extremum-preserving merge (budgeted by `cfg.gradient_bands`),
/// then indistinct cleanup. Visualization and digest harnesses read the same
/// partition, so their downstream stages match the palette the pipeline builds.
pub fn feature_partition(src: &RgbaImage, cfg: &Config) -> MergeResult {
    let (features, labels) = detect_features(src, cfg);
    feature_partition_from(&features, &labels, cfg)
}

/// The merged feature partition for an already-detected `features`/`labels`
/// pair (as [`detect_features`] returns): the extremum-preserving merge
/// budgeted by `cfg.gradient_bands`, then indistinct cleanup. Reads only
/// palette-selection config, never the source image.
pub fn feature_partition_from(
    features: &[Feature],
    labels: &FeatureLabels,
    cfg: &Config,
) -> MergeResult {
    let merged = merge_features(features, labels, cfg);
    merge_indistinct(
        &merged.features,
        &merged.labels,
        INDISTINCT_THIN,
        INDISTINCT_SPECK,
        INDISTINCT_COLOR_JND,
    )
}

/// The palette for one layer's 1x source crop `src`: the [`feature_partition`]
/// merged feature set, dedup grouped, then salience selected. `dim` = max
/// document W/H, normalizing the detail floor against the document.
pub fn layer_palette(src: &RgbaImage, cfg: &Config, dim: u32) -> Vec<[u8; 3]> {
    let part = feature_partition(src, cfg);
    select_features(&group_features(&part.features, cfg, dim), cfg, dim)
}

fn lab_dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (d0, d1, d2) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    (d0 * d0 + d1 * d1 + d2 * d2).sqrt()
}

/// Map every art pixel to nearest palette color (OKLab ΔE); pixels outside
/// the alpha keep their meaningless zero fill.
pub fn remap(flat: &RgbImage, alpha: &GrayImage, palette: &[[u8; 3]]) -> RgbImage {
    let mut out = flat.clone();
    let pal_lab: Vec<[f32; 3]> = palette.iter().map(|&p| srgb_to_oklab(p)).collect();
    let mut cache: HashMap<[u8; 3], [u8; 3]> = HashMap::new();
    // Flat art runs the same color for long spans; checking the previous
    // pixel first skips the hash for the vast majority of pixels.
    let mut last: Option<([u8; 3], [u8; 3])> = None;
    let amask = alpha.as_raw();
    for (p, &av) in out.pixels_mut().zip(amask) {
        if av == 0 { continue; }
        let c = p.0;
        if let Some((lc, lm)) = last {
            if c == lc {
                p.0 = lm;
                continue;
            }
        }
        let mapped = *cache.entry(c).or_insert_with(|| {
            // Squared distance orders identically to color_dist and skips
            // both the sqrt and the per-candidate OKLab re-conversion of c.
            let cl = srgb_to_oklab(c);
            let d2 = |l: [f32; 3]| {
                let (d0, d1, d2) = (cl[0] - l[0], cl[1] - l[1], cl[2] - l[2]);
                d0 * d0 + d1 * d1 + d2 * d2
            };
            palette
                .iter()
                .zip(&pal_lab)
                .min_by(|(_, a), (_, b)| d2(**a).partial_cmp(&d2(**b)).unwrap())
                .map(|(p, _)| *p)
                .unwrap_or(c)
        });
        last = Some((c, mapped));
        p.0 = mapped;
    }
    out
}

/// Mode-filters the quantized labels so color boundaries settle where the
/// local majority sits. Nearest-color remap assigns the resize blend band
/// noisily when two palette colors are perceptually close (dark linework
/// against dark fur), pinching thin lines to nothing in places; majority
/// voting reclaims those pixels. Only art pixels vote: nothing outside the
/// alpha can outvote art, so the silhouette cannot erode.
pub fn label_smooth(quant: &RgbImage, alpha: &GrayImage, k: u32) -> RgbImage {
    crate::raster::majority_vote(quant, alpha, k)
}
