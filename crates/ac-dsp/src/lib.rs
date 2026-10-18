//! Pure DSP primitives: oscillators, wavetables, colored noise, filters,
//! crossovers, delay lines, envelopes, dynamics, waveshaping and
//! oversampling, smoothing, resampling, FFTs and STFT resynthesis,
//! phase-vocoder bin shifting, pitch and onset detection, loudness,
//! breakpoint curves, scales, and a seeded PRNG. No allocation after
//! construction (loudness measurement, which runs off the audio thread,
//! excepted), no I/O, builds for `wasm32-unknown-unknown`.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod biquad;
pub mod crossover;
pub mod curve;
pub mod delay;
pub mod dynamics;
pub mod envelope;
pub mod fft;
pub mod interp;
pub mod loudness;
pub mod noise;
pub mod onset;
pub mod osc;
pub mod oversample;
pub mod pvoc;
pub mod resample;
pub mod rng;
pub mod scale;
pub mod smooth;
pub mod stft;
pub mod util;
pub mod waveshape;
pub mod wavetable;
pub mod yin;
