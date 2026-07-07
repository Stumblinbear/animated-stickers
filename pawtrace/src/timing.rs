//! Per-stage wall-clock accumulators, summed across layers. Zero overhead
//! beyond an atomic add per stage per layer; printing is on demand.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub struct Stage(&'static str, AtomicU64);

pub static PREP: Stage = Stage::new("raster prep (resize+flatten)");
pub static PALETTE: Stage = Stage::new("palette extract");
pub static REMAP: Stage = Stage::new("remap");
pub static ABSORB: Stage = Stage::new("absorb transitions");
pub static SEGMENT: Stage = Stage::new("segment regions");
pub static SHAPES: Stage = Stage::new("region shapes");
pub static TRACE: Stage = Stage::new("trace + fit");

impl Stage {
    const fn new(name: &'static str) -> Self {
        Stage(name, AtomicU64::new(0))
    }

    /// Runs `f`, adding its wall time to this stage's total.
    pub fn time<T>(&self, f: impl FnOnce() -> T) -> T {
        let t = Instant::now();
        let out = f();
        self.1.fetch_add(t.elapsed().as_micros() as u64, Ordering::Relaxed);
        out
    }
}

/// Prints all stage totals to stderr, slowest first.
pub fn report() {
    let mut rows: Vec<(&str, u64)> = [&PREP, &PALETTE, &REMAP, &ABSORB, &SEGMENT, &SHAPES, &TRACE]
        .iter()
        .map(|s| (s.0, s.1.load(Ordering::Relaxed)))
        .collect();
    rows.sort_by_key(|&(_, us)| std::cmp::Reverse(us));
    let total: u64 = rows.iter().map(|&(_, us)| us).sum();
    eprintln!("--- stage timings (all layers)");
    for (name, us) in rows {
        eprintln!(
            "  {:32} {:7.1} ms  {:4.1}%",
            name,
            us as f64 / 1000.0,
            100.0 * us as f64 / total.max(1) as f64
        );
    }
    eprintln!("  {:32} {:7.1} ms", "total (in stages)", total as f64 / 1000.0);
}
