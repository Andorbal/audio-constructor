//! Phase-vocoder remapping of spectral bins to new frequencies, for
//! frequency shifting and pitch scaling in the STFT domain.
//!
//! Each analysis bin's true frequency is estimated from how far its phase
//! advanced since the previous frame, beyond what the bin's center frequency
//! explains (M. Dolson, "The phase vocoder: a tutorial", Computer Music
//! Journal 10(4), 1986). The bin's magnitude moves to the bin nearest the
//! new frequency `f × scale + shift`, and each output bin's phase advances by
//! its new frequency every hop, so partials stay coherent across overlapping
//! frames (the bin-shifting scheme popularised by S. M. Bernsee, "Pitch
//! shifting using the Fourier transform", 1999; refinements such as peak
//! locking are in J. Laroche and M. Dolson, "New phase-vocoder techniques
//! for pitch-shifting, harmonizing and other exotic effects", IEEE WASPAA
//! 1999). When several bins land on one, their magnitudes add and the
//! strongest decides the frequency.

use std::f32::consts::{PI, TAU};

/// Wrap a phase to `[-π, π)`.
#[inline]
pub fn wrap_phase(p: f32) -> f32 {
    if (-PI..PI).contains(&p) {
        p
    } else {
        p - TAU * ((p + PI) / TAU).floor()
    }
}

/// Per-bin state for remapping frames of up to a fixed number of bins.
#[derive(Clone, Debug)]
pub struct BinShifter {
    prev_phase: Vec<f32>,
    synth_phase: Vec<f32>,
    mag: Vec<f32>,
    freq: Vec<f32>,
    best: Vec<f32>,
    fresh: bool,
}

impl BinShifter {
    /// State for frames of up to `max_bins` bins.
    pub fn new(max_bins: usize) -> BinShifter {
        BinShifter {
            prev_phase: vec![0.0; max_bins],
            synth_phase: vec![0.0; max_bins],
            mag: vec![0.0; max_bins],
            freq: vec![0.0; max_bins],
            best: vec![0.0; max_bins],
            fresh: true,
        }
    }

    /// Forget phase history; the next frame starts a new stream.
    pub fn reset(&mut self) {
        self.fresh = true;
    }

    /// Remap one frame. `input` and `output` are interleaved `re, im` bins of
    /// an FFT of `size` samples taken every `hop` samples; `scale` multiplies
    /// every frequency and `shift_bins` then adds to it (in bins of
    /// `rate / size` Hz). Bins beyond the state's size pass through
    /// unchanged; non-finite bins count as 0.
    ///
    /// With `scale` 1 and `shift_bins` 0 the frame is copied exactly, and the
    /// phase state follows along so moving away from identity is seamless.
    pub fn process(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        size: usize,
        hop: usize,
        scale: f32,
        shift_bins: f32,
    ) {
        let bins = (input.len().min(output.len()) / 2).min(self.prev_phase.len());
        let identity = scale == 1.0 && shift_bins == 0.0;
        let expect = TAU * hop as f32 / size.max(1) as f32;
        let to_bins = size as f32 / (TAU * hop.max(1) as f32);
        self.mag[..bins].fill(0.0);
        self.best[..bins].fill(0.0);
        for k in 0..bins {
            let (re, im) = finite_pair(input[2 * k], input[2 * k + 1]);
            let mag = (re * re + im * im).sqrt();
            let phase = im.atan2(re);
            let advance = phase - self.prev_phase[k];
            self.prev_phase[k] = phase;
            if identity {
                self.synth_phase[k] = phase;
                output[2 * k] = re;
                output[2 * k + 1] = im;
                continue;
            }
            // True frequency in bins.
            let true_bin = if self.fresh {
                k as f32
            } else {
                k as f32 + wrap_phase(advance - expect * k as f32) * to_bins
            };
            let target = true_bin * scale + shift_bins;
            if !(target >= -0.5 && target < bins as f32 - 0.5) {
                continue;
            }
            let j = (target + 0.5) as usize;
            if let Some(m) = self.mag.get_mut(j) {
                *m += mag;
            }
            if mag > self.best[j] {
                self.best[j] = mag;
                self.freq[j] = target;
            }
        }
        if identity {
            self.fresh = false;
            pass_rest(input, output, bins);
            return;
        }
        for j in 0..bins {
            let m = self.mag[j];
            if m > 0.0 {
                let p = wrap_phase(self.synth_phase[j] + expect * self.freq[j]);
                self.synth_phase[j] = p;
                let (s, c) = p.sin_cos();
                output[2 * j] = m * c;
                output[2 * j + 1] = m * s;
            } else {
                output[2 * j] = 0.0;
                output[2 * j + 1] = 0.0;
            }
        }
        if self.fresh {
            // Start the synthesis phases from the analysis phases.
            for j in 0..bins {
                if self.mag[j] > 0.0 {
                    let k = self.source_of(j, scale, shift_bins);
                    let p = self.prev_phase.get(k).copied().unwrap_or(0.0);
                    self.synth_phase[j] = p;
                    let m = self.mag[j];
                    let (s, c) = p.sin_cos();
                    output[2 * j] = m * c;
                    output[2 * j + 1] = m * s;
                }
            }
            self.fresh = false;
        }
        pass_rest(input, output, bins);
    }

