use std::sync::Arc;

use ac_core::{
    AudioData, Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature,
    SocketSpec, Unit, node,
};
use ac_dsp::osc::{Phasor, increment, wrap01};
use ac_dsp::wavetable::{BASIC_FRAMES, BASIC_SIZE, Builder, Layout, basic, lookup};

use crate::segments::Runs;

node! {
    kind: "wavetable",
    name: "Wavetable",
    category: Generators,
    keywords: ["wave table", "morph", "scan", "oscillator", "single cycle", "frames", "synth"],
    /// An oscillator that plays single-cycle waveforms from a table and
    /// morphs between them with `position`, as a sample-rate `Float`.
    ///
    /// The `table` config chooses the waveforms:
    /// - `"basic"` (default): sine, triangle, saw, and square, in that
    ///   order, with the shapes and phase of the `Oscillator` node.
    /// - `"buffer"`: the waveforms come from a `table` `Buffer` input
    ///   (adds the socket), such as a `Record` node's take: channel 0 is
    ///   cut into consecutive frames of `frame_size` samples (config,
    ///   default 2048), each one cycle. A trailing partial frame is
    ///   ignored, and so are frames past `max_frames` (config, default 64).
    ///
    /// `position` (0 to 1) scans the frames, crossfading between
    /// neighbours: with the basic table, 0 is the sine, 1/3 the triangle,
    /// 2/3 the saw, and 1 the square. `freq` + `fm` sets the pitch in hertz
    /// (linear FM; negative totals run the waveform backwards), `phase`
    /// offsets it by a fraction of a cycle, and `sync` resets the phase to
    /// 0 (hard sync, not band-limited).
    ///
    /// Every frame is stored once per octave with the harmonics that would
    /// alias at that pitch removed, and playback fades between octaves, so
    /// sweeps stay clean and high notes lose their top harmonics smoothly.
    /// Preparing those copies takes time, and the audio thread can't stop
    /// to do it all at once: when a new buffer arrives, its frames are
    /// prepared one piece at a time while playback goes on (a 2048-sample
    /// frame every 4 ms or so at 48 kHz, a full 64-frame table in about a
    /// quarter of a second). Meanwhile `position` scans the frames that are
    /// ready, and the output is silent until the first one is. Memory for
    /// `max_frames` frames is reserved up front (about 5.8 MB at the
    /// defaults), which is why the frame count has a limit.
    ///
    /// Generator contract: silent until `start`, which starts from phase
    /// 0. `stop` silences it and fires `end`; `reset` returns to phase 0
    /// without changing whether it plays. Events at the same frame apply in
    /// the order stop, start, reset, sync.
    ///
    /// **Try this:** sweep `position` with a slow `Oscillator` (`pure`,
    /// mapped to 0..1) for an evolving pad, or record a few seconds of
    /// sound into a `Record` node and wire its `buffer` into a `"buffer"`
    /// Wavetable to play it back as a scanned waveform.
    pub struct Wavetable {
        layout: Layout,
        /// The built-in tables (basic mode).
        shared: Option<Arc<Vec<f32>>>,
        /// Buffer mode: room for `max_frames` frames, filled lazily.
        own: Vec<f32>,
        builder: Option<Builder>,
        max_frames: usize,
        /// Frames ready to play.
        ready: usize,
        /// The buffer being (or last) prepared.
        source: Option<Arc<AudioData>>,
        /// Frames the source provides (at most `max_frames`).
        total: usize,
        next_frame: usize,
        /// 0: analyze `next_frame`; then render level `next_unit - 1`.
        next_unit: usize,
        countdown: u32,
        /// Input index of `freq` (1 when the `table` input comes first).
        first: usize,
        osc: Phasor,
        playing: bool,
    }
}

// Input positions after the optional `table` socket.
const FREQ: usize = 0;
const POSITION: usize = 1;
const PHASE: usize = 2;
const SYNC: usize = 3;
const FM: usize = 4;
const START: usize = 5;
const STOP: usize = 6;
const RESET: usize = 7;
const TABLE: usize = 0;

const OUT: usize = 0;
const PLAYING: usize = 1;
const END: usize = 2;

/// Samples between preparation steps (one FFT each) while a buffer's
/// frames are being prepared.
const BUILD_INTERVAL: u32 = 16;
/// The most samples (`max_frames × frame_size`) a buffer table reserves.
const MAX_TABLE_SAMPLES: usize = 1 << 19;

fn is_buffer(config: &Config) -> bool {
    config.get_str_or("table", "basic") == "buffer"
}

fn frame_size(config: &Config) -> usize {
    (config.get_u64_or("frame_size", 2048).clamp(32, 8192) as usize) & !1
}

fn max_frames(config: &Config) -> usize {
    let n = config.get_u64_or("max_frames", 64).clamp(1, 256) as usize;
    n.min(MAX_TABLE_SAMPLES / frame_size(config)).max(1)
}

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

