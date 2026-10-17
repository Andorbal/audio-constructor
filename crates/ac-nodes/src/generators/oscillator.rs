use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec,
    Unit, node,
};
use ac_dsp::osc::{Phasor, Shape, increment, wave, wrap01};

use crate::segments::Runs;

node! {
    kind: "oscillator",
    name: "Oscillator",
    category: Generators,
    keywords: ["osc", "vco", "lfo", "tone", "sine", "saw", "square", "triangle", "pulse", "wave"],
    /// A periodic waveform (sine, triangle, saw, square, or pulse) at
    /// `freq` + `fm` Hz, as a sample-rate `Float` that wires into audio or
    /// into any parameter.
    ///
    /// The output swings between -1 and 1. Saw, square, and pulse are
    /// band-limited with PolyBLEP and the triangle with PolyBLAMP, so high
    /// notes stay clean; set the `pure` config flag for the naive shapes
    /// (sharper corners, no smoothing) when using the oscillator as an LFO.
    ///
    /// - `width` sets the pulse's duty cycle (0.5 is a square).
    /// - `phase` offsets the waveform by a fraction of a cycle; wiring a
    ///   signal into it gives phase modulation.
    /// - `sync` resets the phase to 0 at each trigger (hard sync). The jump
    ///   at the sync point is not band-limited.
    /// - `fm` adds Hz to `freq` (linear FM); negative totals run the
    ///   waveform backwards (through-zero FM).
    ///
    /// Generator contract: silent until `start`, which starts (or restarts)
    /// from phase 0. `stop` silences it and fires `end`; `reset` returns to
    /// phase 0 without changing whether it plays. Events at the same frame
    /// apply in the order stop, start, reset, sync.
    ///
    /// **Try this:** wire a second Oscillator (`pure`, 5 Hz, sine) through a
    /// `Map` (-1..1 → -20..20) into `fm` for vibrato, or run a saw through a
    /// `Biquad` lowpass for a classic subtractive synth voice.
    pub struct Oscillator {
        osc: Phasor,
        playing: bool,
        bandlimited: bool,
    }
}

const FREQ: usize = 0;
const SHAPE: usize = 1;
const WIDTH: usize = 2;
const PHASE: usize = 3;
const SYNC: usize = 4;
const FM: usize = 5;
const START: usize = 6;
const STOP: usize = 7;
const RESET: usize = 8;

const OUT: usize = 0;
const PLAYING: usize = 1;
const END: usize = 2;

