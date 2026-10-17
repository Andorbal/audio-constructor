//! Second-order IIR filter sections: RBJ cookbook coefficients and a
//! transposed direct form II state.
//!
//! Coefficients follow Robert Bristow-Johnson, "Cookbook formulae for audio
//! EQ biquad filter coefficients" (the Audio EQ Cookbook). Shelves use the
//! cookbook's Q form of `alpha`, so Q = 0.707 is the steepest shelf without
//! overshoot. Transposed direct form II is used for its good numerical
//! behaviour with floating point and its two-word state (J. O. Smith,
//! "Introduction to Digital Filters", Transposed Direct-Forms). Everything
//! runs in `f64` so low cutoffs at high rates stay accurate.

use std::f64::consts::TAU;

/// Filter responses, in the order of the Biquad's `mode` choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Lowpass,
    Highpass,
    /// Constant 0 dB peak gain.
    Bandpass,
    Notch,
    Peak,
    LowShelf,
    HighShelf,
}

impl Mode {
    /// Choice names, in index order.
    pub const NAMES: &'static [&'static str] = &[
        "lowpass",
        "highpass",
        "bandpass",
        "notch",
        "peak",
        "lowshelf",
        "highshelf",
    ];

    /// The mode for a choice index (out-of-range indices clamp to the last).
    pub fn from_index(i: usize) -> Mode {
        match i {
            0 => Mode::Lowpass,
            1 => Mode::Highpass,
            2 => Mode::Bandpass,
            3 => Mode::Notch,
            4 => Mode::Peak,
            5 => Mode::LowShelf,
            _ => Mode::HighShelf,
        }
    }
}

/// Normalized coefficients (`a0` = 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coeffs {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Coeffs {
    /// A filter that passes its input unchanged.
    pub const IDENTITY: Coeffs = Coeffs {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// RBJ cookbook coefficients. Inputs are sanitized so the result is
    /// always a stable filter: `freq` is clamped to `[1 Hz, 0.49 · rate]`,
    /// `q` to `[0.01, 100]`, `gain_db` to ±48 dB; non-finite values fall back
    /// to 1 kHz, 0.707, and 0 dB.
    pub fn rbj(mode: Mode, freq: f64, q: f64, gain_db: f64, rate: f64) -> Coeffs {
        let rate = if rate.is_finite() && rate > 0.0 {
            rate
        } else {
            48_000.0
        };
        let freq = finite_or(freq, 1000.0).clamp(1.0, 0.49 * rate);
        let q = finite_or(q, std::f64::consts::FRAC_1_SQRT_2).clamp(0.01, 100.0);
        let gain_db = finite_or(gain_db, 0.0).clamp(-48.0, 48.0);

        let w0 = TAU * freq / rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let a = 10f64.powf(gain_db / 40.0);

        let (b0, b1, b2, a0, a1, a2) = match mode {
            Mode::Lowpass => {
                let b = (1.0 - cos) / 2.0;
                (b, 2.0 * b, b, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            Mode::Highpass => {
                let b = (1.0 + cos) / 2.0;
                (b, -2.0 * b, b, 1.0 + alpha, -2.0 * cos, 1.0 - alpha)
            }
            Mode::Bandpass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            Mode::Notch => (1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            Mode::Peak => (
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ),
            Mode::LowShelf => {
                let k = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + k),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - k),
                    (a + 1.0) + (a - 1.0) * cos + k,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - k,
                )
            }
            Mode::HighShelf => {
                let k = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + k),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - k),
                    (a + 1.0) - (a - 1.0) * cos + k,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - k,
                )
            }
        };
        Coeffs {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// Magnitude response at `freq` Hz (for tests and displays).
    pub fn magnitude(&self, freq: f64, rate: f64) -> f64 {
        let w = TAU * freq / rate;
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        let num_re = self.b0 + self.b1 * c1 + self.b2 * c2;
        let num_im = -(self.b1 * s1 + self.b2 * s2);
        let den_re = 1.0 + self.a1 * c1 + self.a2 * c2;
        let den_im = -(self.a1 * s1 + self.a2 * s2);
        (num_re.hypot(num_im) / den_re.hypot(den_im).max(1e-300)).min(1e12)
    }
}

impl Default for Coeffs {
    fn default() -> Self {
        Coeffs::IDENTITY
    }
}

fn finite_or(v: f64, default: f64) -> f64 {
    if v.is_finite() { v } else { default }
}

/// Far beyond any real signal (+240 dB) but well inside `f32`.
const RUNAWAY: f64 = 1e12;

/// Transposed direct form II state for one channel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tdf2 {
    s1: f64,
    s2: f64,
}