impl Wavetable {
    fn building(&self) -> bool {
        self.ready < self.total
    }

    /// Start preparing `buf` (or clear the table for none).
    fn begin(&mut self, buf: Option<&Arc<AudioData>>) {
        self.source = buf.cloned();
        let frames = buf.and_then(|b| b.channels.first()).map_or(0, Vec::len);
        self.total = (frames / self.layout.size).min(self.max_frames);
        self.ready = 0;
        self.next_frame = 0;
        self.next_unit = 0;
        self.countdown = 0;
    }

    /// One preparation step: analyze a frame or render one of its levels.
    fn build_step(&mut self) {
        let Some(builder) = self.builder.as_mut() else {
            self.total = 0;
            return;
        };
        let size = self.layout.size;
        if self.next_unit == 0 {
            let start = self.next_frame * size;
            let cycle = self
                .source
                .as_ref()
                .and_then(|b| b.channels.first())
                .and_then(|c| c.get(start..start + size))
                .unwrap_or(&[]);
            builder.analyze(cycle);
        } else {
            let level = self.next_unit - 1;
            let o = self.layout.offset(self.next_frame, level);
            if let Some(slot) = self.own.get_mut(o..o + self.layout.slot_len()) {
                builder.render(level, slot);
            }
        }
        self.next_unit += 1;
        if self.next_unit > self.layout.levels {
            self.next_unit = 0;
            self.next_frame += 1;
            self.ready = self.next_frame;
        }
    }
}

impl Node for Wavetable {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let mut sig = Signature::new();
        if is_buffer(config) {
            sig = sig.input(
                SocketSpec::buffer("table")
                    .doc("Audio cut into single-cycle frames of `frame_size`."),
            );
        }
        sig.input(
            SocketSpec::float("freq", Unit::Hz, 440.0)
                .range(20.0, 20_000.0)
                .log()
                .doc("Frequency in hertz."),
        )
        .input(
            SocketSpec::float("position", Unit::Generic, 0.0)
                .range(0.0, 1.0)
                .doc("Scans the frames (0 is the first, 1 the last)."),
        )
        .input(
            SocketSpec::float("phase", Unit::Generic, 0.0)
                .range(0.0, 1.0)
                .doc("Phase offset in cycles."),
        )
        .input(SocketSpec::trigger("sync").doc("Hard sync: resets the phase to 0."))
        .input(
            SocketSpec::float("fm", Unit::Hz, 0.0)
                .range(-1000.0, 1000.0)
                .doc("Linear FM: hertz added to `freq`."),
        )
        .generator_inputs(false, false)
        .output(
            SocketSpec::float("out", Unit::Generic, 0.0)
                .rate(Rate::Sample)
                .doc("The waveform."),
        )
        .generator_outputs()
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        let buffer = is_buffer(config);
        let (layout, shared, own, builder, max, ready) = if buffer {
            let layout = Layout::new(frame_size(config));
            let max = max_frames(config);
            (
                layout,
                None,
                vec![0.0; max * layout.frame_len()],
                Some(Builder::new(layout)),
                max,
                0,
            )
        } else {
            (
                Layout::new(BASIC_SIZE),
                Some(basic()),
                Vec::new(),
                None,
                BASIC_FRAMES,
                BASIC_FRAMES,
            )
        };
        Wavetable {
            layout,
            shared,
            own,
            builder,
            max_frames: max,
            ready,
            source: None,
            total: ready,
            next_frame: 0,
            next_unit: 0,
            countdown: 0,
            first: usize::from(buffer),
            osc: Phasor::new(),
            playing: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let rate = if ctx.sample_rate.is_finite() && ctx.sample_rate > 0.0 {
            ctx.sample_rate
        } else {
            48_000.0
        };
        if self.builder.is_some() {
            let buf = io.inputs.buffer(TABLE);
            let same = match (buf, self.source.as_ref()) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
            if !same {
                self.begin(buf);
            }
        }
        let k = self.first;
        let freq = io.inputs.float(k + FREQ);
        let fm = io.inputs.float(k + FM);
        let position = io.inputs.float(k + POSITION);
        let offset = io.inputs.float(k + PHASE);
        let events = [
            io.inputs.triggers(k + STOP),
            io.inputs.triggers(k + START),
            io.inputs.triggers(k + RESET),
            io.inputs.triggers(k + SYNC),
        ];
        let frames = io.frames();
        {
            let (mut out, mut end) = io.outputs.float_trigger(OUT, END);
            for run in Runs::new(events, frames) {
                let [stop, start, reset, sync] = run.fired;
                if stop && self.playing {
                    self.playing = false;
                    end.push(run.at as u32);
                }
                if start {
                    self.playing = true;
                    self.osc.reset();
                }
                if reset || sync {
                    self.osc.reset();
                }
                for i in run.at..run.until {
                    // Preparation runs at fixed sample positions, so the
                    // output doesn't depend on the block size.
                    if self.building() {
                        if self.countdown == 0 {
                            self.build_step();
                            self.countdown = BUILD_INTERVAL;
                        }
                        self.countdown -= 1;
                    }
                    if !self.playing {
                        out.set(i, 0.0);
                        continue;
                    }
                    let inc = increment(f64::from(freq.at(i)) + f64::from(fm.at(i)), rate);
                    let p = wrap01(self.osc.phase() + f64::from(finite_or(offset.at(i), 0.0)));
                    let ready = self.ready;
                    let scan = f64::from(finite_or(position.at(i), 0.0).clamp(0.0, 1.0))
                        * ready.saturating_sub(1) as f64;
                    let data: &[f32] = match &self.shared {
                        Some(d) => d,
                        None => &self.own,
                    };
                    out.set(i, lookup(data, self.layout, ready, scan, p, inc));
                    self.osc.advance(inc);
                }
            }
        }
        io.outputs.set_bool(PLAYING, self.playing);
    }

