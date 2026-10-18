//! Fundamental-frequency estimation with YIN (A. de Cheveigné and
//! H. Kawahara, "YIN, a fundamental frequency estimator for speech and
//! music", J. Acoust. Soc. Am. 111(4), 2002), steps 1–5 of the paper:
//!
//! 1–2. the difference function `d(τ) = Σ (x[j] − x[j − τ])²` over the
//!      newest half of the frame, against samples up to half a frame
//!      older. Integrating over the newest half (the paper's window runs
//!      forward from the oldest) means a new note is recognized once a few
//!      of its periods have arrived, not once it fills most of the frame;
//! 3.   its cumulative mean normalized form `d'(τ) = d(τ) · τ / Σ_{1..τ} d`;
//! 4.   an absolute threshold: the first lag whose `d'` dips below it (then
//!      the bottom of that dip), or the global minimum if none does, which
//!      is what keeps YIN from picking a subharmonic;
//! 5.   parabolic interpolation of the raw `d` around that lag (the paper's
//!      choice, since `d'` is biased).
//!
//! `d(τ)` is expanded into two energies and a cross-correlation, and the
//! correlation is computed with FFTs, so an analysis costs three transforms
//! of the frame size instead of `O(size²)` multiplies. Confidence is
//! `1 − d'` at the chosen lag: near 1 for a clean periodic tone, near 0 for
//! noise.

use crate::fft::Fft;

/// One analysis result.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pitch {
    /// Estimated fundamental in Hz (0 when the frame is silent or no lag is
    /// in range).
    pub freq: f64,
    /// 0..1: how periodic the frame is at that lag.
    pub confidence: f32,
}

/// A YIN analyzer for frames of a fixed size.
#[derive(Clone, Debug)]
pub struct Yin {
    size: usize,
    fft: Fft,
    /// The frame, newest sample first.
    rev: Vec<f32>,
    time: Vec<f32>,
    spec_a: Vec<f32>,
    spec_b: Vec<f32>,
    corr: Vec<f32>,
    diff: Vec<f64>,
    cmnd: Vec<f64>,
}

/// Mean square below which a frame counts as silence (−100 dBFS).
const SILENCE: f64 = 1e-10;

