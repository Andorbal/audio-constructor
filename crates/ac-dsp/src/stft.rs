//! Short-time Fourier transform plumbing shared by the spectral nodes: a
//! sliding window over the newest input, frame timing, and weighted
//! overlap-add resynthesis.
//!
//! Frames are taken every `hop` samples. The frame that completes at sample
//! `n` holds samples `n - size + 1 ..= n`; history before the first sample is
//! zero, so the first frame completes at sample `hop - 1`. Resynthesis adds
//! each inverse frame into the output starting at the sample its spectrum
//! completed, so analysis followed by resynthesis reproduces the input
//! delayed by exactly `size - 1` samples, from the very first sample.
//!
//! The overlap-add weights are the least-squares inverse STFT of D. Griffin
//! and J. Lim, "Signal estimation from modified short-time Fourier
//! transform" (IEEE Trans. ASSP 32(2), 1984): each inverse frame is
//! multiplied by the synthesis window and divided by the overlapped sum of
//! squared windows at that position. That sum repeats every hop, so it is
//! folded into the synthesis window once. The round trip is then exact for
//! every window at every overlap of 2 or more, including the cases where
//! the plain average gain (`fft::ola_gain`) leaves a ripple (Blackman at
//! overlap 4, Hann at overlap 2).

use crate::fft::{Fft, Window};

/// Smallest FFT size the spectral nodes accept.
pub const MIN_SIZE: usize = 64;
/// Largest FFT size the spectral nodes accept.
pub const MAX_SIZE: usize = 16_384;

/// The power of two nearest to `n` in octaves (halfway rounds up), within
/// `min..=max` (both powers of two). Non-finite values give `default`.
pub fn pow2_size(n: f64, min: usize, max: usize, default: usize) -> usize {
    if !n.is_finite() {
        return default.clamp(min, max);
    }
    let n = n.clamp(min.max(1) as f64, max.max(1) as f64);
    let octave = n.log2().round().clamp(0.0, 62.0) as u32;
    (1usize << octave).clamp(min, max)
}

/// An overlap factor (frames per window) repaired to 1, 2, 4, or 8.
pub fn overlap_factor(n: f64) -> usize {
    pow2_size(n, 1, 8, 4)
}

/// The newest `len` samples of a stream, readable as one contiguous slice.
///
/// Every sample is written twice (at `pos` and `pos + len`), so the window
/// never wraps: the classic double-buffer ring.
#[derive(Clone, Debug)]
pub struct SlidingWindow {
    buf: Vec<f32>,
    len: usize,
    pos: usize,
}

