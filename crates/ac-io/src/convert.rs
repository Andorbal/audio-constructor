//! Sample layout and format conversion between devices and the engine.
//!
//! Devices speak interleaved frames in their own sample format; the engine
//! speaks channel-major `f32` (channel `c` is `[c * frames .. (c + 1) * frames]`).
//! Everything here runs on the audio thread: no allocation, no panics, and
//! every index is bounds-checked by slicing rather than assumed.

use cpal::{FromSample, Sample};

/// Make a sample safe to send to a device: non-finite values become silence
/// and everything is clamped to ±1, so a runaway patch can't wrap an integer
/// format into full-scale noise or hand the driver a NaN.
#[inline]
pub fn sanitize(s: f32) -> f32 {
    if s.is_finite() {
        s.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// Write `frames` frames of channel-major `src` (with `src_channels`
/// channels) into the interleaved device buffer `dst` with `dst_channels`
/// channels, converting to the device format. Device channels the engine
/// doesn't have are silent.
pub fn interleave<T>(
    src: &[f32],
    src_channels: usize,
    frames: usize,
    dst: &mut [T],
    dst_channels: usize,
) where
    T: Sample + FromSample<f32>,
{
    if dst_channels == 0 {
        return;
    }
    let frames = frames.min(dst.len() / dst_channels);
    for (i, frame) in dst.chunks_exact_mut(dst_channels).take(frames).enumerate() {
        for (c, out) in frame.iter_mut().enumerate() {
            let s = if c < src_channels {
                src.get(c * frames + i).copied().unwrap_or(0.0)
            } else {
                0.0
            };
            *out = T::from_sample(sanitize(s));
        }
    }
}

/// Read interleaved device samples into channel-major `dst` (stride
/// `frames`). Missing samples are zero.
pub fn deinterleave<T>(src: &[T], channels: usize, dst: &mut [f32], frames: usize)
where
    T: Sample,
    f32: FromSample<T>,
{
    if channels == 0 {
        return;
    }
    let len = (channels * frames).min(dst.len());
    let dst = &mut dst[..len];
    dst.fill(0.0);
    for (i, frame) in src.chunks_exact(channels).take(frames).enumerate() {
        for (c, s) in frame.iter().enumerate() {
            if let Some(d) = dst.get_mut(c * frames + i) {
                *d = f32::from_sample(*s);
            }
        }
    }
}

/// Like [`deinterleave`] for `f32` samples split across two slices (the two
/// halves of a ring-buffer read), writing starting at frame `first` of each
/// channel. Returns the number of whole frames written.
pub fn deinterleave_parts(
    parts: (&[f32], &[f32]),
    channels: usize,
    dst: &mut [f32],
    frames: usize,
    first: usize,
) -> usize {
    if channels == 0 {
        return 0;
    }
    let total = (parts.0.len() + parts.1.len()) / channels;
    let n = total.min(frames.saturating_sub(first));
    for (k, s) in parts.0.iter().chain(parts.1).take(n * channels).enumerate() {
        let (i, c) = (k / channels, k % channels);
        if let Some(d) = dst.get_mut(c * frames + first + i) {
            *d = *s;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interleave_f32_and_extra_channels_are_silent() {
        // Two engine channels into a four-channel device.
        let src = [0.1, 0.2, 0.3, -0.1, -0.2, -0.3];
        let mut dst = [9.0f32; 12];
        interleave(&src, 2, 3, &mut dst, 4);
        assert_eq!(
            dst,
            [
                0.1, -0.1, 0.0, 0.0, 0.2, -0.2, 0.0, 0.0, 0.3, -0.3, 0.0, 0.0
            ]
        );
    }

    #[test]
    fn interleave_drops_engine_channels_the_device_lacks() {
        let src = [1.0, 2.0, 3.0].map(|v: f32| v * 0.1);
        let mut dst = [0.0f32; 1];
        interleave(&src, 3, 1, &mut dst, 1);
        assert_eq!(dst, [0.1]);
    }

    #[test]
    fn interleave_converts_and_clamps() {
        let src = [0.0, 0.5, -1.0, 2.0, f32::NAN, f32::INFINITY];
        let mut i16s = [0i16; 6];
        interleave(&src, 1, 6, &mut i16s, 1);
        assert_eq!(i16s[0], 0);
        assert_eq!(i16s[1], 16_384);
        assert_eq!(i16s[2], i16::MIN);
        assert_eq!(i16s[3], i16::MAX, "over-range clamps to full scale");
        assert_eq!(i16s[4], 0, "NaN is silence");
        assert_eq!(i16s[5], 0, "infinity is silence");

        let mut u16s = [0u16; 6];
        interleave(&src, 1, 6, &mut u16s, 1);
        assert_eq!(u16s[0], 32_768, "unsigned silence is mid-scale");
        assert_eq!(u16s[1], 49_152);
        assert_eq!(u16s[2], 0);
        assert_eq!(u16s[3], u16::MAX);
        assert_eq!(u16s[4], 32_768);
    }

    #[test]
    fn deinterleave_converts_formats() {
        let src: [i16; 4] = [0, 16_384, i16::MIN, -16_384];
        let mut dst = [9.0; 4];
        deinterleave(&src, 2, &mut dst, 2);
        assert_eq!(dst, [0.0, -1.0, 0.5, -0.5]);

        let src: [u16; 2] = [32_768, 0];
        let mut dst = [9.0; 2];
        deinterleave(&src, 1, &mut dst, 2);
        assert_eq!(dst, [0.0, -1.0]);
    }

    #[test]
    fn deinterleave_short_input_is_zero_padded() {
        let src = [0.5f32, -0.5];
        let mut dst = [9.0; 6];
        deinterleave(&src, 2, &mut dst, 3);
        assert_eq!(dst, [0.5, 0.0, 0.0, -0.5, 0.0, 0.0]);
    }

    #[test]
    fn deinterleave_parts_spans_both_halves() {
        let a = [1.0, 10.0, 2.0];
        let b = [20.0, 3.0, 30.0, 4.0];
        let mut dst = [0.0; 8];
        let n = deinterleave_parts((&a, &b), 2, &mut dst, 4, 1);
        assert_eq!(n, 3);
        assert_eq!(dst, [0.0, 1.0, 2.0, 3.0, 0.0, 10.0, 20.0, 30.0]);
    }

    #[test]
    fn round_trip_through_a_device_format() {
        let src: Vec<f32> = (0..64).map(|i| (i as f32 / 64.0) * 2.0 - 1.0).collect();
        let mut dev = vec![0i32; 64];
        interleave(&src, 2, 32, &mut dev, 2);
        let mut back = vec![0.0; 64];
        deinterleave(&dev, 2, &mut back, 32);
        for (a, b) in src.iter().zip(&back) {
            assert!((a - b).abs() < 1e-6, "{a} vs {b}");
        }
    }
}
