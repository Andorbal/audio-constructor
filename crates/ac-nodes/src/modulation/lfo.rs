use ac_core::{
    Config, ConfigExt, FloatIn, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature,
    SocketSpec, Unit, node,
};
use ac_dsp::osc::{self, Shape, wrap01};
use ac_dsp::rng::Rng;

node! {
    kind: "lfo",
    name: "LFO",
    category: Modulation,
    keywords: ["low frequency oscillator", "modulation", "wobble", "vibrato", "tremolo", "sweep", "sync"],
    /// A low-frequency oscillator for modulation: a cheap, block-rate wave
    /// to move other parameters.
    ///
    /// `shape` picks the wave: `sine`, `triangle`, `saw` (rising), `ramp_down`
    /// (falling), `square`, `random` (a new random value each cycle, held),
    /// or `smooth_random` (glides from one random value to the next across
    /// each cycle). Every shape starts its cycle at phase 0: the sine and
    /// triangle at 0 heading up, the saw at its bottom.
    ///
    /// The `unit` config sets what `rate` means:
    ///
    /// - `hz` (default): cycles per second. The LFO runs freely, even with
    ///   the transport stopped, and `retrigger` restarts the cycle at phase
    ///   0 on its exact sample.
    /// - `beats`: beats per cycle (1 = once per beat, 4 = once per 4/4
    ///   bar). The phase is worked out from the song position, so the LFO
    ///   stays locked to the beat through tempo changes, seeks, and loops,
    ///   and holds still while the transport is stopped. Changing `rate`
    ///   jumps to where the new rate falls on that grid. `retrigger` does
    ///   nothing in this mode.
    ///
    /// `phase` (0..1) shifts the wave later in its cycle. `polarity` picks
    /// `bipolar` (−1..1) or `unipolar` (0..1, the bipolar wave moved up and
    /// halved), and the result is multiplied by `amount`. `smooth` (0..1)
    /// softens the random shapes with a low-pass whose time constant is
    /// that fraction of a cycle: 0 leaves the steps sharp, 1 rounds them
    /// into curves (and narrows their swing); other shapes ignore it.
    ///
    /// `out` changes once per block (the value at the block's first
    /// sample), which keeps hundreds of LFOs cheap; use an `Oscillator`
    /// with `pure` on when you need audio-rate modulation. Random values
    /// come from the project's seed and this node's place in the patch, so
    /// renders repeat. NaN or infinite inputs are repaired: a rate is
    /// limited to half the sample rate (Hz) or 1/256..100 000 beats.
    ///
    /// **Try this:** set `unit` to `beats`, `rate` 4, and `shape`
    /// `ramp_down`, then `Map` the output to 200..4000 into a `Biquad`
    /// cutoff for a filter that sweeps down once every bar, in time with
    /// the song. A `random` LFO at 8 Hz with `amount` 0.3 into an
    /// `Oscillator`'s pitch (through `Convert` `semitones_to_ratio`) makes
    /// a sample-and-hold warble.
    pub struct Lfo {
        beats: bool,
        seed: u64,
        /// Hz mode: phase within the cycle, 0..1.
        phase: f64,
        /// Hz mode: whole cycles completed (keys the random values).
        cycle: i64,
        /// Low-passed value for the random shapes.
        smoothed: f64,
        primed: bool,
    }
}

const SHAPE: usize = 0;
const RATE: usize = 1;
const PHASE: usize = 2;
const RETRIGGER: usize = 3;
const POLARITY: usize = 4;
const SMOOTH: usize = 5;
const AMOUNT: usize = 6;
const OUT: usize = 0;

const SHAPES: &[&str] = &[
    "sine",
    "triangle",
    "saw",
    "ramp_down",
    "square",
    "random",
    "smooth_random",
];
const POLARITIES: &[&str] = &["bipolar", "unipolar"];

const RANDOM: usize = 5;
const SMOOTH_RANDOM: usize = 6;

const MIN_BEATS: f64 = 1.0 / 256.0;
const MAX_BEATS: f64 = 100_000.0;

fn uses_beats(config: &Config) -> bool {
    config.get_str_or("unit", "hz") == "beats"
}

