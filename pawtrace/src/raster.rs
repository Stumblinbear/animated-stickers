//! Preprocessing: supersample and alpha threshold.
//! Ported from vectorize.py with deliberate deviations; see the README
//! "provenance" table before changing any step.

use crate::color::Srgb;
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
    /// The single color when every opaque source pixel shares it, letting a
    /// consumer segment straight from the mask and skip palette selection.
    pub uniform: Option<Srgb>,
}

/// Every config value [`prepare`] reads.
#[derive(Debug, Clone, PartialEq)]
pub struct PrepParams {
    pub scale: u32,
    pub alpha_threshold: u8,
}

impl PrepParams {
    pub fn of(cfg: &Config) -> Self {
        Self {
            scale: cfg.scale,
            alpha_threshold: cfg.alpha_threshold,
        }
    }
}

/// The single color of a layer whose opaque pixels are all identical
/// (helper layers: solid fills, border mattes), or `None`.
pub fn uniform_color(src: &RgbaImage, alpha_threshold: u8) -> Option<Srgb> {
    let mut color: Option<Srgb> = None;

    for p in src.pixels() {
        if p.0[3] < alpha_threshold {
            continue;
        }

        let c = Srgb([p.0[0], p.0[1], p.0[2]]);

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
    scale_alpha_plane(src, cfg.scale, cfg.alpha_threshold)
}

fn scale_alpha_plane(src: &RgbaImage, scale: u32, alpha_threshold: u8) -> GrayImage {
    let (w, h) = src.dimensions();
    let (sw, sh) = (w * scale, h * scale);
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
        p.0[0] = if p.0[0] >= alpha_threshold { 255 } else { 0 };
    }

    alpha
}

pub fn prepare(src: &RgbaImage, cfg: &PrepParams) -> Prepared {
    // A uniform-color layer needs no palette, remap, or quantization: the
    // scaled alpha alone determines its regions. The alpha plane resizes
    // identically whether taken alone or from the full RGBA resample, so the
    // mask matches the four-plane path byte for byte, at a quarter of its cost.
    if let Some(color) = uniform_color(src, cfg.alpha_threshold) {
        let alpha = scale_alpha_plane(src, cfg.scale, cfg.alpha_threshold);

        let mut flat = RgbImage::new(alpha.width(), alpha.height());
        {
            let fr: &mut [u8] = &mut flat;
            for (i, &a) in alpha.as_raw().iter().enumerate() {
                if a != 0 {
                    fr[3 * i..3 * i + 3].copy_from_slice(&color.0);
                }
            }
        }

        return Prepared {
            flat,
            alpha,
            uniform: Some(color),
        };
    }

    // Supersample with a SMOOTH filter, like the reference (vectorize.py uses
    // ImageMagick's default -resize). Nearest-neighbor looks safer ("zero new
    // blend colors") but clones every source AA blend color scale^2 times,
    // concentrating histogram counts until blends earn palette slots. Smooth
    // resampling spreads blends into a continuum of tiny counts that stay
    // below the palette floor, and gives the tracer 1px-quantized region
    // boundaries instead of scale-px staircases.
    let (w, h) = src.dimensions();
    let (sw, sh) = (w * cfg.scale, h * cfg.scale);

    let mut dst = fir::images::Image::new(sw, sh, fir::PixelType::U8x4);
    {
        // fast_image_resize multiplies by alpha before resizing and divides
        // after (SIMD), so fully transparent pixels' colors don't bleed into
        // edges (ImageMagick resizes with associated alpha too). Bilinear, not a
        // cubic filter: negative lobes make alpha undershoot at the hard
        // silhouette edge while color does not, and un-premultiplying then skews
        // those pixels bright.
        let src_view = fir::images::ImageRef::new(w, h, src.as_raw(), fir::PixelType::U8x4)
            .expect("buffer matches dimensions");

        fir::Resizer::new()
            .resize(
                &src_view,
                &mut dst,
                &fir::ResizeOptions::new()
                    .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Bilinear)),
            )
            .expect("same pixel type");
    }
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

    Prepared {
        flat,
        alpha,
        uniform: None,
    }
}
