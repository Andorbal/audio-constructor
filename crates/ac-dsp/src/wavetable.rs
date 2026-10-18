//! Band-limited wavetables: single-cycle frames stored as mip-maps (one
//! copy per octave with the upper harmonics removed), so a frame can be
//! played at any pitch without aliasing.
//!
//! Each frame of `size` samples is analyzed with a real FFT; level `ℓ`
//! keeps harmonics up to `size / 2^(ℓ+1)` and is resynthesized with the
//! inverse FFT (the classic mip-mapped wavetable of, e.g., P. Burk,
//! "Band Limited Oscillators Using Wave Table Synthesis", in Audio
//! Anecdotes II, 2004). At phase increment `a` cycles per sample, level
//! `ℓ` is alias-free when `ℓ ≥ log2(size · a)`; playback reads the
//! smallest such level and crossfades toward the next one across each
//! octave, so the top harmonics fade out smoothly as the pitch rises
//! instead of switching off at octave boundaries. Reads interpolate
//! linearly within a level and between neighbouring frames.
//!
//! Storage is one flat `f32` slice: frame-major, then level, then
//! `size + 1` samples (a guard copy of the first sample makes
//! interpolation wrap without a branch).

use std::sync::{Arc, OnceLock};

use crate::fft::Fft;

/// The shape of a table set: samples per frame and mip levels per frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub size: usize,
    pub levels: usize,
}

impl Layout {
    /// A layout for frames of `size` samples (made even, at least 4),
    /// with levels down to a single harmonic.
    pub fn new(size: usize) -> Layout {
        let size = size.max(4) & !1;
        let levels = (usize::BITS - (size / 2).leading_zeros()) as usize;
        Layout { size, levels }
    }

    /// Samples in one level (including the guard sample).
    pub fn slot_len(&self) -> usize {
        self.size + 1
    }

    /// Samples in one frame (all its levels).
    pub fn frame_len(&self) -> usize {
        self.levels * self.slot_len()
    }

    /// Harmonics kept at `level`.
    pub fn harmonics(&self, level: usize) -> usize {
        (self.size / 2) >> level.min(63)
    }

    /// Where `frame`/`level` starts in the flat storage.
    pub fn offset(&self, frame: usize, level: usize) -> usize {
        frame * self.frame_len() + level * self.slot_len()
    }
}

/// Builds levels from a frame's spectrum. Allocates only in `new`, so
/// frames can be built on the audio thread a level at a time.
#[derive(Clone, Debug)]
pub struct Builder {
    layout: Layout,
    fft: Fft,
    /// The frame being built, as interleaved `re, im` bins.
    spectrum: Vec<f32>,
    /// The spectrum truncated for one level.
    work: Vec<f32>,
}

