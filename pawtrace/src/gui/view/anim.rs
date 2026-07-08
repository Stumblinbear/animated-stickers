//! Shared timing for the processing animations: a monotonic epoch and the
//! loop-phase helper every animated widget derives its motion from.

use std::sync::LazyLock;
use std::time::Instant;

static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

/// The fraction `0.0..1.0` through a `period`-second loop at instant `now`.
/// All widgets share one epoch, so animations of equal period stay in phase.
pub fn phase(now: Instant, period: f32) -> f32 {
    let t = now.saturating_duration_since(*EPOCH).as_secs_f32();
    (t / period).rem_euclid(1.0)
}
