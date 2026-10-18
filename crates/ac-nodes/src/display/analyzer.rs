use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit,
    node,
};
use ac_dsp::fft::{Fft, Window};
use ac_dsp::stft::{MAX_SIZE, MIN_SIZE, StftAnalysis, pow2_size};

use super::pass_through;

node! {
    kind: "analyzer",
    name: "Analyzer",
    category: Display,
    keywords: ["spectrum", "analyser", "fft", "frequency", "eq", "rta"],
    /// A spectrum analyzer: shows how loud each part of the frequency range
    /// is, and passes the signal through unchanged so it can sit inline.
    ///
    /// The display has 64 bands spaced evenly in pitch from 20 Hz up to half
    /// the sample rate (about a sixth of an octave each at 48 kHz). Each
    /// band shows its level in dB, where 0 dB is a full-scale sine inside
    /// the band, down to a floor of −120 dB. Bands too narrow to hold a
    /// frequency bin read the spectrum interpolated at their center, so the
    /// curve has no gaps at the low end. Stereo signals show the average
    /// power of the first two channels; all channels pass through.
    ///
    /// Levels jump up at once and fall back by 20 dB every `decay` seconds,
    /// so short peaks stay visible long enough to read; 0 follows the
    /// signal frame by frame.
    ///
    /// Config:
    /// - `size`: FFT size, a power of two from 64 to 16384 (default 4096).
    ///   Bigger sizes resolve low frequencies better and respond slower.
    ///   Frames overlap by four with a Hann window.
    ///
    /// For the editor: telemetry value `k` (0..64) is band `k`'s level in
    /// dB, lowest band first, refreshed every block; `band_edges` gives the
    /// 65 band edges in Hz for a sample rate. NaN or infinite samples count
    /// as silence in the display; `out` is still an exact copy of `in`.
    ///
    /// **Try this:** put an Analyzer after a `Biquad` and sweep the
    /// `cutoff` to watch the slope move, or before `AudioOut` to see where
    /// the energy in a mix sits.
    pub struct Analyzer {
        analysis: [StftAnalysis; 2],
        fft: Fft,
        bins: Vec<f32>,
        power: Vec<f32>,
        /// First bin of each band (end exclusive), or empty.
        ranges: [(usize, usize); TELEMETRY_BANDS],
        /// Center of each band in (fractional) bins.
        centers: [f32; TELEMETRY_BANDS],
        /// Smoothed level of each band in dB.
        levels: [f32; TELEMETRY_BANDS],
        /// Scales a sum of bin powers to sine power (A²).
        sum_scale: f32,
        /// Scales one bin's power to sine power, for interpolated bands.
        peak_scale: f32,
        fresh: bool,
    }
}

/// Number of bands the analyzer publishes; telemetry value `k` is band `k`.
pub const TELEMETRY_BANDS: usize = 64;

/// Lowest band edge in Hz.
pub const LOWEST_HZ: f64 = 20.0;

/// Level floor in dB.
pub const FLOOR_DB: f32 = -120.0;

const IN: usize = 0;
const DECAY: usize = 1;
const OUT: usize = 0;

/// The 65 band edges, in Hz, for `sample_rate`: log-spaced from 20 Hz to
/// half the sample rate. Band `k` spans `edges[k]..edges[k + 1]`.
pub fn band_edges(sample_rate: f64) -> [f64; TELEMETRY_BANDS + 1] {
    let nyquist = if sample_rate.is_finite() {
        (sample_rate / 2.0).max(2.0 * LOWEST_HZ)
    } else {
        24_000.0
    };
    let ratio = nyquist / LOWEST_HZ;
    let mut edges = [0.0; TELEMETRY_BANDS + 1];
    for (k, e) in edges.iter_mut().enumerate() {
        *e = LOWEST_HZ * ratio.powf(k as f64 / TELEMETRY_BANDS as f64);
    }
    edges[TELEMETRY_BANDS] = nyquist;
    edges
}

fn fft_size(config: &Config) -> usize {
    pow2_size(config.get_f64_or("size", 4096.0), MIN_SIZE, MAX_SIZE, 4096)
}

impl Analyzer {
    /// Band power (sine-amplitude² units) from `self.power`.
    fn band_power(&self, k: usize) -> f32 {
        let (lo, hi) = self.ranges[k];
        if lo < hi {
            self.power[lo..hi].iter().sum::<f32>() * self.sum_scale
        } else {
            let p = self.centers[k].max(0.0);
            let i = p as usize;
            let frac = p - i as f32;
            let a = self.power.get(i).copied().unwrap_or(0.0);
            let b = self.power.get(i + 1).copied().unwrap_or(a);
            (a + (b - a) * frac) * self.peak_scale
        }
    }

