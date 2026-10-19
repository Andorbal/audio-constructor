use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit,
    node,
};
use ac_dsp::fft::{Fft, Window};
use ac_dsp::stft::{MAX_SIZE, MIN_SIZE, StftAnalysis, pow2_size};

use super::analyzer::{Bands, FLOOR_DB, TELEMETRY_BANDS, accumulate};
use super::pass_through;

node! {
    kind: "spectrogram",
    name: "Spectrogram",
    category: Display,
    keywords: ["sonogram", "waterfall", "spectrum", "fft", "frequency", "time", "analyser", "voiceprint"],
    /// A scrolling spectrogram: a picture of a sound over time, with
    /// frequency going up, time running left to right (the newest moment
    /// at the right edge), and loudness as brightness. It passes the signal through unchanged, so it can sit
    /// inline.
    ///
    /// Each column of the picture is one slice of time, read in the same
    /// 64 bands as the `Analyzer` (spaced evenly in pitch from 20 Hz to
    /// half the sample rate, each in dB where 0 dB is a full-scale sine in
    /// the band). The display holds 128 columns across `window` seconds;
    /// each column shows the loudest moment of its slice, so short sounds
    /// aren't missed when the window is long. Stereo signals show the
    /// average power of the first two channels; all channels pass through.
    ///
    /// Config:
    /// - `size`: FFT size, a power of two from 64 to 16384 (default 2048).
    ///   Bigger sizes resolve low notes better and blur fast changes more.
    ///   Frames overlap by four with a Hann window.
    ///
    /// For the editor: the scope ring holds 128 columns of 64 levels (dB,
    /// lowest band first, oldest column first from the ring position), and
    /// telemetry value 0 is the time each column covers, in seconds. NaN
    /// or infinite samples count as silence in the picture; `out` is still
    /// an exact copy of `in`.
    ///
    /// **Try this:** put a Spectrogram after a `Formant` with an `LFO` on
    /// its `vowel` to watch the resonances move, or after a `Chaos`-swept
    /// filter on noise to see weather in the spectrum.
    pub struct Spectrogram {
        analysis: [StftAnalysis; 2],
        fft: Fft,
        bins: Vec<f32>,
        power: Vec<f32>,
        bands: Bands,
        /// The loudest level of each band in the column being gathered.
        column: [f32; TELEMETRY_BANDS],
        /// The newest frame's levels, for columns no frame lands in.
        last: [f32; TELEMETRY_BANDS],
        /// A frame landed in the column being gathered.
        fresh: bool,
        /// Samples gathered into the current column.
        count: u64,
        /// The ring still holds what was there before (zeros, which would
        /// read as full scale): fill it with silence on the next block.
        clear: bool,
    }
}

const IN: usize = 0;
const WINDOW: usize = 1;
const OUT: usize = 0;

/// Columns in the picture.
pub const COLUMNS: usize = 128;

/// Telemetry slot: seconds per column.
pub const TELEMETRY_COLUMN_SECONDS: usize = 0;

/// Longest window honoured, in seconds; only guards against absurd values.
const MAX_WINDOW: f64 = 3600.0;

/// Samples per column for a `window` in seconds at `rate`.
fn column_length(window: f32, rate: f64) -> u64 {
    let w = if window.is_finite() {
        f64::from(window).clamp(0.0, MAX_WINDOW)
    } else {
        5.0
    };
    let n = (w * rate / COLUMNS as f64).round();
    if n.is_finite() { (n as u64).max(1) } else { 1 }
}

