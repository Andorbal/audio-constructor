//! Phase-vocoder frequency shifting and scaling in the STFT domain, with
//! peak locking.
//!
//! This is the peak-shifting scheme of J. Laroche and M. Dolson, "New
//! phase-vocoder techniques for pitch-shifting, harmonizing and other
//! exotic effects" (IEEE WASPAA 1999):
//!
//! 1. Spectral peaks are found (local maxima of the magnitude), and every
//!    bin is assigned to the peak whose region it lies in; regions split at
//!    the quietest bin between two peaks.
//! 2. Each peak's true frequency is estimated from how far its phase moved
//!    since the previous frame beyond what the bin's center frequency
//!    explains (M. Dolson, "The phase vocoder: a tutorial", Computer Music
//!    Journal 10(4), 1986), and mapped to `f × scale + shift`.
//! 3. The whole region moves so the peak's true frequency lands on its
//!    target, magnitudes unchanged, so a partial's main lobe keeps its
//!    shape and level. Laroche and Dolson move regions by whole bins and
//!    leave the fractional part to the phase rotation, which leaves each
//!    frame up to half a bin off and costs up to a few dB where frames
//!    overlap; here phases are referenced to the window's center, where a
//!    partial's lobe bins share one phase, so each bin can be spread over
//!    the four bins around its exact new position with Catmull-Rom weights.
//!    The level then holds to about 0.3 dB at any fractional offset.
//! 4. The peak's output phase advances at the new frequency every hop, and
//!    the other bins of the region keep their phase relative to the peak
//!    ("phase locking"), which keeps the lobe coherent and avoids the
//!    phasiness of moving bins independently.
//!
//! Regions that overlap after moving add up; regions moved below 0 or past
//! the last bin are dropped.

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

/// Per-bin state for shifting frames of up to a fixed number of bins.
#[derive(Clone, Debug)]
pub struct BinShifter {
    /// Analysis phase of each bin in the previous frame.
    prev_phase: Vec<f32>,
    /// Output phase of each bin in the previous frame.
    synth_phase: Vec<f32>,
    /// Output phases being built for this frame.
    next_phase: Vec<f32>,
    mag: Vec<f32>,
    phase: Vec<f32>,
    /// Largest contribution to each output bin so far this frame.
    best: Vec<f32>,
    /// Peak bins of this frame, ascending.
    peaks: Vec<usize>,
    fresh: bool,
}

impl BinShifter {
    /// State for frames of up to `max_bins` bins.
    pub fn new(max_bins: usize) -> BinShifter {
        BinShifter {
            prev_phase: vec![0.0; max_bins],
            synth_phase: vec![0.0; max_bins],
            next_phase: vec![0.0; max_bins],
            mag: vec![0.0; max_bins],
            phase: vec![0.0; max_bins],
            best: vec![0.0; max_bins],
            peaks: Vec::with_capacity(max_bins),
            fresh: true,
        }
    }

    /// Forget phase history; the next frame starts a new stream.
    pub fn reset(&mut self) {
        self.fresh = true;
    }

    /// Shift one frame. `input` and `output` are interleaved `re, im` bins
    /// of an FFT of `size` samples taken every `hop` samples; `scale`
    /// multiplies every frequency and `shift_bins` then adds to it (in bins
    /// of `rate / size` Hz). Bins beyond the state's size pass through
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
        let expect = TAU * hop as f32 / size.max(1) as f32;
        let to_bins = size as f32 / (TAU * hop.max(1) as f32);
        for (k, pair) in input.chunks_exact(2).take(bins).enumerate() {
            let (re, im) = if pair[0].is_finite() && pair[1].is_finite() {
                (pair[0], pair[1])
            } else {
                (0.0, 0.0)
            };
            // Phases referenced to the window's center rather than its
            // start (× (−1)^k): a region moved by whole bins is then off
            // its new frequency by at most half a bin around the center,
            // where the window is heavy, instead of drifting across it.
            let sign = center_sign(k);
            self.mag[k] = (re * re + im * im).sqrt();
            self.phase[k] = (sign * im).atan2(sign * re);
        }

