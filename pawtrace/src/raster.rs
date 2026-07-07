//! Preprocessing: supersample, alpha threshold, optional mode filter.
//! Ported from vectorize.py with deliberate deviations; see the README
//! "provenance" table before changing any step.

use crate::config::Config;
use fast_image_resize as fir;
use image::{GrayImage, Luma, Rgb, RgbImage, RgbaImage};

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
    for (i, px) in big.chunks_exact(4).enumerate() {
        let (x, y) = (i as u32 % sw, i as u32 / sw);
        let [r, g, b, a] = [px[0], px[1], px[2], px[3]];
        if a >= cfg.alpha_threshold {
            alpha.put_pixel(x, y, Luma([255]));
            flat.put_pixel(x, y, Rgb([r, g, b]));
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
    let (w, h) = img.dimensions();
    let r = (k / 2) as i64;
    let mut out = img.clone();
    // A window holds at most k*k distinct colors, small enough that a linear
    // scan beats hashing.
    let mut counts: Vec<([u8; 3], u32)> = Vec::with_capacity((k * k) as usize);
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            // Background must not vote: a majority-background window would
            // snap the silhouette's edge pixels to the meaningless zero
            // fill, and the remap turns that into a 1px ring.
            if alpha.get_pixel(x as u32, y as u32)[0] == 0 {
                continue;
            }
            counts.clear();
            for dy in -r..=r {
                for dx in -r..=r {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && nx < w as i64 && ny < h as i64
                        && alpha.get_pixel(nx as u32, ny as u32)[0] != 0
                    {
                        let Rgb(c) = *img.get_pixel(nx as u32, ny as u32);
                        match counts.iter_mut().find(|(cc, _)| *cc == c) {
                            Some((_, n)) => *n += 1,
                            None => counts.push((c, 1)),
                        }
                    }
                }
            }
            if let Some(best) = counts.iter().max_by_key(|(_, n)| *n) {
                out.put_pixel(x as u32, y as u32, Rgb(best.0));
            }
        }
    }
    out
}
