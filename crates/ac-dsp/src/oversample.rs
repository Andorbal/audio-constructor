//! 2× oversampling for nonlinear processing: upsample with a halfband
//! interpolator, run a function at twice the rate, and decimate with the
//! same halfband filter, so harmonics between the original Nyquist
//! frequency and 1.5× it are filtered out instead of folding back.
//!
//! The filter is a 51-tap linear-phase halfband FIR designed with the
//! Kaiser window method (A. V. Oppenheim and R. W. Schafer, "Discrete-Time
//! Signal Processing", 3rd ed., §7.6): an ideal half-band sinc windowed
//! with β = 7.86, for about 80 dB of stopband attenuation, a passband to
//! 0.4 × the original rate (19.2 kHz at 48 kHz) and a stopband from 0.6 ×
//! it. Every other tap of a halfband filter is zero, so both directions
//! run as polyphase pairs: one branch is a 26-tap FIR and the other a pure
//! delay (R. E. Crochiere and L. R. Rabiner, "Multirate Digital Signal
//! Processing", 1983, §4.3).
//!
//! The round trip delays the signal by [`LATENCY`] samples at the original
//! rate. Everything here is allocation-free after construction.

use std::sync::OnceLock;

/// Taps of the halfband filter (`4·13 − 1`).
pub const TAPS: usize = 51;
/// Nonzero side taps (the FIR branch of each polyphase pair).
const BRANCH: usize = TAPS.div_ceil(2);
/// Samples of delay at the original rate for an upsample + downsample
/// round trip: the filter's group delay of 25 samples at the doubled
/// rate, twice.
pub const LATENCY: u32 = (TAPS as u32 - 1) / 2;
/// Delay of the pure-delay branch, in samples of its own rate.
const MID: usize = (TAPS - 1) / 4;

const BETA: f64 = 7.857;

/// The even-indexed taps `h[0], h[2], …, h[50]` (the odd-indexed ones are
/// zero except the centre, 0.5), normalized so they sum to exactly 0.5.
fn branch_taps() -> &'static [f64; BRANCH] {
    static TAPS_CELL: OnceLock<[f64; BRANCH]> = OnceLock::new();
    TAPS_CELL.get_or_init(|| {
        let c = (TAPS - 1) as f64 / 2.0;
        let i0b = bessel_i0(BETA);
        let mut h = [0.0; BRANCH];
        for (j, tap) in h.iter_mut().enumerate() {
            let n = (2 * j) as f64;
            let x = (n - c) / 2.0;
            let sinc = (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x);
            let r = 2.0 * n / (TAPS - 1) as f64 - 1.0;
            let w = bessel_i0(BETA * (1.0 - r * r).max(0.0).sqrt()) / i0b;
            *tap = 0.5 * sinc * w;
        }
        let sum: f64 = h.iter().sum();
        for tap in &mut h {
            *tap *= 0.5 / sum;
        }
        h
    })
}

/// Zeroth-order modified Bessel function of the first kind (power series).
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..64 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// A short history, newest first, readable as one contiguous slice
/// (every sample is stored twice so the slice never wraps).
#[derive(Clone, Copy, Debug, PartialEq)]
struct History<const N: usize, const N2: usize> {
    buf: [f32; N2],
    pos: usize,
}

impl<const N: usize, const N2: usize> History<N, N2> {
    const fn new() -> Self {
        History {
            buf: [0.0; N2],
            pos: 0,
        }
    }

    #[inline]
    fn push(&mut self, x: f32) {
        self.pos = if self.pos == 0 { N - 1 } else { self.pos - 1 };
        self.buf[self.pos] = x;
        self.buf[self.pos + N] = x;
    }

    /// `[0]` is the newest sample, `[k]` the one `k` pushes ago.
    #[inline]
    fn recent(&self) -> &[f32] {
        &self.buf[self.pos..self.pos + N]
    }

    fn clear(&mut self) {
        *self = Self::new();
    }
}

#[inline]
fn dot(h: &[f64; BRANCH], x: &[f32]) -> f64 {
    h.iter().zip(x).map(|(a, b)| a * f64::from(*b)).sum()
}

/// One channel of 2× oversampling.
#[derive(Clone, Debug)]
pub struct Oversampler2x {
    h: &'static [f64; BRANCH],
    /// Upsampler input history.
    up: History<BRANCH, { 2 * BRANCH }>,
    /// Decimator input history, even (first) and odd (second) samples.
    even: History<BRANCH, { 2 * BRANCH }>,
    odd: History<{ MID + 2 }, { 2 * (MID + 2) }>,
}

