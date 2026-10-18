//! The Schroeder allpass: a delay line with feedback and feedforward of
//! equal and opposite gain, which smears a signal in time without changing
//! its magnitude spectrum.
//!
//! M. R. Schroeder, "Natural sounding artificial reverberation" (JAES 10(3),
//! 1962); the single-delay-line form used here is from J. O. Smith,
//! "Physical Audio Signal Processing", Allpass Filters (Schroeder Allpass
//! Sections): `v[n] = x[n] + g·v[n−M]`, `y[n] = v[n−M] − g·v[n]`, so
//! `H(z) = (z⁻ᴹ − g) / (1 − g·z⁻ᴹ)`. With an interpolated read the delay can
//! be modulated (as in J. Dattorro, "Effect Design, Part 1", JAES 45(9),
//! 1997), at the cost of the interpolator's slight high-frequency loss.

use crate::delay::DelayLine;

/// Bound on what the recursion writes. A fixed delay with `|g| < 1` always
/// decays, but audio-rate jumps in the delay can briefly raise the loop
/// gain; this keeps that finite.
const LIMIT: f32 = 1e9;

/// Gains are kept strictly inside the unit circle so the recursion decays.
pub const MAX_GAIN: f32 = 0.999;

/// One allpass section with its own delay line.
#[derive(Clone, Debug)]
pub struct Allpass {
    line: DelayLine,
}

impl Allpass {
    /// A section whose delay can reach `max_delay` samples.
    pub fn new(max_delay: usize) -> Self {
        Allpass {
            line: DelayLine::new(max_delay),
        }
    }

    /// The longest delay `tick` honours, in samples.
    pub fn max_delay(&self) -> f64 {
        self.line.max_delay()
    }

    /// Filter one sample with a delay of `delay` samples (clamped to
    /// `[1, max_delay]`, interpolated) and gain `gain` (clamped to ±0.999).
    /// A non-finite input is treated as silence.
    #[inline]
    pub fn tick(&mut self, x: f32, delay: f64, gain: f32) -> f32 {
        let g = if gain.is_finite() {
            gain.clamp(-MAX_GAIN, MAX_GAIN)
        } else {
            0.0
        };
        let x = if x.is_finite() { x } else { 0.0 };
        let d = self.line.read(delay);
        let v = (x + g * d).clamp(-LIMIT, LIMIT);
        self.line.write(v);
        d - g * v
    }

    /// Silence the section in constant time.
    pub fn clear(&mut self) {
        self.line.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DFT magnitude of `h` at `freq` (cycles per sample).
    fn dft_mag(h: &[f32], freq: f64) -> f64 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (n, &v) in h.iter().enumerate() {
            let w = std::f64::consts::TAU * freq * n as f64;
            re += f64::from(v) * w.cos();
            im -= f64::from(v) * w.sin();
        }
        re.hypot(im)
    }

    #[test]
    fn impulse_response_is_the_textbook_one() {
        let mut ap = Allpass::new(16);
        let g = 0.5;
        let h: Vec<f32> = (0..25)
            .map(|n| ap.tick(if n == 0 { 1.0 } else { 0.0 }, 5.0, g))
            .collect();
        assert_eq!(h[0], -0.5);
        assert_eq!(h[5], 1.0 - 0.25);
        assert_eq!(h[10], 0.5 * 0.75);
        assert_eq!(h[15], 0.25 * 0.75);
        assert_eq!(h[1], 0.0);
    }

    #[test]
    fn magnitude_is_unity_everywhere() {
        for (delay, g) in [(7.0, 0.7), (113.0, -0.6), (1.0, 0.9)] {
            let mut ap = Allpass::new(200);
            let h: Vec<f32> = (0..30_000)
                .map(|n| ap.tick(if n == 0 { 1.0 } else { 0.0 }, delay, g))
                .collect();
            for f in [0.0, 0.01, 0.0713, 0.2, 0.33, 0.49] {
                let m = dft_mag(&h, f);
                assert!((m - 1.0).abs() < 1e-3, "delay {delay} g {g} f {f}: {m}");
            }
        }
    }

    #[test]
    fn bad_input_and_gain_stay_finite() {
        let mut ap = Allpass::new(32);
        for n in 0..1000 {
            let x = [f32::NAN, f32::INFINITY, 1e30, -1.0][n % 4];
            let y = ap.tick(x, 3.5, [f32::NAN, 5.0, -5.0, 0.5][n % 4]);
            assert!(y.is_finite());
        }
        ap.clear();
        assert_eq!(ap.tick(0.0, 3.0, 0.5), 0.0);
    }
}