    /// The analysis bin that maps nearest to output bin `j` (first frame
    /// only, where frequencies are bin centers).
    fn source_of(&self, j: usize, scale: f32, shift_bins: f32) -> usize {
        if scale.abs() < 1e-6 {
            return j;
        }
        let k = (j as f32 - shift_bins) / scale;
        if k.is_finite() && k >= 0.0 {
            (k + 0.5) as usize
        } else {
            0
        }
    }
}

fn finite_pair(re: f32, im: f32) -> (f32, f32) {
    if re.is_finite() && im.is_finite() {
        (re, im)
    } else {
        (0.0, 0.0)
    }
}

/// Copy bins `bins..` from input to output.
fn pass_rest(input: &[f32], output: &mut [f32], bins: usize) {
    let n = input.len().min(output.len());
    if 2 * bins < n {
        output[2 * bins..n].copy_from_slice(&input[2 * bins..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::{Fft, Window};
    use crate::stft::{OverlapAdd, StftAnalysis};

    const RATE: f64 = 48_000.0;

    fn sine(freq: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (0.5 * (std::f64::consts::TAU * freq * i as f64 / RATE).sin()) as f32)
            .collect()
    }

    /// STFT → shifter → ISTFT.
    fn run(x: &[f32], scale: f32, shift_hz: f32) -> Vec<f32> {
        let (size, hop) = (2048, 512);
        let mut fft = Fft::new(size);
        let mut an = StftAnalysis::new(size, hop, Window::Hann);
        let mut ola = OverlapAdd::new(size);
        ola.configure(size, hop, Window::Hann);
        let mut sh = BinShifter::new(fft.bins());
        let mut a = vec![0.0; 2 * fft.bins()];
        let mut b = vec![0.0; 2 * fft.bins()];
        let shift_bins = shift_hz * size as f32 / RATE as f32;
        x.iter()
            .map(|&s| {
                if an.push(s) {
                    an.analyze(&mut fft, &mut a);
                    sh.process(&a, &mut b, size, hop, scale, shift_bins);
                    ola.add(&mut fft, &b);
                }
                ola.pop()
            })
            .collect()
    }

    fn tone(x: &[f32], f: f64) -> f64 {
        let w = std::f64::consts::TAU * f / RATE;
        let (mut re, mut im) = (0.0, 0.0);
        for (i, v) in x.iter().enumerate() {
            re += f64::from(*v) * (w * i as f64).cos();
            im += f64::from(*v) * (w * i as f64).sin();
        }
        2.0 * re.hypot(im) / x.len() as f64
    }

    #[test]
    fn phases_wrap_into_range() {
        assert!((wrap_phase(3.0 * PI) + PI).abs() < 1e-5);
        assert!((wrap_phase(-3.5 * PI) - 0.5 * PI).abs() < 1e-5);
        assert_eq!(wrap_phase(1.0), 1.0);
    }

    #[test]
    fn identity_copies_frames_exactly() {
        let mut sh = BinShifter::new(9);
        let a: Vec<f32> = (0..18).map(|i| i as f32 * 0.1 - 0.4).collect();
        let mut b = vec![0.0; 18];
        sh.process(&a, &mut b, 16, 4, 1.0, 0.0);
        assert_eq!(a, b);
    }

    #[test]
    fn scaling_moves_a_tone() {
        let x = sine(440.0, 48_000);
        let y = run(&x, 1.5, 0.0);
        let tail = &y[16_000..];
        assert!(tone(tail, 660.0) > 0.3, "{}", tone(tail, 660.0));
        assert!(tone(tail, 440.0) < 0.02);
    }

    #[test]
    fn shifting_moves_a_tone_by_hertz() {
        let x = sine(1000.0, 48_000);
        let y = run(&x, 1.0, 250.0);
        let tail = &y[16_000..];
        assert!(tone(tail, 1250.0) > 0.3, "{}", tone(tail, 1250.0));
        assert!(tone(tail, 1000.0) < 0.02);
    }

    #[test]
    fn bins_moved_off_the_ends_vanish() {
        let mut sh = BinShifter::new(9);
        let a = vec![1.0; 18];
        let mut b = vec![5.0; 18];
        sh.process(&a, &mut b, 16, 4, 1.0, 100.0);
        assert!(b.iter().all(|v| *v == 0.0));
        sh.process(&[f32::NAN; 18], &mut b, 16, 4, 2.0, 0.0);
        assert!(b.iter().all(|v| v.is_finite()));
    }
}
