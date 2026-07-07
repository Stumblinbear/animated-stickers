//! PSD -> per-layer RGBA. One traced output per art layer, named from the
//! layer.

use crate::config::Config;
use anyhow::{bail, Result};
use image::RgbaImage;

pub fn layers(bytes: &[u8]) -> Result<Vec<(String, RgbaImage)>> {
    use rayon::prelude::*;
    let psd = psd::Psd::from_bytes(bytes).map_err(|e| anyhow::anyhow!("{e}"))?;
    let (w, h) = (psd.width(), psd.height());

    // rgba() decompresses a document-sized buffer per layer; done serially
    // it dominates startup.
    let mut out: Vec<(String, RgbaImage)> = psd
        .layers()
        .par_iter()
        .map(|layer| {
            let rgba = layer.rgba();

            // Skip empty layers (section dividers, fully transparent).
            // Threshold against the alpha cutoff so near-invisible AA fringe
            // alone doesn't resurrect an "empty" layer.
            let opaque = rgba
                .chunks_exact(4)
                .filter(|c| c[3] >= Config::default().alpha_threshold)
                .count();

            if opaque == 0 {
                return Ok(None);
            }

            match RgbaImage::from_raw(w, h, rgba) {
                Some(img) => Ok(Some((layer.name().to_string(), img))),
                None => bail!(
                    "layer '{}' rgba buffer is {}B, expected {}B for {}x{}",
                    layer.name(),
                    layer.rgba().len(),
                    w * h * 4,
                    w,
                    h
                ),
            }
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();

    if out.is_empty() {
        bail!("no non-empty layers found in PSD");
    }

    // The `psd` crate returns layers top-to-bottom (index 0 = topmost). Output
    // paints in list order (first painted = bottom of the stack), so reverse to
    // bottom-first: the bottommost PSD layer is painted first, the topmost last.
    out.reverse();

    Ok(out)
}
