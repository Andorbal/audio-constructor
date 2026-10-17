//! Offline sample-rate conversion by band-limited (windowed-sinc)
//! interpolation, for converting audio files to the engine's rate on load.
//!
//! Each output sample is a dot product of the input with a Kaiser-windowed
//! sinc centred on the output instant, following J. O. Smith, "Digital
//! Audio Resampling Home Page" (CCRMA, bandlimited interpolation): the
//! kernel is tabulated once with linear interpolation between table points,
//! and when downsampling it is stretched (its cutoff lowered to the output
//! Nyquist) so nothing aliases. The window is Kaiser's I0-sinh window
//! (J. F. Kaiser, "Nonrecursive Digital Filter Design Using the I0-sinh
//! Window Function", Proc. IEEE ISCAS, 1974).
//!
//! With 32 zero crossings per side, β = 8 (about 80 dB of stopband
//! attenuation) and the cutoff at 92% of the lower Nyquist, the passband is
//! flat to about 85% of Nyquist (18.7 kHz for 44.1 kHz) and images or
//! aliases are suppressed by about 80 dB. This allocates and is meant for
//! load time, not the audio thread.

use std::f64::consts::PI;

/// Zero crossings of the sinc on each side of the centre.
const ZEROS: usize = 32;
/// Table points per zero crossing.
const RESOLUTION: usize = 512;
/// Kaiser window shape.
const BETA: f64 = 8.0;
/// Cutoff as a fraction of the lower of the two Nyquist frequencies.
const ROLLOFF: f64 = 0.92;

/// A converter between two fixed rates.
#[derive(Clone, Debug)]
pub struct Resampler {
    /// Output rate / input rate.
    ratio: f64,
    /// Kernel cutoff relative to the input Nyquist.
    cutoff: f64,
    /// Kernel half-width in input samples.
    half_width: f64,
    /// The windowed sinc at `RESOLUTION` points per zero crossing, from 0
    /// to `ZEROS` (it's symmetric), plus a trailing zero.
    table: Vec<f64>,
}

impl Resampler {
    /// A converter from `from` Hz to `to` Hz, or `None` if either rate is
    /// not a positive finite number.
    pub fn new(from: f64, to: f64) -> Option<Resampler> {
        if !(from.is_finite() && to.is_finite() && from > 0.0 && to > 0.0) {
            return None;
        }
        let ratio = to / from;
        let cutoff = ROLLOFF * ratio.min(1.0);
        let n = ZEROS * RESOLUTION;
        let i0_beta = bessel_i0(BETA);
        let mut table: Vec<f64> = (0..=n)
            .map(|j| {
                let x = j as f64 / RESOLUTION as f64;
                let u = x / ZEROS as f64;
                sinc(x) * bessel_i0(BETA * (1.0 - u * u).max(0.0).sqrt()) / i0_beta
            })
            .collect();
        table.push(0.0);
        Some(Resampler {
            ratio,
            cutoff,
            half_width: ZEROS as f64 / cutoff,
            table,
        })
    }

    /// Output length for `input_len` input samples (the same duration).
    pub fn output_len(&self, input_len: usize) -> usize {
        (input_len as f64 * self.ratio).round() as usize
    }

    /// Kernel value at `x` zero crossings from the centre.
    #[inline]
    fn kernel(&self, x: f64) -> f64 {
        let pos = x.abs() * RESOLUTION as f64;
        let j = pos as usize;
        let frac = pos - j as f64;
        match (self.table.get(j), self.table.get(j + 1)) {
            (Some(a), Some(b)) => a + frac * (b - a),
            _ => 0.0,
        }
    }

    /// Convert one channel. Samples outside the input are taken as silence.
    pub fn process(&self, input: &[f32]) -> Vec<f32> {
        let n_out = self.output_len(input.len());
        let last = input.len() as f64 - 1.0;
        let step = 1.0 / self.ratio;
        (0..n_out)
            .map(|n| {
                // Output instant, in input samples.
                let t = n as f64 * step;
                let lo = (t - self.half_width).ceil().max(0.0);
                let hi = (t + self.half_width).floor().min(last);
                if hi < lo {
                    return 0.0;
                }
                let (lo, hi) = (lo as usize, hi as usize);
                let taps = input.get(lo..=hi).unwrap_or(&[]);
                let acc: f64 = taps
                    .iter()
                    .enumerate()
                    .map(|(k, &x)| {
                        let d = t - (lo + k) as f64;
                        f64::from(x) * self.kernel(d * self.cutoff)
                    })
                    .sum();
                (acc * self.cutoff) as f32
            })
            .collect()
    }
}

