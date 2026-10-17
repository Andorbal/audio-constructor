//! The monotonic clock shared by the device callbacks, the MIDI input threads,
//! and the engine's load measurement.
//!
//! MIDI timestamps and callback times must come from the same clock for the
//! offset mapping in [`crate::midi`] to mean anything, so everything in this
//! crate reads time through [`now_ns`].

use std::sync::OnceLock;
use std::time::Instant;

static ORIGIN: OnceLock<Instant> = OnceLock::new();

/// Fix the clock's origin. Call before starting streams so the first read
/// on an audio thread is a plain atomic load rather than the one-time
/// initialization.
pub fn init() {
    let _ = ORIGIN.get_or_init(Instant::now);
}

/// Nanoseconds since [`init`] (or the first call). Real-time safe after
/// [`init`]: no allocation, no locks; one vDSO clock read.
pub fn now_ns() -> u64 {
    let origin = ORIGIN.get_or_init(Instant::now);
    u64::try_from(origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic() {
        init();
        let a = now_ns();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = now_ns();
        assert!(b >= a + 1_000_000, "{a} {b}");
    }
}