impl Default for Oversampler2x {
    fn default() -> Self {
        Self::new()
    }
}

impl Oversampler2x {
    pub fn new() -> Self {
        Oversampler2x {
            h: branch_taps(),
            up: History::new(),
            even: History::new(),
            odd: History::new(),
        }
    }

    /// Upsample `x`, apply `f` to both samples at the doubled rate, and
    /// return the decimated result ([`LATENCY`] samples late).
    #[inline]
    pub fn process(&mut self, x: f32, mut f: impl FnMut(f32) -> f32) -> f32 {
        self.up.push(x);
        let recent = self.up.recent();
        // Interpolator (gain 2): the FIR branch, then the delay branch.
        let first = (2.0 * dot(self.h, recent)) as f32;
        let second = recent[MID];
        self.even.push(f(first));
        self.odd.push(f(second));
        // Decimator: FIR on the first samples plus the centre tap (0.5) on
        // the second ones, `MID + 1` steps back.
        (dot(self.h, self.even.recent()) + 0.5 * f64::from(self.odd.recent()[MID + 1])) as f32
    }

    pub fn reset(&mut self) {
        self.up.clear();
        self.even.clear();
        self.odd.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, rate: f64, n: usize, amp: f64) -> Vec<f32> {
        (0..n)
            .map(|i| (amp * (std::f64::consts::TAU * freq * i as f64 / rate).sin()) as f32)
            .collect()
    }

    fn run(x: &[f32], f: impl Fn(f32) -> f32 + Copy) -> Vec<f32> {
        let mut o = Oversampler2x::new();
        x.iter().map(|&v| o.process(v, f)).collect()
    }

    fn tone(x: &[f32], freq: f64, rate: f64) -> f64 {
        let w = std::f64::consts::TAU * freq / rate;
        let (mut re, mut im) = (0.0, 0.0);
        for (i, v) in x.iter().enumerate() {
            re += f64::from(*v) * (w * i as f64).cos();
            im += f64::from(*v) * (w * i as f64).sin();
        }
        2.0 * (re * re + im * im).sqrt() / x.len() as f64
    }

    #[test]
    fn identity_is_a_pure_delay_in_the_passband() {
        let rate = 48_000.0;
        for freq in [100.0, 1_000.0, 10_000.0, 18_000.0] {
            let x = sine(freq, rate, 4_000, 0.5);
            let y = run(&x, |v| v);
            let d = LATENCY as usize;
            let err = x[..3_000]
                .iter()
                .zip(&y[d..d + 3_000])
                .skip(100)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max);
            assert!(err < 2e-3, "{freq} Hz: {err}");
        }
        // DC passes at exactly unity.
        let y = run(&[1.0; 200], |v| v);
        assert!((y[150] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn saturation_aliases_less_than_without_oversampling() {
        let rate = 48_000.0;
        let x = sine(7_000.0, rate, 48_000, 2.0);
        let sat = |v: f32| v.tanh();
        let naive: Vec<f32> = x.iter().map(|&v| sat(v)).collect();
        let over = run(&x, sat);
        // The 5th harmonic (35 kHz) folds to 13 kHz and the 7th (49 kHz)
        // to 1 kHz; at the doubled rate both are filtered out.
        for alias in [13_000.0, 1_000.0] {
            let (a, b) = (tone(&naive, alias, rate), tone(&over, alias, rate));
            assert!(a > 1e-3, "{alias} Hz: naive {a}");
            assert!(b < 0.05 * a, "{alias} Hz: {b} vs naive {a}");
        }
        // The fundamental and the 3rd harmonic (21 kHz is above the
        // passband, so only the fundamental is compared) are kept.
        let (f0, f1) = (tone(&over, 7_000.0, rate), tone(&naive, 7_000.0, rate));
        assert!((f0 - f1).abs() < 0.01, "{f0} vs {f1}");
    }

    #[test]
    fn reset_clears_history() {
        let mut o = Oversampler2x::new();
        for _ in 0..100 {
            o.process(1.0, |v| v);
        }
        o.reset();
        assert_eq!(o.process(0.0, |v| v), 0.0);
        assert_eq!(LATENCY, 25);
        assert!((branch_taps().iter().sum::<f64>() - 0.5).abs() < 1e-15);
    }
}
