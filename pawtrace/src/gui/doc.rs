//! Document loading: PSD/PNG files as layer stacks ready for the pipeline.

use crate::psd_import;
use image::RgbaImage;
use std::sync::Arc;

#[derive(Debug)]
pub(super) struct Layer {
    pub(super) name: String,
    /// Cropped to the art bbox; `offset` restores document placement.
    pub(super) img: RgbaImage,
    pub(super) offset: (u32, u32),
}

pub(super) struct Doc {
    pub(super) path: std::path::PathBuf,
    /// Document dimensions, which detail normalization needs (README).
    pub(super) size: (u32, u32),
    /// Bottom-first paint order, as psd_import returns them. Arc so
    /// background tasks can borrow the pixels without cloning them.
    pub(super) layers: Arc<Vec<Layer>>,
}

/// Every PSD/PNG directly in `dir`, sorted by name. Not recursive: art
/// folders often nest exports and references that should not batch-load.
pub(super) fn scan_folder(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("psd") || e.eq_ignore_ascii_case("png"))
        })
        .collect();
    files.sort();
    files
}

pub(super) fn doc_label(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

pub(super) fn load_doc(path: &std::path::Path) -> anyhow::Result<Doc> {
    let doc_layers: Vec<(String, RgbaImage)> =
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("psd")) {
            psd_import::layers(&std::fs::read(path)?)?
        } else {
            vec![("layer".into(), image::open(path)?.to_rgba8())]
        };
    let size = (doc_layers[0].1.width(), doc_layers[0].1.height());
    // Crop each layer at load: document-sized buffers per layer add up to
    // gigabytes with several PSDs open.
    let layers: Vec<Layer> = doc_layers
        .into_iter()
        .filter_map(|(name, img)| {
            let (x0, y0, x1, y1) = alpha_bbox(&img)?;
            let crop =
                image::imageops::crop_imm(&img, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image();
            Some(Layer { name, img: crop, offset: (x0, y0) })
        })
        .collect();
    Ok(Doc { path: path.to_path_buf(), size, layers: Arc::new(layers) })
}

fn alpha_bbox(img: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = img.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    for (x, y, p) in img.enumerate_pixels() {
        if p.0[3] > 0 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    (x0 <= x1).then_some((x0, y0, x1, y1))
}
