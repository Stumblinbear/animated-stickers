//! Visual golden harness for the curated manifest (`fixtures/visual/subsets.toml`).
//!
//! For every manifest entry it renders two kinds of golden and diffs them
//! strictly against blessed baselines under `fixtures/visual/golden/<entry>/`:
//!
//!   - a per-limb stage sheet (`<layer>.png`) for each non-Fill layer, tiling
//!     that layer's crop, flattened, feature-label, quantized, region, fit, and
//!     simplified-trace rasters left to right over a transparency grid, each
//!     captioned with its stage name and divided by a vertical rule; and
//!   - the entry's final composite (`_final.png`), rendered over the same
//!     transparency grid so its alpha is legible.
//!
//! The diff fails when more than 0.1% of pixels differ at all or any channel
//! differs by more than 8; the offending render is written beside its golden
//! as `<name>.actual.png` (gitignored) for eyeballing. A secondary gate holds
//! per-entry path and anchor totals in `counts.toml` to within 2%.
//!
//! The pipeline is deterministic, so a correct regeneration matches byte for
//! byte; the budgets are headroom, not expected drift.
//!
//! Run: `cargo test --features preview --test visual`
//! Re-bless: `PAWTRACE_BLESS=1 cargo test --features preview --test visual`
//! rewrites every golden and `counts.toml`, then passes.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use image::RgbaImage;
use serde::{Deserialize, Serialize};

use common::{
    composite_over_grid, contact_sheet, counts, layer_stages, load_manifest, render, resolve_entry,
    slug, trace, visual_golden_dir,
};

/// Fraction of pixels allowed to differ at all before a diff fails.
const PIXEL_FRACTION_BUDGET: f64 = 0.001;
/// Per-channel absolute difference at or below which a pixel is treated as
/// matching for the fraction count; a single pixel above it fails outright.
const CHANNEL_TOLERANCE: u8 = 8;
/// Fraction a per-entry path or anchor total may drift from its recorded
/// value. Exact today; headroom for a future approved non-identical change.
const COUNT_TOLERANCE: f64 = 0.02;

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Counts {
    paths: usize,
    anchors: usize,
}

#[test]
fn visual_goldens_match_baselines() {
    let bless = std::env::var_os("PAWTRACE_BLESS").is_some();
    let root = visual_golden_dir();
    let counts_path = root.join("counts.toml");
    let blessed_counts: BTreeMap<String, Counts> = std::fs::read_to_string(&counts_path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default();
    let mut current_counts: BTreeMap<String, Counts> = BTreeMap::new();

    let mut failures = Vec::new();
    for entry in load_manifest() {
        let dir = root.join(&entry.name);
        if bless {
            std::fs::create_dir_all(&dir).unwrap();
        }

        let resolved = resolve_entry(&entry);
        for m in &resolved.missing {
            failures.push(format!("{}: manifest layer '{m}' resolved to no PSD layer", entry.name));
        }

        let doc_dim = resolved.w.max(resolved.h);
        for layer in &resolved.layers {
            if layer.name.ends_with("Fill") {
                continue;
            }
            let Some(stages) = layer_stages(&layer.img, &layer.cfg, doc_dim) else {
                continue;
            };
            let sheet = contact_sheet(&stages);
            let golden = dir.join(format!("{}.png", slug(&layer.name)));
            check(&format!("{}/{}", entry.name, layer.name), &golden, &sheet, bless, &mut failures);
        }

        let doc = trace(&resolved);
        let final_img = composite_over_grid(&render(&doc));
        check(&format!("{}/_final", entry.name), &dir.join("_final.png"), &final_img, bless, &mut failures);

        let (paths, anchors) = counts(&doc);
        current_counts.insert(entry.name.clone(), Counts { paths, anchors });
        if !bless {
            check_counts(&entry.name, paths, anchors, &blessed_counts, &mut failures);
        }
    }

    if bless {
        std::fs::write(&counts_path, toml::to_string_pretty(&current_counts).unwrap()).unwrap();
        return;
    }
    assert!(failures.is_empty(), "visual golden mismatches:\n{}", failures.join("\n"));
}

/// Blesses or diffs one render against its golden. On bless, writes the
/// golden. Otherwise loads the golden (a miss is a failure), compares, and on
/// mismatch writes `<golden>.actual.png` beside it and records the reason.
fn check(name: &str, golden: &Path, current: &RgbaImage, bless: bool, failures: &mut Vec<String>) {
    if bless {
        current.save(golden).unwrap();
        return;
    }
    let Ok(base) = image::open(golden) else {
        failures.push(format!("{name}: no golden {} (bless with PAWTRACE_BLESS=1)", golden.display()));
        return;
    };
    let base = base.to_rgba8();
    if base.dimensions() != current.dimensions() {
        write_actual(golden, current);
        failures.push(format!(
            "{name}: size {:?} != golden {:?}",
            current.dimensions(),
            base.dimensions()
        ));
        return;
    }
    if let Some(reason) = diff(current, &base) {
        write_actual(golden, current);
        failures.push(format!("{name}: {reason}"));
    }
}

/// The diff verdict, or `None` when the render is within budget. Fails on a
/// single channel past [`CHANNEL_TOLERANCE`] or on too many differing pixels.
fn diff(current: &RgbaImage, base: &RgbaImage) -> Option<String> {
    let mut differing = 0u64;
    let mut worst = 0u8;
    for (c, b) in current.pixels().zip(base.pixels()) {
        let mut d = 0u8;
        for ch in 0..4 {
            d = d.max(c.0[ch].abs_diff(b.0[ch]));
        }
        if d > 0 {
            differing += 1;
            worst = worst.max(d);
        }
    }
    let total = (current.width() as u64 * current.height() as u64).max(1);
    let frac = differing as f64 / total as f64;
    if worst > CHANNEL_TOLERANCE {
        return Some(format!(
            "channel diff {worst} > {CHANNEL_TOLERANCE} over {differing} px ({:.3}%)",
            frac * 100.0
        ));
    }
    if frac > PIXEL_FRACTION_BUDGET {
        return Some(format!(
            "{:.3}% of pixels differ (budget {:.1}%)",
            frac * 100.0,
            PIXEL_FRACTION_BUDGET * 100.0
        ));
    }
    None
}

/// Records a count deviation past [`COUNT_TOLERANCE`], or a missing baseline.
fn check_counts(
    entry: &str,
    paths: usize,
    anchors: usize,
    blessed: &BTreeMap<String, Counts>,
    failures: &mut Vec<String>,
) {
    let Some(base) = blessed.get(entry) else {
        failures.push(format!("{entry}: no blessed counts (bless with PAWTRACE_BLESS=1)"));
        return;
    };
    let drifted = |cur: usize, was: usize| {
        (cur as f64 - was as f64).abs() > COUNT_TOLERANCE * was as f64
    };
    if drifted(paths, base.paths) || drifted(anchors, base.anchors) {
        failures.push(format!(
            "{entry}: counts drifted past {:.0}% — paths {paths} vs {}, anchors {anchors} vs {}",
            COUNT_TOLERANCE * 100.0,
            base.paths,
            base.anchors
        ));
    }
}

/// The `<stem>.actual.png` sibling path a failing render is written to.
fn actual_path(golden: &Path) -> PathBuf {
    let stem = golden.file_stem().unwrap().to_string_lossy();
    golden.with_file_name(format!("{stem}.actual.png"))
}

fn write_actual(golden: &Path, current: &RgbaImage) {
    let _ = current.save(actual_path(golden));
}