/// A random value in −1..1 for cycle `k`, the same every time it is asked
/// for, so the random shapes need no stored history and seek cleanly.
fn cycle_random(seed: u64, k: i64) -> f64 {
    Rng::new(seed ^ (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)).bipolar()
}

/// The bipolar wave at phase `p` (0..1) of cycle `k`.
fn wave(shape: usize, p: f64, k: i64, seed: u64) -> f64 {
    match shape {
        0 => osc::wave(Shape::Sine, p, 0.0, 0.5, false),
        1 => osc::wave(Shape::Triangle, p, 0.0, 0.5, false),
        2 => osc::wave(Shape::Saw, p, 0.0, 0.5, false),
        3 => -osc::wave(Shape::Saw, p, 0.0, 0.5, false),
        4 => osc::wave(Shape::Square, p, 0.0, 0.5, false),
        RANDOM => cycle_random(seed, k),
        _ => {
            // Cosine glide between this cycle's value and the next one's.
            let a = cycle_random(seed, k);
            let b = cycle_random(seed, k.wrapping_add(1));
            a + (b - a) * (0.5 - 0.5 * (std::f64::consts::PI * p).cos())
        }
    }
}

/// Whole cycles and the phase within the cycle (0..1) of a position in
/// cycles. Positions too large for an `i64` saturate.
fn split(x: f64) -> (i64, f64) {
    let whole = x.floor();
    let k = if whole.is_finite() { whole as i64 } else { 0 };
    (k, wrap01(x - whole))
}

fn finite(v: f32, default: f64) -> f64 {
    if v.is_finite() { f64::from(v) } else { default }
}

impl Node for Lfo {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let rate = if uses_beats(config) {
            SocketSpec::float("rate", Unit::Beats, 1.0)
                .range(0.25, 32.0)
                .log()
                .doc("Beats per cycle.")
        } else {
            SocketSpec::float("rate", Unit::Hz, 1.0)
                .range(0.01, 20.0)
                .log()
                .doc("Cycles per second.")
        };
        Signature::new()
            .input(SocketSpec::choice("shape", SHAPES, "sine").doc("The wave."))
            .input(rate)
            .input(
                SocketSpec::float("phase", Unit::Generic, 0.0)
                    .range(0.0, 1.0)
                    .doc("Shifts the wave within its cycle."),
            )
            .input(SocketSpec::trigger("retrigger").doc("Restart the cycle (Hz mode)."))
            .input(SocketSpec::choice("polarity", POLARITIES, "bipolar").doc("−1..1 or 0..1."))
            .input(
                SocketSpec::float("smooth", Unit::Generic, 0.0)
                    .range(0.0, 1.0)
                    .doc("Softens the random shapes."),
            )
            .input(
                SocketSpec::float("amount", Unit::Generic, 1.0)
                    .range(0.0, 1.0)
                    .doc("Scales the output."),
            )
            .output(
                SocketSpec::float("out", Unit::Generic, 0.0)
                    .rate(Rate::Block)
                    .doc("The wave, once per block."),
            )
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        Lfo {
            beats: uses_beats(config),
            seed: ctx.seed,
            phase: 0.0,
            cycle: 0,
            smoothed: 0.0,
            primed: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let shape = io.inputs.choice(SHAPE, SHAPES.len());
        let rate = io.inputs.float(RATE);
        let offset = finite(io.inputs.float(PHASE).first(), 0.0);
        let unipolar = io.inputs.choice(POLARITY, POLARITIES.len()) == 1;
        let smooth = finite(io.inputs.float(SMOOTH).first(), 0.0).clamp(0.0, 1.0);
        let amount = finite(io.inputs.float(AMOUNT).first(), 0.0);
        let frames = ctx.frames;
        let sr = if ctx.sample_rate > 0.0 {
            ctx.sample_rate
        } else {
            ctx.transport.tempo.sample_rate()
        };

        // Where the wave is at this block's first sample (whole cycles and
        // phase), and how many samples one cycle lasts (for smoothing).
        let (k, p, period) = if self.beats {
            let t = ctx.transport;
            let bpc = finite(rate.first(), 1.0);
            let bpc = bpc.abs().clamp(MIN_BEATS, MAX_BEATS).copysign(bpc);
            let (k, p) = split(t.tempo.beat_at(t.position) / bpc + offset);
            (k, p, bpc.abs() * t.tempo.samples_per_beat_at(t.position))
        } else {
            let retriggers = io.inputs.triggers(RETRIGGER);
            // Retriggers on frame 0 apply before the block's value is read.
            let at_zero = retriggers.iter().take_while(|&&r| r == 0).count();
            if at_zero > 0 {
                self.restart();
            }
            let (whole, p) = split(self.phase + offset);
            let k = self.cycle.wrapping_add(whole);
            // Run the phase on to the next block, restarting at each later
            // retrigger (offsets past the block count as its last frame).
            let mut pos = 0;
            for &r in &retriggers[at_zero..] {
                let r = (r as usize).clamp(pos, frames.saturating_sub(1));
                self.advance(&rate, pos, r, sr);
                self.restart();
                pos = r;
            }
            self.advance(&rate, pos, frames, sr);
            let inc = osc::increment(rate.first().into(), sr).abs();
            (k, p, if inc > 0.0 { 1.0 / inc } else { f64::INFINITY })
        };

        let v = wave(shape, p, k, self.seed);
        let v = if shape == RANDOM || shape == SMOOTH_RANDOM {
            if !self.primed || smooth <= 0.0 {
                self.smoothed = v;
            } else {
                // Exact one-pole response over this block to a held value.
                let keep = (-(frames as f64) / (smooth * period)).exp();
                self.smoothed = v + (self.smoothed - v) * keep;
            }
            self.smoothed
        } else {
            self.smoothed = v;
            v
        };
        self.primed = true;
        let v = if unipolar { 0.5 * (v + 1.0) } else { v };
        let y = (v * amount) as f32;
        io.outputs
            .set_float(OUT, if y.is_finite() { y } else { 0.0 });
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.cycle = 0;
        self.smoothed = 0.0;
        self.primed = false;
    }
}

