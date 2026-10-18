//! A four-pole transistor-ladder lowpass with a saturating feedback loop,
//! and the responses mixed from its stage outputs.
//!
//! Four trapezoidal one-poles in series with global negative feedback, the
//! zero-delay feedback solved in closed form: V. Zavalishin, "The Art of VA
//! Filter Design" (rev. 2.1, 2018), the 4-pole ladder chapter. The
//! nonlinearity is a `tanh` saturator at the feedback summing point (the
//! ladder's input stage). Rather than iterate the implicit nonlinear loop,
//! the saturator is applied to the exact *linear* solution, a cheap
//! approximation that keeps every sample a fixed amount of work. Since the
//! saturator's output is bounded and each stage is a stable one-pole, the
//! loop is bounded for any feedback amount: with feedback of 4 or more it
//! self-oscillates at the cutoff (the loop's phase reaches -180° there) and
//! the `tanh` sets the level. The `tanh` is a bounded Padé approximant
//! ([`tanh_pade`]), within 1e-4 of the real one.
//!
//! The other responses mix the input and stage outputs with binomial
//! weights, so the four poles become `(1 - H)ⁿ·Hᵐ` combinations (the
//! multimode ladder of the same book): e.g. a 24 dB/oct highpass is
//! `u − 4y₁ + 6y₂ − 4y₃ + y₄`.

use crate::onepole::{OnePole, prewarp, sane_rate};

/// Responses, in the order of the Ladder node's `mode` choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LadderMode {
    /// 24 dB/oct lowpass: the last stage.
    Lp24,
    /// 12 dB/oct lowpass: the second stage.
    Lp12,
    /// 12 dB/oct on each side: `4·H²(1 − H)²`, 0 dB at the cutoff.
    Bandpass,
    /// 24 dB/oct highpass: `(1 − H)⁴`.
    Highpass,
}

impl LadderMode {
    /// Choice names, in index order.
    pub const NAMES: &'static [&'static str] = &["lp24", "lp12", "bp", "hp"];

    /// The mode for a choice index (out-of-range indices clamp to the last).
    pub fn from_index(i: usize) -> LadderMode {
        match i {
            0 => LadderMode::Lp24,
            1 => LadderMode::Lp12,
            2 => LadderMode::Bandpass,
            _ => LadderMode::Highpass,
        }
    }

    /// This response from one sample's taps.
    #[inline]
    pub fn mix(self, t: &LadderTaps) -> f64 {
        match self {
            LadderMode::Lp24 => t.y[3],
            LadderMode::Lp12 => t.y[1],
            LadderMode::Bandpass => 4.0 * (t.y[1] - 2.0 * t.y[2] + t.y[3]),
            LadderMode::Highpass => t.u - 4.0 * t.y[0] + 6.0 * t.y[1] - 4.0 * t.y[2] + t.y[3],
        }
    }

    /// Whether resonance lowers this response's passband. Feedback divides
    /// a lowpass's DC gain by `1 + k`; highpass and bandpass passbands are
    /// where the feedback signal has died away (or adds to the peak).
    pub fn loses_passband(self) -> bool {
        matches!(self, LadderMode::Lp24 | LadderMode::Lp12)
    }
}

/// Coefficients for one cutoff and feedback amount.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LadderCoeffs {
    /// One-pole instantaneous gain `G = g / (1 + g)`.
    big_g: f64,
    /// `1 / (1 + g)`: how much of each stage's state reaches its output now.
    beta: f64,
    /// Feedback amount (4 is the self-oscillation threshold).
    k: f64,
    /// `1 / (1 + k·G⁴)`, the zero-delay feedback solution's divisor.
    inv: f64,
}

/// Feedback is kept within this; far past self-oscillation, it only makes
/// the oscillation squarer.
pub const MAX_FEEDBACK: f64 = 8.0;

