//! Dynamics building blocks: level detection and the static gain curve of
//! compressors and expanders.
//!
//! Both follow D. Giannoulis, M. Massberg, and J. D. Reiss, "Digital
//! Dynamic Range Compressor Design — A Tutorial and Analysis", JAES 60(6),
//! 2012:
//!
//! - the *smooth branching peak detector*, a one-pole smoother whose
//!   coefficient switches between attack and release depending on whether
//!   the input is above or below the current level, with the paper's time
//!   constant `α = e^(−1 / (τ·fs))` (the level covers 1 − 1/e ≈ 63 % of a
//!   step in `τ`); the RMS detector is the same smoother on squared input;
//! - the *soft-knee gain computer*: unity below the knee, slope `1/R` above
//!   it, and a quadratic blend across a knee of width `W` dB centred on the
//!   threshold. The downward expander mirrors it below the threshold
//!   (slope `R` there).

/// Smoothing coefficient for time constant `seconds` at `rate`:
/// `e^(−1 / (τ·fs))`. Zero (instant) for zero, negative, or non-finite
/// times.
pub fn time_coef(seconds: f64, rate: f64) -> f64 {
    let n = seconds * rate;
    if n.is_finite() && n > 0.0 {
        (-1.0 / n).exp()
    } else {
        0.0
    }
}

/// The smooth branching peak detector's state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Detector {
    y: f64,
}

/// Levels below this are flushed to zero so a decaying detector never
/// lingers in the slow subnormal range.
const FLUSH: f64 = 1e-30;

impl Detector {
    pub fn new() -> Self {
        Self::default()
    }

    /// The current level.
    pub fn level(&self) -> f64 {
        self.y
    }

    /// Feed one non-negative input (`|x|` for peak detection, `x²` for
    /// RMS) with attack and release coefficients from [`time_coef`].
    #[inline]
    pub fn tick(&mut self, x: f64, attack: f64, release: f64) -> f64 {
        let a = if x > self.y { attack } else { release };
        self.y = a * self.y + (1.0 - a) * x;
        if self.y < FLUSH {
            self.y = 0.0;
        }
        self.y
    }

    pub fn reset(&mut self) {
        self.y = 0.0;
    }
}

/// Which side of the threshold the gain computer acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Turn levels above the threshold down (compressor, limiter).
    Compress,
    /// Turn levels below the threshold down further (downward expander,
    /// gate).
    Expand,
}

impl Mode {
    pub const NAMES: &'static [&'static str] = &["compress", "expand"];

    pub fn from_index(i: usize) -> Mode {
        if i == 0 { Mode::Compress } else { Mode::Expand }
    }
}

/// Gain change in dB (zero or negative) for an input `level` in dB.
///
/// `ratio` should be at least 1 (callers clamp it); `knee` is the knee's
/// full width in dB (0 for a hard knee).
#[inline]
pub fn gain_change_db(level: f64, threshold: f64, ratio: f64, knee: f64, mode: Mode) -> f64 {
    let d = level - threshold;
    let half = 0.5 * knee;
    match mode {
        Mode::Compress => {
            let slope = 1.0 / ratio - 1.0;
            if d <= -half {
                0.0
            } else if d < half {
                let u = d + half;
                slope * u * u / (2.0 * knee)
            } else {
                slope * d
            }
        }
        Mode::Expand => {
            let slope = ratio - 1.0;
            if d >= half {
                0.0
            } else if d > -half {
                let u = d - half;
                -slope * u * u / (2.0 * knee)
            } else {
                slope * d
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_constant_reaches_63_percent() {
        let rate = 48_000.0;
        let a = time_coef(0.01, rate);
        let mut d = Detector::new();
        for _ in 0..480 {
            d.tick(1.0, a, a);
        }
        assert!((d.level() - (1.0 - (-1f64).exp())).abs() < 1e-3);
        assert_eq!(time_coef(0.0, rate), 0.0);
        assert_eq!(time_coef(f64::NAN, rate), 0.0);
    }

    #[test]
    fn attack_and_release_branch() {
        let (fast, slow) = (time_coef(0.0001, 48_000.0), time_coef(1.0, 48_000.0));
        let mut d = Detector::new();
        for _ in 0..100 {
            d.tick(1.0, fast, slow);
        }
        assert!(d.level() > 0.99);
        for _ in 0..100 {
            d.tick(0.0, fast, slow);
        }
        assert!(d.level() > 0.99, "release is slow");
        d.reset();
        assert_eq!(d.level(), 0.0);
    }

    #[test]
    fn compressor_curve() {
        let c = |x| gain_change_db(x, -20.0, 4.0, 0.0, Mode::Compress);
        assert_eq!(c(-30.0), 0.0);
        assert_eq!(c(-20.0), 0.0);
        assert_eq!(c(0.0), -15.0);
        // Soft knee: continuous at both edges and halfway at the threshold.
        let s = |x| gain_change_db(x, -20.0, 4.0, 10.0, Mode::Compress);
        assert_eq!(s(-25.0), 0.0);
        assert!((s(-15.0) - (-3.75)).abs() < 1e-12);
        assert!((s(-20.0) - (-0.75 * 25.0 / 20.0)).abs() < 1e-12);
        // Above the knee it matches the hard knee.
        assert!((s(-5.0) - c(-5.0)).abs() < 1e-12);
    }

    #[test]
    fn expander_curve() {
        let e = |x| gain_change_db(x, -40.0, 2.0, 0.0, Mode::Expand);
        assert_eq!(e(-30.0), 0.0);
        assert_eq!(e(-50.0), -10.0);
        let s = |x| gain_change_db(x, -40.0, 2.0, 6.0, Mode::Expand);
        assert_eq!(s(-37.0), 0.0);
        assert!((s(-43.0) - (-3.0)).abs() < 1e-12);
        assert!(s(-40.0) < 0.0);
    }
}
