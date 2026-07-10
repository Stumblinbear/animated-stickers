//! Perceptual golden test over the curated manifest (`fixtures/visual/subsets.toml`).
//! For each entry it traces every manifest layer exactly as `src/main.rs`
//! does, rasterizes the assembled composite at native size, and compares
//! against the entry's `_final.png` with an OKLab ΔE metric. A separate stats
//! guard catches anchor explosions a pixel diff cannot see.
//!
//! The `_final.png` baselines are owned by the visual harness (`tests/visual.rs`);
//! this test reads them and blesses only `fixtures/golden/stats.toml`.
//!
//! Run: `cargo test --features preview --test golden`
//! Re-bless: bless the visual harness first (`PAWTRACE_BLESS=1`), then
//! `UPDATE_GOLDENS=1 cargo test --features preview --test golden` rewrites
//! `stats.toml`.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use image::{Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

use pawtrace::config::srgb_to_oklab;

use common::{
    composite_over_grid, counts, load_manifest, render, resolve_entry, trace, visual_golden_dir,
};

/// Per-pixel OKLab ΔE at or above which a pixel counts as visibly changed.
/// Below this is anti-alias jitter along edges; a dropped outline or a
/// re-colored region lands far above it.
const VISIBLE_DELTA_E: f32 = 0.06;
/// Mean ΔE budget over the compared pixel set. The pipeline is deterministic,
/// so an unregressed render matches its baseline exactly at mean 0.
const MEAN_DELTA_E_BUDGET: f32 = 0.01;
/// Budget on the fraction of compared pixels that are visibly changed.
const VISIBLE_FRACTION_BUDGET: f32 = 0.02;
/// Stats may grow this much over the blessed totals before failing. Anchor
/// explosions multiply the count several-fold, well past this.
const STATS_GROWTH: f64 = 1.25;

fn stats_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join("golden").join("stats.toml")
}

/// Path count and cubic-segment count summed across every layer of an entry.
#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct DocStats {
    paths: usize,
    cubics: usize,
}

/// Straight sRGB of a premultiplied-RGBA pixel over white, so an alpha drop
/// reads as a large color change instead of vanishing.
fn over_white(p: &Rgba<u8>) -> [u8; 3] {
    let bg = 255 - p.0[3];
    [
        p.0[0].saturating_add(bg),
        p.0[1].saturating_add(bg),
        p.0[2].saturating_add(bg),
    ]
}

fn delta_e(a: &Rgba<u8>, b: &Rgba<u8>) -> f32 {
    let la = srgb_to_oklab(over_white(a));
    let lb = srgb_to_oklab(over_white(b));
    let d0 = la[0] - lb[0];
    let d1 = la[1] - lb[1];
    let d2 = la[2] - lb[2];
    (d0 * d0 + d1 * d1 + d2 * d2).sqrt()
}

/// Mean ΔE and visibly-changed fraction over pixels painted in either image.
fn compare(current: &RgbaImage, baseline: &RgbaImage) -> (f32, f32) {
    let mut sum = 0.0f64;
    let mut compared = 0u64;
    let mut visible = 0u64;
    for (c, b) in current.pixels().zip(baseline.pixels()) {
        if c.0[3] == 0 && b.0[3] == 0 {
            continue;
        }
        let d = delta_e(c, b);
        sum += d as f64;
        compared += 1;
        if d >= VISIBLE_DELTA_E {
            visible += 1;
        }
    }
    if compared == 0 {
        return (0.0, 0.0);
    }
    ((sum / compared as f64) as f32, visible as f32 / compared as f32)
}

#[test]
fn golden_documents_match_baselines() {
    let bless = std::env::var_os("UPDATE_GOLDENS").is_some();
    let stats_path = stats_path();
    // Bless rebuilds the map from scratch so entries dropped from the manifest
    // leave no stale key behind.
    let mut blessed_stats: BTreeMap<String, DocStats> = if bless {
        BTreeMap::new()
    } else {
        std::fs::read_to_string(&stats_path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    };

    let mut failures = Vec::new();
    for entry in load_manifest() {
        let resolved = resolve_entry(&entry);
        for m in &resolved.missing {
            failures.push(format!("{}: manifest layer '{m}' resolved to no PSD layer", entry.name));
        }

        let doc = trace(&resolved);
        // The `_final.png` baseline is composited over the transparency grid by
        // the visual harness, so match it here to compare like for like.
        let current = composite_over_grid(&render(&doc));
        let (paths, cubics) = counts(&doc);
        let stats = DocStats { paths, cubics };

        if bless {
            blessed_stats.insert(entry.name.clone(), stats);
            continue;
        }

        let png = visual_golden_dir().join(&entry.name).join("_final.png");
        let baseline = match image::open(&png) {
            Ok(img) => img.to_rgba8(),
            Err(e) => {
                failures.push(format!(
                    "{}: no baseline {} ({e}); bless the visual harness with PAWTRACE_BLESS=1",
                    entry.name,
                    png.display()
                ));
                continue;
            }
        };
        if baseline.dimensions() != current.dimensions() {
            failures.push(format!(
                "{}: size {:?} != baseline {:?}",
                entry.name,
                current.dimensions(),
                baseline.dimensions()
            ));
            continue;
        }

        let (mean, visible) = compare(&current, &baseline);
        if mean > MEAN_DELTA_E_BUDGET || visible > VISIBLE_FRACTION_BUDGET {
            failures.push(format!(
                "{}: mean ΔE {mean:.4} (budget {MEAN_DELTA_E_BUDGET}), visible {:.3}% (budget {:.1}%)",
                entry.name,
                visible * 100.0,
                VISIBLE_FRACTION_BUDGET * 100.0
            ));
        }

        match blessed_stats.get(&entry.name) {
            None => failures.push(format!("{}: no blessed stats; bless with UPDATE_GOLDENS=1", entry.name)),
            Some(blessed) => {
                let grew = |cur: usize, base: usize| cur as f64 > base as f64 * STATS_GROWTH;
                if grew(stats.paths, blessed.paths) || grew(stats.cubics, blessed.cubics) {
                    failures.push(format!(
                        "{}: stats grew past {STATS_GROWTH}x — paths {} vs {}, cubics {} vs {}",
                        entry.name, stats.paths, blessed.paths, stats.cubics, blessed.cubics
                    ));
                }
            }
        }
    }

    if bless {
        std::fs::write(&stats_path, toml::to_string_pretty(&blessed_stats).unwrap()).unwrap();
        return;
    }
    assert!(failures.is_empty(), "golden mismatches:\n{}", failures.join("\n"));
}
