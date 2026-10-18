use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node,
};
use ac_dsp::biquad::Tdf2;
use ac_dsp::crossover::Crossover;
use ac_dsp::fft::{Fft as Transform, Window};
use ac_dsp::stft::{OverlapAdd, StftAnalysis};
use serde_json::Value;

use super::fft_size;

node! {
    kind: "band_split",
    name: "Band Split",
    category: Spectral,
    keywords: ["crossover", "multiband", "bands", "linkwitz", "split", "frequency"],
    /// Splits a signal into frequency bands, one output per band, that add
    /// back up to the original.
    ///
    /// Config:
    /// - `edges`: the band edges in Hz, a list such as `[200, 2000]`
    ///   (the default: three bands, below 200 Hz, 200 Hz to 2 kHz, and
    ///   above). Edges are sorted and kept within 20 Hz to 20 kHz; at most
    ///   15 are used (16 bands), and entries that aren't numbers are
    ///   ignored. An empty list gives one band. Outputs are `band_0` (the
    ///   lowest) up to `band_N`. At low sample rates, edges near or above
    ///   half the sample rate leave the bands above them (almost) empty.
    /// - `mode`: how to split.
    ///   - `"crossover"` (default): Linkwitz-Riley 4th-order crossovers
    ///     (24 dB per octave, −6 dB at each edge) arranged in a tree, with
    ///     allpass filters keeping every band in phase with the others. The
    ///     bands sum to the input with a flat level and a smoothly shifted
    ///     phase, with no latency. The way to build multiband dynamics.
    ///   - `"fft"`: brick-wall bands from a short-time Fourier transform
    ///     (Hann window, overlap 4): each frequency bin goes to exactly one
    ///     band, so the bands sum back to the input exactly, delayed by
    ///     `size − 1` samples, which the node reports as latency. Sharper
    ///     than crossovers, but sharp edges ring around transients.
    /// - `size`: FFT size for `"fft"` mode, a power of two from 64 to 16384
    ///   (default 2048).
    ///
    /// Every band has the input's channel count. The first two channels are
    /// split; any further channels go to `band_0` unchanged (not delayed in
    /// `"fft"` mode) and are silent in the other bands.
    ///
    /// **Try this:** split a drum loop at `[150, 2500]`, run each band
    /// through its own `Gain` (and later a compressor block), and `Mix`
    /// them back together: a multiband processor. Mute the middle band to
    /// hear just the kick and the cymbals.
    pub struct BandSplit {
        engine: Engine,
        /// One block of each band, for one channel at a time.
        scratch: Vec<Vec<f32>>,
        /// Samples processed since the last reset.
        clock: u64,
    }
}

/// The two ways of splitting.
enum Engine {
    Crossover {
        tree: Crossover,
        states: [Vec<Tdf2>; CHANNELS],
    },
    Fft(Box<FftBands>),
}

/// State for brick-wall STFT bands.
struct FftBands {
    transform: Transform,
    analysis: [StftAnalysis; CHANNELS],
    /// Resynthesis per channel, then per band.
    ola: Vec<OverlapAdd>,
    /// First bin of each band (plus the bin count at the end).
    starts: Vec<usize>,
    spectrum: Vec<f32>,
    masked: Vec<f32>,
}

const IN: usize = 0;

/// Channels that get split.
const CHANNELS: usize = 2;
/// Most edges honoured.
const MAX_EDGES: usize = 15;
/// Edges are kept in this range (Hz).
const LOWEST: f64 = 20.0;
const HIGHEST: f64 = 20_000.0;
/// Tiny filter state is flushed on this grid of samples, counted from the
/// last reset, so the output doesn't depend on the block size.
const FLUSH_EVERY: u64 = 64;

/// The edges from config: sorted, clamped, at most [`MAX_EDGES`].
pub fn edges(config: &Config) -> Vec<f64> {
    let mut v: Vec<f64> = match config.get("edges") {
        None => vec![200.0, 2000.0],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_f64)
            .filter(|f| f.is_finite())
            .take(MAX_EDGES)
            .collect(),
        Some(v) => v
            .as_f64()
            .filter(|f| f.is_finite())
            .map_or_else(|| vec![200.0, 2000.0], |f| vec![f]),
    };
    for e in &mut v {
        *e = e.clamp(LOWEST, HIGHEST);
    }
    v.sort_by(f64::total_cmp);
    v
}

