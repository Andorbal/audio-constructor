//! The trapezoidal (topology-preserving) state-variable filter: lowpass,
//! bandpass, highpass, and notch from one two-integrator loop.
//!
//! The structure is the analog state-variable filter discretized with the
//! trapezoidal rule and its zero-delay feedback solved exactly: V. Zavalishin,
//! "The Art of VA Filter Design" (rev. 2.1, 2018), the 2-pole SVF; written in
//! the solved form of A. Simper, "Linear Trapezoidal Integrated SVF"
//! (Cytomic technical paper, 2013). Because the state is the integrators'
//! (not past outputs, as in a direct-form biquad), the filter stays stable
//! and free of zipper noise when cutoff and resonance move every sample.
//! Everything runs in `f64` so low cutoffs at high rates stay accurate.

use crate::onepole::{prewarp, sane_rate};

/// Coefficients for one cutoff and Q.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvfCoeffs {
    /// Damping, `1 / Q`.
    k: f64,
    a1: f64,
    a2: f64,
    a3: f64,
    /// Prewarped integrator gain, kept for [`SvfCoeffs::magnitude`].
    g: f64,
}

/// Bounds on Q: below 0.5 the poles are real (no resonance, nothing gained);
/// above 1000 the filter rings for many seconds at low cutoffs.
pub const MIN_Q: f64 = 0.5;
pub const MAX_Q: f64 = 1000.0;

impl SvfCoeffs {
    /// Coefficients for `cutoff` Hz and quality `q` at `rate`. The cutoff is
    /// clamped to `[1 Hz, 0.49·rate]` and Q to `[0.5, 1000]`; non-finite
    /// values fall back to 1 kHz and 0.707. Any result is a stable filter.
    pub fn new(cutoff: f64, q: f64, rate: f64) -> SvfCoeffs {
        let g = prewarp(cutoff, 1000.0, sane_rate(rate));
        let q = if q.is_finite() {
            q
        } else {
            std::f64::consts::FRAC_1_SQRT_2
        };
        let k = 1.0 / q.clamp(MIN_Q, MAX_Q);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        SvfCoeffs { k, a1, a2, a3, g }
    }

    /// Magnitudes of (lowpass, bandpass, highpass, notch) at `freq` Hz, from
    /// the prewarped analog prototype the filter matches exactly (for tests
    /// and displays). The bandpass is normalized to 0 dB at the center.
    pub fn magnitude(&self, freq: f64, rate: f64) -> [f64; 4] {
        let w = (std::f64::consts::PI * freq / sane_rate(rate)).tan() / self.g.max(1e-300);
        // H(s) denominators at s = jw: 1 - w² + j·k·w.
        let (re, im) = (1.0 - w * w, self.k * w);
        let den = re.hypot(im).max(1e-300);
        [
            1.0 / den,
            self.k * w / den,
            w * w / den,
            (1.0 - w * w).abs() / den,
        ]
    }
}

/// One sample of every response.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SvfOut {
    pub lowpass: f32,
    /// Normalized to 0 dB at the center frequency.
    pub bandpass: f32,
    pub highpass: f32,
    /// The input minus the bandpass (lowpass plus highpass).
    pub notch: f32,
}

/// Far beyond any real signal (+240 dB) but well inside `f32`.
const RUNAWAY: f64 = 1e12;

/// The two integrator states of one channel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Svf {
    ic1: f64,
    ic2: f64,
}

impl Svf {
    /// Filter one sample. If the state runs away (a NaN input or a wild
    /// coefficient sweep) it is cleared and silence is returned.
    #[inline]
    pub fn tick(&mut self, c: &SvfCoeffs, x: f32) -> SvfOut {
        let v0 = f64::from(x);
        let v3 = v0 - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        // Comparisons are false for NaN, so this also catches non-finite values.
        if !(self.ic1.abs() < RUNAWAY && self.ic2.abs() < RUNAWAY && v0.abs() < RUNAWAY) {
            self.reset();
            return SvfOut::default();
        }
        let band = c.k * v1;
        SvfOut {
            lowpass: v2 as f32,
            bandpass: band as f32,
            highpass: (v0 - c.k * v1 - v2) as f32,
            notch: (v0 - band) as f32,
        }
    }

