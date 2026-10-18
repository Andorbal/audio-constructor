//! The topology-preserving (trapezoidal) one-pole filter and the cutoff
//! prewarping shared by the zero-delay-feedback filters.
//!
//! From V. Zavalishin, "The Art of VA Filter Design" (rev. 2.1, 2018), the
//! TPT one-pole: an analog RC integrator discretized with the trapezoidal
//! rule, its cutoff prewarped with `g = tan(π·fc/fs)` so the digital -3 dB
//! point lands exactly on `fc`. Its state is the integrator's, which keeps
//! it stable and free of zipper noise when the cutoff moves every sample.

use std::f64::consts::PI;

/// The highest cutoff the filters accept, as a fraction of the sample rate.
/// `tan` grows without bound towards Nyquist; 0.49 keeps `g` near 32.
pub const MAX_CUTOFF_RATIO: f64 = 0.49;

/// A usable sample rate: non-finite or non-positive rates fall back to
/// 48 kHz so coefficient math never divides by zero.
#[inline]
pub fn sane_rate(rate: f64) -> f64 {
    if rate.is_finite() && rate > 0.0 {
        rate
    } else {
        48_000.0
    }
}

/// The prewarped integrator gain `g = tan(π·fc/fs)`. `cutoff` is clamped to
/// `[1 Hz, 0.49·rate]`; a non-finite cutoff uses `fallback` (also clamped).
#[inline]
pub fn prewarp(cutoff: f64, fallback: f64, rate: f64) -> f64 {
    let rate = sane_rate(rate);
    let fc = if cutoff.is_finite() { cutoff } else { fallback };
    let fc = if fc.is_finite() { fc } else { 1000.0 };
    let fc = fc.clamp(1.0, MAX_CUTOFF_RATIO * rate);
    (PI * fc / rate).tan()
}

/// One trapezoidal one-pole section. `big_g` is `g / (1 + g)` from
/// [`prewarp`] (see [`OnePole::gain`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OnePole {
    s: f64,
}

impl OnePole {
    /// The instantaneous gain `G = g / (1 + g)` for a prewarped `g`.
    #[inline]
    pub fn gain(g: f64) -> f64 {
        g / (1.0 + g)
    }

    /// Lowpass one sample.
    #[inline]
    pub fn lowpass(&mut self, big_g: f64, x: f64) -> f64 {
        let v = (x - self.s) * big_g;
        let y = v + self.s;
        self.s = y + v;
        y
    }

    /// Highpass one sample: the input minus the lowpass, which puts an exact
    /// zero at DC.
    #[inline]
    pub fn highpass(&mut self, big_g: f64, x: f64) -> f64 {
        x - self.lowpass(big_g, x)
    }

    /// The integrator state (what the output would be with no input change),
    /// for zero-delay feedback solutions that need it.
    #[inline]
    pub fn state(&self) -> f64 {
        self.s
    }

    /// True while the state is finite and within `limit`.
    #[inline]
    pub fn is_sane(&self, limit: f64) -> bool {
        // Comparisons are false for NaN, so this also catches non-finite values.
        self.s.abs() < limit
    }

    /// Clear a tiny state so a decaying tail never lingers in the slow
    /// subnormal range. Call once per block.
    #[inline]
    pub fn flush(&mut self) {
        if self.s.abs() < 1e-30 {
            self.s = 0.0;
        }
    }

    pub fn reset(&mut self) {
        self.s = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    /// Steady-state gain of a sine through `f`.
    fn gain_at(freq: f64, mut f: impl FnMut(f64) -> f64) -> f64 {
        let n = 48_000;
        let mut peak = 0.0f64;
        for i in 0..n {
            let y = f((std::f64::consts::TAU * freq * i as f64 / RATE).sin());
            if i > n / 2 {
                peak = peak.max(y.abs());
            }
        }
        peak
    }

    #[test]
    fn cutoff_is_minus_three_db_exactly() {
        let g = OnePole::gain(prewarp(1000.0, 1000.0, RATE));
        let mut lp = OnePole::default();
        let at = gain_at(1000.0, |x| lp.lowpass(g, x));
        assert!((at - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-3, "{at}");
        let mut hp = OnePole::default();
        let at = gain_at(1000.0, |x| hp.highpass(g, x));
        assert!((at - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-3, "{at}");
    }

    #[test]
    fn highpass_removes_dc() {
        let g = OnePole::gain(prewarp(10.0, 10.0, RATE));
        let mut hp = OnePole::default();
        let mut y = 1.0;
        for _ in 0..48_000 {
            y = hp.highpass(g, 1.0);
        }
        assert!(y.abs() < 1e-12, "{y}");
    }

    #[test]
    fn prewarp_is_clamped() {
        let top = prewarp(1e9, 1000.0, RATE);
        assert!((top - (PI * 0.49).tan()).abs() < 1e-9);
        assert_eq!(prewarp(f64::NAN, 1000.0, RATE), prewarp(1000.0, 1.0, RATE));
        assert_eq!(prewarp(-5.0, 1.0, RATE), prewarp(1.0, 1.0, RATE));
        assert!(prewarp(1000.0, 1000.0, f64::NAN).is_finite());
    }
}