impl Tdf2 {
    /// Filter one sample. If the state runs away (a NaN input, or a wild
    /// per-sample coefficient sweep, which can make even a chain of stable
    /// filters grow) it is cleared and 0 is returned.
    #[inline]
    pub fn tick(&mut self, k: &Coeffs, x: f32) -> f32 {
        let x = f64::from(x);
        let y = k.b0 * x + self.s1;
        self.s1 = k.b1 * x - k.a1 * y + self.s2;
        self.s2 = k.b2 * x - k.a2 * y;
        // Comparisons are false for NaN, so this also catches non-finite values.
        if y.abs() < RUNAWAY && self.s1.abs() < RUNAWAY && self.s2.abs() < RUNAWAY {
            y as f32
        } else {
            self.reset();
            0.0
        }
    }

    /// Clear tiny state values. Call once per block so a decaying tail never
    /// lingers in the slow subnormal range.
    pub fn flush(&mut self) {
        if self.s1.abs() < 1e-30 {
            self.s1 = 0.0;
        }
        if self.s2.abs() < 1e-30 {
            self.s2 = 0.0;
        }
    }

    pub fn reset(&mut self) {
        *self = Tdf2::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    fn db(x: f64) -> f64 {
        20.0 * x.log10()
    }

    #[test]
    fn lowpass_and_highpass_responses() {
        let lp = Coeffs::rbj(Mode::Lowpass, 1000.0, 0.707, 0.0, RATE);
        assert!((lp.magnitude(10.0, RATE) - 1.0).abs() < 1e-3);
        assert!((db(lp.magnitude(1000.0, RATE)) + 3.01).abs() < 0.05);
        assert!(db(lp.magnitude(4000.0, RATE)) < -23.0);
        let hp = Coeffs::rbj(Mode::Highpass, 1000.0, 0.707, 0.0, RATE);
        assert!((hp.magnitude(20_000.0, RATE) - 1.0).abs() < 1e-2);
        assert!(db(hp.magnitude(250.0, RATE)) < -23.0);
    }

    #[test]
    fn peak_notch_and_shelves() {
        let pk = Coeffs::rbj(Mode::Peak, 2000.0, 1.0, 12.0, RATE);
        assert!((db(pk.magnitude(2000.0, RATE)) - 12.0).abs() < 0.01);
        assert!(db(pk.magnitude(20.0, RATE)).abs() < 0.05);
        let n = Coeffs::rbj(Mode::Notch, 2000.0, 1.0, 0.0, RATE);
        assert!(n.magnitude(2000.0, RATE) < 1e-6);
        let bp = Coeffs::rbj(Mode::Bandpass, 2000.0, 2.0, 0.0, RATE);
        assert!((bp.magnitude(2000.0, RATE) - 1.0).abs() < 1e-6);
        let ls = Coeffs::rbj(Mode::LowShelf, 500.0, 0.707, 6.0, RATE);
        assert!((db(ls.magnitude(20.0, RATE)) - 6.0).abs() < 0.1);
        assert!(db(ls.magnitude(15_000.0, RATE)).abs() < 0.1);
        let hs = Coeffs::rbj(Mode::HighShelf, 5000.0, 0.707, -6.0, RATE);
        assert!((db(hs.magnitude(20_000.0, RATE)) + 6.0).abs() < 0.2);
        assert!(db(hs.magnitude(50.0, RATE)).abs() < 0.1);
    }

    #[test]
    fn bad_parameters_give_stable_filters() {
        for mode in Mode::NAMES
            .iter()
            .enumerate()
            .map(|(i, _)| Mode::from_index(i))
        {
            for (f, q, g) in [
                (0.0, 0.0, 0.0),
                (f64::NAN, f64::INFINITY, f64::NAN),
                (1e9, 1e9, 1e9),
                (-5.0, -1.0, -1e9),
            ] {
                let k = Coeffs::rbj(mode, f, q, g, RATE);
                // Poles inside the unit circle: |a2| < 1 and |a1| < 1 + a2.
                assert!(k.a2.abs() < 1.0, "{mode:?} {k:?}");
                assert!(k.a1.abs() < 1.0 + k.a2, "{mode:?} {k:?}");
            }
        }
    }

    #[test]
    fn tdf2_matches_the_difference_equation() {
        let k = Coeffs::rbj(Mode::Lowpass, 3000.0, 2.0, 0.0, RATE);
        let x: Vec<f32> = (0..64).map(|i| ((i * 7919) % 13) as f32 - 6.0).collect();
        let mut st = Tdf2::default();
        let (mut x1, mut x2, mut y1, mut y2) = (0.0f64, 0.0, 0.0, 0.0);
        for &s in &x {
            let s64 = f64::from(s);
            let y = k.b0 * s64 + k.b1 * x1 + k.b2 * x2 - k.a1 * y1 - k.a2 * y2;
            (x2, x1, y2, y1) = (x1, s64, y1, y);
            assert!((f64::from(st.tick(&k, s)) - y).abs() < 1e-4);
        }
    }

    #[test]
    fn nan_input_clears_state() {
        let k = Coeffs::rbj(Mode::Lowpass, 1000.0, 0.707, 0.0, RATE);
        let mut st = Tdf2::default();
        st.tick(&k, 1.0);
        assert_eq!(st.tick(&k, f32::NAN), 0.0);
        assert_eq!(st, Tdf2::default());
    }
}
