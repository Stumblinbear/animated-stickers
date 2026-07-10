//! Document loading: PSD/PNG files as layer stacks ready for the pipeline.

use super::app::DocState;
use super::ids::{DocId, LayerId};
use crate::psd_import;
use image::RgbaImage;
use rustc_hash::FxHashMap;
use std::sync::Arc;

#[derive(Debug)]
pub(super) struct Layer {
    /// Stable identity, born with the layer and independent of its paint-order
    /// position, so per-layer state and compute results follow the layer rather
    /// than its slot.
    pub(super) id: LayerId,
    pub(super) name: String,
    /// Cropped to the art bbox; `offset` restores document placement.
    pub(super) img: RgbaImage,
    pub(super) offset: (u32, u32),
}

/// What the artist did to this layer beyond the raster: everything fed into the
/// pipeline for this one layer. Undoable and persisted.
#[derive(Debug, Clone)]
pub(super) struct LayerInputs {
    /// Off skips the layer in the preview composite only.
    pub(super) visible: bool,
    /// Off excludes the layer from processing and export.
    pub(super) enabled: bool,
    /// Speckle-floor exemption points in document source px. Any region
    /// containing one survives the floor and re-segmentation, marking a small
    /// feature (a tooth, a glint) as deliberate.
    pub(super) pins: Vec<[u32; 2]>,
}

impl Default for LayerInputs {
    fn default() -> Self {
        Self { visible: true, enabled: true, pins: Vec::new() }
    }
}

/// Derived from the render for one layer. Overwritten wholesale by each full
/// render; never undone or persisted.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct LayerOutputs {
    /// Total path anchors across the layer's traced shapes, for the rail's
    /// per-layer count and ramp.
    pub(super) anchors: usize,
}

pub(super) struct Doc {
    /// Stable identity, distinct from the document's tab-strip position, so a
    /// background compute result routes to this document even after another
    /// tab closes and the rest shift.
    pub(super) id: DocId,
    pub(super) path: std::path::PathBuf,
    /// Document dimensions, which detail normalization needs (README).
    pub(super) size: (u32, u32),
    /// Bottom-first paint order, as psd_import returns them, the sole home of
    /// layer ordering. Arc so background tasks can borrow the pixels without
    /// cloning them.
    pub(super) layers: Arc<Vec<Layer>>,
    /// Per-layer artist inputs, keyed by [`LayerId`](super::ids::LayerId). The
    /// id is the only join to `layers`: there is no positional alignment to
    /// keep, and ordered reads walk `layers` and look each id up here. Separate
    /// from `layers` because the pixels are shared immutably with background
    /// tasks while these inputs stay mutable. One of the two per-layer maps
    /// (the other, derived render outputs, lives in the session); both grow by
    /// lifetime, never by feature.
    pub(super) inputs: FxHashMap<LayerId, LayerInputs>,
    /// Everything the user sees and selects for this document, preserved
    /// across tab switches.
    pub(super) session: DocState,
}

impl Doc {
    /// The paint-order position of the layer identified by `id`, or `None` when
    /// no layer in this document has that identity. The one place a `LayerId`
    /// becomes a position, for the genuinely positional gestures (range select,
    /// above/below); per-layer state never routes through it.
    pub(super) fn layer_pos(&self, id: LayerId) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }

    /// The layer identified by `id`, or `None` when none has that identity.
    pub(super) fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    /// The topmost layer's identity (last in paint order), or `None` for a
    /// document with no layers.
    pub(super) fn top_layer(&self) -> Option<LayerId> {
        self.layers.last().map(|l| l.id)
    }

    /// A mutable handle to layer `id`'s inputs, or `None` when none has that
    /// identity.
    pub(super) fn inputs_mut(&mut self, id: LayerId) -> Option<&mut LayerInputs> {
        self.inputs.get_mut(&id)
    }
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
            Some(Layer { id: LayerId::new(), name, img: crop, offset: (x0, y0) })
        })
        .collect();
    let inputs = layers.iter().map(|l| (l.id, LayerInputs::default())).collect();
    Ok(Doc {
        id: DocId::new(),
        path: path.to_path_buf(),
        size,
        layers: Arc::new(layers),
        inputs,
        session: DocState::default(),
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::app::DocState;

    fn layer(id: LayerId) -> Layer {
        Layer { id, name: "l".into(), img: RgbaImage::new(1, 1), offset: (0, 0) }
    }

    // Per-layer state is keyed by id with no positional join to `layers`, so an
    // input set on one layer stays with that layer no matter where it sits in
    // paint order.
    #[test]
    fn per_layer_inputs_follow_the_id_not_the_slot() {
        let (a, b, c) = (LayerId::from_raw(10), LayerId::from_raw(20), LayerId::from_raw(30));
        let mut inputs: FxHashMap<LayerId, LayerInputs> =
            [a, b, c].into_iter().map(|id| (id, LayerInputs::default())).collect();
        // Disable only the middle layer, addressing it by id.
        inputs.get_mut(&b).unwrap().enabled = false;

        let mut doc = Doc {
            id: DocId::from_raw(0),
            path: "test.psd".into(),
            size: (1, 1),
            layers: Arc::new(vec![layer(a), layer(b), layer(c)]),
            inputs,
            session: DocState::default(),
        };

        // Positions reflect paint order; the disabled flag belongs to b alone.
        assert_eq!(doc.layer_pos(b), Some(1));
        assert!(doc.inputs[&a].enabled);
        assert!(!doc.inputs[&b].enabled);
        assert!(doc.inputs[&c].enabled);

        // Reorder so b moves to the front. Its input, keyed by id, is untouched:
        // nothing positional joins the inputs to the layer order.
        doc.layers = Arc::new(vec![layer(b), layer(a), layer(c)]);
        assert_eq!(doc.layer_pos(b), Some(0));
        assert!(!doc.inputs[&b].enabled, "b keeps its input across the reorder");
        assert!(doc.inputs[&a].enabled);

        // An id naming no layer resolves to nothing.
        assert_eq!(doc.layer_pos(LayerId::from_raw(999)), None);
        assert!(doc.layer(LayerId::from_raw(999)).is_none());
    }
}