impl LadderCoeffs {
    /// Coefficients for `cutoff` Hz and feedback `k` at `rate`. The cutoff
    /// is clamped to `[1 Hz, 0.49·rate]` (non-finite: 1 kHz) and `k` to
    /// `[0, 8]` (non-finite: 0).
    pub fn new(cutoff: f64, k: f64, rate: f64) -> LadderCoeffs {
        let g = prewarp(cutoff, 1000.0, sane_rate(rate));
        let k = if k.is_finite() {
            k.clamp(0.0, MAX_FEEDBACK)
        } else {
            0.0
        };
        let big_g = OnePole::gain(g);
        let g4 = big_g * big_g * big_g * big_g;
        LadderCoeffs {
            big_g,
            beta: 1.0 / (1.0 + g),
            k,
            inv: 1.0 / (1.0 + k * g4),
        }
    }

    /// The feedback amount these coefficients use.
    pub fn feedback(&self) -> f64 {
        self.k
    }
}

/// One sample's taps: the saturated ladder input `u` and the four stage
/// outputs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LadderTaps {
    pub u: f64,
    pub y: [f64; 4],
}

/// Far beyond any real signal (+240 dB) but well inside `f32`.
const RUNAWAY: f64 = 1e12;

/// Where [`tanh_pade`] stops following its polynomial and holds.
const PADE_LIMIT: f64 = 4.97;

/// A fast `tanh`: the [7/6] Padé approximant from Lambert's continued
/// fraction for `tanh`, `x(135135 + 17325x² + 378x⁴ + x⁶) / (135135 +
/// 62370x² + 3150x⁴ + 28x⁶)`, with `x` clamped to ±4.97 where it reaches
/// 0.9999994. Within 1e-4 of `tanh` everywhere, monotonic, and bounded by
/// ±1, at a fraction of the library call's cost (the ladder runs one per
/// sample per channel). NaN stays NaN.
#[inline]
pub fn tanh_pade(x: f64) -> f64 {
    let x = x.clamp(-PADE_LIMIT, PADE_LIMIT);
    let x2 = x * x;
    let num = x * (135_135.0 + x2 * (17_325.0 + x2 * (378.0 + x2)));
    let den = 135_135.0 + x2 * (62_370.0 + x2 * (3_150.0 + x2 * 28.0));
    num / den
}

/// The four stage states of one channel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ladder {
    stages: [OnePole; 4],
}

impl Ladder {
    /// Run one sample of input `x` (already scaled by any drive). A
    /// non-finite input clears the state and returns silent taps.
    #[inline]
    pub fn tick(&mut self, c: &LadderCoeffs, x: f64) -> LadderTaps {
        if !x.is_finite() || x.abs() >= RUNAWAY {
            self.reset();
            return LadderTaps::default();
        }
        let g = c.big_g;
        let [s1, s2, s3, s4] = self.stages.map(|s| s.state());
        // What the last stage would output with zero input to the ladder.
        let sum = c.beta * (((s1 * g + s2) * g + s3) * g + s4);
        // Exact linear solution of u = x − k·y4, with y4 = G⁴·u + sum,
        // then the input stage's saturation.
        let u = tanh_pade((x - c.k * sum) * c.inv);
        let mut y = [0.0; 4];
        let mut v = u;
        for (stage, out) in self.stages.iter_mut().zip(&mut y) {
            v = stage.lowpass(g, v);
            *out = v;
        }
        if !self.stages.iter().all(|s| s.is_sane(RUNAWAY)) {
            self.reset();
            return LadderTaps::default();
        }
        LadderTaps { u, y }
    }

    /// Clear tiny state values; call once per block.
    pub fn flush(&mut self) {
        for s in &mut self.stages {
            s.flush();
        }
    }