/// Convert `input` from `from` Hz to `to` Hz. Equal or invalid rates return
/// a copy.
pub fn resample(input: &[f32], from: f64, to: f64) -> Vec<f32> {
    match Resampler::new(from, to) {
        Some(r) if from != to => r.process(input),
        _ => input.to_vec(),
    }
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-12 {
        1.0
    } else {
        (PI * x).sin() / (PI * x)
    }
}

/// Modified Bessel function of the first kind, order 0, by its power series
/// (Abramowitz & Stegun 9.6.12); converges quickly for the β used here.
fn bessel_i0(x: f64) -> f64 {
    let half = x / 2.0;
    let mut term = 1.0;
    let mut sum = 1.0;
    for k in 1..200 {
        term *= half / k as f64;
        let t2 = term * term;
        sum += t2;
        if t2 < sum * 1e-16 {
            break;
        }
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    fn sine(freq: f64, rate: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (TAU * freq * i as f64 / rate).sin() as f32)
            .collect()
    }

    /// Goertzel magnitude, normalized so a full-scale sine reads about 1.
    fn magnitude(x: &[f32], freq: f64, rate: f64) -> f64 {
        let w = TAU * freq / rate;
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &v in x {
            let s = f64::from(v) + coeff * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        2.0 * (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0).sqrt() / x.len() as f64
    }

    #[test]
    fn bessel_matches_known_values() {
        assert!((bessel_i0(0.0) - 1.0).abs() < 1e-15);
        assert!((bessel_i0(1.0) - 1.266_065_877_752_008).abs() < 1e-12);
        assert!((bessel_i0(8.0) - 427.564_115_721_804_7).abs() < 1e-9);
    }

    #[test]
    fn equal_rates_copy() {
        let x = sine(440.0, 48_000.0, 100);
        assert_eq!(resample(&x, 48_000.0, 48_000.0), x);
        assert_eq!(resample(&x, 0.0, 48_000.0), x);
        assert_eq!(resample(&x, 44_100.0, f64::NAN), x);
    }

    #[test]
    fn length_follows_the_ratio() {
        let r = Resampler::new(44_100.0, 48_000.0).unwrap();
        assert_eq!(r.output_len(44_100), 48_000);
        assert_eq!(r.process(&[0.0; 441]).len(), 480);
        assert!(r.process(&[]).is_empty());
    }

    #[test]
    fn tones_keep_frequency_and_level_both_ways() {
        for (from, to) in [
            (44_100.0, 48_000.0),
            (48_000.0, 44_100.0),
            (22_050.0, 96_000.0),
        ] {
            let x = sine(1_000.0, from, from as usize);
            let y = resample(&x, from, to);
            // Skip the edges, where the input starts and stops abruptly.
            let mid = &y[y.len() / 10..y.len() * 9 / 10];
            let m = magnitude(mid, 1_000.0, to);
            assert!((m - 1.0).abs() < 0.01, "{from}->{to}: {m}");
            // The tone's first image sits at `from - 1 kHz`; if the kernel
            // let it through it would land here in the output band.
            let mut image = (from - 1_000.0) % to;
            if image > to / 2.0 {
                image = to - image;
            }
            let leak = magnitude(mid, image, to);
            assert!(leak < 1e-3, "{from}->{to}: image at {image} Hz is {leak}");
        }
    }

    #[test]
    fn passband_is_flat_to_high_frequencies() {
        let x = sine(16_000.0, 44_100.0, 44_100);
        let y = resample(&x, 44_100.0, 48_000.0);
        let m = magnitude(&y[4_800..43_200], 16_000.0, 48_000.0);
        assert!((m - 1.0).abs() < 0.01, "{m}");
    }

    #[test]
    fn downsampling_does_not_alias() {
        // 30 kHz is above the 24 kHz output Nyquist; it would fold to 18 kHz.
        let x = sine(30_000.0, 96_000.0, 96_000);
        let y = resample(&x, 96_000.0, 48_000.0);
        let alias = magnitude(&y[4_800..43_200], 18_000.0, 48_000.0);
        assert!(alias < 1e-3, "{alias}");
    }

    #[test]
    fn dc_is_preserved() {
        let y = resample(&[1.0; 4_410], 44_100.0, 48_000.0);
        for v in &y[200..y.len() - 200] {
            assert!((v - 1.0).abs() < 1e-3, "{v}");
        }
    }
}
