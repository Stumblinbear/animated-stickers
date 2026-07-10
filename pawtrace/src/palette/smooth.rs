//! Structure-preserving smoothing that detection grows over, so compression
//! and anti-alias speckle does not spawn a feature per artifact pixel.

use crate::config::Config;
use image::RgbaImage;

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
/// Jia, SIGGRAPH Asia 2012), the growing guide [`Partition::detect`] smooths
/// with before growing components. Runs per RGB channel over `src`'s opaque
/// pixels ([`Config::alpha_threshold`]); below-threshold pixels are copied
/// through untouched and never couple across the silhouette. RTV separates
/// structure from texture by local anisotropy, keeping a faint but coherent
/// edge while flattening incoherent texture at the same amplitude.
///
/// [`Partition::detect`]: super::Partition::detect
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