fn is_fft(config: &Config) -> bool {
    config.get_str_or("mode", "crossover") == "fft"
}

impl FftBands {
    fn new(edges: &[f64], size: usize, rate: f64) -> FftBands {
        let bands = edges.len() + 1;
        let transform = Transform::new(size);
        let bins = transform.bins();
        let hop = size / 4;
        let mut starts = vec![0];
        // A bin belongs to the band its center frequency falls in.
        for &e in edges {
            let k = (e * size as f64 / rate).ceil().clamp(0.0, bins as f64) as usize;
            starts.push(k);
        }
        starts.push(bins);
        let ola = (0..CHANNELS * bands)
            .map(|_| {
                let mut o = OverlapAdd::new(size);
                o.configure(size, hop, Window::Hann);
                o
            })
            .collect();
        FftBands {
            analysis: [
                StftAnalysis::new(size, hop, Window::Hann),
                StftAnalysis::new(size, hop, Window::Hann),
            ],
            transform,
            ola,
            starts,
            spectrum: vec![0.0; 2 * bins],
            masked: vec![0.0; 2 * bins],
        }
    }

    /// Split channel `c` of one block into `scratch`.
    fn process(&mut self, c: usize, x: &[f32], scratch: &mut [Vec<f32>]) {
        let bands = self.starts.len() - 1;
        let Some(analysis) = self.analysis.get_mut(c) else {
            return;
        };
        for (i, &s) in x.iter().enumerate() {
            if analysis.push(s) {
                analysis.analyze(&mut self.transform, &mut self.spectrum);
                for b in 0..bands {
                    let (lo, hi) = (2 * self.starts[b], 2 * self.starts[b + 1]);
                    self.masked.fill(0.0);
                    if lo < hi {
                        self.masked[lo..hi].copy_from_slice(&self.spectrum[lo..hi]);
                    }
                    if let Some(o) = self.ola.get_mut(c * bands + b) {
                        o.add(&mut self.transform, &self.masked);
                    }
                }
            }
            for (b, band) in scratch.iter_mut().enumerate().take(bands) {
                let y = self.ola.get_mut(c * bands + b).map_or(0.0, OverlapAdd::pop);
                if let Some(v) = band.get_mut(i) {
                    *v = y;
                }
            }
        }
    }

    fn reset(&mut self) {
        for a in &mut self.analysis {
            a.reset();
        }
        for o in &mut self.ola {
            o.clear();
        }
    }
}

