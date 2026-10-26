use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec, Unit, node,
};
use ac_dsp::stft::SlidingWindow;
use ac_dsp::yin::Yin;

use super::pass_through;

node! {
    kind: "tuner",
    name: "Tuner",
    category: Display,
    keywords: ["tune", "pitch", "guitar", "note", "cents", "intonation", "strobe", "frequency"],
    /// A tuner: the note a sound is playing and how many cents sharp or
    /// flat, as a needle on the card.
    ///
    /// The first channel of `in` is analyzed with the YIN method about
    /// twenty times a second, from 47 Hz up (the low E of a bass and every
    /// string of a guitar). `freq` is the pitch in Hz, `note` the nearest
    /// MIDI note, and `cents` how far from it (−50 to 50); they hold while
    /// the sound is unclear, so the needle rests on the last note. `in`
    /// passes through to `out` unchanged.
    ///
    /// The `reference` input sets the A above middle C (default 440 Hz).
    ///
    /// **Try this:** wire an `Audio In` with your guitar into a Tuner, pin
    /// it to a panel, and tune each string until the needle stands in the
    /// middle.
    pub struct Tuner {
        yin: Yin,
        history: SlidingWindow,
        count: usize,
        freq: f32,
        clear: bool,
    }
}

const IN: usize = 0;
const REFERENCE: usize = 1;

const OUT: usize = 0;
const FREQ: usize = 1;
const NOTE: usize = 2;
const CENTS: usize = 3;

/// Samples per analysis, and between analyses.
const WINDOW: usize = 2_048;
const HOP: usize = 2_048;
/// Confidence a reading needs.
const THRESHOLD: f64 = 0.85;

/// Telemetry: the pitch (Hz), the nearest note, its cents, and whether the
/// latest reading was clear (1) or not (0).
pub const TELEMETRY_FREQ: usize = 0;
pub const TELEMETRY_NOTE: usize = 1;
pub const TELEMETRY_CENTS: usize = 2;
pub const TELEMETRY_CLEAR: usize = 3;

/// The nearest note to `freq` and the cents from it, with A4 at `reference`.
pub fn note_and_cents(freq: f32, reference: f32) -> (i32, f32) {
    if !(freq > 0.0 && freq.is_finite()) {
        return (0, 0.0);
    }
    let reference = if reference > 0.0 && reference.is_finite() {
        reference
    } else {
        440.0
    };
    let exact = 69.0 + 12.0 * (f64::from(freq) / f64::from(reference)).log2();
    let note = exact.round().clamp(0.0, 127.0);
    (
        note as i32,
        ((exact - note) * 100.0).clamp(-50.0, 50.0) as f32,
    )
}

impl Node for Tuner {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(4)
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").doc("The sound to tune (first channel)."))
            .input(
                SocketSpec::float("reference", Unit::Hz, 440.0)
                    .range(415.0, 466.0)
                    .doc("The pitch of A above middle C."),
            )
            .output(SocketSpec::audio("out").doc("`in`, unchanged."))
            .output(
                SocketSpec::float("freq", Unit::Hz, 0.0)
                    .rate(Rate::Block)
                    .doc("The pitch; holds while unclear."),
            )
            .output(SocketSpec::int("note", 0).doc("The nearest MIDI note."))
            .output(
                SocketSpec::float("cents", Unit::Generic, 0.0)
                    .rate(Rate::Block)
                    .doc("How far from the note, −50 to 50."),
            )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Tuner {
            yin: Yin::new(WINDOW),
            history: SlidingWindow::new(WINDOW),
            count: 0,
            freq: 0.0,
            clear: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(IN);
        for &s in input.channel(0) {
            self.history.push(if s.is_finite() { s } else { 0.0 });
            self.count += 1;
            if self.count >= HOP {
                self.count = 0;
                let rate = ctx.sample_rate;
                let lo = self.yin.min_freq(rate);
                let p =
                    self.yin
                        .analyze(self.history.latest(), rate, lo, rate / 4.0, 1.0 - THRESHOLD);
                self.clear = p.freq > 0.0 && p.confidence >= THRESHOLD as f32;
                if self.clear {
                    self.freq = p.freq as f32;
                }
            }
        }
        let mut out = io.outputs.audio(OUT);
        pass_through(&input, &mut out);
        let (note, cents) = note_and_cents(self.freq, io.inputs.float(REFERENCE).first());
        io.outputs.set_float(FREQ, self.freq);
        io.outputs.set_int(NOTE, note);
        io.outputs.set_float(CENTS, cents);
        io.telemetry.set(TELEMETRY_FREQ, self.freq);
        io.telemetry.set(TELEMETRY_NOTE, note as f32);
        io.telemetry.set(TELEMETRY_CENTS, cents);
        io.telemetry
            .set(TELEMETRY_CLEAR, if self.clear { 1.0 } else { 0.0 });
    }

    fn reset(&mut self) {
        self.history.clear();
        self.count = 0;
        self.freq = 0.0;
        self.clear = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn sine(freq: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (std::f64::consts::TAU * freq * i as f64 / 48_000.0).sin() as f32 * 0.5)
            .collect()
    }

    #[test]
    fn notes_and_cents_from_a_pitch() {
        assert_eq!(note_and_cents(440.0, 440.0), (69, 0.0));
        let (n, c) = note_and_cents(445.0, 440.0);
        assert_eq!(n, 69);
        assert!((c - 19.56).abs() < 0.1, "{c}");
        assert_eq!(note_and_cents(0.0, 440.0), (0, 0.0));
        assert_eq!(note_and_cents(442.0, 442.0).1, 0.0);
    }

    #[test]
    fn a_low_e_string_reads_as_e2_and_passes_through() {
        // E2 is 82.41 Hz, a little sharp here.
        let x = sine(83.0, 12_000);
        let mut h = Harness::new::<Tuner>(json!({}));
        h.set_audio("in", vec![x.clone()]);
        let out = h.run(12_000);
        assert_eq!(out.audio("out")[0], x);
        assert_eq!(*out.int("note").last().unwrap_or(&0), 40);
        let cents = *out.float("cents").last().unwrap_or(&0.0);
        assert!((cents - 12.3).abs() < 3.0, "{cents}");
        assert_eq!(h.telemetry(TELEMETRY_CLEAR), 1.0);
    }
}