    pub fn reset(&mut self) {
        *self = Ladder::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    /// Steady-state peak of a mode's response to a small sine (small so the
    /// saturator stays linear).
    fn gain(mode: LadderMode, fc: f64, k: f64, freq: f64) -> f64 {
        let c = LadderCoeffs::new(fc, k, RATE);
        let mut st = Ladder::default();
        let amp = 1e-3;
        let n = 96_000;
        let mut peak = 0.0f64;
        for i in 0..n {
            let x = amp * (std::f64::consts::TAU * freq * i as f64 / RATE).sin();
            let y = mode.mix(&st.tick(&c, x));
            if i > n / 2 {
                peak = peak.max(y.abs());
            }
        }
        peak / amp
    }

    fn db(x: f64) -> f64 {
        20.0 * x.log10()
    }

    #[test]
    fn unresonant_responses_at_the_cutoff() {
        // Four poles at the cutoff: (1/√2)⁴ = -12 dB; two poles: -6 dB.
        assert!((db(gain(LadderMode::Lp24, 1000.0, 0.0, 1000.0)) + 12.04).abs() < 0.05);
        assert!((db(gain(LadderMode::Lp12, 1000.0, 0.0, 1000.0)) + 6.02).abs() < 0.05);
        assert!((db(gain(LadderMode::Highpass, 1000.0, 0.0, 1000.0)) + 12.04).abs() < 0.05);
        assert!(db(gain(LadderMode::Bandpass, 1000.0, 0.0, 1000.0)).abs() < 0.05);
    }

    #[test]
    fn slopes_and_passbands() {
        assert!((gain(LadderMode::Lp24, 1000.0, 0.0, 50.0) - 1.0).abs() < 0.01);
        // Two octaves up: at least 48 dB down (bilinear warping adds more).
        assert!(db(gain(LadderMode::Lp24, 1000.0, 0.0, 4000.0)) < -46.0);
        assert!(db(gain(LadderMode::Lp12, 1000.0, 0.0, 4000.0)) < -23.0);
        assert!((gain(LadderMode::Highpass, 500.0, 0.0, 15_000.0) - 1.0).abs() < 0.02);
        assert!(db(gain(LadderMode::Highpass, 2000.0, 0.0, 500.0)) < -46.0);
    }

    #[test]
    fn feedback_divides_dc_gain_and_peaks_at_the_cutoff() {
        let dc = gain(LadderMode::Lp24, 1000.0, 2.0, 20.0);
        assert!((dc - 1.0 / 3.0).abs() < 0.01, "{dc}");
        // At the cutoff, H⁴ = -1/4, so the loop divides by 1 - k/4.
        let peak = gain(LadderMode::Lp24, 1000.0, 3.0, 1000.0);
        assert!((peak - 0.25 / 0.25).abs() < 0.02, "{peak}");
    }

    #[test]
    fn self_oscillates_at_the_cutoff_and_stays_bounded() {
        let c = LadderCoeffs::new(440.0, 4.5, RATE);
        let mut st = Ladder::default();
        let mut out = Vec::new();
        for i in 0..96_000 {
            let x = if i == 0 { 0.1 } else { 0.0 };
            out.push(st.tick(&c, x).y[3]);
        }
        let tail = &out[48_000..];
        let peak = tail.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(peak > 0.05 && peak < 1.0, "{peak}");
        let crossings = tail
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        assert!((crossings as f64 - 440.0).abs() < 3.0, "{crossings}");
    }

    #[test]
    fn the_fast_tanh_is_close_bounded_and_monotonic() {
        let mut prev = -2.0;
        for i in -100_000..=100_000 {
            let x = f64::from(i) / 10_000.0;
            let t = tanh_pade(x);
            assert!((t - x.tanh()).abs() < 1e-4, "{x}");
            assert!(t.abs() < 1.0 && t >= prev, "{x}");
            prev = t;
        }
        assert_eq!(tanh_pade(0.0), 0.0);
        assert_eq!(tanh_pade(-1e300), -tanh_pade(1e300));
        assert!(tanh_pade(f64::NAN).is_nan());
    }

    #[test]
    fn extreme_inputs_stay_finite() {
        let c = LadderCoeffs::new(f64::NAN, f64::INFINITY, RATE);
        assert_eq!(c.feedback(), 0.0);
        let c = LadderCoeffs::new(1e9, 1e9, RATE);
        let mut st = Ladder::default();
        for i in 0..10_000 {
            let x = if i % 3 == 0 { 1e30 } else { -1e5 };
            let t = st.tick(&c, x);
            assert!(t.y.iter().all(|v| v.is_finite()));
        }
        assert_eq!(st.tick(&c, f64::NAN), LadderTaps::default());
        assert_eq!(st, Ladder::default());
    }
}