impl SlidingWindow {
    /// A window of `len` samples (at least 1), all zero.
    pub fn new(len: usize) -> SlidingWindow {
        let len = len.max(1);
        SlidingWindow {
            buf: vec![0.0; 2 * len],
            len,
            pos: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// Append one sample, dropping the oldest.
    #[inline]
    pub fn push(&mut self, x: f32) {
        let (lo, hi) = (self.pos, self.pos + self.len);
        if let Some(s) = self.buf.get_mut(lo) {
            *s = x;
        }
        if let Some(s) = self.buf.get_mut(hi) {
            *s = x;
        }
        self.pos += 1;
        if self.pos == self.len {
            self.pos = 0;
        }
    }

    /// The newest `len` samples, oldest first.
    #[inline]
    pub fn latest(&self) -> &[f32] {
        self.buf.get(self.pos..self.pos + self.len).unwrap_or(&[])
    }

    /// Forget everything (all zeros again).
    pub fn clear(&mut self) {
        self.buf.fill(0.0);
        self.pos = 0;
    }
}

/// Windowed analysis frames every `hop` samples.
#[derive(Clone, Debug)]
pub struct StftAnalysis {
    window: Vec<f32>,
    history: SlidingWindow,
    hop: usize,
    count: usize,
    frame: Vec<f32>,
}

impl StftAnalysis {
    /// Frames of `size` samples every `hop` samples (clamped to 1..=size).
    pub fn new(size: usize, hop: usize, window: Window) -> StftAnalysis {
        let size = size.max(2);
        StftAnalysis {
            window: window.table(size),
            history: SlidingWindow::new(size),
            hop: hop.clamp(1, size),
            count: 0,
            frame: vec![0.0; size],
        }
    }

    pub fn size(&self) -> usize {
        self.history.len()
    }

    pub fn hop(&self) -> usize {
        self.hop
    }

    /// Append one sample (non-finite values count as 0). Returns true when a
    /// frame completes at this sample.
    #[inline]
    pub fn push(&mut self, x: f32) -> bool {
        self.history.push(if x.is_finite() { x } else { 0.0 });
        self.count += 1;
        if self.count >= self.hop {
            self.count = 0;
            true
        } else {
            false
        }
    }

    /// The newest `size` input samples, oldest first (unwindowed).
    pub fn latest(&self) -> &[f32] {
        self.history.latest()
    }

    /// Windowed forward FFT of the newest `size` samples into interleaved
    /// `re, im` bins. `fft` must have this analysis's size; otherwise `out`
    /// is zeroed.
    pub fn analyze(&mut self, fft: &mut Fft, out: &mut [f32]) {
        if fft.size() != self.frame.len() {
            out.fill(0.0);
            return;
        }
        for ((f, x), w) in self
            .frame
            .iter_mut()
            .zip(self.history.latest())
            .zip(&self.window)
        {
            *f = x * w;
        }
        fft.forward(&self.frame, out);
    }

    /// Back to silence, with the first frame `hop` samples away.
    pub fn reset(&mut self) {
        self.history.clear();
        self.count = 0;
    }
}

/// Weighted overlap-add resynthesis of spectra into a sample stream.
///
/// Sized once for the largest frame it will see; [`OverlapAdd::configure`]
/// switches frame size, hop, or window without allocating.
#[derive(Clone, Debug)]
pub struct OverlapAdd {
    size: usize,
    hop: usize,
    window: Window,
    /// Synthesis window with the overlap normalization folded in.
    synth: Vec<f32>,
    /// Pending output, a ring of `size` samples starting at `pos`.
    acc: Vec<f32>,
    pos: usize,
    scratch: Vec<f32>,
}

impl OverlapAdd {
    /// Room for frames of up to `max_size` samples; configured for no frame
    /// (outputs silence) until [`OverlapAdd::configure`] is called.
    pub fn new(max_size: usize) -> OverlapAdd {
        let max_size = max_size.max(2);
        OverlapAdd {
            size: 0,
            hop: 1,
            window: Window::Hann,
            synth: vec![0.0; max_size],
            acc: vec![0.0; max_size],
            pos: 0,
            scratch: vec![0.0; max_size],
        }
    }

    /// The current frame size (0 before configuring).
    pub fn size(&self) -> usize {
        self.size
    }

    pub fn hop(&self) -> usize {
        self.hop
    }

    /// True if configured for exactly this layout.
    pub fn is(&self, size: usize, hop: usize, window: Window) -> bool {
        self.size == size && self.hop == hop && self.window == window
    }

    /// Set the frame size, hop, and window (the analysis window the frames
    /// were made with), clearing pending output. Returns false, leaving the
    /// stream silent, if `size` doesn't fit.
    ///
    /// Positions where the overlapped squared windows sum to less than a
    /// tenth of their largest sum (only tapered windows at overlap 1) are
    /// weighted as if they reached that tenth, so a frame edge is never
    /// boosted without bound.
    pub fn configure(&mut self, size: usize, hop: usize, window: Window) -> bool {
        self.clear();
        let fits = size >= 2 && size <= self.synth.len();
        if !fits {
            self.size = 0;
            return false;
        }
        let hop = hop.clamp(1, size);
        self.size = size;
        self.hop = hop;
        self.window = window;
        let (synth, sums) = (&mut self.synth[..size], &mut self.scratch[..hop]);
        window.fill(synth);
        let mut largest = 0.0f64;
        for (r, s) in sums.iter_mut().enumerate() {
            let sum: f64 = synth
                .iter()
                .skip(r)
                .step_by(hop)
                .map(|w| f64::from(*w) * f64::from(*w))
                .sum();
            *s = sum as f32;
            largest = largest.max(sum);
        }
        let floor = (0.1 * largest).max(1e-12) as f32;
        for (j, w) in synth.iter_mut().enumerate() {
            let sum = sums.get(j % hop).copied().unwrap_or(1.0).max(floor);
            *w /= sum;
        }
        true
    }

    /// Add the inverse transform of one frame (interleaved bins for
    /// `fft.size()`, which must equal the configured size) starting at the
    /// next sample [`OverlapAdd::pop`] returns. Non-finite bins count as 0.
    pub fn add(&mut self, fft: &mut Fft, bins: &[f32]) {
        let n = self.size;
        if n == 0 || fft.size() != n {
            return;
        }
        let frame = &mut self.scratch[..n];
        fft.inverse(bins, frame);
        let (synth, acc) = (&self.synth[..n], &mut self.acc[..n]);
        let (tail, head) = acc.split_at_mut(self.pos);
        // `head` holds outputs pos..n, `tail` wraps around to 0..pos.
        let split = head.len();
        for ((a, y), w) in head.iter_mut().zip(frame.iter()).zip(synth.iter()) {
            let v = y * w;
            *a += if v.is_finite() { v } else { 0.0 };
        }
        for ((a, y), w) in tail
            .iter_mut()
            .zip(frame[split..].iter())
            .zip(synth[split..].iter())
        {
            let v = y * w;
            *a += if v.is_finite() { v } else { 0.0 };
        }
    }

    /// The next output sample.
    #[inline]
    pub fn pop(&mut self) -> f32 {
        if self.size == 0 {
            return 0.0;
        }
        let y = match self.acc.get_mut(self.pos) {
            Some(a) => std::mem::take(a),
            None => 0.0,
        };
        self.pos += 1;
        if self.pos >= self.size {
            self.pos = 0;
        }
        y
    }

    /// Drop pending output.
    pub fn clear(&mut self) {
        let n = self.size.min(self.acc.len());
        self.acc[..n].fill(0.0);
        self.pos = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f32;
                0.5 * (t * 0.031).sin()
                    + 0.3 * (t * 0.47).cos()
                    + if i % 97 == 0 { 0.4 } else { 0.0 }
            })
            .collect()
    }