    /// Add channel `c`'s spectrum power into `self.power`.
    fn accumulate(&mut self, c: usize, weight: f32) {
        if let Some(a) = self.analysis.get_mut(c) {
            a.analyze(&mut self.fft, &mut self.bins);
        }
        for (p, pair) in self.power.iter_mut().zip(self.bins.chunks_exact(2)) {
            *p += (pair[0] * pair[0] + pair[1] * pair[1]) * weight;
        }
    }
}

impl Node for Analyzer {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(TELEMETRY_BANDS)
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to analyze."))
            .input(
                SocketSpec::float("decay", Unit::Seconds, 0.5)
                    .range(0.0, 5.0)
                    .doc("Time for a band to fall by 20 dB."),
            )
            .output(SocketSpec::audio("out").doc("The same signal, unchanged."))
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let size = fft_size(config);
        let hop = size / 4;
        let fft = Fft::new(size);
        let bins = fft.bins();
        let rate = ctx.sample_rate.max(1.0);
        let edges = band_edges(rate);
        let to_bin = size as f64 / rate;
        let mut ranges = [(0, 0); TELEMETRY_BANDS];
        let mut centers = [0.0; TELEMETRY_BANDS];
        for k in 0..TELEMETRY_BANDS {
            let lo = (edges[k] * to_bin).ceil() as usize;
            let hi = if k + 1 == TELEMETRY_BANDS {
                bins
            } else {
                ((edges[k + 1] * to_bin).ceil() as usize).min(bins)
            };
            ranges[k] = (lo.min(bins), hi);
            centers[k] = ((edges[k] * edges[k + 1]).sqrt() * to_bin) as f32;
        }
        let window = Window::Hann.table(size);
        let sum_w2: f64 = window.iter().map(|w| f64::from(*w).powi(2)).sum();
        let coherent = f64::from(Window::Hann.coherent_gain()) * size as f64 / 2.0;
        Analyzer {
            analysis: [
                StftAnalysis::new(size, hop, Window::Hann),
                StftAnalysis::new(size, hop, Window::Hann),
            ],
            fft,
            bins: vec![0.0; 2 * bins],
            power: vec![0.0; bins],
            ranges,
            centers,
            levels: [FLOOR_DB; TELEMETRY_BANDS],
            // One-sided Parseval: a sine of amplitude A puts N·A²·Σw²/4
            // into its positive-frequency bins.
            sum_scale: (4.0 / (size as f64 * sum_w2)) as f32,
            peak_scale: (1.0 / (coherent * coherent)) as f32,
            fresh: true,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(IN);
        pass_through(&input, &mut io.outputs.audio(OUT));
        let stereo = input.channels() > 1;
        let decay = io.inputs.float(DECAY);
        let (left, right) = (input.channel(0), input.channel(1));
        let hop = self.analysis[0].hop() as f64;
        for (i, &l) in left.iter().enumerate() {
            let a = self.analysis[0].push(l);
            let b = self.analysis[1].push(right.get(i).copied().unwrap_or(0.0));
            if !(a && b) {
                continue;
            }
            self.power.fill(0.0);
            if stereo {
                self.accumulate(0, 0.5);
                self.accumulate(1, 0.5);
            } else {
                self.accumulate(0, 1.0);
            }
            let d = f64::from(decay.at(i));
            let fall = if d.is_finite() && d > 0.0 {
                (20.0 * hop / (d * ctx.sample_rate)) as f32
            } else {
                f32::INFINITY
            };
            for k in 0..TELEMETRY_BANDS {
                let p = self.band_power(k);
                let db = if p > 1e-12 {
                    (10.0 * p.log10()).max(FLOOR_DB)
                } else {
                    FLOOR_DB
                };
                let held = if self.fresh {
                    FLOOR_DB
                } else {
                    self.levels[k] - fall
                };
                self.levels[k] = db.max(held).max(FLOOR_DB);
            }
            self.fresh = false;
        }
        for (k, l) in self.levels.iter().enumerate() {
            io.telemetry.set(k, *l);
        }
    }

    fn reset(&mut self) {
        for a in &mut self.analysis {
            a.reset();
        }
        self.levels = [FLOOR_DB; TELEMETRY_BANDS];
        self.fresh = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    const RATE: f64 = 48_000.0;

    fn sine(freq: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (amp * (std::f64::consts::TAU * freq * i as f64 / RATE).sin()) as f32)
            .collect()
    }

    fn band_of(freq: f64) -> usize {
        let e = band_edges(RATE);
        (0..TELEMETRY_BANDS)
            .find(|&k| freq >= e[k] && freq < e[k + 1])
            .unwrap()
    }

    fn levels(h: &Harness) -> Vec<f32> {
        (0..TELEMETRY_BANDS).map(|k| h.telemetry(k)).collect()
    }