impl Lfo {
    /// Hz mode: move the phase on over frames `from..to` of this block.
    fn advance(&mut self, rate: &FloatIn, from: usize, to: usize, sr: f64) {
        if to <= from {
            return;
        }
        let advance = if rate.is_varying() {
            (from..to)
                .map(|i| osc::increment(rate.at(i).into(), sr))
                .sum()
        } else {
            osc::increment(rate.first().into(), sr) * (to - from) as f64
        };
        let mut next = self.phase + advance;
        // Absorb rounding so a cycle that ends on a block boundary
        // (0.1 + 0.1 + ... = 0.9999999999999999) wraps there.
        let nearest = next.round();
        if (next - nearest).abs() < 1e-9 {
            next = nearest;
        }
        let whole = next.floor();
        self.cycle = self.cycle.wrapping_add(whole as i64);
        self.phase = wrap01(next - whole);
    }

    /// Retrigger: back to phase 0, starting a new cycle (a new random value).
    fn restart(&mut self) {
        if self.phase != 0.0 {
            self.cycle = self.cycle.wrapping_add(1);
        }
        self.phase = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{NoResources, OutRate, TempoMap, TempoSegment};
    use serde_json::json;

    /// One value per block.
    fn blocks(out: &[f32], block: usize) -> Vec<f32> {
        out.iter().step_by(block).copied().collect()
    }

    #[test]
    fn output_is_block_rate() {
        let sig = Lfo::describe(&json!({}));
        assert_eq!(sig.outputs[OUT].rate, OutRate::Fixed(Rate::Block));
        assert_eq!(sig.inputs[RATE].unit, Unit::Hz);
        assert_eq!(
            Lfo::describe(&json!({"unit": "beats"})).inputs[RATE].unit,
            Unit::Beats
        );
        let mut h = Harness::new::<Lfo>(json!({}));
        let o = h.run(128).float("out").to_vec();
        assert!(o[..64].iter().all(|&v| v == o[0]));
    }

    #[test]
    fn hz_sine_follows_the_phase() {
        // 750 Hz at 48 kHz is 64 samples per cycle: a quarter cycle per 16-frame block.
        let mut h = Harness::with_resources::<Lfo>(json!({}), 48_000.0, 16, &NoResources);
        h.set("rate", 750.0);
        let v = blocks(h.run(128).float("out"), 16);
        let want = [0.0, 1.0, 0.0, -1.0, 0.0, 1.0, 0.0, -1.0];
        for (got, want) in v.iter().zip(want) {
            assert!((got - want).abs() < 1e-5, "{v:?}");
        }
    }

    #[test]
    fn shapes_at_known_phases() {
        // Read each shape at phases 0, 0.25, 0.5, 0.75 via the phase input.
        let cases = [
            ("triangle", [0.0, 1.0, 0.0, -1.0]),
            ("saw", [-1.0, -0.5, 0.0, 0.5]),
            ("ramp_down", [1.0, 0.5, 0.0, -0.5]),
            ("square", [1.0, 1.0, -1.0, -1.0]),
        ];
        for (shape, want) in cases {
            for (k, w) in want.iter().enumerate() {
                let mut h = Harness::new::<Lfo>(json!({}));
                h.set("shape", shape)
                    .set("phase", k as f32 * 0.25)
                    .set("rate", 0.0);
                let got = h.run(1).float("out")[0];
                assert!((got - w).abs() < 1e-6, "{shape} at {k}: {got}");
            }
        }
    }

    #[test]
    fn polarity_and_amount() {
        let mut h = Harness::new::<Lfo>(json!({}));
        h.set("shape", "square")
            .set("polarity", "unipolar")
            .set("amount", 0.5)
            .set("phase", 0.75)
            .set("rate", 0.0);
        assert_eq!(h.run(1).float("out")[0], 0.0);
        h.set("phase", 0.25);
        assert_eq!(h.run(1).float("out")[0], 0.5);
    }

    #[test]
    fn retrigger_restarts_on_its_sample() {
        // 375 Hz: 128 samples per cycle, blocks of 32 (a quarter cycle).
        let mut h = Harness::with_resources::<Lfo>(json!({}), 48_000.0, 32, &NoResources);
        h.set("rate", 375.0)
            .set("shape", "saw")
            .triggers("retrigger", &[80]);
        let v = blocks(h.run(192).float("out"), 32);
        // Block 3 starts at 96: 16 samples after the retrigger at 80.
        let expect = |samples: f64| (2.0 * samples / 128.0 - 1.0) as f32;
        assert!((v[2] - expect(64.0)).abs() < 1e-5, "{v:?}");
        assert!((v[3] - expect(16.0)).abs() < 1e-5, "{v:?}");
        assert!((v[4] - expect(48.0)).abs() < 1e-5, "{v:?}");
    }

    #[test]
    fn beats_mode_locks_to_the_song_position() {
        // 120 BPM at 48 kHz: 24 000 samples per beat. One cycle per beat.
        let mut h =
            Harness::with_resources::<Lfo>(json!({"unit": "beats"}), 48_000.0, 6_000, &NoResources);
        h.set("shape", "saw");
        let v = blocks(h.run(48_000).float("out"), 6_000);
        let want = [-1.0, -0.5, 0.0, 0.5, -1.0, -0.5, 0.0, 0.5];
        assert_eq!(v, want);

        // Stopped, it holds still.
        h.stop();
        let held = h.run(12_000).float("out").to_vec();
        assert!(held.iter().all(|&x| x == -1.0), "{:?}", &held[..2]);
    }

    #[test]
    fn beats_mode_follows_a_tempo_change() {
        let tempo = TempoMap::new(
            48_000.0,
            &[
                TempoSegment {
                    beat: 0.0,
                    bpm: 120.0,
                    time_sig: [4, 4],
                },
                TempoSegment {
                    beat: 2.0,
                    bpm: 60.0,
                    time_sig: [4, 4],
                },
            ],
        );
        let mut h = Harness::with_resources::<Lfo>(
            json!({"unit": "beats"}),
            48_000.0,
            12_000,
            &NoResources,
        );
        h.tempo(tempo).set("shape", "saw").set("rate", 2.0);
        let v = blocks(h.run(144_000).float("out"), 12_000);
        // Two beats per cycle: 48 000 samples at 120 BPM, then 96 000 at 60.
        assert_eq!(&v[..4], &[-1.0, -0.5, 0.0, 0.5]);
        assert_eq!(v[4], -1.0);
        assert_eq!(v[5], -0.75);
        assert_eq!(v[6], -0.5);
        assert_eq!(v[8], 0.0);
    }

    #[test]
    fn random_shapes_repeat_and_stay_in_range() {
        for shape in ["random", "smooth_random"] {
            let render = || {
                let mut h = Harness::new::<Lfo>(json!({}));
                h.set("shape", shape).set("rate", 100.0);
                h.run(48_000).float("out").to_vec()
            };
            let a = render();
            assert_eq!(a, render(), "{shape}");
            assert!(a.iter().all(|v| (-1.0..=1.0).contains(v)), "{shape}");
            let distinct: std::collections::BTreeSet<u32> = a.iter().map(|v| v.to_bits()).collect();
            assert!(distinct.len() > 50, "{shape}: {}", distinct.len());
        }
    }

    #[test]
    fn random_holds_for_a_cycle_and_smooth_softens_it() {
        let mut h = Harness::with_resources::<Lfo>(json!({}), 48_000.0, 64, &NoResources);
        h.set("shape", "random").set("rate", 75.0); // 640 samples per cycle.
        let v = blocks(h.run(6_400).float("out"), 64);
        assert!(v[..10].iter().all(|&x| x == v[0]));
        assert_ne!(v[10], v[0]);

        let jumps = |smooth: f32| {
            let mut h = Harness::new::<Lfo>(json!({}));
            h.set("shape", "random")
                .set("rate", 75.0)
                .set("smooth", smooth);
            let v = blocks(h.run(64_000).float("out"), 64);
            v.windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0, f32::max)
        };
        assert!(
            jumps(1.0) < 0.5 * jumps(0.0),
            "{} vs {}",
            jumps(1.0),
            jumps(0.0)
        );
    }