        if scale == 1.0 && shift_bins == 0.0 {
            // An exact copy (non-finite bins as zero).
            for (o, i) in output[..2 * bins]
                .chunks_exact_mut(2)
                .zip(input.chunks_exact(2))
            {
                let finite = i[0].is_finite() && i[1].is_finite();
                o[0] = if finite { i[0] } else { 0.0 };
                o[1] = if finite { i[1] } else { 0.0 };
            }
            self.prev_phase[..bins].copy_from_slice(&self.phase[..bins]);
            self.synth_phase[..bins].copy_from_slice(&self.phase[..bins]);
            self.fresh = false;
            pass_rest(input, output, bins);
            return;
        }

        // 1. Peaks.
        self.peaks.clear();
        for k in 0..bins {
            let m = self.mag[k];
            let left = k == 0 || m > self.mag[k - 1];
            let right = k + 1 == bins || m >= self.mag[k + 1];
            if m > 0.0 && left && right {
                self.peaks.push(k);
            }
        }

        // Unclaimed output bins keep turning at their center frequency, so
        // a peak that arrives there later starts from a sensible phase.
        for j in 0..bins {
            self.next_phase[j] = wrap_phase(self.synth_phase[j] + expect * j as f32);
        }
        self.best[..bins].fill(0.0);
        output[..2 * bins].fill(0.0);

        for (n, &p) in self.peaks.iter().enumerate() {
            // Region: from the quietest bin after the previous peak to the
            // quietest bin before the next one.
            let lo = match n.checked_sub(1).and_then(|i| self.peaks.get(i)) {
                Some(&q) => quietest(&self.mag, q, p),
                None => 0,
            };
            let hi = match self.peaks.get(n + 1) {
                Some(&q) => quietest(&self.mag, p, q),
                None => bins,
            };
            // 2. True frequency and target.
            let true_bin = if self.fresh {
                p as f32
            } else {
                let advance = self.phase[p] - self.prev_phase[p] - expect * p as f32;
                p as f32 + wrap_phase(advance) * to_bins
            };
            let target = true_bin * scale + shift_bins;
            if !(target >= -0.5 && target < bins as f32 - 0.5) {
                continue;
            }
            let jp = (target + 0.5) as usize;
            // 4. The peak's phase advances at the new frequency.
            let peak_phase = if self.fresh {
                self.phase[p]
            } else {
                wrap_phase(self.synth_phase[jp] + expect * target)
            };
            // 3. Move the region so its true frequency lands on the target,
            // spreading each bin over the four around its new position.
            let delta = target - true_bin;
            let whole = delta.floor();
            let taps = cubic_taps(delta - whole);
            let whole = whole as isize;
            for k in lo..hi {
                let theta = wrap_phase(peak_phase + self.phase[k] - self.phase[p]);
                let (s, c) = theta.sin_cos();
                for (t, weight) in taps.iter().enumerate() {
                    let step = whole + t as isize - 1;
                    let Some(j) = k.checked_add_signed(step).filter(|&j| j < bins) else {
                        continue;
                    };
                    let m = self.mag[k] * weight;
                    if m == 0.0 {
                        continue;
                    }
                    output[2 * j] += center_sign(j) * m * c;
                    output[2 * j + 1] += center_sign(j) * m * s;
                    if m > self.best[j] {
                        self.best[j] = m;
                        self.next_phase[j] = theta;
                    }
                }
            }
        }

        self.prev_phase[..bins].copy_from_slice(&self.phase[..bins]);
        std::mem::swap(&mut self.synth_phase, &mut self.next_phase);
        self.fresh = false;
        pass_rest(input, output, bins);
    }
}