impl Yin {
    /// An analyzer for frames of `size` samples (even, at least 16). Lags
    /// go up to `size / 2`.
    pub fn new(size: usize) -> Yin {
        let fft = Fft::new(size.max(16));
        let size = fft.size();
        let bins = fft.bins();
        Yin {
            size,
            rev: vec![0.0; size],
            time: vec![0.0; size],
            spec_a: vec![0.0; 2 * bins],
            spec_b: vec![0.0; 2 * bins],
            corr: vec![0.0; size],
            diff: vec![0.0; size / 2 + 1],
            cmnd: vec![0.0; size / 2 + 1],
            fft,
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// The longest period (in samples) a frame can measure.
    pub fn max_lag(&self) -> usize {
        self.size / 2 - 1
    }

    /// The lowest frequency a frame can measure at `rate`.
    pub fn min_freq(&self, rate: f64) -> f64 {
        rate / self.max_lag() as f64
    }

    /// Estimate the pitch of `frame` (`size` samples, oldest first), looking
    /// for fundamentals between `min_freq` and `max_freq` Hz. `threshold` is
    /// YIN's absolute threshold on `d'` (the paper uses 0.1; larger accepts
    /// noisier tones).
    pub fn analyze(
        &mut self,
        frame: &[f32],
        rate: f64,
        min_freq: f64,
        max_freq: f64,
        threshold: f64,
    ) -> Pitch {
        let n = self.size;
        let w = n / 2;
        let Some(frame) = frame.get(..n) else {
            return Pitch::default();
        };
        if !(rate.is_finite() && rate > 0.0) {
            return Pitch::default();
        }
        // Work newest-first, so the sums below run over the newest half.
        for (r, s) in self.rev.iter_mut().zip(frame.iter().rev()) {
            *r = *s;
        }
        let x: &[f32] = &self.rev;
        let e0: f64 = x[..w].iter().map(|v| f64::from(*v) * f64::from(*v)).sum();
        if e0 < SILENCE * w as f64 {
            return Pitch::default();
        }
        // Lag range from the frequency range.
        let lo = (rate / max_freq.max(1e-3)).floor().max(2.0);
        let hi = (rate / min_freq.max(1e-3))
            .ceil()
            .min(self.max_lag() as f64);
        if lo >= hi {
            return Pitch::default();
        }
        let (lo, hi) = (lo as usize, hi as usize);

        // Cross-correlation c(τ) = Σ_{j<w} x[j] x[j+τ] as IFFT(conj(A)·B),
        // with A the first half zero-padded. It doesn't wrap for τ ≤ w.
        self.time[..w].copy_from_slice(&x[..w]);
        self.time[w..].fill(0.0);
        self.fft.forward(&self.time, &mut self.spec_a);
        self.fft.forward(x, &mut self.spec_b);
        for (a, b) in self
            .spec_a
            .chunks_exact_mut(2)
            .zip(self.spec_b.chunks_exact(2))
        {
            let (ar, ai, br, bi) = (a[0], a[1], b[0], b[1]);
            a[0] = ar * br + ai * bi;
            a[1] = ar * bi - ai * br;
        }
        self.fft.inverse(&self.spec_a, &mut self.corr);

        // d(τ) = e0 + e(τ) − 2c(τ), with e(τ) the energy of x[τ..τ+w).
        let top = hi + 1;
        let mut energy = e0;
        let mut running = 0.0f64;
        self.diff[0] = 0.0;
        self.cmnd[0] = 1.0;
        for tau in 1..=top {
            let out = f64::from(x[tau - 1]);
            let inn = f64::from(x[tau + w - 1]);
            energy += inn * inn - out * out;
            let d = (e0 + energy - 2.0 * f64::from(self.corr[tau])).max(0.0);
            running += d;
            self.diff[tau] = d;
            self.cmnd[tau] = if running > 1e-20 {
                d * tau as f64 / running
            } else {
                1.0
            };
        }

        // Absolute threshold, then the bottom of that dip.
        let mut best = None;
        for tau in lo..=hi {
            if self.cmnd[tau] < threshold {
                let mut t = tau;
                while t < hi && self.cmnd[t + 1] < self.cmnd[t] {
                    t += 1;
                }
                best = Some(t);
                break;
            }
        }
        let t = best.unwrap_or_else(|| {
            (lo..=hi)
                .min_by(|a, b| self.cmnd[*a].total_cmp(&self.cmnd[*b]))
                .unwrap_or(lo)
        });

        // Parabolic interpolation of the raw difference function.
        let (a, b, c) = (self.diff[t - 1], self.diff[t], self.diff[t + 1]);
        let denom = a - 2.0 * b + c;
        let shift = if denom.abs() > 1e-20 {
            (0.5 * (a - c) / denom).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let period = t as f64 + shift;
        Pitch {
            freq: rate / period,
            confidence: (1.0 - self.cmnd[t]).clamp(0.0, 1.0) as f32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    fn cents(a: f64, b: f64) -> f64 {
        1200.0 * (a / b).log2()
    }

    fn sine(freq: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (0.6 * (std::f64::consts::TAU * freq * i as f64 / RATE).sin()) as f32)
            .collect()
    }

    /// A band-limited sawtooth: harmonics up to 20 kHz.
    fn saw(freq: f64, n: usize) -> Vec<f32> {
        let harmonics = (20_000.0 / freq) as usize;
        (0..n)
            .map(|i| {
                let t = std::f64::consts::TAU * freq * i as f64 / RATE;
                let s: f64 = (1..=harmonics)
                    .map(|k| (k as f64 * t).sin() / k as f64)
                    .sum();
                (0.3 * s) as f32
            })
            .collect()
    }

    #[test]
    fn sines_and_saws_are_found_within_a_few_cents() {
        let mut yin = Yin::new(2048);
        for f in [82.41, 110.0, 196.0, 440.0, 1000.0, 1760.0] {
            for x in [sine(f, 2048), saw(f, 2048)] {
                let p = yin.analyze(&x, RATE, 50.0, 2000.0, 0.15);
                assert!(cents(p.freq, f).abs() < 5.0, "{f}: {}", p.freq);
                assert!(p.confidence > 0.9, "{f}: {}", p.confidence);
            }
        }
    }

    #[test]
    fn noise_and_silence_have_low_confidence() {
        let mut yin = Yin::new(2048);
        let mut s = 12_345u32;
        let noise: Vec<f32> = (0..2048)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                s as f32 / u32::MAX as f32 - 0.5
            })
            .collect();
        let p = yin.analyze(&noise, RATE, 50.0, 2000.0, 0.15);
        assert!(p.confidence < 0.5, "{}", p.confidence);
        let p = yin.analyze(&[0.0; 2048], RATE, 50.0, 2000.0, 0.15);
        assert_eq!(p, Pitch::default());
    }

    #[test]
    fn the_range_limits_the_search() {
        let mut yin = Yin::new(2048);
        assert!((yin.min_freq(RATE) - 48_000.0 / 1023.0).abs() < 1e-9);
        // A range that excludes the tone finds no clear period.
        let p = yin.analyze(&sine(440.0, 2048), RATE, 1000.0, 2000.0, 0.15);
        assert!(p.confidence < 0.5 || cents(p.freq, 440.0).abs() > 100.0);
        // An empty or bogus range gives nothing.
        let p = yin.analyze(&sine(440.0, 2048), RATE, 2000.0, 1000.0, 0.15);
        assert_eq!(p, Pitch::default());
        let p = yin.analyze(&sine(440.0, 100), RATE, 50.0, 2000.0, 0.15);
        assert_eq!(p, Pitch::default());
    }
}
