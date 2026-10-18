//! Pure DSP primitives: oscillators, filters (biquad, state-variable,
//! ladder, one-pole, allpass), delay lines, envelopes, smoothing,
//! resampling, FFTs, cepstral envelopes, breakpoint curves, scales, and a
//! seeded PRNG. No allocation after
//! construction, no I/O, builds for `wasm32-unknown-unknown`.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod allpass;
pub mod biquad;
pub mod cepstrum;
pub mod curve;
pub mod delay;
pub mod envelope;
pub mod fft;
pub mod interp;
pub mod ladder;
pub mod onepole;
pub mod osc;
pub mod resample;
pub mod rng;
pub mod scale;
pub mod smooth;
pub mod svf;
pub mod util;
