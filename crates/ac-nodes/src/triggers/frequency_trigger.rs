use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec,
    Unit, node,
};
use ac_dsp::biquad::{Coeffs, Mode, Tdf2};

node! {
    kind: "frequency_trigger",
    name: "Frequency Trigger",
    category: Triggers,
    keywords: ["band", "detector", "pitch", "range", "kick", "sidechain", "threshold"],
    /// Fires when there is enough energy in a frequency band, or when a
    /// pitch enters a range.
    ///
    /// Config `mode` picks what it listens to:
    ///
    /// - `"band"` (default): the first channel of `in` is filtered to the
    ///   band from `low` to `high` Hz (4th-order Butterworth highpass and
    ///   lowpass, 24 dB per octave outside the band) and its level is
    ///   followed with a mean-square detector averaging over 10 ms, or one
    ///   period of `low` if that is longer. The level reads in dB where a
    ///   full-scale sine is 0 dB. The condition is met when the level rises
    ///   to `threshold` and stays met until it falls below `threshold −
    ///   hysteresis`, so a level hovering at the threshold doesn't chatter.
    ///   `low` and `high` are read once per block, kept between 10 Hz and
    ///   just under half the sample rate, and swapped if reversed.
    /// - `"pitch"`: `freq` is a frequency in Hz, usually wired from
    ///   `PitchDetect`. `condition` chooses `within` (`low` ≤ `freq` ≤
    ///   `high`), `above` (`freq` > `high`), or `below` (`freq` < `low`).
    ///   A `freq` of 0 or less (no pitch yet), or NaN, never meets the
    ///   condition. Inputs are read every sample, so an audio-rate `freq`
    ///   is followed exactly.
    ///
    /// `trigger` fires on the sample the condition becomes true; `gate` is
    /// high while it holds.
    ///
    /// **Try this:** in band mode, set `low` 40 and `high` 120 Hz on a drum
    /// loop and wire `trigger` into an `ADSR` that ducks a pad: a kick
    /// sidechain. In pitch mode, wire `PitchDetect`'s `freq` in with
    /// `condition` above 400 Hz to switch effects on only for high notes.
    pub struct FrequencyTrigger {
        pitch: bool,
        filters: [Tdf2; 4],
        coeffs: [Coeffs; 4],
        key: Option<(f32, f32)>,
        /// Mean square of the band.
        power: f64,
        gate: bool,
        clock: u64,
    }
}

// Band mode inputs.
const IN: usize = 0;
const THRESHOLD: usize = 3;
const HYSTERESIS: usize = 4;
// Pitch mode inputs.
const FREQ: usize = 0;
const CONDITION: usize = 3;
// Both.
const LOW: usize = 1;
const HIGH: usize = 2;

const TRIGGER: usize = 0;
const GATE: usize = 1;

const CONDITIONS: &[&str] = &["within", "above", "below"];

/// Pole Qs of a 4th-order Butterworth filter as two biquads.
const BUTTERWORTH_Q: [f64; 2] = [0.541_196_100_146_197, 1.306_562_964_876_376_5];
/// Shortest detector averaging time in seconds.
const MIN_AVERAGE: f64 = 0.01;
/// Tiny filter state is flushed on this grid of samples.
const FLUSH_EVERY: u64 = 64;

fn is_pitch(config: &Config) -> bool {
    config.get_str_or("mode", "band") == "pitch"
}