impl Node for Spectrogram {
    fn meta() -> NodeMeta {
        Self::base_meta()
            .telemetry(1)
            .scope(COLUMNS * TELEMETRY_BANDS)
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to picture."))
            .input(
                SocketSpec::float("window", Unit::Seconds, 5.0)
                    .range(0.5, 30.0)
                    .log()
                    .no_smooth()
                    .doc("Time the picture spans."),
            )
            .output(SocketSpec::audio("out").doc("The same signal, unchanged."))
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let size = pow2_size(config.get_f64_or("size", 2048.0), MIN_SIZE, MAX_SIZE, 2048);
        let hop = size / 4;
        let fft = Fft::new(size);
        let bins = fft.bins();
        Spectrogram {
            analysis: [
                StftAnalysis::new(size, hop, Window::Hann),
                StftAnalysis::new(size, hop, Window::Hann),
            ],
            fft,
            bins: vec![0.0; 2 * bins],
            power: vec![0.0; bins],
            bands: Bands::new(size, ctx.sample_rate),
            column: [FLOOR_DB; TELEMETRY_BANDS],
            last: [FLOOR_DB; TELEMETRY_BANDS],
            fresh: false,
            count: 0,
            clear: true,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(IN);
        pass_through(&input, &mut io.outputs.audio(OUT));
        let stereo = input.channels() > 1;
        let len = column_length(io.inputs.float(WINDOW).first(), ctx.sample_rate);
        let tele = io.telemetry;
        if self.clear {
            // A whole ring of silent columns leaves its position where it was.
            for _ in 0..COLUMNS {
                tele.push_scope([FLOOR_DB; TELEMETRY_BANDS]);
            }
            self.clear = false;
        }
        tele.set(
            TELEMETRY_COLUMN_SECONDS,
            (len as f64 / ctx.sample_rate.max(1.0)) as f32,
        );
        let (left, right) = (input.channel(0), input.channel(1));
        for (i, &l) in left.iter().enumerate() {
            let clean = |x: f32| if x.is_finite() { x } else { 0.0 };
            let a = self.analysis[0].push(clean(l));
            let b = self.analysis[1].push(clean(right.get(i).copied().unwrap_or(0.0)));
            if a && b {
                self.power.fill(0.0);
                let weight = if stereo { 0.5 } else { 1.0 };
                let [first, second] = &mut self.analysis;
                accumulate(
                    first,
                    &mut self.fft,
                    &mut self.bins,
                    &mut self.power,
                    weight,
                );
                if stereo {
                    accumulate(
                        second,
                        &mut self.fft,
                        &mut self.bins,
                        &mut self.power,
                        weight,
                    );
                }
                for k in 0..TELEMETRY_BANDS {
                    let db = self.bands.db(&self.power, k);
                    self.last[k] = db;
                    self.column[k] = self.column[k].max(db);
                }
                self.fresh = true;
            }
            self.count += 1;
            if self.count >= len {
                let col = if self.fresh { &self.column } else { &self.last };
                tele.push_scope(col.iter().copied());
                self.column = [FLOOR_DB; TELEMETRY_BANDS];
                self.fresh = false;
                self.count = 0;
            }
        }
    }

    fn reset(&mut self) {
        for a in &mut self.analysis {
            a.reset();
        }
        self.column = [FLOOR_DB; TELEMETRY_BANDS];
        self.last = [FLOOR_DB; TELEMETRY_BANDS];
        self.fresh = false;
        self.count = 0;
        self.clear = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::analyzer::band_edges;
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
            .unwrap_or(0)
    }

    /// The ring as columns, oldest first.
    fn columns(h: &Harness) -> Vec<Vec<f32>> {
        h.scope()
            .chunks(TELEMETRY_BANDS)
            .map(<[f32]>::to_vec)
            .collect()
    }

    #[test]
    fn a_tone_lights_its_band_in_every_new_column() {
        // A 1 s window: one column per 375 samples.
        let mut h = Harness::new::<Spectrogram>(json!({}));
        let x = sine(1000.0, 0.5, 48_000);
        h.set_audio("in", vec![x.clone()]).set("window", 1.0);
        let out = h.run(48_000);
        assert_eq!(out.audio("out")[0], x);
        let k = band_of(1000.0);
        let cols = columns(&h);
        assert_eq!(cols.len(), COLUMNS);
        // A second of tone fills the ring; past the first frame every
        // column shows it near −6 dB, and far bands near the floor.
        for (c, col) in cols.iter().enumerate().skip(8) {
            let near = (k - 1..=k + 1).map(|j| col[j]).fold(FLOOR_DB, f32::max);
            assert!((near + 6.0).abs() < 2.0, "column {c}: {}", near);
            assert!(col[k + 20] < -60.0, "column {c}: {}", col[k + 20]);
        }
        assert!((h.telemetry(TELEMETRY_COLUMN_SECONDS) - 375.0 / 48_000.0).abs() < 1e-6);
    }

    #[test]
    fn the_picture_starts_silent() {
        // Before any column is complete, the ring reads as the floor, not
        // as the zeros it was made with.
        let mut h = Harness::new::<Spectrogram>(json!({}));
        h.set_audio("in", vec![vec![0.5; 64]]);
        h.run(64);
        let ring = h.scope();
        assert_eq!(ring.len(), COLUMNS * TELEMETRY_BANDS);
        assert!(ring.iter().all(|v| *v == FLOOR_DB));
    }

    #[test]
    fn time_runs_through_the_columns() {
        // Silence, then a burst, then silence: the burst shows in the
        // columns it falls in and nowhere else.
        let mut x = vec![0.0f32; 24_000];
        x.extend(sine(3000.0, 0.8, 12_000));
        x.extend(vec![0.0; 12_000]);
        let mut h = Harness::new::<Spectrogram>(json!({ "size": 512 }));
        h.set_audio("in", vec![x]).set("window", 1.0);
        h.run(48_000);
        let k = band_of(3000.0);
        let lit: Vec<usize> = columns(&h)
            .iter()
            .enumerate()
            .filter(|(_, col)| col[k] > -20.0)
            .map(|(c, _)| c)
            .collect();
        // 128 columns of 375 samples: the burst covers columns 64..96,
        // plus the window's spread of half a frame on each side.
        let (first, last) = (lit[0], *lit.last().unwrap_or(&0));
        assert!((63..=65).contains(&first), "{lit:?}");
        assert!((95..=97).contains(&last), "{lit:?}");
        assert_eq!(lit.len(), last - first + 1, "{lit:?}");
    }

    #[test]
    fn short_sounds_are_kept_in_long_columns() {
        // A 30 s window gives columns of 11 250 samples; a 20 ms click of
        // tone in the middle of one still shows at its level.
        let mut x = vec![0.0f32; 11_250 * 2];
        for (i, v) in sine(2000.0, 1.0, 960).into_iter().enumerate() {
            x[16_000 + i] = v;
        }
        let mut h = Harness::new::<Spectrogram>(json!({ "size": 512 }));
        h.set_audio("in", vec![x]).set("window", 30.0);
        h.run(11_250 * 2);
        let cols = columns(&h);
        let newest = &cols[COLUMNS - 1];
        assert!(newest[band_of(2000.0)] > -10.0, "{newest:?}");
        assert!(cols[COLUMNS - 2][band_of(2000.0)] < -60.0);
    }

    #[test]
    fn block_size_does_not_change_the_picture() {
        let x: Vec<f32> = (0..30_000)
            .map(|i| (i as f32 * 0.05).sin() * 0.3 + (i as f32 * 0.9).sin() * 0.2)
            .collect();
        let run = |block: usize| {
            let mut h = Harness::with_resources::<Spectrogram>(
                json!({ "size": 256 }),
                RATE,
                block,
                &NoResources,
            );
            h.set_audio("in", vec![x.clone(), x.clone()])
                .set("window", 0.5);
            let out = h.run(30_000);
            (h.scope(), out.audio("out").to_vec())
        };
        let one = run(1);
        assert_eq!(one, run(64));
        assert_eq!(one, run(1024));
    }

    #[test]
    fn bad_input_and_settings_stay_finite_and_reset_clears() {
        let mut x = sine(500.0, 0.5, 20_000);
        x[4_000] = f32::NAN;
        x[4_001] = f32::INFINITY;
        for window in [f64::NAN, 0.0, 1e12, -4.0] {
            let mut h = Harness::new::<Spectrogram>(json!({ "size": "huge" }));
            h.set_audio("in", vec![x.clone()]).set("window", window);
            let out = h.run(20_000);
            assert!(h.scope().iter().all(|v| v.is_finite()));
            assert!(out.audio("out")[0][4_000].is_nan());
        }
        assert_eq!(column_length(0.0, RATE), 1);
        assert_eq!(column_length(f32::NAN, RATE), 1875);
        let mut h = Harness::new::<Spectrogram>(json!({}));
        h.set_audio("in", vec![sine(500.0, 0.5, 4_000)])
            .set("window", 0.5);
        h.run(4_000);
        h.reset();
        h.set_audio("in", vec![vec![0.0; 4_000]]);
        h.run(4_000);
        let cols = columns(&h);
        assert!(cols[COLUMNS - 1].iter().all(|v| *v == FLOOR_DB));
    }
}
