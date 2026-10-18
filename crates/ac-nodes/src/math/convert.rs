use ac_core::{
    Config, FloatIn, FloatOut, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec,
    Unit, node, note_to_hz,
};
use ac_dsp::util::gain_to_db;

use super::finite_or_zero;

node! {
    kind: "convert",
    name: "Convert",
    category: Math,
    keywords: ["db", "gain", "midi", "note", "pitch", "hz", "semitones", "ratio", "beats", "seconds", "samples", "units"],
    /// Converts `in` from one unit to another.
    ///
    /// | `op` | `out` |
    /// | --- | --- |
    /// | `db_to_gain` | decibels → linear gain, 10^(dB/20): 0 dB is 1, −6 dB about 0.5 |
    /// | `gain_to_db` | linear gain → decibels, 20·log10(\|gain\|), never below −120 dB |
    /// | `hz_to_note` | frequency → MIDI note number (A4 = 440 Hz = 69), with fractions |
    /// | `note_to_hz` | MIDI note number → frequency, equal temperament |
    /// | `semitones_to_ratio` | semitones → frequency ratio: 12 is 2, −12 is 0.5 |
    /// | `ratio_to_semitones` | frequency ratio → semitones |
    /// | `seconds_to_beats` | seconds → beats at the current tempo |
    /// | `beats_to_seconds` | beats → seconds at the current tempo |
    /// | `samples_to_seconds` | samples → seconds at the engine's sample rate |
    /// | `seconds_to_samples` | seconds → samples at the engine's sample rate |
    ///
    /// The beat conversions use the tempo at the song position where the
    /// block starts, so they follow tempo changes. `db_to_gain` has no
    /// floor: −60 dB is 0.001, not silence (the `Gain` node is the one that
    /// treats −60 dB as off). The output socket is labeled as a gain (×),
    /// the unit of the default op; whatever op is chosen, the number is in
    /// that op's output unit.
    ///
    /// The output runs at the rate of `in` and is always finite: a NaN or
    /// infinite input counts as 0, `hz_to_note` and `ratio_to_semitones` of
    /// 0 or less give 0, and a result too large to represent is 0.
    ///
    /// **Try this:** wire a `MidiNote` note (or a `Slider` from 36 to 84)
    /// through `note_to_hz` into an `Oscillator` freq to play it in tune,
    /// or put `semitones_to_ratio` between an `LFO` (amount 0.2) and a
    /// `Math` multiply on the frequency for a vibrato measured in
    /// semitones.
    pub struct Convert {}
}

const IN: usize = 0;
const OP: usize = 1;
const OUT: usize = 0;

const OPS: &[&str] = &[
    "db_to_gain",
    "gain_to_db",
    "hz_to_note",
    "note_to_hz",
    "semitones_to_ratio",
    "ratio_to_semitones",
    "seconds_to_beats",
    "beats_to_seconds",
    "samples_to_seconds",
    "seconds_to_samples",
];

impl Node for Convert {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::float("in", Unit::Generic, 0.0).doc("The value to convert."))
            .input(SocketSpec::choice("op", OPS, "db_to_gain").doc("From which unit to which."))
            .output(
                SocketSpec::float("out", Unit::Ratio, 1.0)
                    .poly()
                    .doc("The converted value, in the op's output unit."),
            )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Convert {}
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let x = io.inputs.float(IN);
        let op = io.inputs.choice(OP, OPS.len());
        let t = ctx.transport;
        let rate = if ctx.sample_rate > 0.0 {
            ctx.sample_rate
        } else {
            t.tempo.sample_rate()
        };
        let beats_per_second = t.tempo.bpm_at(t.position) / 60.0;
        let mut out = io.outputs.float(OUT);
        match op {
            0 => apply(&mut out, x, |v| 10f64.powf(v / 20.0)),
            1 => apply(&mut out, x, |v| f64::from(gain_to_db(v.abs() as f32))),
            2 => apply(&mut out, x, |v| {
                if v > 0.0 {
                    69.0 + 12.0 * (v / 440.0).log2()
                } else {
                    0.0
                }
            }),
            3 => apply(&mut out, x, |v| f64::from(note_to_hz(v as f32))),
            4 => apply(&mut out, x, |v| (v / 12.0).exp2()),
            5 => apply(&mut out, x, |v| if v > 0.0 { 12.0 * v.log2() } else { 0.0 }),
            6 => apply(&mut out, x, |v| v * beats_per_second),
            7 => apply(&mut out, x, |v| v / beats_per_second),
            8 => apply(&mut out, x, |v| v / rate),
            _ => apply(&mut out, x, |v| v * rate),
        }
    }

    fn reset(&mut self) {}
}

