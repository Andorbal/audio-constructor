use ac_core::{
    Channels, Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec,
    SpectrumShape, node,
};
use ac_dsp::fft::{Fft as Transform, Window};
use ac_dsp::stft::{MAX_SIZE, OverlapAdd};

use super::window;

node! {
    kind: "ifft",
    name: "IFFT",
    category: Spectral,
    keywords: ["inverse", "resynthesis", "overlap-add", "spectrum", "fourier", "istft"],
    /// Turns a stream of spectra back into audio by overlap-adding inverse
    /// transforms.
    ///
    /// The frame size and hop come from the incoming frames, so IFFT works
    /// behind any `FFT`. Each frame is transformed back, multiplied by the
    /// window, and added into the output starting at the sample the frame
    /// is stamped with. The windows' overlap is normalized position by
    /// position (least-squares resynthesis), so an unmodified FFT → IFFT
    /// round trip is exact, within rounding, for every window at overlap 2
    /// or more. At overlap 1 only the rectangular window reconstructs;
    /// tapered windows dip toward their frame edges.
    ///
    /// Config:
    /// - `window`: the window the frames were analyzed with, `"hann"`
    ///   (default), `"hamming"`, `"blackman"`, or `"rectangular"`. Set it to
    ///   match the `FFT`, or the level will ripple at the hop rate.
    ///
    /// IFFT adds no latency of its own: the `FFT` reports the round trip's
    /// `size − 1` samples. With nothing wired the output is silent. When the
    /// incoming frame size or hop changes, pending output is dropped and
    /// resynthesis starts over. Frame sizes other than powers of two from
    /// 16 to 16384 give silence. NaN or infinite bins count as zero, and
    /// the output is mono.
    ///
    /// **Try this:** wire `FFT` → IFFT with nothing in between and mix the
    /// result against the dry signal with one side inverted: the engine
    /// lines the paths up, so they cancel to silence. Then put a
    /// `SpectralGate` in the middle and listen to what it takes away.
    pub struct Ifft {
        /// One planned transform per power-of-two size, smallest first.
        transforms: Vec<Transform>,
        current: Option<usize>,
        shape: Option<SpectrumShape>,
        ola: OverlapAdd,
        window: Window,
    }
}

const SPECTRUM: usize = 0;
const OUT: usize = 0;

/// Smallest frame size resynthesized (`SpectrumShape` allows 16).
const SMALLEST: usize = 16;

impl Ifft {
    /// Switch to frames of `shape`, if a transform for its size exists.
    fn configure(&mut self, shape: Option<SpectrumShape>) {
        self.shape = shape;
        self.current = None;
        let Some(s) = shape else {
            self.ola.configure(0, 1, self.window);
            return;
        };
        let size = s.size as usize;
        self.current = self.transforms.iter().position(|t| t.size() == size);
        if self.current.is_none() || !self.ola.configure(size, s.hop as usize, self.window) {
            self.current = None;
            self.ola.configure(0, 1, self.window);
        }
    }
}

