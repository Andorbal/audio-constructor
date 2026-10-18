//! A circular delay line with fractional, interpolated reads.
//!
//! Reads use 4-point Hermite interpolation ([`crate::interp::hermite4`]), so
//! the delay time can be swept smoothly (chorus, flanger, tape-style time
//! changes) without the zipper noise of rounding to whole samples; see
//! J. O. Smith, "Physical Audio Signal Processing", Delay Lines and
//! Interpolated Delay Lines.

use crate::interp::hermite4;

/// A fixed-length delay line. Allocates once in [`DelayLine::new`].
///
/// Per sample, [`read`](DelayLine::read) the delayed value first, then
/// [`write`](DelayLine::write) the new input: that order lets a feedback
/// path read and write the same line with a one-sample minimum delay.
#[derive(Clone, Debug)]
pub struct DelayLine {
    buf: Vec<f32>,
    /// Slot the next `write` fills.
    write: usize,
    /// Writes since the last clear (up to the buffer length). Older slots
    /// read as silence, which makes `clear` O(1) however long the line is.
    filled: usize,
}

/// Extra slots so the interpolator's neighbours stay inside the buffer.
const PAD: usize = 3;

impl DelayLine {
    /// A line that can delay by up to `max_delay` samples.
    pub fn new(max_delay: usize) -> Self {
        DelayLine {
            buf: vec![0.0; max_delay.max(1) + PAD],
            write: 0,
            filled: 0,
        }
    }

    /// The longest delay `read` honours, in samples.
    pub fn max_delay(&self) -> f64 {
        (self.buf.len() - PAD) as f64
    }

    /// The input from `delay` samples ago, interpolated. `delay` is clamped
    /// to `[1, max_delay]`; non-finite values read one sample back.
    #[inline]
    pub fn read(&self, delay: f64) -> f32 {
        let d = if delay.is_finite() {
            delay.clamp(1.0, self.max_delay())
        } else {
            1.0
        };
        // `d >= 1`, so truncating equals `floor`, without the library call
        // `floor` costs on baseline x86-64.
        let k = d as usize;
        let t = (d - k as f64) as f32;
        // Fast path: the four taps sit next to each other in the buffer
        // without wrapping (all but a few reads per lap). Same values as `ago`.
        if k >= 2 && k + 2 <= self.filled && self.write >= k + 2 {
            let i = self.write - (k + 2);
            if let Some(&[x2, x1, x0, xm1]) = self.buf.get(i..i + 4) {
                return hermite4(xm1, x0, x1, x2, t);
            }
        }
        let x0 = self.ago(k);
        // At one sample the newer neighbour is the sample not yet written;
        // repeating `x0` keeps the read causal.
        let xm1 = if k > 1 { self.ago(k - 1) } else { x0 };
        hermite4(xm1, x0, self.ago(k + 1), self.ago(k + 2), t)
    }

    /// Append one sample.
    #[inline]
    pub fn write(&mut self, x: f32) {
        if let Some(slot) = self.buf.get_mut(self.write) {
            *slot = x;
        }
        self.write += 1;
        if self.write >= self.buf.len() {
            self.write = 0;
        }
        if self.filled < self.buf.len() {
            self.filled += 1;
        }
    }

    /// The input from exactly `k` samples ago (`1 <= k <= max_delay`), with
    /// no interpolation; silence outside that range or older than the last
    /// clear.
    #[inline]
    pub fn tap(&self, k: usize) -> f32 {
        if k as f64 > self.max_delay() {
            return 0.0;
        }
        self.ago(k)
    }

    /// Silence the line. Constant time (nothing is overwritten), so it is
    /// safe on the audio thread even for minute-long lines.
    pub fn clear(&mut self) {
        self.write = 0;
        self.filled = 0;
    }

    /// The sample written `k` writes ago (`1 <= k <= len`); silence for
    /// anything older than the last clear.
    #[inline]
    fn ago(&self, k: usize) -> f32 {
        if k == 0 || k > self.filled {
            return 0.0;
        }
        let len = self.buf.len();
        let mut i = self.write + len - k.min(len);
        if i >= len {
            i -= len;
        }
        self.buf.get(i).copied().unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_delay_is_exact() {
        let mut d = DelayLine::new(16);
        let mut out = Vec::new();
        for n in 0..20 {
            out.push(d.read(5.0));
            d.write(if n == 0 { 1.0 } else { 0.0 });
        }
        let expect: Vec<f32> = (0..20).map(|n| if n == 5 { 1.0 } else { 0.0 }).collect();
        assert_eq!(out, expect);
    }

    #[test]
    fn fractional_delay_interpolates_a_ramp() {
        let mut d = DelayLine::new(16);
        for n in 0..10 {
            d.write(n as f32);
        }
        // Last written is 9 (one ago); 3.5 ago is halfway between 7 and 6.
        assert!((d.read(3.5) - 6.5).abs() < 1e-6);
        assert_eq!(d.read(1.0), 9.0);
    }

    #[test]
    fn reads_are_clamped_to_the_line() {
        let mut d = DelayLine::new(8);
        for n in 0..30 {
            d.write(n as f32);
        }
        assert_eq!(d.max_delay(), 8.0);
        assert_eq!(d.read(1e9), d.read(8.0));
        assert_eq!(d.read(-3.0), d.read(1.0));
        assert_eq!(d.read(f64::NAN), d.read(1.0));
        assert_eq!(d.read(8.0), 22.0);
    }

    #[test]
    fn taps_read_whole_samples() {
        let mut d = DelayLine::new(8);
        for n in 0..5 {
            d.write(n as f32 + 1.0);
        }
        assert_eq!(d.tap(1), 5.0);
        assert_eq!(d.tap(5), 1.0);
        assert_eq!(d.tap(6), 0.0);
        assert_eq!(d.tap(0), 0.0);
        assert_eq!(d.tap(9), 0.0);
        assert_eq!(d.tap(3), d.read(3.0));
    }

    #[test]
    fn clear_silences_old_history() {
        let mut d = DelayLine::new(8);
        for n in 0..30 {
            d.write(n as f32 + 1.0);
        }
        d.clear();
        for k in 1..=8 {
            assert_eq!(d.read(k as f64), 0.0, "{k}");
        }
        d.write(5.0);
        assert_eq!(d.read(1.0), 5.0);
        assert_eq!(d.read(2.0), 0.0);
        assert_eq!(d.read(8.0), 0.0);
    }
}
