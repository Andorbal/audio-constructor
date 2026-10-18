//! Spectral envelopes by cepstral smoothing: the slowly varying outline of a
//! magnitude spectrum (a voice's formants) without its fine harmonic
//! structure (its pitch).
//!
//! The natural-log magnitude spectrum is transformed to the real cepstrum;
//! the envelope lives at low quefrencies and a pitched sound's harmonics at
//! its pitch period and beyond, so a lowpass "lifter" keeps the envelope and
//! the transform back gives its log magnitude (A. V. Oppenheim and
//! R. W. Schafer, "Discrete-Time Signal Processing", the real cepstrum and
//! homomorphic deconvolution; used for pitch shifting with envelope
//! preservation in A. Röbel and X. Rodet, "Efficient spectral envelope
//! estimation and its application to pitch shifting and envelope
//! preservation", DAFx 2005). Plain cepstral smoothing follows the average
//! of the log spectrum, so it sits somewhat below the harmonic peaks; when
//! two envelopes from the same estimator are divided, that bias cancels.

use crate::fft::Fft;

/// Log-magnitude floor relative to the frame's loudest bin (-120 dB), so
/// silent bins don't send the logarithm to minus infinity.
const FLOOR: f32 = 1e-6;
/// Absolute floor for an all-silent frame.
const SILENT: f32 = 1e-20;

/// Estimates log-magnitude envelopes for spectra of one FFT size. Allocates
/// in [`CepstralEnvelope::new`] only.
#[derive(Clone, Debug)]
pub struct CepstralEnvelope {
    size: usize,
    /// Lifter weights for quefrencies `0..lifter.len()` (mirrored for the
    /// negative quefrencies).
    lifter: Vec<f32>,
    /// Interleaved `re, im` bins (the log spectrum, then the envelope).
    bins: Vec<f32>,
    /// The real cepstrum.
    cepstrum: Vec<f32>,
}

impl CepstralEnvelope {
    /// An estimator for spectra of an FFT of `size` samples, keeping
    /// quefrencies below `cutoff` samples (clamped to `2..size / 2`). The
    /// top quarter of the kept range tapers off with a half cosine so the
    /// envelope doesn't ripple.
    pub fn new(size: usize, cutoff: usize) -> Self {
        let size = size.max(4) & !1;
        let cutoff = cutoff.clamp(2, size / 2);
        let taper = (cutoff / 4).max(1);
        let lifter = (0..cutoff)
            .map(|q| {
                let into = q as f64 - (cutoff - taper) as f64;
                if into < 0.0 {
                    1.0
                } else {
                    (0.5 + 0.5 * (std::f64::consts::PI * (into + 1.0) / (taper + 1) as f64).cos())
                        as f32
                }
            })
            .collect();
        CepstralEnvelope {
            size,
            lifter,
            bins: vec![0.0; 2 * (size / 2 + 1)],
            cepstrum: vec![0.0; size],
        }
    }

    /// The FFT size this estimator expects.
    pub fn size(&self) -> usize {
        self.size
    }

    /// The natural-log magnitude envelope of `spectrum` (interleaved
    /// `re, im`, `size / 2 + 1` bins), one value per bin into `out`. `fft`
    /// must be of this estimator's size; otherwise `out` is left at 0.
    pub fn log_envelope(&mut self, fft: &mut Fft, spectrum: &[f32], out: &mut [f32]) {
        let bins = self.size / 2 + 1;
        if fft.size() != self.size {
            out.fill(0.0);
            return;
        }
        let mut loudest = 0.0f32;
        for k in 0..bins {
            let re = spectrum.get(2 * k).copied().unwrap_or(0.0);
            let im = spectrum.get(2 * k + 1).copied().unwrap_or(0.0);
            let m = (re * re + im * im).sqrt();
            if m.is_finite() {
                loudest = loudest.max(m);
            }
        }
        let floor = (loudest * FLOOR).max(SILENT);
        for k in 0..bins {
            let re = spectrum.get(2 * k).copied().unwrap_or(0.0);
            let im = spectrum.get(2 * k + 1).copied().unwrap_or(0.0);
            let m = (re * re + im * im).sqrt();
            let m = if m.is_finite() { m.max(floor) } else { floor };
            if let Some(pair) = self.bins.get_mut(2 * k..2 * k + 2) {
                pair[0] = m.ln();
                pair[1] = 0.0;
            }
        }
        // A real, even log spectrum has a real, even cepstrum.
        fft.inverse(&self.bins, &mut self.cepstrum);
        let n = self.size;
        let kept = self.lifter.len();
        for (q, c) in self.cepstrum.iter_mut().enumerate() {
            // Quefrency q and its mirror n − q share a weight.
            let dist = q.min(n - q);
            *c *= if dist < kept {
                self.lifter.get(dist).copied().unwrap_or(0.0)
            } else {
                0.0
            };
        }
        fft.forward(&self.cepstrum, &mut self.bins);
        for (k, o) in out.iter_mut().take(bins).enumerate() {
            *o = self.bins.get(2 * k).copied().unwrap_or(0.0);
        }
    }
}