#[inline]
fn apply(out: &mut FloatOut, x: FloatIn, f: impl Fn(f64) -> f64) {
    out.fill_with(|i| finite_or_zero(f(f64::from(finite_or_zero(x.at(i)))) as f32));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{NoResources, TempoMap};
    use serde_json::json;

    fn eval(op: &str, x: f32) -> f32 {
        let mut h = Harness::new::<Convert>(json!({}));
        h.set("in", x).set("op", op);
        h.run(1).float("out")[0]
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-4 * b.abs().max(1.0)
    }

    #[test]
    fn level_conversions() {
        assert_eq!(eval("db_to_gain", 0.0), 1.0);
        assert!(close(eval("db_to_gain", -6.0206), 0.5));
        assert!(close(eval("db_to_gain", 20.0), 10.0));
        assert!(close(eval("db_to_gain", -60.0), 0.001));
        assert!(close(eval("gain_to_db", 0.5), -6.0206));
        assert!(close(eval("gain_to_db", -0.5), -6.0206));
        assert_eq!(eval("gain_to_db", 0.0), -120.0);
        assert_eq!(eval("gain_to_db", 1.0), 0.0);
    }

    #[test]
    fn pitch_conversions() {
        assert_eq!(eval("note_to_hz", 69.0), 440.0);
        assert!(close(eval("note_to_hz", 60.0), 261.6256));
        assert!(close(eval("hz_to_note", 440.0), 69.0));
        assert!(close(eval("hz_to_note", 880.0), 81.0));
        assert!(close(eval("hz_to_note", eval("note_to_hz", 47.3)), 47.3));
        assert!(close(eval("semitones_to_ratio", 12.0), 2.0));
        assert!(close(eval("semitones_to_ratio", -12.0), 0.5));
        assert!(close(eval("semitones_to_ratio", 7.0), 1.498_307));
        assert!(close(eval("ratio_to_semitones", 2.0), 12.0));
        assert!(close(eval("ratio_to_semitones", 1.5), 7.019_55));
    }

    #[test]
    fn time_conversions_follow_tempo_and_rate() {
        // 120 BPM: two beats per second.
        assert_eq!(eval("seconds_to_beats", 1.5), 3.0);
        assert_eq!(eval("beats_to_seconds", 3.0), 1.5);
        assert_eq!(eval("seconds_to_samples", 0.5), 24_000.0);
        assert_eq!(eval("samples_to_seconds", 480.0), 0.01);

        let mut h = Harness::with_resources::<Convert>(json!({}), 44_100.0, 64, &NoResources);
        h.tempo(TempoMap::constant(44_100.0, 90.0));
        h.set("in", 1.0).set("op", "beats_to_seconds");
        assert!(close(h.run(1).float("out")[0], 60.0 / 90.0));
        h.set("op", "seconds_to_samples");
        assert_eq!(h.run(1).float("out")[0], 44_100.0);
    }

    #[test]
    fn domain_errors_give_zero() {
        assert_eq!(eval("hz_to_note", 0.0), 0.0);
        assert_eq!(eval("hz_to_note", -100.0), 0.0);
        assert_eq!(eval("ratio_to_semitones", 0.0), 0.0);
        assert_eq!(eval("ratio_to_semitones", -2.0), 0.0);
        assert_eq!(eval("db_to_gain", 1_000.0), 0.0);
        assert_eq!(eval("note_to_hz", 10_000.0), 0.0);
        assert_eq!(eval("seconds_to_samples", 3e38), 0.0);
    }

    #[test]
    fn non_finite_inputs_count_as_zero() {
        let mut h = Harness::new::<Convert>(json!({}));
        h.set_samples("in", vec![f32::NAN, f32::INFINITY, 0.0])
            .set("op", "db_to_gain");
        assert_eq!(h.run(3).float("out"), vec![1.0, 1.0, 1.0]);
    }

    #[test]
    fn rate_follows_the_input() {
        let ramp: Vec<f32> = (0..128).map(|i| i as f32).collect();
        let mut h = Harness::new::<Convert>(json!({}));
        h.set_block("in", ramp.clone())
            .set("op", "seconds_to_samples");
        let out = h.run(128);
        assert_eq!(out.float("out")[70], 64.0 * 48_000.0);

        let mut h = Harness::new::<Convert>(json!({}));
        h.set_samples("in", ramp).set("op", "samples_to_seconds");
        let out = h.run(128);
        assert_eq!(out.float("out")[96], 96.0 / 48_000.0);
    }

    #[test]
    fn output_unit_is_the_default_ops() {
        let sig = Convert::describe(&json!({}));
        assert_eq!(sig.outputs[OUT].unit, Unit::Ratio);
    }
}
