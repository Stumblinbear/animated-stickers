//! Content-keyed cache of per-layer pipeline outputs, one store per document.
//!
//! Each pipeline output has a [`Memo`] holding the last `(key, value)`. A memo
//! serves the cached value when the fresh key equals the stored one (plain
//! `PartialEq`) and recomputes otherwise. A stage's key is its
//! [`Inputs`](super::stages) struct: the upstream outputs it consumes as
//! content-hashed [`Artifact`]s plus the config fields it reads. That structural
//! chain is the whole invalidation rule: a content change moves the artifact,
//! which moves every downstream key that embeds it.
//!
//! The intermediate strip outputs (prep through shapes) chain by `Artifact`.
//! The per-layer traces (fit, simplify) instead key on the full layer config:
//! the full-document render produces its traces monolithically, exposing no
//! intermediate artifacts to chain from, so the one cache both it and the strip
//! share keys on the inputs both can name.

use super::stages::LayerStages;
use crate::gui::ids::LayerId;
use crate::trace::TracedPath;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

/// Per-layer stage sets are held for at most this many distinct layers. A dozen
/// keeps typical layer switching warm at a few MB per layer.
const SLOT_LAYERS: usize = 12;
/// Total per-shape fitted-path entries across every layer.
const SHAPE_ENTRIES: usize = 4096;

/// Fitted paths per shape, keyed by shape content and fit params (see
/// [`super::shape_memo`]), shared with the stage worker across recomputes.
pub(in crate::gui) type ShapeCache = Arc<Mutex<LruCache<u64, Arc<Vec<TracedPath>>>>>;

/// A document's per-layer pipeline stage outputs: hand it a layer, get that
/// layer's [`LayerStages`]. The last several layers are retained in an LRU;
/// eviction is a memory bound, not part of the interface. Also holds the shared
/// per-shape fitted-path cache handed to workers.
pub(in crate::gui) struct DocStages {
    layers: LruCache<LayerId, LayerStages>,
    shapes: ShapeCache,
}

impl Default for DocStages {
    fn default() -> Self {
        Self {
            layers: LruCache::new(NonZeroUsize::new(SLOT_LAYERS).unwrap()),
            shapes: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(SHAPE_ENTRIES).unwrap(),
            ))),
        }
    }
}

impl DocStages {
    /// A read-only borrow of `layer`'s stages without touching LRU order, for
    /// off-session lookups (the pin hit test) and the full render's reuse.
    pub fn peek(&self, layer: LayerId) -> Option<&LayerStages> {
        self.layers.peek(&layer)
    }

    /// A clone of `layer`'s stages (empty when the layer is cold), marked
    /// most-recent, for the stage worker to run against.
    pub fn stages(&mut self, layer: LayerId) -> LayerStages {
        self.stages_mut(layer).clone()
    }

    /// `layer`'s stages, created empty if absent, marked most-recent.
    pub fn stages_mut(&mut self, layer: LayerId) -> &mut LayerStages {
        if self.layers.get(&layer).is_none() {
            self.layers.put(layer, LayerStages::default());
        }
        self.layers.get_mut(&layer).unwrap()
    }

    /// Replaces `layer`'s stages with a worker's completed set.
    pub fn install(&mut self, layer: LayerId, slots: LayerStages) {
        self.layers.put(layer, slots);
    }

    /// The shared per-shape fitted-path cache, cloned into stage workers.
    pub fn shape_cache(&self) -> ShapeCache {
        self.shapes.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::super::artifact::Artifact;
    use super::super::stages::PrepInputs;
    use super::*;
    use crate::config::Config;
    use crate::raster::Prepared;

    fn dummy_prep() -> Artifact<Prepared> {
        let prep = crate::raster::prepare(
            &image::RgbaImage::new(2, 2),
            &PrepInputs::of(&Config::default()),
        );

        Artifact::new_with(Arc::new(prep), |_, _| {})
    }

    #[test]
    fn slots_evict_past_capacity() {
        let mut m = DocStages::default();

        for i in 0..=SLOT_LAYERS {
            m.stages_mut(LayerId::from_raw(i as u128)).prep.put(
                PrepInputs {
                    scale: 3,
                    alpha_threshold: 128,
                    mode_filter: 0,
                },
                dummy_prep(),
            );
        }

        assert!(
            m.peek(LayerId::from_raw(0)).is_none(),
            "the oldest layer evicted"
        );

        assert!(m.peek(LayerId::from_raw(SLOT_LAYERS as u128)).is_some());
    }
}