    /// Analysis followed by resynthesis, sample by sample.
    fn round_trip(x: &[f32], size: usize, hop: usize, window: Window) -> Vec<f32> {
        let mut fft = Fft::new(size);
        let mut an = StftAnalysis::new(size, hop, window);
        let mut ola = OverlapAdd::new(MAX_SIZE);
        assert!(ola.configure(size, hop, window));
        let mut bins = vec![0.0; 2 * fft.bins()];
        x.iter()
            .map(|&s| {
                if an.push(s) {
                    an.analyze(&mut fft, &mut bins);
                    ola.add(&mut fft, &bins);
                }
                ola.pop()
            })
            .collect()
    }

    #[test]
    fn sizes_are_repaired_to_powers_of_two() {
        assert_eq!(pow2_size(2048.0, 64, 16_384, 2048), 2048);
        assert_eq!(pow2_size(3000.0, 64, 16_384, 2048), 4096);
        assert_eq!(pow2_size(2800.0, 64, 16_384, 2048), 2048);
        assert_eq!(pow2_size(1.0, 64, 16_384, 2048), 64);
        assert_eq!(pow2_size(1e12, 64, 16_384, 2048), 16_384);
        assert_eq!(pow2_size(f64::NAN, 64, 16_384, 2048), 2048);
        assert_eq!(pow2_size(-5.0, 64, 16_384, 2048), 64);
        assert_eq!(overlap_factor(3.0), 4);
        assert_eq!(overlap_factor(0.0), 1);
        assert_eq!(overlap_factor(100.0), 8);
        assert_eq!(overlap_factor(2.0), 2);
    }

