//! Content-keyed cache of per-layer pipeline outputs, one store per document.
//!
//! Each stage's output is keyed by a `u64` that folds that stage's `Config`
//! subset into the previous stage's key, so an entry is valid exactly when
//! every input up to and including that stage matches. The subset each stage
//! reads is declared once, in [`StageKeys::of`].
//!
//! Prep and quant rasters live in a small layer-capped cache; geometry (the
//! traces, regions, palette, and smooth-view image) lives in a larger one.

use super::{Img, LayerTrace};
use crate::config::Config;
use crate::gui::ids::LayerId;
use crate::palette::Partition;
use crate::raster::Prepared;
use crate::regions::{MergePlan, Region};
use crate::trace::TracedPath;
use image::RgbImage;
use lru::LruCache;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

/// Prep and quant rasters are held for at most this many distinct layers.
/// A dozen keeps typical layer switching warm at a few MB per layer.
const PIXEL_LAYERS: usize = 12;
/// Total geometry entries across every layer and stage.
const GEO_ENTRIES: usize = 256;
/// Total per-shape fitted-path entries across every layer.
const SHAPE_ENTRIES: usize = 4096;

/// Fitted paths per shape, keyed by shape content and fit params (see
/// [`super::shape_memo`]), shared with the stage worker across recomputes.
pub(in crate::gui) type ShapeCache = Arc<Mutex<LruCache<u64, Arc<Vec<TracedPath>>>>>;

/// The content keys for one layer's config, one per pipeline stage. Two
/// configs share a stage's key exactly when every field that stage or an
/// earlier one reads is equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::gui) struct StageKeys {
    pub(in crate::gui) prep: u64,
    pub(in crate::gui) detect: u64,
    pub(in crate::gui) quant: u64,
    pub(in crate::gui) regions: u64,
    /// Regions plus the pins drawn over them.
    pub(in crate::gui) regions_view: u64,
    pub(in crate::gui) fit: u64,
    pub(in crate::gui) simplify: u64,
}

/// Folds `prev` and whatever `f` hashes into one key.
fn fold(prev: u64, f: impl FnOnce(&mut DefaultHasher)) -> u64 {
    let mut h = DefaultHasher::new();
    prev.hash(&mut h);
    f(&mut h);
    h.finish()
}

impl StageKeys {
    /// The stage keys for `cfg`. Every field a stage reads is hashed here and
    /// nowhere else. Floats hash by bit pattern; `Vec` fields hash
    /// element-wise.
    pub(in crate::gui) fn of(cfg: &Config) -> Self {
        let prep = fold(0x9E37_79B9_7F4A_7C15, |h| {
            cfg.scale.hash(h);
            cfg.alpha_threshold.hash(h);
            cfg.mode_filter.hash(h);
        });
        let detect = fold(0x9E37_79B9_7F4A_7C15, |h| {
            cfg.alpha_threshold.hash(h);
        });
        let quant = fold(prep, |h| {
            cfg.detail.to_bits().hash(h);
            cfg.max_colors.hash(h);
            cfg.locked.hash(h);
            cfg.shade_split.to_bits().hash(h);
            cfg.shade_noise.to_bits().hash(h);
            cfg.color_cleanup.hash(h);
        });
        let regions = fold(quant, |h| {
            cfg.absorb_dist.to_bits().hash(h);
            cfg.absorb_aggr.to_bits().hash(h);
            cfg.stroke_merge_dist.to_bits().hash(h);
            cfg.stroke_merge_width.to_bits().hash(h);
        });
        let regions_view = fold(regions, |h| cfg.pins.hash(h));
        // Pins gate which sub-floor regions get traced, so they belong to the
        // fit key even though the regions themselves are unchanged.
        let fit = fold(regions, |h| {
            cfg.alphamax.to_bits().hash(h);
            cfg.opttolerance.to_bits().hash(h);
            cfg.seam_slack.to_bits().hash(h);
            cfg.smoothing.to_bits().hash(h);
            cfg.pins.hash(h);
        });
        let simplify = fold(fit, |h| cfg.simplify.to_bits().hash(h));
        Self {
            prep,
            detect,
            quant,
            regions,
            regions_view,
            fit,
            simplify,
        }
    }
}

/// The raster-heavy values for one layer, each tagged with the key it was
/// built under.
#[derive(Default)]
struct PixelSlot {
    prep: Option<(u64, Arc<Prepared>)>,
    /// Keyed by `detect`, not `prep`: detection ignores every prep field but
    /// `alpha_threshold`.
    detect: Option<(u64, Arc<Partition>)>,
    quant: Option<(u64, Arc<RgbImage>)>,
    /// Keyed by `regions_view`: the plan folds the pins into the merge.
    plan: Option<(u64, Arc<MergePlan>)>,
}

