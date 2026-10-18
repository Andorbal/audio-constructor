//! Pure DSP primitives: oscillators, filters, delay lines, envelopes,
//! smoothing, resampling, FFTs, breakpoint curves, scales, and a seeded
//! PRNG, plus STFT analysis and resynthesis, crossovers, pitch and onset
//! detection, and phase-vocoder bin shifting. No allocation after
//! construction, no I/O, builds for `wasm32-unknown-unknown`.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod biquad;
pub mod crossover;
pub mod curve;
pub mod delay;
pub mod envelope;
pub mod fft;
pub mod interp;
pub mod onset;
pub mod osc;
pub mod pvoc;
pub mod resample;
pub mod rng;
pub mod scale;
pub mod smooth;
pub mod stft;
pub mod util;
pub mod yin;
