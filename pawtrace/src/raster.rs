//! Preprocessing: supersample, alpha threshold, optional mode filter.
//! Ported from vectorize.py with deliberate deviations; see the README
//! "provenance" table before changing any step.

use crate::config::Config;
use fast_image_resize as fir;
use image::{GrayImage, RgbImage, RgbaImage};

#[derive(Debug)]
pub struct Prepared {
    /// RGB at scaled size. Only pixels under `alpha` are meaningful; the
    /// rest are zeroed and every consumer must gate on `alpha`.
    pub flat: RgbImage,
    /// Binary alpha at scaled size (255 = art).
    pub alpha: GrayImage,
}

/// The single color of a layer whose opaque pixels are all identical
/// (helper layers: solid fills, border mattes), or `None`.
pub fn uniform_color(src: &RgbaImage, alpha_threshold: u8) -> Option<[u8; 3]> {
    let mut color: Option<[u8; 3]> = None;
    for p in src.pixels() {
        if p.0[3] < alpha_threshold {
            continue;
        }
        let c = [p.0[0], p.0[1], p.0[2]];
        match color {
            None => color = Some(c),
            Some(first) if first != c => return None,
            _ => {}
        }
    }
    color
}

/// Binary alpha at scaled size for a uniform-color layer: the same bilinear
/// resample and threshold as `prepare`, on the alpha plane alone.
pub fn scale_alpha(src: &RgbaImage, cfg: &Config) -> GrayImage {
    let (w, h) = src.dimensions();
    let (sw, sh) = (w * cfg.scale, h * cfg.scale);
    let plane: Vec<u8> = src.pixels().map(|p| p.0[3]).collect();
    let src_view = fir::images::ImageRef::new(w, h, &plane, fir::PixelType::U8)
        .expect("buffer matches dimensions");
    let mut dst = fir::images::Image::new(sw, sh, fir::PixelType::U8);
    fir::Resizer::new()
        .resize(
            &src_view,
            &mut dst,
            &fir::ResizeOptions::new()
                .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Bilinear)),
        )
        .expect("same pixel type");
    let mut alpha = GrayImage::from_raw(sw, sh, dst.into_vec()).unwrap();
    for p in alpha.pixels_mut() {
        p.0[0] = if p.0[0] >= cfg.alpha_threshold { 255 } else { 0 };
    }
    alpha
}

pub fn prepare(src: &RgbaImage, cfg: &Config) -> Prepared {
    // Supersample with a SMOOTH filter, like the reference (vectorize.py uses
    // ImageMagick's default -resize). Nearest-neighbor looks safer ("zero new
    // blend colors") but clones every source AA blend color scale^2 times,
    // concentrating histogram counts until blends earn palette slots. Smooth
    // resampling spreads blends into a continuum of tiny counts that stay
    // below the palette floor, and gives the tracer 1px-quantized region
    // boundaries instead of scale-px staircases.
    let (w, h) = src.dimensions();
    let (sw, sh) = (w * cfg.scale, h * cfg.scale);

    // fast_image_resize multiplies by alpha before resizing and divides
    // after (SIMD), so fully transparent pixels' colors don't bleed into
    // edges (ImageMagick resizes with associated alpha too). Bilinear, not a
    // cubic filter: negative lobes make alpha undershoot at the hard
    // silhouette edge while color does not, and un-premultiplying then skews
    // those pixels bright.
    let src_view = fir::images::ImageRef::new(w, h, src.as_raw(), fir::PixelType::U8x4)
        .expect("buffer matches dimensions");
    let mut dst = fir::images::Image::new(sw, sh, fir::PixelType::U8x4);
    fir::Resizer::new()
        .resize(
            &src_view,
            &mut dst,
            &fir::ResizeOptions::new()
                .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Bilinear)),
        )
        .expect("same pixel type");
    let big = dst.into_vec();

    // Threshold alpha FIRST (kills edge blending). Sub-threshold pixels
    // keep the zero fill; alpha is the only record of what is art.
    let mut alpha = GrayImage::new(sw, sh);
    let mut flat = RgbImage::new(sw, sh);
    {
        let ar: &mut [u8] = &mut alpha;
        let fr: &mut [u8] = &mut flat;
        for (i, px) in big.chunks_exact(4).enumerate() {
            if px[3] >= cfg.alpha_threshold {
                ar[i] = 255;
                fr[3 * i..3 * i + 3].copy_from_slice(&px[..3]);
            }
        }
    }

    if cfg.mode_filter > 0 {
        flat = mode_filter(&flat, &alpha, cfg.mode_filter);
    }

    Prepared { flat, alpha }
}

/// Snaps AA blend pixels to their neighborhood's dominant color. Only art
/// pixels (alpha on) vote, and only art pixels change.
fn mode_filter(img: &RgbImage, alpha: &GrayImage, k: u32) -> RgbImage {
    majority_vote(img, alpha, k)
}

/// Per-pixel k x k majority vote over art pixels: each art pixel becomes its
/// window's dominant color (ties to the last-seen candidate in row-major
/// window order); background pixels neither vote nor change. Backs both the
/// pre-quantization mode filter and post-remap label smoothing.
pub(crate) fn majority_vote(img: &RgbImage, alpha: &GrayImage, k: u32) -> RgbImage {
    let (w, h) = img.dimensions();
    let r = (k / 2) as i64;
    let src = img.as_raw();
    let amask = alpha.as_raw();
    let mut out = img.clone();
    use rayon::prelude::*;
    let out_buf: &mut [u8] = &mut out;
    out_buf
        .par_chunks_exact_mut(w as usize * 3)
        .enumerate()
        .for_each(|(y, row)| {
            let y = y as i64;
            // A window holds at most k*k distinct colors, small enough that a
            // linear scan beats hashing.
            let mut counts: Vec<([u8; 3], u32)> = Vec::with_capacity((k * k) as usize);
            for x in 0..w as i64 {
                // Background must not vote: a majority-background window
                // would snap the silhouette's edge pixels to the meaningless
                // zero fill, and the remap turns that into a 1px ring.
                if amask[(y * w as i64 + x) as usize] == 0 {
                    continue;
                }
                counts.clear();
                for dy in -r..=r {
                    let ny = y + dy;
                    if ny < 0 || ny >= h as i64 {
                        continue;
                    }
                    for dx in -r..=r {
                        let nx = x + dx;
                        if nx < 0 || nx >= w as i64 {
                            continue;
                        }
                        let ni = (ny * w as i64 + nx) as usize;
                        if amask[ni] != 0 {
                            let c = [src[3 * ni], src[3 * ni + 1], src[3 * ni + 2]];
                            match counts.iter_mut().find(|(cc, _)| *cc == c) {
                                Some((_, n)) => *n += 1,
                                None => counts.push((c, 1)),
                            }
                        }
                    }
                }
                if let Some(best) = counts.iter().max_by_key(|(_, n)| *n) {
                    let xi = x as usize * 3;
                    row[xi..xi + 3].copy_from_slice(&best.0);
                }
            }
        });
    out
}