impl Builder {
    pub fn new(layout: Layout) -> Builder {
        let fft = Fft::new(layout.size);
        let bins = 2 * fft.bins();
        Builder {
            layout,
            fft,
            spectrum: vec![0.0; bins],
            work: vec![0.0; bins],
        }
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// Take the spectrum of one cycle (`size` samples; shorter is
    /// zero-padded).
    pub fn analyze(&mut self, cycle: &[f32]) {
        self.fft.forward(cycle, &mut self.spectrum);
    }

    /// Set the spectrum from sine amplitudes: harmonic `k` (from 1) is
    /// `amp(k) · sin(2πk·phase)`. No DC.
    pub fn set_sines(&mut self, amp: impl Fn(usize) -> f64) {
        let n = self.layout.size as f64;
        self.spectrum.fill(0.0);
        for k in 1..self.spectrum.len() / 2 {
            // A sine of amplitude A at bin k transforms to −i·A·N/2.
            if let Some(im) = self.spectrum.get_mut(2 * k + 1) {
                *im = (-amp(k) * n / 2.0) as f32;
            }
        }
    }

    /// Resynthesize `level` of the current spectrum into `slot` (at least
    /// `size + 1` samples).
    pub fn render(&mut self, level: usize, slot: &mut [f32]) {
        let keep = self.layout.harmonics(level);
        let cut = (2 * (keep + 1)).min(self.work.len());
        self.work[..cut].copy_from_slice(&self.spectrum[..cut]);
        self.work[cut..].fill(0.0);
        let size = self.layout.size;
        if slot.len() <= size {
            return;
        }
        self.fft.inverse(&self.work, &mut slot[..size]);
        slot[size] = slot[0];
    }
}

/// Linear read of one level at `phase` (cycles, wrapped to `[0, 1)`).
#[inline]
fn read(slot: &[f32], size: usize, phase: f64) -> f32 {
    let x = phase * size as f64;
    let k = (x as usize).min(size - 1);
    let t = (x - k as f64) as f32;
    match slot.get(k..k + 2) {
        Some(&[a, b]) => a + t * (b - a),
        _ => 0.0,
    }
}

/// The levels to read at phase increment `inc` and the weight of the
/// second: the smallest alias-free level, crossfading to the next across
/// the octave.
#[inline]
pub fn levels_for(layout: Layout, inc: f64) -> (usize, usize, f32) {
    let last = layout.levels.saturating_sub(1);
    let x = layout.size as f64 * inc.abs();
    if x.is_nan() || x <= 0.5 {
        // Below half an octave under the first level: the full table.
        return (0, 0, 0.0);
    }
    let lambda = x.log2();
    let lo = lambda.ceil();
    let w = (lambda - lo + 1.0) as f32;
    let lo = (lo.max(0.0) as usize).min(last);
    (lo, (lo + 1).min(last), w.clamp(0.0, 1.0))
}

/// Read a table set: `pos` is a fractional frame index in
/// `[0, frames − 1]`, `phase` in cycles, `inc` the phase increment per
/// sample (which picks the mip levels).
#[inline]
pub fn lookup(data: &[f32], layout: Layout, frames: usize, pos: f64, phase: f64, inc: f64) -> f32 {
    if frames == 0 {
        return 0.0;
    }
    let (l0, l1, w) = levels_for(layout, inc);
    let last = frames - 1;
    let pos = if pos.is_finite() {
        pos.clamp(0.0, last as f64)
    } else {
        0.0
    };
    let f0 = (pos as usize).min(last);
    let f1 = (f0 + 1).min(last);
    let t = (pos - f0 as f64) as f32;
    let phase = if phase.is_finite() {
        phase - phase.floor()
    } else {
        0.0
    };
    let size = layout.size;
    let at = |frame: usize, level: usize| {
        let o = layout.offset(frame, level);
        data.get(o..o + layout.slot_len())
            .map_or(0.0, |slot| read(slot, size, phase))
    };
    let frame = |f: usize| {
        let a = at(f, l0);
        if w > 0.0 && l1 != l0 {
            a + w * (at(f, l1) - a)
        } else {
            a
        }
    };
    let a = frame(f0);
    if t > 0.0 && f1 != f0 {
        a + t * (frame(f1) - a)
    } else {
        a
    }
}

/// Frame size of the built-in table set.
pub const BASIC_SIZE: usize = 2048;
/// Frames of the built-in set: sine, triangle, saw, square.
pub const BASIC_FRAMES: usize = 4;

/// The built-in table set (sine → triangle → saw → square, the shapes and
/// phases of the Oscillator node), built once and shared.
pub fn basic() -> Arc<Vec<f32>> {
    static BASIC: OnceLock<Arc<Vec<f32>>> = OnceLock::new();
    BASIC
        .get_or_init(|| {
            use std::f64::consts::PI;
            let layout = Layout::new(BASIC_SIZE);
            let mut data = vec![0.0; BASIC_FRAMES * layout.frame_len()];
            let mut b = Builder::new(layout);
            // Fourier series of the Oscillator's naive shapes.
            let odd = |k: usize| k % 2 == 1;
            let shapes: [&dyn Fn(usize) -> f64; BASIC_FRAMES] = [
                &|k| if k == 1 { 1.0 } else { 0.0 },
                &|k| {
                    if odd(k) {
                        let sign = if (k / 2) % 2 == 0 { 1.0 } else { -1.0 };
                        sign * 8.0 / (PI * PI * (k * k) as f64)
                    } else {
                        0.0
                    }
                },
                &|k| -2.0 / (PI * k as f64),
                &|k| if odd(k) { 4.0 / (PI * k as f64) } else { 0.0 },
            ];
            for (f, shape) in shapes.iter().enumerate() {
                b.set_sines(shape);
                for level in 0..layout.levels {
                    let o = layout.offset(f, level);
                    b.render(level, &mut data[o..o + layout.slot_len()]);
                }
            }
            Arc::new(data)
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_counts_levels_down_to_one_harmonic() {
        let l = Layout::new(2048);
        assert_eq!(l.levels, 11);
        assert_eq!(l.harmonics(0), 1024);
        assert_eq!(l.harmonics(10), 1);
        assert_eq!(Layout::new(3).size, 4);
        assert_eq!(Layout::new(1000).levels, 9);
    }

    #[test]
    fn level_choice_is_alias_free() {
        let l = Layout::new(2048);
        for inc in [1e-5, 0.001, 440.0 / 48_000.0, 0.01, 0.1, 0.3, 0.5] {
            let (lo, hi, w) = levels_for(l, inc);
            assert!(l.harmonics(lo) as f64 * inc <= 0.5 + 1e-12, "{inc}");
            assert!(hi >= lo && (0.0..=1.0).contains(&w));
        }
        assert_eq!(levels_for(l, 0.0), (0, 0, 0.0));
        assert_eq!(levels_for(l, f64::NAN), (0, 0, 0.0));
    }

    #[test]
    fn basic_frames_match_the_shapes() {
        let data = basic();
        let l = Layout::new(BASIC_SIZE);
        let slow = 1e-6;
        // Sine at a quarter cycle, triangle peak, saw midpoint, square high.
        let v = |frame: f64, phase: f64| lookup(&data, l, BASIC_FRAMES, frame, phase, slow);
        assert!((v(0.0, 0.25) - 1.0).abs() < 1e-4);
        assert!((v(1.0, 0.25) - 1.0).abs() < 2e-3);
        assert!(v(2.0, 0.25) + 0.5 < 1e-2);
        assert!((v(3.0, 0.25) - 1.0).abs() < 1e-2);
        // Halfway between sine and triangle is their average.
        let mid = v(0.5, 0.1);
        assert!((mid - 0.5 * (v(0.0, 0.1) + v(1.0, 0.1))).abs() < 1e-6);
    }

    #[test]
    fn high_levels_keep_only_low_harmonics() {
        let data = basic();
        let l = Layout::new(BASIC_SIZE);
        // The last level of the saw is a pure sine of amplitude 2/π.
        let o = l.offset(2, l.levels - 1);
        let slot = &data[o..o + l.slot_len()];
        let peak = slot.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 2.0 / std::f32::consts::PI).abs() < 1e-3);
    }

    #[test]
    fn analyzed_cycles_round_trip() {
        let l = Layout::new(64);
        let mut b = Builder::new(l);
        let cycle: Vec<f32> = (0..64).map(|i| ((i * 5) % 7) as f32 / 7.0 - 0.4).collect();
        b.analyze(&cycle);
        let mut slot = vec![0.0; l.slot_len()];
        b.render(0, &mut slot);
        for (a, b) in cycle.iter().zip(&slot) {
            assert!((a - b).abs() < 1e-5);
        }
        assert_eq!(slot[64], slot[0]);
    }
}
