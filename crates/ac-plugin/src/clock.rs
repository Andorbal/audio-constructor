//! A monotonic clock for the engine's load meter and per-node timing.

use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();

/// Start the clock (off the audio thread, so reading it never initializes).
pub(crate) fn init() {
    START.get_or_init(Instant::now);
}

/// Nanoseconds since [`init`]; 0 before it.
pub(crate) fn now_ns() -> u64 {
    START.get().map_or(0, |s| {
        u64::try_from(s.elapsed().as_nanos()).unwrap_or(u64::MAX)
    })
}