/// Linear interpolation into a per-bin table at fractional bin `pos`,
/// holding the end values beyond either edge.
#[inline]
pub fn interpolate(table: &[f32], pos: f32) -> f32 {
    let last = table.len().saturating_sub(1);
    // Also catches NaN.
    if pos.is_nan() || pos <= 0.0 {
        return table.first().copied().unwrap_or(0.0);
    }
    // `pos > 0`, so truncation is `floor`.
    let i = pos as usize;
    if i >= last {
        return table.get(last).copied().unwrap_or(0.0);
    }
    let t = pos - i as f32;
    let a = table.get(i).copied().unwrap_or(0.0);
    let b = table.get(i + 1).copied().unwrap_or(a);
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::Window;

    const N: usize = 1024;
    const RATE: f64 = 48_000.0;

    /// A pulse train at `f0` through a two-pole resonator at `formant`: a
    /// crude vowel with one formant.
    fn vowel(f0: f64, formant: f64) -> Vec<f32> {
        let period = (RATE / f0) as usize;
        let r = 0.97f64;
        let w = std::f64::consts::TAU * formant / RATE;
        let (a1, a2) = (-2.0 * r * w.cos(), r * r);
        let (mut y1, mut y2) = (0.0f64, 0.0f64);
        (0..N * 4)
            .map(|i| {
                let x = if i % period == 0 { 1.0 } else { 0.0 };
                let y = x - a1 * y1 - a2 * y2;
                y2 = y1;
                y1 = y;
                y as f32
            })
            .collect()
    }

    fn envelope_of(x: &[f32], cutoff: usize) -> Vec<f32> {
        let mut fft = Fft::new(N);
        let win = Window::Hann.table(N);
        let frame: Vec<f32> = x[N * 2..N * 3]
            .iter()
            .zip(&win)
            .map(|(a, b)| a * b)
            .collect();
        let mut spec = vec![0.0; 2 * fft.bins()];
        fft.forward(&frame, &mut spec);
        let mut env = vec![0.0; fft.bins()];
        CepstralEnvelope::new(N, cutoff).log_envelope(&mut fft, &spec, &mut env);
        env
    }

    fn peak_hz(env: &[f32]) -> f64 {
        let (k, _) = env
            .iter()
            .enumerate()
            .skip(2)
            .fold((0, f32::MIN), |b, (k, &v)| if v > b.1 { (k, v) } else { b });
        k as f64 * RATE / N as f64
    }

    #[test]
    fn finds_the_formant_not_the_harmonics() {
        // 1.5 ms lifter: below the 5 ms pitch period of a 200 Hz voice.
        let env = envelope_of(&vowel(200.0, 1500.0), 72);
        let p = peak_hz(&env);
        assert!((p - 1500.0).abs() < 200.0, "{p}");
        // Smooth: neighbouring bins differ little, unlike the harmonic comb.
        let ripple = env
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(ripple < 0.5, "{ripple}");
    }

    #[test]
    fn the_same_formant_at_another_pitch_gives_the_same_envelope() {
        let a = envelope_of(&vowel(150.0, 1000.0), 72);
        let b = envelope_of(&vowel(240.0, 1000.0), 72);
        assert!((peak_hz(&a) - peak_hz(&b)).abs() < 200.0);
    }

    #[test]
    fn silence_and_garbage_give_flat_finite_envelopes() {
        let mut fft = Fft::new(N);
        let mut est = CepstralEnvelope::new(N, 50);
        let mut env = vec![1.0; fft.bins()];
        let silent = vec![0.0; 2 * fft.bins()];
        est.log_envelope(&mut fft, &silent, &mut env);
        let first = env[0];
        assert!(
            env.iter()
                .all(|v| v.is_finite() && (v - first).abs() < 1e-3)
        );
        let bad: Vec<f32> = (0..2 * fft.bins())
            .map(|i| [f32::NAN, f32::INFINITY, 1.0][i % 3])
            .collect();
        est.log_envelope(&mut fft, &bad, &mut env);
        assert!(env.iter().all(|v| v.is_finite()));
        // A mismatched FFT size leaves zeros instead of panicking.
        est.log_envelope(&mut Fft::new(64), &bad, &mut env);
        assert!(env.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn interpolation_holds_the_edges() {
        let t = [0.0, 1.0, 4.0];
        assert_eq!(interpolate(&t, -1.0), 0.0);
        assert_eq!(interpolate(&t, f32::NAN), 0.0);
        assert_eq!(interpolate(&t, 0.5), 0.5);
        assert_eq!(interpolate(&t, 1.25), 1.75);
        assert_eq!(interpolate(&t, 9.0), 4.0);
        assert_eq!(interpolate(&[], 1.0), 0.0);
    }
}