    #[test]
    fn reset_restarts_the_phase_and_wild_inputs_stay_finite() {
        let mut h = Harness::new::<Lfo>(json!({}));
        h.set("rate", 3.0).set("shape", "saw");
        let first = h.run(1_000).float("out").to_vec();
        h.reset();
        assert_eq!(h.run(1_000).float("out"), first.as_slice());

        let wild = [f32::NAN, f32::INFINITY, -1e30, 1e30, 0.0, -5.0];
        for unit in ["hz", "beats"] {
            for shape in SHAPES {
                let mut h = Harness::new::<Lfo>(json!({ "unit": unit }));
                let w: Vec<f32> = wild.iter().cycle().take(640).copied().collect();
                h.set("shape", *shape)
                    .set_block("rate", w.clone())
                    .set_block("phase", w.clone())
                    .set_block("amount", w.iter().rev().copied().collect())
                    .set_block("smooth", w.clone());
                assert!(
                    h.run(640).float("out").iter().all(|v| v.is_finite()),
                    "{unit} {shape}"
                );
            }
        }
    }

    #[test]
    fn phase_keeps_time_at_any_block_size() {
        // Blocks of 1, 64, and 512 all start a block every 512 frames; the
        // values there must agree, retrigger and modulated rate included.
        let rate: Vec<f32> = (0..8_192)
            .map(|i| 3.0 + (i % 1_000) as f32 * 0.01)
            .collect();
        let render = |block: usize| {
            let mut h = Harness::with_resources::<Lfo>(json!({}), 48_000.0, block, &NoResources);
            h.set_samples("rate", rate.clone())
                .set("shape", "smooth_random")
                .triggers("retrigger", &[1_030, 1_100, 5_000]);
            blocks(h.run(8_192).float("out"), 512)
        };
        let one = render(1);
        for other in [render(64), render(512)] {
            for (a, b) in one.iter().zip(&other) {
                assert!((a - b).abs() < 1e-5, "{one:?}\n{other:?}");
            }
        }
    }
}