/// Catmull-Rom weights for spreading a value at fractional position
/// `frac` (0..1) past a bin over the bins at offsets −1, 0, 1, 2 (E. Catmull
/// and R. Rom, "A class of local interpolating splines", 1974). They sum to
/// 1 and reduce to a plain copy at `frac` 0.
fn cubic_taps(frac: f32) -> [f32; 4] {
    let k = |x: f32| {
        let x = x.abs();
        if x <= 1.0 {
            1.5 * x * x * x - 2.5 * x * x + 1.0
        } else if x < 2.0 {
            -0.5 * x * x * x + 2.5 * x * x - 4.0 * x + 2.0
        } else {
            0.0
        }
    };
    [k(1.0 + frac), k(frac), k(1.0 - frac), k(2.0 - frac)]
}

/// `(−1)^k`: moves bin `k`'s phase reference between the frame's start and
/// its center.
#[inline]
fn center_sign(k: usize) -> f32 {
    if k % 2 == 1 { -1.0 } else { 1.0 }
}

/// The quietest bin strictly after `a` and up to `b` (the first region
/// boundary; it belongs to the region on its right).
fn quietest(mag: &[f32], a: usize, b: usize) -> usize {
    let mut best = b;
    let mut least = f32::INFINITY;
    for (k, m) in mag.iter().enumerate().take(b).skip(a + 1) {
        if *m < least {
            least = *m;
            best = k;
        }
    }
    best
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

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
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
    fn scaling_moves_a_tone_and_keeps_its_level() {
        let x = sine(440.0, 48_000);
        for (scale, to) in [(1.5, 660.0), (0.75, 330.0), (2.0, 880.0)] {
            let y = run(&x, scale, 0.0);
            let tail = &y[16_000..];
            let level = tone(tail, to);
            assert!((level - 0.5).abs() < 0.03, "{scale}: {level}");
            assert!(tone(tail, 440.0) < 0.01);
            // Nothing else gets added: the moved tone is all there is.
            assert!(
                (rms(tail) - 0.5 / 2f64.sqrt()).abs() < 0.02,
                "{}",
                rms(tail)
            );
        }
    }

    #[test]
    fn shifting_moves_a_tone_by_hertz() {
        let x = sine(1000.0, 48_000);
        for (shift, to) in [(250.0, 1250.0), (-333.0, 667.0), (7.0, 1007.0)] {
            let y = run(&x, 1.0, shift);
            let tail = &y[16_000..];
            let level = tone(tail, to);
            assert!((level - 0.5).abs() < 0.03, "{shift}: {level}");
            assert!(tone(tail, 1000.0) < 0.05 || shift == 7.0);
        }
    }

    #[test]
    fn level_holds_at_every_fractional_offset() {
        // A bin is 23.4 Hz; step through it in eighths.
        let x = sine(1000.0, 48_000);
        let bin = RATE / 2048.0;
        let mut worst = 0.0f64;
        for e in 0..=8 {
            let shift = 100.0 + bin * e as f64 / 8.0;
            let y = run(&x, 1.0, shift as f32);
            let level = tone(&y[16_000..], 1000.0 + shift);
            worst = worst.max((20.0 * (level / 0.5).log10()).abs());
        }
        assert!(worst < 0.35, "{worst} dB");
    }

    #[test]
    fn partials_of_a_chord_move_together() {
        let a = sine(300.0, 48_000);
        let b = sine(450.0, 48_000);
        let x: Vec<f32> = a.iter().zip(&b).map(|(p, q)| 0.5 * (p + q)).collect();
        let y = run(&x, 1.25, 0.0);
        let tail = &y[16_000..];
        assert!((tone(tail, 375.0) - 0.25).abs() < 0.03);
        assert!((tone(tail, 562.5) - 0.25).abs() < 0.03);
    }

    #[test]
    fn bins_moved_off_the_ends_vanish() {
        let mut sh = BinShifter::new(9);
        let a: Vec<f32> = (0..18)
            .map(|i| if i % 4 == 0 { 1.0 } else { 0.2 })
            .collect();
        let mut b = vec![5.0; 18];
        sh.process(&a, &mut b, 16, 4, 1.0, 100.0);
        assert!(b.iter().all(|v| *v == 0.0));
        sh.process(&[f32::NAN; 18], &mut b, 16, 4, 2.0, 0.0);
        assert!(b.iter().all(|v| v.is_finite()));
    }
}