impl Node for Ifft {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::spectrum("spectrum").doc("Frames to turn back into audio."))
            .output(
                SocketSpec::audio("out")
                    .channels(Channels::Fixed(1))
                    .doc("The resynthesized signal."),
            )
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        // Frames can arrive in any size the producer chooses, and planning
        // isn't allowed on the audio thread, so every size is planned here.
        let mut transforms = Vec::new();
        let mut size = SMALLEST;
        while size <= MAX_SIZE {
            transforms.push(Transform::new(size));
            size *= 2;
        }
        Ifft {
            transforms,
            current: None,
            shape: None,
            ola: OverlapAdd::new(MAX_SIZE),
            window: window(config),
        }
    }

    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.spectrum(SPECTRUM);
        if input.shape() != self.shape && input.shape().is_some() {
            self.configure(input.shape());
        }
        let mut out = io.outputs.audio(OUT);
        let dst = out.channel(0);
        let mut frames = input.iter().peekable();
        for (i, o) in dst.iter_mut().enumerate() {
            while let Some((_, bins)) = frames.next_if(|f| f.0 as usize <= i) {
                if let Some(t) = self.current.and_then(|k| self.transforms.get_mut(k)) {
                    self.ola.add(t, bins);
                }
            }
            *o = self.ola.pop();
        }
    }

    fn reset(&mut self) {
        self.ola.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spectral::fft::Fft;
    use ac_core::NoResources;
    use ac_core::testing::{Frame, Harness, peak};
    use serde_json::json;

    const RATE: f64 = 48_000.0;

    fn analyze(config: Config, x: &[f32], block: usize) -> (SpectrumShape, Vec<Frame>) {
        let shape = Fft::describe(&config).outputs[0].spectrum.unwrap();
        let mut h = Harness::with_resources::<Fft>(config, RATE, block, &NoResources);
        h.set_audio("in", vec![x.to_vec()]);
        (shape, h.run(x.len()).spectrum("spectrum").to_vec())
    }

    fn resynth(
        config: Config,
        shape: SpectrumShape,
        f: Vec<Frame>,
        n: usize,
        block: usize,
    ) -> Vec<f32> {
        let mut h = Harness::with_resources::<Ifft>(config, RATE, block, &NoResources);
        h.spectrum("spectrum", shape, f);
        h.run(n).audio("out")[0].clone()
    }

    /// FFT → IFFT with matching windows.
    fn round_trip(config: Config, x: &[f32], block: usize) -> Vec<f32> {
        let (shape, f) = analyze(config.clone(), x, block);
        resynth(config, shape, f, x.len(), block)
    }

    fn signal(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f64 / RATE;
                (0.4 * (std::f64::consts::TAU * 220.0 * t).sin()
                    + 0.3 * (std::f64::consts::TAU * 3150.7 * t).sin()) as f32
            })
            .collect()
    }

    fn assert_delayed(x: &[f32], y: &[f32], delay: usize, tol: f32) {
        for t in 0..x.len() {
            let want = if t >= delay { x[t - delay] } else { 0.0 };
            assert!((y[t] - want).abs() < tol, "at {t}: {} vs {want}", y[t]);
        }
    }

    #[test]
    fn round_trip_is_the_input_delayed_by_the_reported_latency() {
        let x = signal(12_000);
        for (size, overlap, win) in [
            (2048, 4, "hann"),
            (1024, 2, "hann"),
            (512, 8, "blackman"),
            (256, 4, "hamming"),
            (64, 1, "rectangular"),
        ] {
            let config = json!({ "size": size, "overlap": overlap, "window": win });
            let ctx = PrepareCtx {
                sample_rate: RATE,
                max_block: 64,
                seed: 0,
                node_key: "t",
                name: "t",
                resources: &NoResources,
            };
            let latency =
                (Fft::new(&config, &ctx).latency() + Ifft::new(&config, &ctx).latency()) as usize;
            assert_eq!(latency, size - 1);
            let y = round_trip(config, &x, 64);
            assert_delayed(&x, &y, latency, 1e-4);
        }
    }

    #[test]
    fn an_impulse_comes_back_as_an_impulse() {
        let mut x = vec![0.0; 6000];
        x[1000] = 1.0;
        x[3333] = -0.5;
        let y = round_trip(json!({}), &x, 64);
        assert_delayed(&x, &y, 2047, 1e-5);
        assert!((y[3047] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let x = signal(6000);
        let config = json!({ "size": 512 });
        let one = round_trip(config.clone(), &x, 1);
        assert_eq!(one, round_trip(config.clone(), &x, 64));
        assert_eq!(one, round_trip(config, &x, 512));
    }

    #[test]
    fn unwired_is_silent_and_odd_sizes_too() {
        let mut h = Harness::new::<Ifft>(json!({}));
        assert_eq!(peak(&h.run(1000).audio("out")[0]), 0.0);
        // A 48-point frame has no planned transform.
        let shape = SpectrumShape::new(48, 12);
        let f: Vec<Frame> = (0..10).map(|k| (11 + 12 * k, vec![1.0; 50])).collect();
        assert_eq!(peak(&resynth(json!({}), shape, f, 200, 64)), 0.0);
    }

    #[test]
    fn non_finite_bins_count_as_zero() {
        let x = signal(4096);
        let (shape, mut f) = analyze(json!({ "size": 256 }), &x, 64);
        for frame in &mut f {
            frame.1[10] = f32::NAN;
            frame.1[11] = f32::INFINITY;
        }
        let y = resynth(json!({}), shape, f, 4096, 64);
        assert!(y.iter().all(|v| v.is_finite()));
        assert!(peak(&y) > 0.3);
    }

    #[test]
    fn shape_changes_restart_resynthesis() {
        let x = signal(4096);
        let (s1, f1) = analyze(json!({ "size": 256 }), &x, 64);
        let mut h = Harness::new::<Ifft>(json!({}));
        h.spectrum("spectrum", s1, f1);
        let a = h.run(4096).audio("out")[0].clone();
        assert_delayed(&x, &a, 255, 1e-4);
        // Now 512-point frames from the same signal, 4096 samples later.
        let (s2, f2) = analyze(json!({ "size": 512 }), &x, 64);
        let shifted: Vec<Frame> = f2.into_iter().map(|(t, b)| (t + 4096, b)).collect();
        h.spectrum("spectrum", s2, shifted);
        let b = h.run(4096).audio("out")[0].clone();
        assert_delayed(&x, &b, 511, 1e-4);
    }

    #[test]
    fn reset_drops_pending_output() {
        let x = signal(4096);
        let (shape, f) = analyze(json!({ "size": 1024 }), &x, 64);
        let mut h = Harness::new::<Ifft>(json!({}));
        h.spectrum(
            "spectrum",
            shape,
            f.into_iter().filter(|f| f.0 < 2048).collect(),
        );
        h.run(2048);
        h.reset();
        assert_eq!(peak(&h.run(2048).audio("out")[0]), 0.0);
    }
}