    fn reset(&mut self) {
        self.osc.reset();
        self.playing = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::{Harness, peak, rms, tone_magnitude, zero_crossing_freq};
    use serde_json::json;

    fn harness(config: Config, block: usize) -> Harness {
        Harness::with_resources::<Wavetable>(config, 48_000.0, block, &NoResources)
    }

    fn basic_at(position: f64, freq: f64, n: usize) -> Vec<f32> {
        let mut h = harness(json!({}), 64);
        h.set("position", position).set("freq", freq);
        h.run(n).float("out").to_vec()
    }

    #[test]
    fn sockets_follow_the_table_config() {
        let names = |c: Config| -> Vec<String> {
            Wavetable::describe(&c)
                .inputs
                .iter()
                .map(|s| s.name.to_string())
                .collect()
        };
        assert_eq!(names(json!({}))[0], "freq");
        assert_eq!(names(json!({ "table": "buffer" }))[0], "table");
        assert_eq!(names(json!({ "table": "nonsense" }))[0], "freq");
        assert_eq!(frame_size(&json!({ "frame_size": 3 })), 32);
        assert_eq!(frame_size(&json!({ "frame_size": 1e9 })), 8192);
        assert_eq!(
            max_frames(&json!({ "frame_size": 8192, "max_frames": 256 })),
            64
        );
    }

    #[test]
    fn basic_frames_sound_like_the_oscillator_shapes() {
        let sine = basic_at(0.0, 440.0, 48_000);
        assert!((zero_crossing_freq(&sine, 48_000.0) - 440.0).abs() < 0.5);
        assert!((rms(&sine) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
        for (i, v) in sine.iter().enumerate().take(500) {
            let want = (std::f64::consts::TAU * 440.0 * i as f64 / 48_000.0).sin() as f32;
            assert!((v - want).abs() < 1e-4, "{i}");
        }
        let square = basic_at(1.0, 100.0, 48_000);
        assert!((rms(&square) - 1.0).abs() < 0.02, "{}", rms(&square));
        let tri = basic_at(1.0 / 3.0, 100.0, 48_000);
        assert!((peak(&tri) - 1.0).abs() < 0.01);
        // The saw's first partial is 2/π.
        let saw = basic_at(2.0 / 3.0, 1_000.0, 48_000);
        let f = tone_magnitude(&saw, 1_000.0, 48_000.0);
        assert!((f - 2.0 / std::f64::consts::PI).abs() < 0.01, "{f}");
    }

    #[test]
    fn position_morphs_between_neighbours() {
        let tri = basic_at(1.0 / 3.0, 200.0, 1_000);
        let saw = basic_at(2.0 / 3.0, 200.0, 1_000);
        let mid = basic_at(0.5, 200.0, 1_000);
        for i in 0..1_000 {
            assert!((mid[i] - 0.5 * (tri[i] + saw[i])).abs() < 1e-5, "{i}");
        }
    }

    #[test]
    fn high_notes_do_not_alias() {
        // Harmonics of a 4.7 kHz saw above Nyquist would fold back here.
        let saw = basic_at(2.0 / 3.0, 4_700.0, 48_000);
        let aliases = [19_800.0, 15_100.0, 10_400.0, 5_700.0, 1_000.0];
        for f in aliases {
            let m = tone_magnitude(&saw, f, 48_000.0);
            assert!(m < 1e-3, "{f} Hz: {m}");
        }
        assert!(tone_magnitude(&saw, 4_700.0, 48_000.0) > 0.6);
    }

    /// Two 64-sample frames: a sine, then an inverted sine.
    fn two_frames() -> Arc<AudioData> {
        let s: Vec<f32> = (0..64)
            .map(|i| (std::f32::consts::TAU * i as f32 / 64.0).sin())
            .collect();
        let mut ch = s.clone();
        ch.extend(s.iter().map(|v| -v));
        ch.extend([0.5; 10]); // a partial frame, ignored
        Arc::new(AudioData {
            sample_rate: 48_000,
            channels: vec![ch],
        })
    }

    fn buffer_harness(block: usize, config: Config) -> Harness {
        let mut h = harness(config, block);
        h.buffer("table", two_frames()).set("freq", 750.0);
        h
    }

    #[test]
    fn buffer_frames_are_prepared_then_played() {
        let config = json!({ "table": "buffer", "frame_size": 64 });
        // 64-sample frames have 6 levels: 7 steps of 16 samples per frame.
        let per_frame = 7 * BUILD_INTERVAL as usize;
        let mut h = buffer_harness(64, config.clone());
        let o = h.run(2_000).float("out").to_vec();
        assert!(
            o[..per_frame - BUILD_INTERVAL as usize]
                .iter()
                .all(|&v| v == 0.0)
        );
        // After both frames: position 0 is the sine.
        let tail = &o[1_000..];
        let want = (std::f64::consts::TAU * 750.0 * 1_000.0 / 48_000.0).sin() as f32;
        // Linear reads of a 64-sample sine are good to about 1e-3.
        assert!((tail[0] - want).abs() < 3e-3, "{} vs {want}", tail[0]);
        // Position 1 is the inverted sine, and 0.5 cancels out.
        let mut h = buffer_harness(64, config.clone());
        h.set("position", 1.0);
        let inv = h.run(2_000).float("out").to_vec();
        assert!((inv[1_000] + want).abs() < 3e-3);
        let mut h = buffer_harness(64, config.clone());
        h.set("position", 0.5);
        assert!(peak(&h.run(2_000).float("out")[1_000..]) < 1e-4);
        // max_frames 1 keeps only the sine.
        let mut h = buffer_harness(
            64,
            json!({ "table": "buffer", "frame_size": 64, "max_frames": 1 }),
        );
        h.set("position", 1.0);
        assert!((h.run(2_000).float("out")[1_000] - want).abs() < 3e-3);
    }

    #[test]
    fn buffer_mode_without_a_buffer_is_silent() {
        let mut h = harness(json!({ "table": "buffer" }), 64);
        assert!(h.run(1_000).float("out").iter().all(|&v| v == 0.0));
    }

    #[test]
    fn output_is_independent_of_block_size() {
        let fm: Vec<f32> = (0..3_000)
            .map(|i| 300.0 * (i as f32 * 0.003).sin())
            .collect();
        let pos: Vec<f32> = (0..3_000).map(|i| i as f32 / 3_000.0).collect();
        for config in [json!({}), json!({ "table": "buffer", "frame_size": 64 })] {
            let render = |block: usize| {
                let mut h = harness(config.clone(), block);
                if is_buffer(&config) {
                    h.buffer("table", two_frames());
                }
                h.set("freq", 500.0)
                    .set_samples("fm", fm.clone())
                    .set_samples("position", pos.clone())
                    .triggers("sync", &[1_234])
                    .triggers("stop", &[2_500]);
                let out = h.run(3_000);
                (out.float("out").to_vec(), out.triggers("end").to_vec())
            };
            let one = render(1);
            assert_eq!(one.1, vec![2_500]);
            assert_eq!(one, render(64), "{config}");
            assert_eq!(one, render(512), "{config}");
        }
    }

    #[test]
    fn extreme_inputs_stay_finite() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 2.5, 1e-30];
        let wild: Vec<f32> = wild.iter().cycle().take(512).copied().collect();
        let mut h = harness(json!({}), 64);
        h.set_samples("freq", wild.clone())
            .set_samples("fm", wild.iter().rev().copied().collect())
            .set_samples("position", wild.clone())
            .set_samples("phase", wild);
        let out = h.run(512);
        assert!(
            out.float("out")
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 2.0)
        );
    }

    #[test]
    fn generator_contract_and_reset() {
        let mut h = harness(json!({}), 64);
        h.stopped()
            .triggers("start", &[100])
            .triggers("stop", &[400]);
        let out = h.run(512);
        let o = out.float("out");
        assert_eq!(peak(&o[..100]), 0.0);
        assert!(o[100].abs() < 1e-6, "starts at phase 0 (a sine's zero)");
        assert!(peak(&o[100..400]) > 0.9);
        assert_eq!(peak(&o[400..]), 0.0);
        assert_eq!(out.triggers("end"), &[400]);
        let mut h = harness(json!({}), 64);
        let first = h.run(100).float("out").to_vec();
        h.reset();
        h.play();
        assert_eq!(h.run(100).float("out"), first.as_slice());
    }
}