    #[test]
    fn sliding_window_keeps_the_newest_samples_in_order() {
        let mut w = SlidingWindow::new(4);
        assert_eq!(w.latest(), &[0.0; 4]);
        for i in 1..=6 {
            w.push(i as f32);
        }
        assert_eq!(w.latest(), &[3.0, 4.0, 5.0, 6.0]);
        w.push(7.0);
        assert_eq!(w.latest(), &[4.0, 5.0, 6.0, 7.0]);
        w.clear();
        assert_eq!(w.latest(), &[0.0; 4]);
    }

    #[test]
    fn frames_complete_every_hop_starting_at_hop_minus_one() {
        let mut an = StftAnalysis::new(16, 4, Window::Hann);
        let at: Vec<usize> = (0..20).filter(|_| an.push(1.0)).collect::<Vec<_>>();
        // `filter` consumes in order, so the indices are the push numbers.
        assert_eq!(at, vec![3, 7, 11, 15, 19]);
    }

    #[test]
    fn round_trip_is_the_input_delayed_by_size_minus_one() {
        let x = signal(6000);
        for window in [
            Window::Hann,
            Window::Hamming,
            Window::Blackman,
            Window::Rectangular,
        ] {
            for overlap in [2, 4, 8] {
                let size = 256;
                let y = round_trip(&x, size, size / overlap, window);
                for t in 0..x.len() {
                    let want = if t >= size - 1 {
                        x[t - (size - 1)]
                    } else {
                        0.0
                    };
                    assert!(
                        (y[t] - want).abs() < 1e-4,
                        "{window:?} overlap {overlap} at {t}: {} vs {want}",
                        y[t]
                    );
                }
            }
        }
        // Rectangular frames that don't overlap reconstruct too.
        let y = round_trip(&x, 128, 128, Window::Rectangular);
        assert!((y[1000] - x[1000 - 127]).abs() < 1e-5);
    }

    #[test]
    fn reconfiguring_clears_and_rejects_oversized_frames() {
        let mut ola = OverlapAdd::new(256);
        assert!(!ola.configure(512, 128, Window::Hann));
        assert_eq!(ola.size(), 0);
        assert_eq!(ola.pop(), 0.0);
        assert!(ola.configure(256, 64, Window::Hann));
        assert!(ola.is(256, 64, Window::Hann));
        let mut fft = Fft::new(256);
        // A DC-only spectrum: a constant frame.
        let mut bins = vec![0.0; 2 * fft.bins()];
        bins[0] = 256.0;
        ola.add(&mut fft, &bins);
        assert!((0..256).any(|_| ola.pop() != 0.0));
        ola.add(&mut fft, &bins);
        ola.configure(256, 64, Window::Hann);
        assert!((0..256).all(|_| ola.pop() == 0.0));
        // A size mismatch between the FFT and the layout adds nothing.
        let mut small = Fft::new(64);
        ola.add(&mut small, &bins);
        assert!((0..256).all(|_| ola.pop() == 0.0));
    }

    #[test]
    fn non_finite_input_and_bins_stay_finite() {
        let mut an = StftAnalysis::new(64, 16, Window::Hann);
        let mut fft = Fft::new(64);
        let mut bins = vec![0.0; 2 * fft.bins()];
        for i in 0..64 {
            an.push(if i % 3 == 0 { f32::NAN } else { f32::INFINITY });
        }
        an.analyze(&mut fft, &mut bins);
        assert!(bins.iter().all(|b| b.is_finite()));
        let mut ola = OverlapAdd::new(64);
        ola.configure(64, 16, Window::Hann);
        bins[6] = f32::NAN;
        ola.add(&mut fft, &bins);
        assert!((0..64).all(|_| ola.pop().is_finite()));
    }
}