impl Node for BandSplit {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let mut sig = Signature::new().input(SocketSpec::audio("in").doc("Signal to split."));
        for b in 0..edges(config).len() + 1 {
            sig = sig.output(
                SocketSpec::audio(format!("band_{b}"))
                    .doc("One frequency band of the input (band_0 is the lowest)."),
            );
        }
        sig
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let edges = edges(config);
        let bands = edges.len() + 1;
        let engine = if is_fft(config) {
            Engine::Fft(Box::new(FftBands::new(
                &edges,
                fft_size(config, 2048),
                ctx.sample_rate,
            )))
        } else {
            let tree = Crossover::new(&edges, ctx.sample_rate);
            Engine::Crossover {
                states: [tree.state(), tree.state()],
                tree,
            }
        };
        BandSplit {
            engine,
            scratch: vec![vec![0.0; ctx.max_block.max(1)]; bands],
            clock: 0,
        }
    }

    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(IN);
        let frames = io.frames();
        let bands = self.scratch.len();
        let channels = io.outputs.audio(0).channels();
        let clock = self.clock;
        for c in 0..channels {
            let src = input.channel(c);
            if c >= CHANNELS {
                // Extra channels: unchanged in band 0, silent elsewhere.
                for b in 0..bands {
                    let mut out = io.outputs.audio(b);
                    let dst = out.channel(c);
                    if b == 0 {
                        let n = dst.len().min(src.len());
                        dst[..n].copy_from_slice(&src[..n]);
                    } else {
                        dst.fill(0.0);
                    }
                }
                continue;
            }
            match &mut self.engine {
                Engine::Crossover { tree, states } => {
                    let state = &mut states[c];
                    let mut i = 0;
                    let mut t = clock;
                    while i < frames {
                        let n = ((FLUSH_EVERY - t % FLUSH_EVERY) as usize).min(frames - i);
                        let Some(chunk) = src.get(i..i + n) else {
                            break;
                        };
                        tree.process(state, chunk, &mut self.scratch);
                        for (b, band) in self.scratch.iter().enumerate() {
                            let mut out = io.outputs.audio(b);
                            if let (Some(dst), Some(s)) =
                                (out.channel(c).get_mut(i..i + n), band.get(..n))
                            {
                                dst.copy_from_slice(s);
                            }
                        }
                        t += n as u64;
                        if t.is_multiple_of(FLUSH_EVERY) {
                            Crossover::flush(state);
                        }
                        i += n;
                    }
                }
                Engine::Fft(f) => {
                    f.process(c, src, &mut self.scratch);
                    for (b, band) in self.scratch.iter().enumerate() {
                        let mut out = io.outputs.audio(b);
                        let dst = out.channel(c);
                        let n = dst.len().min(band.len());
                        dst[..n].copy_from_slice(&band[..n]);
                    }
                }
            }
        }
        // Keep the analysis clocks of unused channels in step, so a mono
        // input that turns stereo starts both channels on the same hop.
        if let Engine::Fft(f) = &mut self.engine {
            for c in channels..CHANNELS {
                if let Some(a) = f.analysis.get_mut(c) {
                    for _ in 0..frames {
                        a.push(0.0);
                    }
                }
            }
        }
        self.clock += frames as u64;
    }

    fn latency(&self) -> u32 {
        match &self.engine {
            Engine::Crossover { .. } => 0,
            Engine::Fft(f) => f.analysis[0].size().saturating_sub(1) as u32,
        }
    }

    fn reset(&mut self) {
        match &mut self.engine {
            Engine::Crossover { states, .. } => {
                for s in states.iter_mut().flatten() {
                    s.reset();
                }
            }
            Engine::Fft(f) => f.reset(),
        }
        self.clock = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::{Harness, peak, tone_magnitude};
    use serde_json::json;

    const RATE: f64 = 48_000.0;

    fn sine(freq: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f64::consts::TAU * freq * i as f64 / RATE).sin() as f32)
            .collect()
    }

    fn split(config: Config, input: Vec<Vec<f32>>, block: usize) -> Vec<Vec<Vec<f32>>> {
        let n = input[0].len();
        let bands = edges(&config).len() + 1;
        let mut h = Harness::with_resources::<BandSplit>(config, RATE, block, &NoResources);
        h.set_audio("in", input);
        let out = h.run(n);
        (0..bands)
            .map(|b| out.audio(&format!("band_{b}")).to_vec())
            .collect()
    }

    fn sum(bands: &[Vec<Vec<f32>>], c: usize) -> Vec<f32> {
        let n = bands[0][c].len();
        (0..n)
            .map(|i| bands.iter().map(|b| b[c][i]).sum())
            .collect()
    }

    fn db(x: f64) -> f64 {
        20.0 * x.log10()
    }

    #[test]
    fn edges_are_parsed_and_repaired() {
        assert_eq!(edges(&json!({})), vec![200.0, 2000.0]);
        assert_eq!(
            edges(&json!({ "edges": [5000, 100, 1e9, -3] })),
            vec![20.0, 100.0, 5000.0, 20_000.0]
        );
        assert_eq!(edges(&json!({ "edges": [] })), Vec::<f64>::new());
        assert_eq!(edges(&json!({ "edges": 440 })), vec![440.0]);
        assert_eq!(edges(&json!({ "edges": "x" })), vec![200.0, 2000.0]);
        assert_eq!(
            edges(&json!({ "edges": [1, "a", null, 300] })),
            vec![20.0, 300.0]
        );
        let many: Vec<f64> = (1..40).map(|k| k as f64 * 100.0).collect();
        assert_eq!(edges(&json!({ "edges": many })).len(), 15);
        let names: Vec<String> = BandSplit::describe(&json!({ "edges": [100, 1000, 5000] }))
            .outputs
            .iter()
            .map(|s| s.name.to_string())
            .collect();
        assert_eq!(names, ["band_0", "band_1", "band_2", "band_3"]);
    }

    #[test]
    fn crossover_bands_sum_to_flat_magnitude() {
        let config = json!({ "edges": [120, 800, 3000, 9000] });
        for f in [40.0, 120.0, 400.0, 800.0, 1700.0, 3000.0, 6000.0, 15_000.0] {
            let bands = split(config.clone(), vec![sine(f, 24_000)], 64);
            let s = sum(&bands, 0);
            let level = db(tone_magnitude(&s[12_000..], f, RATE));
            assert!(level.abs() < 0.02, "{f} Hz: {level} dB");
        }
    }

    #[test]
    fn each_band_holds_its_own_frequencies() {
        let config = json!({ "edges": [200, 2000] });
        for (f, band) in [(60.0, 0), (650.0, 1), (8000.0, 2)] {
            let bands = split(config.clone(), vec![sine(f, 24_000)], 64);
            for (b, out) in bands.iter().enumerate() {
                let level = db(tone_magnitude(&out[0][12_000..], f, RATE));
                if b == band {
                    assert!(level > -1.0, "{f} Hz band {b}: {level}");
                } else {
                    assert!(level < -18.0, "{f} Hz band {b}: {level}");
                }
            }
        }
    }

    #[test]
    fn fft_mode_sums_back_exactly_after_its_latency() {
        let config = json!({ "edges": [300, 1200, 5000], "mode": "fft", "size": 512 });
        let ctx = PrepareCtx {
            sample_rate: RATE,
            max_block: 64,
            seed: 0,
            node_key: "t",
            name: "t",
            resources: &NoResources,
        };
        assert_eq!(BandSplit::new(&config, &ctx).latency(), 511);
        assert_eq!(BandSplit::new(&json!({}), &ctx).latency(), 0);
        let x: Vec<f32> = (0..8000)
            .map(|i| {
                let t = i as f32;
                0.3 * (t * 0.02).sin()
                    + 0.3 * (t * 0.4).sin()
                    + if i % 1000 == 0 { 0.5 } else { 0.0 }
            })
            .collect();
        let bands = split(config, vec![x.clone()], 64);
        let s = sum(&bands, 0);
        for t in 0..x.len() {
            let want = if t >= 511 { x[t - 511] } else { 0.0 };
            assert!((s[t] - want).abs() < 1e-4, "at {t}: {} vs {want}", s[t]);
        }
    }

    #[test]
    fn fft_mode_bands_are_brick_walls() {
        let config = json!({ "edges": [1000], "mode": "fft" });
        let bands = split(config, vec![sine(500.0, 24_000)], 64);
        assert!(tone_magnitude(&bands[0][0][8000..], 500.0, RATE) > 0.99);
        assert!(peak(&bands[1][0][8000..]) < 1e-3);
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let x: Vec<f32> = (0..5000)
            .map(|i| (i as f32 * 0.013).sin() * 0.5 + (i as f32 * 0.31).sin() * 0.3)
            .collect();
        for config in [
            json!({ "edges": [300, 3000] }),
            json!({ "edges": [300, 3000], "mode": "fft", "size": 256 }),
        ] {
            let input = vec![x.clone(), x.iter().map(|v| -v).collect()];
            let one = split(config.clone(), input.clone(), 1);
            assert_eq!(one, split(config.clone(), input.clone(), 64));
            assert_eq!(one, split(config, input, 512));
        }
    }

    #[test]
    fn stereo_is_split_and_extra_channels_go_to_band_zero() {
        let l = sine(100.0, 4000);
        let r = sine(5000.0, 4000);
        let extra = vec![0.25; 4000];
        let bands = split(json!({}), vec![l, r, extra.clone()], 64);
        assert_eq!(bands[0].len(), 3);
        assert!(tone_magnitude(&bands[0][0][2000..], 100.0, RATE) > 0.9);
        assert!(tone_magnitude(&bands[2][1][2000..], 5000.0, RATE) > 0.9);
        assert_eq!(bands[0][2], extra);
        assert_eq!(peak(&bands[1][2]), 0.0);
    }

    #[test]
    fn nan_input_stays_finite_and_reset_silences() {
        let mut x = sine(440.0, 4000);
        x[100] = f32::NAN;
        x[2000] = f32::INFINITY;
        for config in [json!({}), json!({ "mode": "fft", "size": 256 })] {
            let bands = split(config.clone(), vec![x.clone()], 64);
            assert!(bands.iter().flatten().flatten().all(|v| v.is_finite()));
            let mut h = Harness::new::<BandSplit>(config);
            h.set_audio("in", vec![sine(440.0, 1000)]);
            h.run(1000);
            h.reset();
            let out = h.run(1000);
            for b in 0..3 {
                assert_eq!(peak(&out.audio(&format!("band_{b}"))[0]), 0.0);
            }
        }
    }
}