    #[test]
    fn band_edges_span_20_hz_to_nyquist() {
        let e = band_edges(RATE);
        assert_eq!(e[0], 20.0);
        assert_eq!(e[64], 24_000.0);
        assert!(e.windows(2).all(|w| w[1] > w[0]));
        let r = e[1] / e[0];
        assert!((e[33] / e[32] - r).abs() < 1e-9);
        assert_eq!(band_edges(44_100.0)[64], 22_050.0);
        assert_eq!(band_edges(f64::NAN)[64], 24_000.0);
    }

    fn analyze(x: Vec<f32>) -> Vec<f32> {
        let mut h = Harness::new::<Analyzer>(json!({}));
        let n = x.len();
        h.set_audio("in", vec![x]).set("decay", 0.0);
        h.run(n);
        levels(&h)
    }

    #[test]
    fn a_tone_lights_its_band_at_its_level() {
        for (f, amp) in [(1000.0, 0.5), (3000.0, 1.0), (8000.0, 0.1)] {
            let l = analyze(sine(f, amp, 24_000));
            let k = band_of(f);
            let want = 20.0 * amp.log10() as f32;
            // The band (with its neighbors, if the tone sits near an edge)
            // holds the tone's power.
            let near: f32 = (k - 1..=k + 1).map(|j| 10f32.powf(l[j] / 10.0)).sum();
            assert!((10.0 * near.log10() - want).abs() < 0.5, "{f}: {l:?}");
            assert!(l[k] > want - 4.0, "{f}: band {k} {}", l[k]);
            // Far-away bands are far down.
            assert!(l[k - 20] < want - 60.0, "{f}: {}", l[k - 20]);
        }
        // Low tones, where bands are about a bin wide, still peak in place.
        let l = analyze(sine(100.0, 1.0, 24_000));
        let top = (0..64).max_by(|a, b| l[*a].total_cmp(&l[*b])).unwrap();
        assert!(top.abs_diff(band_of(100.0)) <= 1, "{top}");
        assert!(l[top] > -6.0, "{}", l[top]);
    }

    #[test]
    fn silence_reads_the_floor_and_passes_through() {
        let mut h = Harness::new::<Analyzer>(json!({}));
        let x = vec![0.0; 2000];
        h.set_audio("in", vec![x.clone(), x.clone()]);
        let out = h.run(2000);
        assert_eq!(out.audio("out"), &[x.clone(), x]);
        assert!(levels(&h).iter().all(|l| *l == FLOOR_DB));
    }

    #[test]
    fn levels_fall_by_20_db_per_decay() {
        let mut x = sine(1000.0, 1.0, 24_000);
        x.extend(vec![0.0; 48_000]);
        let mut h = Harness::new::<Analyzer>(json!({ "size": 1024 }));
        h.set_audio("in", vec![x]).set("decay", 0.5);
        h.run(24_000 + 2048);
        let k = band_of(1000.0);
        let start = h.telemetry(k);
        assert!(start < -1.0 && start > -10.0, "{start}");
        // Half a second later, 20 dB lower (to within a frame).
        h.run(24_000);
        let later = h.telemetry(k);
        assert!((start - later - 20.0).abs() < 0.5, "{start} → {later}");
    }

    #[test]
    fn low_bands_are_interpolated_not_empty() {
        let mut h = Harness::new::<Analyzer>(json!({ "size": 256 }));
        h.set_audio("in", vec![sine(30.0, 1.0, 12_000)])
            .set("decay", 0.0);
        h.run(12_000);
        // Every band below 200 Hz sees the low tone, none reads the floor.
        let e = band_edges(RATE);
        for k in (0..64).filter(|&k| e[k + 1] < 200.0) {
            assert!(h.telemetry(k) > -30.0, "band {k}: {}", h.telemetry(k));
        }
    }

    #[test]
    fn block_size_does_not_change_the_display() {
        let x: Vec<f32> = (0..9000)
            .map(|i| (i as f32 * 0.05).sin() * 0.3 + (i as f32 * 0.7).sin() * 0.1)
            .collect();
        let run = |block: usize| {
            let mut h = Harness::with_resources::<Analyzer>(
                json!({ "size": 512 }),
                RATE,
                block,
                &NoResources,
            );
            h.set_audio("in", vec![x.clone()]);
            let out = h.run(9000);
            (levels(&h), out.audio("out")[0].clone())
        };
        let one = run(1);
        assert_eq!(one, run(64));
        assert_eq!(one, run(512));
    }

    #[test]
    fn non_finite_input_reads_as_silence_and_reset_clears() {
        let mut x = sine(500.0, 0.5, 9000);
        x[4000] = f32::NAN;
        x[4001] = f32::INFINITY;
        let mut h = Harness::new::<Analyzer>(json!({}));
        h.set_audio("in", vec![x.clone()]);
        let out = h.run(9000);
        assert!(levels(&h).iter().all(|l| l.is_finite()));
        assert!(out.audio("out")[0][4000].is_nan());
        h.reset();
        h.run(64);
        assert!(levels(&h).iter().all(|l| *l == FLOOR_DB));
    }
}