impl Node for Oscillator {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(
                SocketSpec::float("freq", Unit::Hz, 440.0)
                    .range(20.0, 20_000.0)
                    .log()
                    .doc("Frequency in hertz."),
            )
            .input(SocketSpec::choice("shape", Shape::NAMES, "sine").doc("Waveform."))
            .input(
                SocketSpec::float("width", Unit::Generic, 0.5)
                    .range(0.01, 0.99)
                    .doc("Pulse width (fraction of the cycle spent high)."),
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
                    .doc("The waveform, -1 to 1."),
            )
            .generator_outputs()
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        Oscillator {
            osc: Phasor::new(),
            playing: false,
            bandlimited: !config.get_bool_or("pure", false),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let rate = if ctx.sample_rate.is_finite() && ctx.sample_rate > 0.0 {
            ctx.sample_rate
        } else {
            48_000.0
        };
        let freq = io.inputs.float(FREQ);
        let fm = io.inputs.float(FM);
        let width = io.inputs.float(WIDTH);
        let offset = io.inputs.float(PHASE);
        let shape = Shape::from_index(io.inputs.choice(SHAPE, Shape::NAMES.len()));
        let events = [
            io.inputs.triggers(STOP),
            io.inputs.triggers(START),
            io.inputs.triggers(RESET),
            io.inputs.triggers(SYNC),
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
                if !self.playing {
                    for i in run.at..run.until {
                        out.set(i, 0.0);
                    }
                    continue;
                }
                for i in run.at..run.until {
                    let inc = increment(f64::from(freq.at(i)) + f64::from(fm.at(i)), rate);
                    let w = finite_or(width.at(i), 0.5).clamp(0.01, 0.99);
                    let p = wrap01(self.osc.phase() + f64::from(finite_or(offset.at(i), 0.0)));
                    let v = wave(shape, p, inc.abs(), f64::from(w), self.bandlimited);
                    out.set(i, v as f32);
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

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::{Harness, peak, rms, tone_magnitude, zero_crossing_freq};
    use serde_json::json;

    fn harness(config: Config, rate: f64, block: usize) -> Harness {
        Harness::with_resources::<Oscillator>(config, rate, block, &NoResources)
    }

    #[test]
    fn frequency_is_right_at_several_rates() {
        for rate in [44_100.0, 96_000.0] {
            for (shape, freq) in [("sine", 440.0), ("saw", 1000.0), ("square", 220.0)] {
                let mut h = harness(json!({}), rate, 64);
                h.set("shape", shape).set("freq", freq);
                let out = h.run(rate as usize);
                let f = zero_crossing_freq(out.float("out"), rate);
                assert!((f - freq).abs() < 0.5, "{shape} at {rate}: {f}");
            }
        }
    }

    #[test]
    fn polyblep_saw_aliases_less_than_naive() {
        let rate = 48_000.0;
        let render = |pure: bool| {
            let mut h = harness(json!({ "pure": pure }), rate, 64);
            h.set("shape", "saw").set("freq", 4_700.0);
            h.run(48_000).float("out").to_vec()
        };
        let (bl, naive) = (render(false), render(true));
        // Harmonics 6..=10 of 4.7 kHz fold back to these frequencies.
        let aliases = [19_800.0, 15_100.0, 10_400.0, 5_700.0, 1_000.0];
        let energy = |x: &[f32]| -> f64 {
            aliases
                .iter()
                .map(|&f| tone_magnitude(x, f, rate).powi(2))
                .sum()
        };
        let (e_bl, e_naive) = (energy(&bl), energy(&naive));
        assert!(
            e_bl < 0.25 * e_naive,
            "band-limited {e_bl} vs naive {e_naive}"
        );
        // The fundamental is untouched: a saw's first partial is 2/π.
        let fund = tone_magnitude(&bl, 4_700.0, rate);
        assert!((fund - 2.0 / std::f64::consts::PI).abs() < 0.03, "{fund}");
    }

    #[test]
    fn silent_until_started_then_starts_on_the_frame() {
        let mut h = harness(json!({}), 48_000.0, 64);
        h.stopped();
        let out = h.run(256);
        assert_eq!(peak(out.float("out")), 0.0);
        assert!(out.float("playing").iter().all(|&p| p == 0.0));

        let mut h = harness(json!({}), 48_000.0, 64);
        h.stopped().triggers("start", &[300]).set("phase", 0.25);
        let out = h.run(512);
        let o = out.float("out");
        assert_eq!(peak(&o[..300]), 0.0);
        // Phase offset 0.25 on a sine starts at the top.
        assert!((o[300] - 1.0).abs() < 1e-6);
        assert_eq!(out.float("playing")[511], 1.0);
    }

    #[test]
    fn plays_on_transport_play_by_default() {
        let mut h = harness(json!({}), 48_000.0, 64);
        let out = h.run(4800);
        assert!((rms(out.float("out")) - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.01);
    }

    #[test]
    fn stop_at_offset_silences_and_fires_end() {
        let mut h = harness(json!({}), 48_000.0, 64);
        h.triggers("stop", &[200]);
        let out = h.run(512);
        let o = out.float("out");
        assert!(rms(&o[..200]) > 0.5);
        assert_eq!(peak(&o[200..]), 0.0);
        assert_eq!(out.triggers("end"), &[200]);
        assert_eq!(out.float("playing")[511], 0.0);
    }

    #[test]
    fn transport_stop_fires_end_once() {
        let mut h = harness(json!({}), 48_000.0, 64);
        h.run(128);
        h.stop();
        let out = h.run(256);
        assert_eq!(out.triggers("end"), &[128]);
        assert_eq!(peak(out.float("out")), 0.0);
    }

    #[test]
    fn sync_resets_phase_like_a_fresh_start() {
        let mut a = harness(json!({}), 48_000.0, 64);
        a.set("shape", "saw")
            .set("freq", 333.0)
            .triggers("sync", &[1000]);
        let mut b = harness(json!({}), 48_000.0, 64);
        b.stopped()
            .set("shape", "saw")
            .set("freq", 333.0)
            .triggers("start", &[1000]);
        let (oa, ob) = (a.run(2000), b.run(2000));
        assert_eq!(&oa.float("out")[1000..], &ob.float("out")[1000..]);
        assert_ne!(&oa.float("out")[..1000], &ob.float("out")[..1000]);
    }

    #[test]
    fn reset_returns_to_phase_zero_and_keeps_playing() {
        let mut h = harness(json!({}), 48_000.0, 64);
        h.set("phase", 0.25).triggers("reset", &[777]);
        let out = h.run(1024);
        let o = out.float("out");
        assert!((o[777] - 1.0).abs() < 1e-6);
        assert!(out.triggers("end").is_empty());
        assert_eq!(out.float("playing")[1023], 1.0);
    }

    #[test]
    fn output_is_independent_of_block_size() {
        let fm: Vec<f32> = (0..3000)
            .map(|i| 300.0 * (i as f32 * 0.003).sin())
            .collect();
        let render = |block: usize, shape: &str| {
            let mut h = harness(json!({}), 48_000.0, block);
            h.set("shape", shape)
                .set("freq", 500.0)
                .set_samples("fm", fm.clone())
                .triggers("sync", &[1234])
                .triggers("stop", &[2500]);
            h.run(3000).float("out").to_vec()
        };
        for shape in Shape::NAMES {
            let one = render(1, shape);
            assert_eq!(one, render(64, shape), "{shape}");
            assert_eq!(one, render(1000, shape), "{shape}");
        }
    }

    #[test]
    fn shapes_have_expected_levels() {
        let level = |shape: &str, width: f64| {
            let mut h = harness(json!({}), 48_000.0, 64);
            h.set("shape", shape).set("freq", 100.0).set("width", width);
            h.run(48_000).float("out").to_vec()
        };
        let sq = level("square", 0.5);
        assert!((rms(&sq) - 1.0).abs() < 0.01);
        let tri = level("triangle", 0.5);
        assert!((peak(&tri) - 1.0).abs() < 0.01);
        assert!((rms(&tri) - 1.0 / 3f32.sqrt()).abs() < 0.01);
        let pulse = level("pulse", 0.25);
        let mean = pulse.iter().sum::<f32>() / pulse.len() as f32;
        assert!((mean + 0.5).abs() < 0.01, "{mean}");
    }

    #[test]
    fn long_runs_do_not_drift() {
        // 10 s of 441 Hz at 44.1 kHz ends exactly on a cycle boundary.
        let mut h = harness(json!({}), 44_100.0, 1024);
        h.set("freq", 441.0);
        h.run(441_000);
        let next = h.run(1);
        assert!(next.float("out")[0].abs() < 1e-6);
    }

    #[test]
    fn extreme_inputs_stay_finite() {
        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0, 2.5, 1e-30];
        let wild: Vec<f32> = wild.iter().cycle().take(512).copied().collect();
        for shape in Shape::NAMES {
            let mut h = harness(json!({}), 48_000.0, 64);
            h.set("shape", *shape)
                .set_samples("freq", wild.clone())
                .set_samples("fm", wild.iter().rev().copied().collect())
                .set_samples("width", wild.clone())
                .set_samples("phase", wild.clone());
            let out = h.run(512);
            assert!(
                out.float("out")
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 2.0),
                "{shape}"
            );
        }
    }

    #[test]
    fn negative_frequency_runs_backwards() {
        let mut fwd = harness(json!({ "pure": true }), 48_000.0, 64);
        fwd.set("shape", "saw").set("freq", 0.0).set("fm", 480.0);
        let mut back = harness(json!({ "pure": true }), 48_000.0, 64);
        back.set("shape", "saw").set("freq", 0.0).set("fm", -480.0);
        let (f, b) = (fwd.run(100), back.run(100));
        // Rising saw forwards, falling saw backwards.
        assert!(f.float("out")[10] > f.float("out")[0]);
        assert!(b.float("out")[10] < b.float("out")[1]);
    }
}