fn finite_or(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

impl FrequencyTrigger {
    /// Recompute the band filters if `low`/`high` changed.
    fn update(&mut self, low: f32, high: f32, rate: f64) {
        let key = (low, high);
        if self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        let top = (0.49 * rate) as f32;
        let (mut lo, mut hi) = (
            finite_or(low, 200.0).clamp(10.0, top),
            finite_or(high, 2000.0).clamp(10.0, top),
        );
        if lo > hi {
            std::mem::swap(&mut lo, &mut hi);
        }
        let (lo, hi) = (f64::from(lo), f64::from(hi));
        for (j, q) in BUTTERWORTH_Q.iter().enumerate() {
            self.coeffs[j] = Coeffs::rbj(Mode::Highpass, lo, *q, 0.0, rate);
            self.coeffs[j + 2] = Coeffs::rbj(Mode::Lowpass, hi, *q, 0.0, rate);
        }
    }

    fn process_band(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let rate = ctx.sample_rate;
        let low = io.inputs.float(LOW).first();
        self.update(low, io.inputs.float(HIGH).first(), rate);
        let average = (1.0 / f64::from(finite_or(low, 200.0)).clamp(10.0, rate)).max(MIN_AVERAGE);
        let a = 1.0 - (-1.0 / (average * rate)).exp();
        let input = io.inputs.audio(IN);
        let x = input.channel(0);
        let threshold = io.inputs.float(THRESHOLD);
        let hysteresis = io.inputs.float(HYSTERESIS);
        let (mut gate, mut trigger) = io.outputs.float_trigger(GATE, TRIGGER);
        for (i, &s) in x.iter().enumerate() {
            let mut y = s;
            for (f, k) in self.filters.iter_mut().zip(&self.coeffs) {
                y = f.tick(k, y);
            }
            self.power += (f64::from(y) * f64::from(y) - self.power) * a;
            let level = 10.0 * (2.0 * self.power + 1e-30).log10() as f32;
            let on = finite_or(threshold.at(i), -30.0);
            let off = on - finite_or(hysteresis.at(i), 3.0).max(0.0);
            if !self.gate && level >= on {
                self.gate = true;
                trigger.push(i as u32);
            } else if self.gate && level < off {
                self.gate = false;
            }
            gate.set(i, if self.gate { 1.0 } else { 0.0 });
            self.clock += 1;
            if self.clock.is_multiple_of(FLUSH_EVERY) {
                for f in &mut self.filters {
                    f.flush();
                }
                if self.power < 1e-30 {
                    self.power = 0.0;
                }
            }
        }
    }

    fn process_pitch(&mut self, io: &mut Io) {
        let freq = io.inputs.float(FREQ);
        let low = io.inputs.float(LOW);
        let high = io.inputs.float(HIGH);
        let condition = io.inputs.choice(CONDITION, CONDITIONS.len());
        let frames = io.frames();
        let (mut gate, mut trigger) = io.outputs.float_trigger(GATE, TRIGGER);
        for i in 0..frames {
            let f = freq.at(i);
            let (mut lo, mut hi) = (low.at(i), high.at(i));
            if lo > hi {
                std::mem::swap(&mut lo, &mut hi);
            }
            let met = f.is_finite()
                && f > 0.0
                && match condition {
                    0 => f >= lo && f <= hi,
                    1 => f > hi,
                    _ => f < lo,
                };
            if met && !self.gate {
                trigger.push(i as u32);
            }
            self.gate = met;
            gate.set(i, if met { 1.0 } else { 0.0 });
        }
    }
}

impl Node for FrequencyTrigger {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let low = SocketSpec::float("low", Unit::Hz, 200.0)
            .range(20.0, 20_000.0)
            .log();
        let high = SocketSpec::float("high", Unit::Hz, 2000.0)
            .range(20.0, 20_000.0)
            .log();
        let sig = if is_pitch(config) {
            Signature::new()
                .input(
                    SocketSpec::float("freq", Unit::Hz, 0.0)
                        .range(20.0, 5000.0)
                        .log()
                        .no_smooth()
                        .doc("Frequency to watch, e.g. from Pitch Detect."),
                )
                .input(low.doc("Bottom of the range."))
                .input(high.doc("Top of the range."))
                .input(
                    SocketSpec::choice("condition", CONDITIONS, "within")
                        .doc("Within, above, or below the range."),
                )
        } else {
            Signature::new()
                .input(SocketSpec::audio("in").doc("Signal to listen to (first channel)."))
                .input(low.doc("Bottom of the band."))
                .input(high.doc("Top of the band."))
                .input(
                    SocketSpec::float("threshold", Unit::Db, -30.0)
                        .range(-80.0, 0.0)
                        .doc("Band level that fires the trigger."),
                )
                .input(
                    SocketSpec::float("hysteresis", Unit::Db, 3.0)
                        .range(0.0, 24.0)
                        .doc("How far the level must fall before it can fire again."),
                )
        };
        sig.output(SocketSpec::trigger("trigger").doc("Fires when the condition becomes true."))
            .output(
                SocketSpec::boolean("gate", false)
                    .rate(Rate::Sample)
                    .doc("High while the condition holds."),
            )
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        FrequencyTrigger {
            pitch: is_pitch(config),
            filters: [Tdf2::default(); 4],
            coeffs: [Coeffs::IDENTITY; 4],
            key: None,
            power: 0.0,
            gate: false,
            clock: 0,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        if self.pitch {
            self.process_pitch(io);
        } else {
            self.process_band(ctx, io);
        }
    }

    fn reset(&mut self) {
        for f in &mut self.filters {
            f.reset();
        }
        self.power = 0.0;
        self.gate = false;
        self.clock = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::{Harness, Output};
    use serde_json::json;

    const RATE: f64 = 48_000.0;

    fn sine(freq: f64, amp: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (amp * (std::f64::consts::TAU * freq * i as f64 / RATE).sin()) as f32)
            .collect()
    }

    fn band(x: Vec<f32>, set: &[(&str, f64)], block: usize) -> Output {
        let n = x.len();
        let mut h =
            Harness::with_resources::<FrequencyTrigger>(json!({}), RATE, block, &NoResources);
        h.set_audio("in", vec![x]);
        for (k, v) in set {
            h.set(k, *v);
        }
        h.run(n)
    }

    #[test]
    fn band_mode_fires_for_tones_inside_the_band_only() {
        let set = [("low", 500.0), ("high", 2000.0), ("threshold", -20.0)];
        // A −6 dB tone in the band fires once and holds the gate.
        let out = band(sine(1000.0, 0.5, 9600), &set, 64);
        assert_eq!(out.triggers("trigger").len(), 1);
        let t = out.triggers("trigger")[0] as usize;
        assert!(t < 1500, "{t}");
        assert!(out.float("gate")[t..].iter().all(|g| *g == 1.0));
        assert!(out.float("gate")[..t].iter().all(|g| *g == 0.0));
        // The same tone far outside the band doesn't.
        for f in [60.0, 12_000.0] {
            let out = band(sine(f, 0.5, 9600), &set, 64);
            assert!(out.triggers("trigger").is_empty(), "{f}");
        }
        // Too quiet: no trigger.
        let out = band(sine(1000.0, 0.05, 9600), &set, 64);
        assert!(out.triggers("trigger").is_empty());
    }

    #[test]
    fn hysteresis_keeps_the_gate_open_through_small_dips() {
        // −10 dB, then −14 dB, then −30 dB.
        let mut x = sine(1000.0, 0.316, 9600);
        x.extend(sine(1000.0, 0.2, 9600));
        x.extend(sine(1000.0, 0.0316, 9600));
        let set = [("threshold", -12.0), ("hysteresis", 6.0)];
        let out = band(x, &set, 64);
        assert_eq!(out.triggers("trigger").len(), 1);
        let gate = out.float("gate");
        assert_eq!(gate[18_000], 1.0, "still open at -14 dB");
        assert_eq!(gate[28_000], 0.0, "closed at -30 dB");
    }

    #[test]
    fn bursts_fire_once_each() {
        let mut x = Vec::new();
        for _ in 0..4 {
            x.extend(sine(800.0, 0.5, 2400));
            x.extend(vec![0.0; 4800]);
        }
        let out = band(x, &[("threshold", -20.0)], 64);
        assert_eq!(out.triggers("trigger").len(), 4);
    }

    #[test]
    fn pitch_mode_compares_freq_with_the_range() {
        let freq: Vec<f32> = (0..1000)
            .map(|i| match i {
                0..=99 => 0.0,
                100..=399 => 300.0,
                400..=699 => 900.0,
                _ => 150.0,
            })
            .collect();
        let run = |cond: &str, block: usize| {
            let mut h = Harness::with_resources::<FrequencyTrigger>(
                json!({ "mode": "pitch" }),
                RATE,
                block,
                &NoResources,
            );
            h.set_samples("freq", freq.clone())
                .set("low", 200.0)
                .set("high", 600.0)
                .set("condition", cond);
            h.run(1000)
        };
        assert_eq!(run("within", 64).triggers("trigger"), &[100]);
        assert_eq!(run("above", 64).triggers("trigger"), &[400]);
        // No pitch (0) is not "below".
        assert_eq!(run("below", 64).triggers("trigger"), &[700]);
        let w = run("within", 64);
        assert_eq!(w.float("gate")[99], 0.0);
        assert_eq!(w.float("gate")[100], 1.0);
        assert_eq!(w.float("gate")[400], 0.0);
        for cond in ["within", "above", "below"] {
            let one = run(cond, 1);
            assert_eq!(one.triggers("trigger"), run(cond, 512).triggers("trigger"));
            assert_eq!(one.float("gate"), run(cond, 512).float("gate"));
        }
    }

    #[test]
    fn sockets_follow_the_mode() {
        let names = |c: Config| -> Vec<String> {
            FrequencyTrigger::describe(&c)
                .inputs
                .iter()
                .map(|s| s.name.to_string())
                .collect()
        };
        assert_eq!(
            names(json!({})),
            ["in", "low", "high", "threshold", "hysteresis"]
        );
        assert_eq!(
            names(json!({ "mode": "pitch" })),
            ["freq", "low", "high", "condition"]
        );
        assert_eq!(names(json!({ "mode": "nonsense" })), names(json!({})));
    }

    #[test]
    fn block_size_does_not_change_band_mode() {
        let mut x = sine(700.0, 0.4, 3000);
        x.extend(vec![0.0; 3000]);
        x.extend(sine(1500.0, 0.6, 3000));
        let one = band(x.clone(), &[], 1);
        for block in [64, 512] {
            let other = band(x.clone(), &[], block);
            assert_eq!(one.triggers("trigger"), other.triggers("trigger"));
            assert_eq!(one.float("gate"), other.float("gate"));
        }
    }

    #[test]
    fn bad_input_is_harmless_and_reset_closes_the_gate() {
        let mut x = sine(1000.0, 0.5, 4000);
        x[10] = f32::NAN;
        x[2000] = f32::INFINITY;
        let out = band(
            x.clone(),
            &[("low", f64::NAN), ("high", -5.0), ("threshold", f64::NAN)],
            64,
        );
        assert!(out.float("gate").iter().all(|g| g.is_finite()));
        let mut h = Harness::new::<FrequencyTrigger>(json!({}));
        h.set_audio("in", vec![sine(1000.0, 0.5, 4000)]);
        let out = h.run(4000);
        assert_eq!(*out.float("gate").last().unwrap(), 1.0);
        h.reset();
        let out = h.run(64);
        assert_eq!(out.float("gate")[0], 0.0);
    }
}
