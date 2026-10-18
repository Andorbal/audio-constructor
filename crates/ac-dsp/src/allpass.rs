//! The Schroeder allpass: a delay line with feedback and feedforward of
//! equal and opposite gain, which smears a signal in time without changing
//! its magnitude spectrum.
//!
//! M. R. Schroeder, "Natural sounding artificial reverberation" (JAES 10(3),
//! 1962); the single-delay-line form used here is from J. O. Smith,
//! "Physical Audio Signal Processing", Allpass Filters (Schroeder Allpass
//! Sections): `v[n] = x[n] + g·v[n−M]`, `y[n] = v[n−M] − g·v[n]`, so
//! `H(z) = (z⁻ᴹ − g) / (1 − g·z⁻ᴹ)`.
//!
//! Fractional and modulated delays (as in J. Dattorro, "Effect Design,
//! Part 1", JAES 45(9), 1997) are read through a first-order allpass
//! interpolator (T. I. Laakso et al., "Splitting the unit delay", IEEE
//! Signal Processing Magazine, 1996; Smith, PASP, First-Order Allpass
//! Interpolation). A polynomial interpolator loses treble at fractional
//! positions, and inside a recirculating section that loss compounds into
//! audible dulling; an allpass interpolator has unity magnitude, so the
//! section stays exactly lossless at any delay. Whole-sample delays read the
//! line directly.

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
    /// The longest delay honoured, in samples.
    max: f64,
    /// The fractional-delay interpolator's previous output.
    frac_y: f32,
}

impl Allpass {
    /// A section whose delay can reach `max_delay` samples.
    pub fn new(max_delay: usize) -> Self {
        let max = max_delay.max(1);
        Allpass {
            // One spare slot for the interpolator's older tap.
            line: DelayLine::new(max + 1),
            max: max as f64,
            frac_y: 0.0,
        }
    }

    /// The longest delay `tick` honours, in samples.
    pub fn max_delay(&self) -> f64 {
        self.max
    }

    /// The line's output `delay` samples back. The delay splits into whole
    /// samples `k` and a fraction in `[0.5, 1.5)`, which keeps the
    /// interpolator's pole (at `−η`) within ±1/3, so a change of delay
    /// settles within a few samples. A whole-sample delay has a fraction of
    /// exactly 1 and `η = 0`: a plain read. Delays under 1.5 samples are held
    /// at 1.
    #[inline]
    fn read(&mut self, delay: f64) -> f32 {
        let d = if delay.is_finite() {
            delay.clamp(1.0, self.max)
        } else {
            1.0
        };
        let (k, frac) = if d < 1.5 {
            (0, 1.0)
        } else {
            // `d − 0.5 >= 1`, so truncation is `floor`.
            let k = (d - 0.5) as usize;
            (k, d - k as f64)
        };
        let eta = ((1.0 - frac) / (1.0 + frac)) as f32;
        // y[n] = η·(s[n] − y[n−1]) + s[n−1], with s the line delayed by k.
        let (newer, older) = self.line.tap_pair(k);
        let y = eta * (newer - self.frac_y) + older;
        self.frac_y = if y.is_finite() { y } else { 0.0 };
        self.frac_y
    }

    /// Filter one sample with a delay of `delay` samples (clamped to
    /// `[1, max_delay]`) and gain `gain` (clamped to ±0.999). A non-finite
    /// input is treated as silence.
    #[inline]
    pub fn tick(&mut self, x: f32, delay: f64, gain: f32) -> f32 {
        let g = if gain.is_finite() {
            gain.clamp(-MAX_GAIN, MAX_GAIN)
        } else {
            0.0
        };
        let x = if x.is_finite() { x } else { 0.0 };
        let d = self.read(delay);
        let v = (x + g * d).clamp(-LIMIT, LIMIT);
        self.line.write(v);
        d - g * v
    }

    /// Silence the section in constant time.
    pub fn clear(&mut self) {
        self.line.clear();
        self.frac_y = 0.0;
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

    fn impulse_response(delay: f64, g: f32, n: usize) -> Vec<f32> {
        let mut ap = Allpass::new(200);
        (0..n)
            .map(|i| ap.tick(if i == 0 { 1.0 } else { 0.0 }, delay, g))
            .collect()
    }

    #[test]
    fn impulse_response_is_the_textbook_one() {
        let h = impulse_response(5.0, 0.5, 25);
        assert_eq!(h[0], -0.5);
        assert_eq!(h[5], 1.0 - 0.25);
        assert_eq!(h[10], 0.5 * 0.75);
        assert_eq!(h[15], 0.25 * 0.75);
        assert_eq!(h[1], 0.0);
    }

    #[test]
    fn magnitude_is_unity_everywhere() {
        for (delay, g) in [(7.0, 0.7), (113.0, -0.6), (1.0, 0.9)] {
            let h = impulse_response(delay, g, 30_000);
            for f in [0.0, 0.01, 0.0713, 0.2, 0.33, 0.49] {
                let m = dft_mag(&h, f);
                assert!((m - 1.0).abs() < 1e-3, "delay {delay} g {g} f {f}: {m}");
            }
        }
    }

    #[test]
    fn fractional_delays_are_lossless_too() {
        for (delay, g) in [(7.3, 0.7), (40.5, 0.9), (2.01, -0.5), (1.6, 0.6)] {
            let h = impulse_response(delay, g, 60_000);
            for f in [0.01, 0.1, 0.25, 0.4, 0.49] {
                let m = dft_mag(&h, f);
                assert!((m - 1.0).abs() < 1e-3, "delay {delay} g {g} f {f}: {m}");
            }
            // With no gain the section is the interpolated delay alone, whose
            // delay at DC is exact: a ramp comes out `delay` samples late.
            let mut ap = Allpass::new(64);
            let mut y = 0.0;
            for n in 0..200 {
                y = ap.tick(n as f32, delay, 0.0);
            }
            assert!((y - (199.0 - delay as f32)).abs() < 1e-3, "{delay}: {y}");
        }
    }

    #[test]
    fn a_swept_delay_stays_lossless() {
        let mut ap = Allpass::new(200);
        let n = 48_000;
        let mut energy_in = 0.0f64;
        let mut energy_out = 0.0f64;
        let mut s = 1u32;
        for i in 0..n {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let x = if i < n - 4_000 {
                (s as f32 / u32::MAX as f32) - 0.5
            } else {
                0.0
            };
            let d = 50.0 + 30.0 * (i as f64 * 0.0005).sin();
            let y = ap.tick(x, d, 0.7);
            energy_in += f64::from(x).powi(2);
            energy_out += f64::from(y).powi(2);
        }
        let ratio = energy_out / energy_in;
        assert!((ratio - 1.0).abs() < 0.01, "{ratio}");
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
