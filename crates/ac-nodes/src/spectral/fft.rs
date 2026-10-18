use ac_core::{Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node};
use ac_dsp::fft::Fft as Transform;
use ac_dsp::stft::StftAnalysis;

use super::{fft_size, overlap, window};

node! {
    kind: "fft",
    name: "FFT",
    category: Spectral,
    keywords: ["spectrum", "fourier", "stft", "frequency", "analysis", "bins"],
    /// Turns audio into a stream of spectra: a short-time Fourier transform
    /// of `in`, one frame every hop.
    ///
    /// Every hop (`size / overlap` samples) the newest `size` samples of the
    /// first channel of `in` are multiplied by the window and transformed.
    /// Each frame is stamped with the sample at which its last sample
    /// arrived, so spectral processing stays sample-accurate at any block
    /// size. A frame holds `size / 2 + 1` complex bins from 0 Hz up to half
    /// the sample rate, `sample_rate / size` Hz apart. Values are unscaled:
    /// a full-scale sine centered on a bin reads `size / 4` with the Hann
    /// window.
    ///
    /// Config:
    /// - `size`: samples per frame, a power of two from 64 to 16384
    ///   (default 2048). Other values go to the nearest power of two in
    ///   that range. Larger sizes resolve frequency more finely but blur
    ///   timing.
    /// - `overlap`: frames per `size` samples, 1, 2, 4, or 8 (default 4);
    ///   other values go to the nearest of those. More overlap is smoother
    ///   after processing and costs more.
    /// - `window`: `"hann"` (default), `"hamming"`, `"blackman"`, or
    ///   `"rectangular"`; unknown names use Hann.
    ///
    /// The node reports `size − 1` samples of latency: `IFFT` turns each
    /// frame back into audio starting at the frame's stamp, so FFT → IFFT
    /// gives back the input exactly `size − 1` samples later, and the engine
    /// delays parallel dry paths by the same amount so they line up. Before
    /// the first `size` samples the missing history counts as silence; the
    /// first frame arrives after one hop. Only the first channel is
    /// analyzed (use `SplitChannels` and one FFT per channel for more), and
    /// NaN or infinite samples count as silence.
    ///
    /// **Try this:** wire FFT → `SpectralGate` → `IFFT` to strip hiss from a
    /// recording, or FFT → `SpectralShift` (`scale` 2) → `IFFT` and mix it
    /// quietly under the dry signal for an octave shimmer.
    pub struct Fft {
        analysis: StftAnalysis,
        transform: Transform,
    }
}

const IN: usize = 0;
const SPECTRUM: usize = 0;