/// A geometry cache entry, addressed by `(layer, stage, key)`.
#[derive(Clone)]
enum Geo {
    Palette(Arc<Vec<[u8; 3]>>),
    Regions(Arc<Vec<Region>>),
    Trace(Arc<LayerTrace>),
    Smooth(Option<Img>),
}

/// Which geometry a `(layer, key)` addresses, so fit and simplify traces (or
/// regions and the smooth image, which share the fit key) never collide.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum GeoStage {
    Palette,
    Regions,
    Fit,
    Simplify,
    Smooth,
}

/// Per-document memo of pipeline stage outputs.
pub(in crate::gui) struct Memo {
    pixel: LruCache<LayerId, PixelSlot>,
    geo: LruCache<(LayerId, GeoStage, u64), Geo>,
    shapes: ShapeCache,
}

impl Default for Memo {
    fn default() -> Self {
        Self {
            pixel: LruCache::new(NonZeroUsize::new(PIXEL_LAYERS).unwrap()),
            geo: LruCache::new(NonZeroUsize::new(GEO_ENTRIES).unwrap()),
            shapes: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(SHAPE_ENTRIES).unwrap(),
            ))),
        }
    }
}

impl Memo {
    pub fn prep(&mut self, layer: LayerId, key: u64) -> Option<Arc<Prepared>> {
        match self.pixel.get(&layer)?.prep {
            Some((k, ref v)) if k == key => Some(v.clone()),
            _ => None,
        }
    }

    pub fn detect(&mut self, layer: LayerId, key: u64) -> Option<Arc<Partition>> {
        match self.pixel.get(&layer)?.detect {
            Some((k, ref v)) if k == key => Some(v.clone()),
            _ => None,
        }
    }

    pub fn quant(&mut self, layer: LayerId, key: u64) -> Option<Arc<RgbImage>> {
        match self.pixel.get(&layer)?.quant {
            Some((k, ref v)) if k == key => Some(v.clone()),
            _ => None,
        }
    }

    pub fn plan(&mut self, layer: LayerId, key: u64) -> Option<Arc<MergePlan>> {
        match self.pixel.get(&layer)?.plan {
            Some((k, ref v)) if k == key => Some(v.clone()),
            _ => None,
        }
    }

    /// The shared per-shape fitted-path cache, cloned into stage workers.
    pub fn shape_cache(&self) -> ShapeCache {
        self.shapes.clone()
    }

    pub fn put_prep(&mut self, layer: LayerId, key: u64, v: Arc<Prepared>) {
        self.pixel_slot(layer).prep = Some((key, v));
    }

    pub fn put_detect(&mut self, layer: LayerId, key: u64, v: Arc<Partition>) {
        self.pixel_slot(layer).detect = Some((key, v));
    }

    pub fn put_quant(&mut self, layer: LayerId, key: u64, v: Arc<RgbImage>) {
        self.pixel_slot(layer).quant = Some((key, v));
    }

    pub fn put_plan(&mut self, layer: LayerId, key: u64, v: Arc<MergePlan>) {
        self.pixel_slot(layer).plan = Some((key, v));
    }

    /// The layer's pixel slot, created empty if absent, marked most-recent.
    fn pixel_slot(&mut self, layer: LayerId) -> &mut PixelSlot {
        if self.pixel.get(&layer).is_none() {
            self.pixel.put(layer, PixelSlot::default());
        }
        self.pixel.get_mut(&layer).unwrap()
    }

    pub fn palette(&mut self, layer: LayerId, key: u64) -> Option<Arc<Vec<[u8; 3]>>> {
        match self.geo.get(&(layer, GeoStage::Palette, key))? {
            Geo::Palette(v) => Some(v.clone()),
            _ => None,
        }
    }

    pub fn regions(&mut self, layer: LayerId, key: u64) -> Option<Arc<Vec<Region>>> {
        match self.geo.get(&(layer, GeoStage::Regions, key))? {
            Geo::Regions(v) => Some(v.clone()),
            _ => None,
        }
    }

    /// Regions without marking the entry most-recent, for read-only lookups
    /// off the immutable session (the pin hit test).
    pub fn peek_regions(&self, layer: LayerId, key: u64) -> Option<Arc<Vec<Region>>> {
        match self.geo.peek(&(layer, GeoStage::Regions, key))? {
            Geo::Regions(v) => Some(v.clone()),
            _ => None,
        }
    }

    pub fn fit(&mut self, layer: LayerId, key: u64) -> Option<Arc<LayerTrace>> {
        match self.geo.get(&(layer, GeoStage::Fit, key))? {
            Geo::Trace(v) => Some(v.clone()),
            _ => None,
        }
    }

