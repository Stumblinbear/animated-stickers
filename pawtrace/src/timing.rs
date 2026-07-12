//! Per-stage wall-clock accumulators, summed across layers. Costs one atomic
//! add per timed scope; printing is on demand.
//!
//! Stages come in two kinds. The top-level ones partition the pipeline and sum
//! to its total. The detail ones nest inside a top-level parent and run on
//! every rayon thread at once, so they are attribution within that parent, not
//! elapsed time: a detail group can sum past its parent, and past wall clock.

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
pub static SIMPLIFY: Stage = Stage::new("simplify");

pub static FIELD: Stage = Stage::new("  simplify: cross-field build");
pub static GUARD: Stage = Stage::new("  simplify: guard construction");
pub static SWEEP: Stage = Stage::new("  simplify: anchor sweep");
pub static SEAM_WALK: Stage = Stage::new("  seams: walk + densify");
pub static SEAM_MATCH: Stage = Stage::new("  seams: occurrence match");
pub static SEAM_RUNS: Stage = Stage::new("  seams: shared runs");
pub static SEAM_ASM: Stage = Stage::new("  seams: contour assembly");

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
    let read = |stages: &[&Stage]| -> Vec<(&'static str, u64)> {
        let mut rows: Vec<(&'static str, u64)> = stages
            .iter()
            .map(|s| (s.0, s.1.load(Ordering::Relaxed)))
            .collect();
        rows.sort_by_key(|&(_, us)| std::cmp::Reverse(us));
        rows
    };
    let row = |name: &str, us: u64, of: u64| {
        eprintln!(
            "  {:32} {:7.1} ms  {:4.1}%",
            name,
            us as f64 / 1000.0,
            100.0 * us as f64 / of.max(1) as f64
        );
    };

    let top = read(&[&PREP, &PALETTE, &REMAP, &ABSORB, &SEGMENT, &SHAPES, &TRACE, &SIMPLIFY]);
    let total: u64 = top.iter().map(|&(_, us)| us).sum();
    eprintln!("--- stage timings (all layers)");
    for (name, us) in top {
        row(name, us, total);
    }
    eprintln!("  {:32} {:7.1} ms", "total (in stages)", total as f64 / 1000.0);

    // These sit inside SIMPLIFY or TRACE and accumulate on every rayon thread
    // at once, so a group's sum can exceed its parent and wall time. Read
    // them as attribution within the named parent, not as elapsed time.
    let groups: [(&str, &[&Stage], u64); 2] = [
        ("within simplify:", &[&FIELD, &GUARD, &SWEEP], SIMPLIFY.1.load(Ordering::Relaxed)),
        (
            "within trace + fit:",
            &[&SEAM_WALK, &SEAM_MATCH, &SEAM_RUNS, &SEAM_ASM],
            TRACE.1.load(Ordering::Relaxed),
        ),
    ];
    if groups.iter().any(|&(_, _, parent)| parent > 0) {
        eprintln!("--- detail (nested; sums across threads)");
        for (label, stages, parent) in groups {
            if parent == 0 {
                continue;
            }
            eprintln!("  {label}");
            for (name, us) in read(stages) {
                row(name, us, parent);
            }
        }
    }
}
