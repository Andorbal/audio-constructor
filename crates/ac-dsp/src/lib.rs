//! Pure DSP primitives: oscillators, filters, delay lines, envelopes,
//! smoothing, resampling, and a seeded PRNG. No allocation after
//! construction, no I/O, builds for `wasm32-unknown-unknown`.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod osc;
pub mod util;