    /// Clear tiny state values. Call once per block so a decaying tail never
    /// lingers in the slow subnormal range.
    pub fn flush(&mut self) {
        if self.ic1.abs() < 1e-30 {
            self.ic1 = 0.0;
        }
        if self.ic2.abs() < 1e-30 {
            self.ic2 = 0.0;
        }
    }

    pub fn reset(&mut self) {
        *self = Svf::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    /// Steady-state magnitude of each response for a sine at `freq`, from a
    /// DFT projection over the second half of the run.
    fn measured(c: &SvfCoeffs, freq: f64) -> [f64; 4] {
        let mut st = Svf::default();
        let n = 96_000;
        let mut acc = [(0.0f64, 0.0f64); 4];
        for i in 0..n {
            let ph = std::f64::consts::TAU * freq * i as f64 / RATE;
            let o = st.tick(c, ph.sin() as f32);
            if i >= n / 2 {
                for (a, v) in acc
                    .iter_mut()
                    .zip([o.lowpass, o.bandpass, o.highpass, o.notch])
                {
                    a.0 += f64::from(v) * ph.cos();
                    a.1 += f64::from(v) * ph.sin();
                }
            }
        }
        acc.map(|(re, im)| 2.0 * re.hypot(im) / (n / 2) as f64)
    }

    #[test]
    fn matches_the_prewarped_analog_response() {
        for (fc, q) in [(1000.0, 0.707), (300.0, 4.0), (8000.0, 0.5), (60.0, 20.0)] {
            let c = SvfCoeffs::new(fc, q, RATE);
            for f in [fc / 4.0, fc / 1.5, fc, fc * 1.5, fc * 3.0] {
                if f > 20_000.0 {
                    continue;
                }
                let want = c.magnitude(f, RATE);
                let got = measured(&c, f);
                for r in 0..4 {
                    assert!(
                        (got[r] - want[r]).abs() < 2e-3 * want[r].max(1.0),
                        "fc {fc} q {q} f {f} response {r}: {} vs {}",
                        got[r],
                        want[r]
                    );
                }
            }
        }
    }

    #[test]
    fn known_points() {
        let c = SvfCoeffs::new(1000.0, std::f64::consts::FRAC_1_SQRT_2, RATE);
        let m = c.magnitude(1000.0, RATE);
        assert!((m[0] - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-9);
        assert!((m[1] - 1.0).abs() < 1e-9);
        assert!(m[3] < 1e-9);
        // Q sets the lowpass peak at the cutoff.
        let c = SvfCoeffs::new(1000.0, 10.0, RATE);
        assert!((c.magnitude(1000.0, RATE)[0] - 10.0).abs() < 1e-9);
    }

    #[test]
    fn bad_parameters_stay_stable() {
        for (f, q) in [
            (f64::NAN, f64::NAN),
            (0.0, 0.0),
            (1e9, 1e9),
            (-1.0, -1.0),
            (f64::INFINITY, f64::NEG_INFINITY),
        ] {
            let c = SvfCoeffs::new(f, q, RATE);
            let mut st = Svf::default();
            let mut last = 0.0f32;
            for i in 0..10_000 {
                let o = st.tick(&c, if i == 0 { 1.0 } else { 0.0 });
                last = o.lowpass;
                assert!(o.lowpass.is_finite() && o.highpass.is_finite());
            }
            assert!(last.abs() < 1.0, "{f} {q}: {last}");
        }
    }

    #[test]
    fn nan_input_clears_state() {
        let c = SvfCoeffs::new(1000.0, 2.0, RATE);
        let mut st = Svf::default();
        st.tick(&c, 1.0);
        assert_eq!(st.tick(&c, f32::NAN), SvfOut::default());
        assert_eq!(st, Svf::default());
    }

    #[test]
    fn notch_plus_bandpass_is_the_input() {
        let c = SvfCoeffs::new(700.0, 3.0, RATE);
        let mut st = Svf::default();
        for i in 0..1000 {
            let x = ((i * 7919) % 17) as f32 / 17.0 - 0.5;
            let o = st.tick(&c, x);
            assert!((o.notch + o.bandpass - x).abs() < 1e-6);
            assert!((o.lowpass + o.highpass - o.notch).abs() < 1e-6);
        }
    }
}
