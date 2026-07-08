//! Per-stage golden digests for the fixture PSDs, so an identity regression
//! localizes to the stage that diverged. For every layer: a digest of the
//! quantized raster, of the region list in emission order, of the fitted
//! anchor list, and of the final translated paths. All four must stay
//! byte-identical across any refactor; the visual golden alone cannot say
//! which stage moved.
//!
//! Run: `cargo test --features preview --test digests`
//! Re-bless: `UPDATE_GOLDENS=1 cargo test --features preview --test digests`
//! rewrites `fixtures/golden/digests.toml`.

#![cfg(feature = "preview")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use pawtrace::profiles::ProfileStack;
use pawtrace::regions::Region;
use pawtrace::{palette, pipeline, psd_import, raster, regions};

const FIXTURES: &[&str] = &["seff_deer_a.psd", "seff_deer_b.psd"];

/// FNV-1a 64. Not DefaultHasher: these digests persist in a fixture file, so
/// the function must be stable across Rust releases.
#[derive(Clone, Copy)]
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn u32(&mut self, v: u32) {
        self.write(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.write(&v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }
    fn hex(self) -> String {
        format!("{:016x}", self.0)
    }
}

/// One layer's four stage digests, as stable hex strings.
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
struct LayerDigests {
    quant: String,
    region_count: usize,
    regions: String,
    fit: String,
    output: String,
}

/// Digest of the region list exactly as the trace consumes it: sequence order
/// and per-region color, bbox, and pixel set. The pixel set folds in
/// order-invariantly (wrapping sum of per-pixel hashes): pixel storage order
/// inside a region is unobservable downstream, and hashing it would forbid
/// refactors that only reorder it.
fn region_digest(regs: &[Region]) -> String {
    let mut h = Fnv::new();
    for r in regs {
        h.write(&r.color);
        h.u32(r.x0);
        h.u32(r.y0);
        h.u32(r.x1);
        h.u32(r.y1);
        h.u64(r.pixels.len() as u64);
        let mut set = 0u64;
        for &(px, py) in &r.pixels {
            let mut ph = Fnv::new();
            ph.u32(px);
            ph.u32(py);
            set = set.wrapping_add(ph.0);
        }
        h.u64(set);
    }
    h.hex()
}

/// Digest of traced colors: hex strings, path starts, and every cubic's
/// control points, in emission order, bit-exact.
fn trace_digest(colors: &[(String, Vec<pawtrace::trace::TracedPath>)]) -> String {
    let mut h = Fnv::new();
    for (hex, paths) in colors {
        h.write(hex.as_bytes());
        h.u64(paths.len() as u64);
        for p in paths {
            h.f64(p.start.0);
            h.f64(p.start.1);
            for &(c1, c2, to) in &p.cubics {
                for (x, y) in [c1, c2, to] {
                    h.f64(x);
                    h.f64(y);
                }
            }
        }
    }
    h.hex()
}

/// The four digests for one layer, replicating `pipeline::run`'s stages.
fn layer_digests(img: &image::RgbaImage, cfg: &pawtrace::config::Config, doc_dim: u32) -> LayerDigests {
    let empty = || Fnv::new().hex();
    let Some((src, ox, oy)) = pipeline::crop_to_alpha(img, cfg) else {
        return LayerDigests {
            quant: empty(),
            region_count: 0,
            regions: empty(),
            fit: empty(),
            output: empty(),
        };
    };
    let pins = pipeline::scale_pins(&cfg.pins, (ox, oy), cfg.scale, (src.width(), src.height()));

    let (alpha, regs, quant_digest) =
        if let Some(color) = raster::uniform_color(&src, cfg.alpha_threshold) {
            let alpha = raster::scale_alpha(&src, cfg);
            let mut h = Fnv::new();
            h.write(&color);
            h.write(alpha.as_raw());
            let regs = regions::from_mask(&alpha, color);
            (alpha, regs, h.hex())
        } else {
            let prep = raster::prepare(&src, cfg);
            let pal = palette::extract_palette(&prep.flat, &prep.alpha, cfg, doc_dim);
            let mut quant = palette::remap(&prep.flat, &prep.alpha, &pal);
            if cfg.color_cleanup > 0 {
                quant = palette::label_smooth(&quant, &prep.alpha, cfg.color_cleanup);
            }
            let mut h = Fnv::new();
            h.write(quant.as_raw());
            h.write(prep.alpha.as_raw());
            let regs = regions::segment_absorbed(&quant, &prep.alpha, cfg);
            (prep.alpha, regs, h.hex())
        };

    let fit = pipeline::trace_regions(&regs, &alpha, cfg, doc_dim, &pins);
    let fit_digest = trace_digest(&fit);

    let mut out = pipeline::simplify_paths(fit, cfg);
    let (sx, sy) = ((ox * cfg.scale) as f64, (oy * cfg.scale) as f64);
    for (_, paths) in &mut out {
        for p in paths {
            p.translate(sx, sy);
        }
    }

    LayerDigests {
        quant: quant_digest,
        region_count: regs.len(),
        regions: region_digest(&regs),
        fit: fit_digest,
        output: trace_digest(&out),
    }
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn slug(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[test]
fn per_stage_digests_match_baselines() {
    let bless = std::env::var_os("UPDATE_GOLDENS").is_some();
    let path = fixtures_dir().join("golden").join("digests.toml");
    let blessed: BTreeMap<String, BTreeMap<String, LayerDigests>> =
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default();

    let mut current: BTreeMap<String, BTreeMap<String, LayerDigests>> = Default::default();
    for &fixture in FIXTURES {
        let psd = fixtures_dir().join(fixture);
        assert!(psd.exists(), "missing fixture: {}", psd.display());
        let profiles = ProfileStack::load_near(&psd);
        let bytes = std::fs::read(&psd).unwrap();
        let layers = psd_import::layers(&bytes).unwrap();
        let (w, h) = (layers[0].1.width(), layers[0].1.height());
        let per_layer: BTreeMap<String, LayerDigests> = layers
            .par_iter()
            .enumerate()
            .map(|(i, (name, img))| {
                let (cfg, _) = profiles.resolve(name);
                // Keyed by document index: unique per layer and stable in
                // document order regardless of layer name.
                let key = format!("{i:03}");
                (key, layer_digests(img, &cfg, w.max(h)))
            })
            .collect();
        current.insert(slug(fixture), per_layer);
    }

    if bless {
        std::fs::write(&path, toml::to_string_pretty(&current).unwrap()).unwrap();
        return;
    }

    let mut failures = Vec::new();
    for (fixture, layers) in &current {
        let Some(base) = blessed.get(fixture) else {
            failures.push(format!("{fixture}: no blessed digests; bless with UPDATE_GOLDENS=1"));
            continue;
        };
        for (layer, d) in layers {
            match base.get(layer) {
                None => failures.push(format!("{fixture}/{layer}: not in baseline")),
                Some(b) if b == d => {}
                Some(b) => {
                    // Report the first diverged stage: each digest hashes its
                    // stage's full input chain, so the earliest mismatch names
                    // the culprit.
                    let stage = if b.quant != d.quant {
                        "quant"
                    } else if b.region_count != d.region_count || b.regions != d.regions {
                        "regions"
                    } else if b.fit != d.fit {
                        "fit"
                    } else {
                        "output"
                    };
                    failures.push(format!("{fixture}/{layer}: {stage} diverged ({b:?} != {d:?})"));
                }
            }
        }
        for layer in base.keys() {
            if !layers.contains_key(layer) {
                failures.push(format!("{fixture}/{layer}: in baseline but not produced"));
            }
        }
    }
    assert!(failures.is_empty(), "digest mismatches:\n{}", failures.join("\n"));
}
