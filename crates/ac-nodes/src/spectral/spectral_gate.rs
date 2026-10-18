use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, SpectrumShape, Unit,
    node,
};

use super::{MAX_BINS, finite_or};

node! {
    kind: "spectral_gate",
    name: "Spectral Gate",
    category: Spectral,
    keywords: ["denoise", "noise reduction", "hiss", "gate", "spectrum", "cleanup"],
    /// Silences the bins of a spectrum that fall below a threshold, leaving
    /// the louder ones: a noise gate per frequency.
    ///
    /// - `threshold` is an absolute level in dB for each bin, where 0 dB is
    ///   a full-scale sine analyzed with the Hann window (with Hamming it
    ///   reads 0.7 dB higher, Blackman 1.5 dB lower, rectangular 6 dB
    ///   higher). Bins at or above it pass; quieter bins are turned down.
    /// - `floor` is the gain, in dB, for bins below the threshold: −120 or
    ///   less removes them entirely; −20 just tucks the noise under.
    /// - `smoothing` is how long, in seconds, each bin's gain takes to move
    ///   between open and closed (a one-pole glide, the same both ways).
    ///   Without it, bins hovering near the threshold flicker on and off
    ///   from frame to frame and the leftover noise turns into watery
    ///   "musical noise"; 0 switches at once.
    ///
    /// Inputs are read at each frame's own sample, so a wired `threshold`
    /// changes the gate at the right moment. The output has the input's
    /// frame size and hop, and phases pass through untouched. The gate opens
    /// or closes fully on the first frame after a reset or a change of frame
    /// size. NaN or infinite bins come out as zero.
    ///
    /// **Try this:** put it between `FFT` and `IFFT` on a hissy recording,
    /// start with `threshold` at −60 dB and raise it until the hiss goes,
    /// then raise `smoothing` until what remains stops warbling.
    pub struct SpectralGate {
        gains: Vec<f32>,
        shape: Option<SpectrumShape>,
        fresh: bool,
    }
}

const SPECTRUM: usize = 0;
const THRESHOLD: usize = 1;
const SMOOTHING: usize = 2;
const FLOOR: usize = 3;
const OUT: usize = 0;

/// Gains at or below this (dB) are exactly zero.
const SILENT_DB: f32 = -120.0;