    pub fn simplify(&mut self, layer: LayerId, key: u64) -> Option<Arc<LayerTrace>> {
        match self.geo.get(&(layer, GeoStage::Simplify, key))? {
            Geo::Trace(v) => Some(v.clone()),
            _ => None,
        }
    }

    pub fn smooth(&mut self, layer: LayerId, key: u64) -> Option<Option<Img>> {
        match self.geo.get(&(layer, GeoStage::Smooth, key))? {
            Geo::Smooth(v) => Some(v.clone()),
            _ => None,
        }
    }

    pub fn put_palette(&mut self, layer: LayerId, key: u64, v: Arc<Vec<[u8; 3]>>) {
        self.geo
            .put((layer, GeoStage::Palette, key), Geo::Palette(v));
    }

    pub fn put_regions(&mut self, layer: LayerId, key: u64, v: Arc<Vec<Region>>) {
        self.geo
            .put((layer, GeoStage::Regions, key), Geo::Regions(v));
    }

    pub fn put_fit(&mut self, layer: LayerId, key: u64, v: Arc<LayerTrace>) {
        self.geo.put((layer, GeoStage::Fit, key), Geo::Trace(v));
    }

    pub fn put_simplify(&mut self, layer: LayerId, key: u64, v: Arc<LayerTrace>) {
        self.geo
            .put((layer, GeoStage::Simplify, key), Geo::Trace(v));
    }

    pub fn put_smooth(&mut self, layer: LayerId, key: u64, v: Option<Img>) {
        self.geo.put((layer, GeoStage::Smooth, key), Geo::Smooth(v));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config::default()
    }

    #[test]
    fn simplify_change_leaves_earlier_keys_fixed() {
        let a = StageKeys::of(&cfg());
        let b = StageKeys::of(&Config {
            simplify: 5.0,
            ..cfg()
        });
        assert_eq!(a.fit, b.fit);
        assert_eq!(a.regions, b.regions);
        assert_ne!(a.simplify, b.simplify);
    }

    #[test]
    fn detect_key_holds_across_palette_edits_and_breaks_on_alpha() {
        let base = StageKeys::of(&cfg());
        // shade_split and detail are consolidation/selection params: detection
        // is invariant to them, so its cache must stay valid across the edit.
        let bands = StageKeys::of(&Config {
            shade_split: cfg().shade_split + 0.01,
            ..cfg()
        });
        let detail = StageKeys::of(&Config {
            detail: cfg().detail + 1.0,
            ..cfg()
        });
        assert_eq!(base.detect, bands.detect);
        assert_eq!(base.detect, detail.detect);
        assert_ne!(base.quant, bands.quant);
        // alpha_threshold is detection's only input, so it must invalidate.
        let alpha = StageKeys::of(&Config {
            alpha_threshold: cfg().alpha_threshold.wrapping_add(1),
            ..cfg()
        });
        assert_ne!(base.detect, alpha.detect);
    }

    #[test]
    fn detail_change_ripples_through_quant_and_below() {
        let a = StageKeys::of(&cfg());
        let b = StageKeys::of(&Config {
            detail: 9.0,
            ..cfg()
        });
        assert_eq!(a.prep, b.prep);
        assert_ne!(a.quant, b.quant);
        assert_ne!(a.regions, b.regions);
        assert_ne!(a.fit, b.fit);
        assert_ne!(a.simplify, b.simplify);
    }

    #[test]
    fn pins_change_fit_not_regions() {
        let a = StageKeys::of(&cfg());
        let b = StageKeys::of(&Config {
            pins: vec![[3, 4]],
            ..cfg()
        });
        assert_eq!(a.regions, b.regions);
        assert_ne!(a.regions_view, b.regions_view);
        assert_ne!(a.fit, b.fit);
    }

    #[test]
    fn pixel_cache_evicts_past_capacity() {
        let mut m = Memo::default();
        let prep = || Arc::new(crate::raster::prepare(&image::RgbaImage::new(2, 2), &cfg()));
        for i in 0..=PIXEL_LAYERS {
            m.put_prep(LayerId(i), i as u64, prep());
        }
        assert!(m.prep(LayerId(0), 0).is_none());
        assert!(m.prep(LayerId(PIXEL_LAYERS), PIXEL_LAYERS as u64).is_some());
    }

    #[test]
    fn geo_cache_evicts_the_least_recent() {
        let mut m = Memo::default();
        let regs = || Arc::new(Vec::<Region>::new());
        for i in 0..(GEO_ENTRIES as u64 + 5) {
            m.put_regions(LayerId(0), i, regs());
        }
        assert!(m.regions(LayerId(0), 0).is_none());
        assert!(m.regions(LayerId(0), GEO_ENTRIES as u64 + 4).is_some());
    }
}