impl Node for Fft {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let size = fft_size(config, 2048);
        let hop = size / overlap(config);
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to analyze (first channel)."))
            .output(
                SocketSpec::spectrum("spectrum")
                    .shape(size as u32, hop as u32)
                    .doc("One frame of complex bins every hop."),
            )
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        let size = fft_size(config, 2048);
        Fft {
            analysis: StftAnalysis::new(size, size / overlap(config), window(config)),
            transform: Transform::new(size),
        }
    }

    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(IN);
        let x = input.channel(0);
        let mut out = io.outputs.spectrum(SPECTRUM);
        for (i, &s) in x.iter().enumerate() {
            if !self.analysis.push(s) {
                continue;
            }
            // Keep the history running even when nothing listens.
            if let Some(bins) = out.as_deref_mut().and_then(|o| o.push(i as u32)) {
                self.analysis.analyze(&mut self.transform, bins);
            }
        }
    }

    fn latency(&self) -> u32 {
        self.analysis.size().saturating_sub(1) as u32
    }

    fn reset(&mut self) {
        self.analysis.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::{Frame, Harness};
    use ac_core::{NoResources, SpectrumShape};
    use ac_dsp::fft::magnitude;
    use serde_json::json;

    const RATE: f64 = 48_000.0;

    fn sine(freq: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f64::consts::TAU * freq * i as f64 / RATE).sin() as f32)
            .collect()
    }

    fn frames(config: Config, x: &[f32], block: usize) -> Vec<Frame> {
        let mut h = Harness::with_resources::<Fft>(config, RATE, block, &NoResources);
        h.set_audio("in", vec![x.to_vec()]);
        h.run(x.len()).spectrum("spectrum").to_vec()
    }

    fn peak_bin(bins: &[f32]) -> usize {
        (0..bins.len() / 2)
            .max_by(|a, b| magnitude(bins, *a).total_cmp(&magnitude(bins, *b)))
            .unwrap()
    }

    #[test]
    fn frames_come_every_hop_with_the_declared_shape() {
        let sig = Fft::describe(&json!({}));
        assert_eq!(sig.outputs[0].spectrum, Some(SpectrumShape::new(2048, 512)));
        let f = frames(json!({ "size": 256, "overlap": 4 }), &[0.0; 1000], 64);
        let at: Vec<u64> = f.iter().map(|f| f.0).collect();
        assert_eq!(at, (0..15).map(|k| 63 + 64 * k).collect::<Vec<u64>>());
        assert!(f.iter().all(|f| f.1.len() == 2 * 129));
    }

    #[test]
    fn known_frequencies_land_in_their_bins() {
        // Bin k of a 1024-point FFT at 48 kHz is k × 46.875 Hz.
        for k in [3usize, 40, 200, 500] {
            let freq = k as f64 * RATE / 1024.0;
            let f = frames(json!({ "size": 1024 }), &sine(freq, 4096), 64);
            let (_, bins) = f.last().unwrap();
            assert_eq!(peak_bin(bins), k, "{freq} Hz");
            // A full-scale sine reads size / 4 with the Hann window.
            assert!(
                (magnitude(bins, k) - 256.0).abs() < 0.5,
                "{}",
                magnitude(bins, k)
            );
        }
        // An off-bin tone peaks at the nearest bin.
        let f = frames(json!({ "size": 2048 }), &sine(1000.0, 8192), 64);
        assert_eq!(peak_bin(&f.last().unwrap().1), 43);
    }

    #[test]
    fn config_is_repaired() {
        let shape = |c: Config| Fft::describe(&c).outputs[0].spectrum.unwrap();
        assert_eq!(
            shape(json!({ "size": 3000 })),
            SpectrumShape::new(4096, 1024)
        );
        assert_eq!(shape(json!({ "size": 1 })), SpectrumShape::new(64, 16));
        assert_eq!(
            shape(json!({ "size": 1e9 })),
            SpectrumShape::new(16_384, 4096)
        );
        assert_eq!(
            shape(json!({ "size": "big" })),
            SpectrumShape::new(2048, 512)
        );
        assert_eq!(
            shape(json!({ "overlap": 3 })),
            SpectrumShape::new(2048, 512)
        );
        assert_eq!(
            shape(json!({ "overlap": 0 })),
            SpectrumShape::new(2048, 2048)
        );
        assert_eq!(
            shape(json!({ "overlap": 99 })),
            SpectrumShape::new(2048, 256)
        );
        // An unknown window is Hann.
        let x = sine(3000.0, 4096);
        assert_eq!(
            frames(json!({ "window": "nope" }), &x, 64),
            frames(json!({ "window": "hann" }), &x, 64)
        );
        assert_ne!(
            frames(json!({ "window": "blackman" }), &x, 64),
            frames(json!({ "window": "hann" }), &x, 64)
        );
    }

    #[test]
    fn latency_is_size_minus_one() {
        let ctx = PrepareCtx {
            sample_rate: RATE,
            max_block: 64,
            seed: 0,
            node_key: "t",
            name: "t",
            resources: &NoResources,
        };
        assert_eq!(Fft::new(&json!({}), &ctx).latency(), 2047);
        assert_eq!(Fft::new(&json!({ "size": 64 }), &ctx).latency(), 63);
    }

    #[test]
    fn block_size_does_not_change_the_frames() {
        let x: Vec<f32> = (0..5000)
            .map(|i| (i as f32 * 0.01).sin() + 0.3 * (i as f32 * 0.37).cos())
            .collect();
        let config = json!({ "size": 512, "overlap": 8 });
        let one = frames(config.clone(), &x, 1);
        assert_eq!(one.len(), 5000 / 64);
        assert_eq!(one, frames(config.clone(), &x, 64));
        assert_eq!(one, frames(config, &x, 512));
    }

    #[test]
    fn non_finite_input_counts_as_silence() {
        let mut x = sine(500.0, 4096);
        for v in x.iter_mut().step_by(7) {
            *v = f32::NAN;
        }
        x[100] = f32::INFINITY;
        let f = frames(json!({ "size": 256 }), &x, 64);
        assert!(f.iter().all(|f| f.1.iter().all(|b| b.is_finite())));
    }

    #[test]
    fn reset_forgets_history_and_restarts_the_hop() {
        let mut h = Harness::new::<Fft>(json!({ "size": 128, "overlap": 2 }));
        h.set_audio("in", vec![vec![1.0; 100]]);
        h.run(100);
        h.reset();
        let out = h.run(128);
        let f = out.spectrum("spectrum");
        // Frames 63 and 127 samples after the reset (absolute 163, 227);
        // the input is silent from 100 on, so they are all zero.
        assert_eq!(f.iter().map(|f| f.0).collect::<Vec<_>>(), vec![163, 227]);
        assert!(f.iter().all(|f| f.1.iter().all(|b| *b == 0.0)));
    }
}