impl Node for SpectralGate {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::spectrum("spectrum").doc("Frames to gate."))
            .input(
                SocketSpec::float("threshold", Unit::Db, -50.0)
                    .range(-120.0, 0.0)
                    .doc("Bins quieter than this are turned down."),
            )
            .input(
                SocketSpec::float("smoothing", Unit::Seconds, 0.05)
                    .range(0.0, 1.0)
                    .doc("Time each bin's gain takes to open or close."),
            )
            .input(
                SocketSpec::float("floor", Unit::Db, -80.0)
                    .range(-120.0, 0.0)
                    .doc("Gain for gated bins; -120 removes them."),
            )
            .output(SocketSpec::spectrum("spectrum").doc("The gated frames."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        SpectralGate {
            gains: vec![1.0; MAX_BINS],
            shape: None,
            fresh: true,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.spectrum(SPECTRUM);
        if input.shape() != self.shape {
            self.shape = input.shape();
            self.fresh = true;
        }
        let (Some(shape), Some(out)) = (self.shape, io.outputs.spectrum(OUT)) else {
            return;
        };
        let threshold = io.inputs.float(THRESHOLD);
        let smoothing = io.inputs.float(SMOOTHING);
        let floor = io.inputs.float(FLOOR);
        // Bin magnitude of a full-scale, bin-centered sine with Hann.
        let full_scale = shape.size as f32 / 4.0;
        let hop = f64::from(shape.hop);
        for (offset, bins) in input.iter() {
            let i = offset as usize;
            let thr_db = finite_or(threshold.at(i), -50.0).clamp(-200.0, 100.0);
            let thr = full_scale * 10f32.powf(thr_db / 20.0);
            let thr2 = thr * thr;
            let floor_db = finite_or(floor.at(i), -80.0).min(0.0);
            let closed = if floor_db <= SILENT_DB {
                0.0
            } else {
                10f32.powf(floor_db / 20.0)
            };
            let time = f64::from(finite_or(smoothing.at(i), 0.05));
            let coef = if time > 0.0 {
                (1.0 - (-hop / (time * ctx.sample_rate)).exp()) as f32
            } else {
                1.0
            };
            let coef = if self.fresh { 1.0 } else { coef };
            let Some(dst) = out.push(offset) else {
                continue;
            };
            for ((d, s), g) in dst
                .chunks_exact_mut(2)
                .zip(bins.chunks_exact(2))
                .zip(self.gains.iter_mut())
            {
                let (re, im) = (s[0], s[1]);
                if !(re.is_finite() && im.is_finite()) {
                    d[0] = 0.0;
                    d[1] = 0.0;
                    continue;
                }
                let target = if re * re + im * im >= thr2 {
                    1.0
                } else {
                    closed
                };
                *g += (target - *g) * coef;
                d[0] = re * *g;
                d[1] = im * *g;
            }
            self.fresh = false;
        }
    }

    fn reset(&mut self) {
        self.fresh = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spectral::fft::Fft;
    use crate::spectral::ifft::Ifft;
    use ac_core::NoResources;
    use ac_core::testing::{Frame, Harness, peak, tone_magnitude};
    use ac_dsp::fft::magnitude;
    use serde_json::json;

    const RATE: f64 = 48_000.0;
    const SHAPE: SpectrumShape = SpectrumShape { size: 16, hop: 4 };

    /// Frames of 9 bins with bin k at magnitude `mags[k]` (real).
    fn frame(at: u64, mags: &[f32]) -> Frame {
        let mut v = vec![0.0; 18];
        for (k, m) in mags.iter().enumerate() {
            v[2 * k] = *m;
        }
        (at, v)
    }

    fn gate(frames: Vec<Frame>, set: &[(&str, f64)], block: usize) -> Vec<Frame> {
        let n = frames.last().map_or(64, |f| f.0 as usize + 1);
        let mut h = Harness::with_resources::<SpectralGate>(json!({}), RATE, block, &NoResources);
        h.spectrum("spectrum", SHAPE, frames);
        for (k, v) in set {
            h.set(k, *v);
        }
        h.run(n).spectrum("spectrum").to_vec()
    }

    #[test]
    fn bins_below_the_threshold_are_removed() {
        // 0 dB is size / 4 = 4 for a 16-point frame.
        let f = vec![frame(3, &[4.0, 0.04, 0.4, 4.0, 0.0, 0.0, 0.0, 0.0, 0.004])];
        let out = gate(f, &[("threshold", -30.0), ("floor", -120.0)], 64);
        assert_eq!(out.len(), 1);
        let (at, b) = &out[0];
        assert_eq!(*at, 3);
        // −20 dB (0.4) passes, −40 dB (0.04) and −60 dB go.
        let mags: Vec<f32> = (0..9).map(|k| magnitude(b, k)).collect();
        assert_eq!(mags, vec![4.0, 0.0, 0.4, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn floor_sets_the_gain_of_gated_bins() {
        let f = vec![frame(3, &[0.1; 9])];
        let out = gate(f, &[("threshold", 0.0), ("floor", -20.0)], 64);
        assert!((magnitude(&out[0].1, 4) - 0.01).abs() < 1e-6);
    }

    #[test]
    fn smoothing_glides_the_gains() {
        // A bin that is loud for a while, then drops below the threshold.
        let mut frames: Vec<Frame> = (0..20).map(|k| frame(3 + 4 * k, &[4.0; 9])).collect();
        frames.extend((20..60).map(|k| frame(3 + 4 * k, &[0.04; 9])));
        // 0.04 is −40 dB for a 16-point frame; the threshold is −30 dB.
        let set = |smoothing: f64| {
            [
                ("smoothing", smoothing),
                ("floor", -120.0),
                ("threshold", -30.0),
            ]
        };
        let instant = gate(frames.clone(), &set(0.0), 64);
        assert_eq!(magnitude(&instant[20].1, 0), 0.0);
        // 4 samples per frame at 48 kHz with a 1 ms time constant: the gain
        // falls by exp(−4/48) per frame.
        let smooth = gate(frames, &set(0.001), 64);
        let g20 = magnitude(&smooth[20].1, 0) / 0.04;
        let g21 = magnitude(&smooth[21].1, 0) / 0.04;
        let fall = (-4.0f32 / 48.0).exp();
        assert!((g20 - fall).abs() < 1e-4, "{g20}");
        assert!((g21 - fall * fall).abs() < 1e-4, "{g21}");
        // The first frame starts at its target, without a glide.
        assert_eq!(magnitude(&smooth[0].1, 0), 4.0);
    }

    #[test]
    fn block_size_and_wired_inputs_follow_frame_times() {
        let frames: Vec<Frame> = (0..100)
            .map(|k| frame(3 + 4 * k, &[(k % 7) as f32 * 0.5; 9]))
            .collect();
        let thr: Vec<f32> = (0..400).map(|i| -30.0 + (i % 50) as f32 * 0.5).collect();
        let run = |block: usize| {
            let mut h =
                Harness::with_resources::<SpectralGate>(json!({}), RATE, block, &NoResources);
            h.spectrum("spectrum", SHAPE, frames.clone())
                .set_samples("threshold", thr.clone());
            h.run(400).spectrum("spectrum").to_vec()
        };
        let one = run(1);
        assert_eq!(one.len(), 100);
        assert_eq!(one, run(64));
        assert_eq!(one, run(512));
    }

    #[test]
    fn non_finite_bins_and_inputs_stay_finite() {
        let mut f = frame(3, &[4.0; 9]);
        f.1[4] = f32::NAN;
        f.1[7] = f32::INFINITY;
        let out = gate(
            vec![f],
            &[("threshold", f64::NAN), ("smoothing", f64::INFINITY)],
            64,
        );
        assert!(out[0].1.iter().all(|v| v.is_finite()));
        assert_eq!(out[0].1[4], 0.0);
        assert_eq!(out[0].1[0], 4.0);
    }

    #[test]
    fn reset_and_unwired_are_harmless() {
        let mut h = Harness::new::<SpectralGate>(json!({}));
        let out = h.run(256);
        assert!(out.spectra.is_empty());
        h.reset();
        h.run(64);
    }

    /// FFT → gate → IFFT on `x`.
    fn chain(x: &[f32], threshold: f64) -> Vec<f32> {
        let n = x.len();
        let shape = SpectrumShape::new(2048, 512);
        let mut fft = Harness::new::<Fft>(json!({}));
        fft.set_audio("in", vec![x.to_vec()]);
        let frames = fft.run(n).spectrum("spectrum").to_vec();
        let mut g = Harness::new::<SpectralGate>(json!({}));
        g.spectrum("spectrum", shape, frames)
            .set("threshold", threshold)
            .set("floor", -120.0);
        let gated = g.run(n).spectrum("spectrum").to_vec();
        let mut ifft = Harness::new::<Ifft>(json!({}));
        ifft.spectrum("spectrum", shape, gated);
        ifft.run(n).audio("out")[0].clone()
    }

    #[test]
    fn removes_quiet_noise_and_keeps_a_tone_through_fft_and_ifft() {
        let n = 24_000;
        let mut s = 7u32;
        let noise: Vec<f32> = (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32 - 0.5) * 0.002
            })
            .collect();
        // Noise around −90 dB per bin is removed completely.
        assert_eq!(peak(&chain(&noise, -40.0)), 0.0);
        // A −6 dB tone on top of it survives at its level.
        let x: Vec<f32> = noise
            .iter()
            .enumerate()
            .map(|(i, v)| v + 0.5 * (std::f64::consts::TAU * 1000.0 * i as f64 / RATE).sin() as f32)
            .collect();
        let y = chain(&x, -40.0);
        assert!((tone_magnitude(&y[8000..], 1000.0, RATE) - 0.5).abs() < 0.01);
    }
}
