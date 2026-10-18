use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec,
    Unit, node,
};
use ac_dsp::stft::{SlidingWindow, pow2_size};
use ac_dsp::yin::Yin;

use super::finite_or;

node! {
    kind: "pitch_detect",
    name: "Pitch Detect",
    category: Spectral,
    keywords: ["pitch", "tracker", "tuner", "frequency", "yin", "f0", "guitar"],
    /// Follows the pitch of a monophonic sound: frequency, nearest note,
    /// how sure it is, and a trigger when a new note starts.
    ///
    /// Every `hop` samples the newest `window` samples of the first channel
    /// of `in` are analyzed with the YIN method, which compares the newest
    /// half of the window with delayed copies of itself to find its period.
    ///
    /// - `freq` (Hz) is the detected fundamental. It holds its last value
    ///   while the sound is unclear (confidence below `threshold`) and is 0
    ///   until the first pitch is found.
    /// - `confidence` (0..1) is how periodic the latest analysis was: near
    ///   1 for a clean tone, near 0 for noise or silence.
    /// - `note` is the MIDI note nearest to `freq` (0 before any pitch).
    /// - `onset` fires when a new stable pitch starts: two analyses in a row
    ///   agree within half a semitone, after a pause or more than three
    ///   quarters of a semitone away from the last note it fired for.
    ///
    /// `min_freq` and `max_freq` limit the search (a narrow range avoids
    /// octave mistakes); the lowest pitch a window can measure is about
    /// `2 × sample_rate / window` (47 Hz with the defaults at 48 kHz), and
    /// `max_freq` is kept below a quarter of the sample rate. If they are
    /// reversed they are swapped. `threshold` is the confidence a reading
    /// needs to count; lower it for breathy or distorted sources.
    ///
    /// Config:
    /// - `window`: samples per analysis, a power of two from 256 to 16384
    ///   (default 2048). Bigger windows reach lower notes and react slower.
    /// - `hop`: samples between analyses, 16 up to `window` (default 256).
    ///
    /// The outputs update once per block from the latest analysis. A note
    /// after a pause is recognized once a few of its periods have arrived
    /// (sooner for higher notes); a change straight from one note to
    /// another takes about half a window, until the old note has left the
    /// analysis; `onset` then needs one more hop to confirm. Readings
    /// describe the newest half of the window, centered a quarter window
    /// back, so the node reports `window / 4 + hop` samples of latency (768
    /// with the defaults) and the engine delays a dry path by that much to
    /// keep `onset` close to the sound. NaN or infinite samples count as
    /// silence.
    ///
    /// **Try this:** play a guitar or a voice into PitchDetect, wire `freq`
    /// to an `Oscillator` and `onset` to an `ADSR` on its gain: a synth that
    /// follows you. Use `confidence` through a `Compare` to mute it when
    /// nothing clear is playing.
    pub struct PitchDetect {
        yin: Yin,
        history: SlidingWindow,
        hop: usize,
        count: usize,
        freq: f32,
        confidence: f32,
        prev_voiced: bool,
        prev_freq: f64,
        in_note: bool,
        note_freq: f64,
    }
}

const IN: usize = 0;
const MIN_FREQ: usize = 1;
const MAX_FREQ: usize = 2;
const THRESHOLD: usize = 3;

const FREQ: usize = 0;
const CONFIDENCE: usize = 1;
const NOTE: usize = 2;
const ONSET: usize = 3;

/// Steady: consecutive readings within this many semitones.
const STEADY: f64 = 0.5;
/// A new note: this many semitones from the last one fired.
const NEW_NOTE: f64 = 0.75;

fn window_size(config: &Config) -> usize {
    pow2_size(config.get_f64_or("window", 2048.0), 256, 16_384, 2048)
}

fn hop_size(config: &Config, window: usize) -> usize {
    let h = config.get_f64_or("hop", 256.0);
    if h.is_finite() {
        (h.round() as usize).clamp(16, window)
    } else {
        256.min(window)
    }
}

fn semitones(a: f64, b: f64) -> f64 {
    12.0 * (a / b).log2()
}

/// Nearest MIDI note to `freq` (0 for no pitch), within 0..=127.
fn midi_note(freq: f32) -> i32 {
    if freq > 0.0 && freq.is_finite() {
        (69.0 + 12.0 * (f64::from(freq) / 440.0).log2())
            .round()
            .clamp(0.0, 127.0) as i32
    } else {
        0
    }
}

impl PitchDetect {
    /// Analyze the window ending now; returns true if a new note starts.
    fn analyze(&mut self, rate: f64, min_freq: f32, max_freq: f32, threshold: f32) -> bool {
        let floor = self.yin.min_freq(rate);
        let ceiling = rate / 4.0;
        let (mut lo, mut hi) = (
            f64::from(finite_or(min_freq, 60.0)),
            f64::from(finite_or(max_freq, 1500.0)),
        );
        if lo > hi {
            std::mem::swap(&mut lo, &mut hi);
        }
        let lo = lo.clamp(floor, ceiling);
        let hi = hi.clamp(lo, ceiling);
        let threshold = finite_or(threshold, 0.8).clamp(0.0, 1.0);
        let p = self.yin.analyze(
            self.history.latest(),
            rate,
            lo,
            hi,
            f64::from(1.0 - threshold),
        );
        self.confidence = p.confidence;
        let voiced = p.freq > 0.0 && p.confidence >= threshold && p.confidence > 0.0;
        let mut onset = false;
        if voiced {
            self.freq = p.freq as f32;
            let steady = self.prev_voiced && semitones(p.freq, self.prev_freq).abs() < STEADY;
            if steady && (!self.in_note || semitones(p.freq, self.note_freq).abs() > NEW_NOTE) {
                onset = true;
                self.in_note = true;
                self.note_freq = p.freq;
            }
            self.prev_freq = p.freq;
        } else {
            self.in_note = false;
        }
        self.prev_voiced = voiced;
        onset
    }
}

impl Node for PitchDetect {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").doc("Monophonic sound to follow (first channel)."))
            .input(
                SocketSpec::float("min_freq", Unit::Hz, 60.0)
                    .range(20.0, 1000.0)
                    .log()
                    .doc("Lowest pitch to look for."),
            )
            .input(
                SocketSpec::float("max_freq", Unit::Hz, 1500.0)
                    .range(100.0, 5000.0)
                    .log()
                    .doc("Highest pitch to look for."),
            )
            .input(
                SocketSpec::float("threshold", Unit::Generic, 0.8)
                    .range(0.0, 1.0)
                    .doc("Confidence a reading needs to count."),
            )
            .output(
                SocketSpec::float("freq", Unit::Hz, 0.0)
                    .rate(Rate::Block)
                    .doc("Detected pitch; holds while unclear."),
            )
            .output(
                SocketSpec::float("confidence", Unit::Generic, 0.0)
                    .rate(Rate::Block)
                    .doc("How clear the latest reading was, 0..1."),
            )
            .output(SocketSpec::int("note", 0).doc("Nearest MIDI note."))
            .output(SocketSpec::trigger("onset").doc("Fires when a new stable pitch starts."))
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        let window = window_size(config);
        PitchDetect {
            yin: Yin::new(window),
            history: SlidingWindow::new(window),
            hop: hop_size(config, window),
            count: 0,
            freq: 0.0,
            confidence: 0.0,
            prev_voiced: false,
            prev_freq: 0.0,
            in_note: false,
            note_freq: 0.0,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(IN);
        let x = input.channel(0);
        let min_freq = io.inputs.float(MIN_FREQ);
        let max_freq = io.inputs.float(MAX_FREQ);
        let threshold = io.inputs.float(THRESHOLD);
        for (i, &s) in x.iter().enumerate() {
            self.history.push(if s.is_finite() { s } else { 0.0 });
            self.count += 1;
            if self.count < self.hop {
                continue;
            }
            self.count = 0;
            let onset = self.analyze(
                ctx.sample_rate,
                min_freq.at(i),
                max_freq.at(i),
                threshold.at(i),
            );
            if onset {
                io.outputs.trigger(ONSET).push(i as u32);
            }
        }
        io.outputs.set_float(FREQ, self.freq);
        io.outputs.set_float(CONFIDENCE, self.confidence);
        io.outputs.set_int(NOTE, midi_note(self.freq));
    }

    fn latency(&self) -> u32 {
        (self.yin.size() / 4 + self.hop) as u32
    }

    fn reset(&mut self) {
        self.history.clear();
        self.count = 0;
        self.freq = 0.0;
        self.confidence = 0.0;
        self.prev_voiced = false;
        self.prev_freq = 0.0;
        self.in_note = false;
        self.note_freq = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::{Harness, Output};
    use serde_json::json;

    const RATE: f64 = 48_000.0;

    fn cents(a: f64, b: f64) -> f64 {
        1200.0 * (a / b).log2()
    }

    fn sine(freq: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (0.5 * (std::f64::consts::TAU * freq * i as f64 / RATE).sin()) as f32)
            .collect()
    }

    /// A band-limited sawtooth: harmonics up to 20 kHz.
    fn saw(freq: f64, n: usize) -> Vec<f32> {
        let harmonics = (20_000.0 / freq) as usize;
        (0..n)
            .map(|i| {
                let t = std::f64::consts::TAU * freq * i as f64 / RATE;
                let s: f64 = (1..=harmonics)
                    .map(|k| (k as f64 * t).sin() / k as f64)
                    .sum();
                (0.3 * s) as f32
            })
            .collect()
    }

    fn noise(n: usize) -> Vec<f32> {
        let mut s = 99u32;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                s as f32 / u32::MAX as f32 - 0.5
            })
            .collect()
    }

    fn detect(x: &[f32], block: usize) -> Output {
        let mut h = Harness::with_resources::<PitchDetect>(json!({}), RATE, block, &NoResources);
        h.set_audio("in", vec![x.to_vec()]);
        h.run(x.len())
    }

    #[test]
    fn sines_and_saws_are_found_within_a_few_cents() {
        for f in [65.41, 110.0, 261.63, 440.0, 987.77, 1400.0] {
            for x in [sine(f, 9600), saw(f, 9600)] {
                let out = detect(&x, 64);
                let freq = f64::from(*out.float("freq").last().unwrap());
                assert!(cents(freq, f).abs() < 5.0, "{f}: {freq}");
                assert!(*out.float("confidence").last().unwrap() > 0.9);
                let note = (69.0 + 12.0 * (f / 440.0).log2()).round() as i32;
                assert_eq!(*out.int("note").last().unwrap(), note);
            }
        }
    }

    #[test]
    fn noise_has_low_confidence_and_holds_the_last_pitch() {
        let out = detect(&noise(9600), 64);
        assert!(out.float("confidence").iter().all(|c| *c < 0.5));
        assert_eq!(*out.float("freq").last().unwrap(), 0.0);
        assert_eq!(*out.int("note").last().unwrap(), 0);
        assert!(out.triggers("onset").is_empty());
        // A tone, then noise: the tone's pitch holds.
        let mut x = sine(330.0, 9600);
        x.extend(noise(9600));
        let out = detect(&x, 64);
        assert!(*out.float("confidence").last().unwrap() < 0.5);
        assert!(cents(f64::from(*out.float("freq").last().unwrap()), 330.0).abs() < 5.0);
    }

    #[test]
    fn onsets_fire_once_per_new_note_near_the_reported_latency() {
        // Silence, A3 for 0.25 s, E4 for 0.25 s, silence, A3 again.
        let quarter = 12_000;
        let mut x = vec![0.0; quarter];
        x.extend(saw(220.0, quarter));
        x.extend(saw(329.63, quarter));
        x.extend(vec![0.0; quarter]);
        x.extend(saw(220.0, quarter));
        let out = detect(&x, 64);
        let onsets = out.triggers("onset");
        assert_eq!(onsets.len(), 3, "{onsets:?}");
        // After silence: a few periods plus a hop, near the latency.
        let latency = (2048 / 4 + 256) as i64;
        let late = |k: usize, start: usize| onsets[k] as i64 - start as i64;
        for (k, start) in [(0, quarter), (2, 4 * quarter)] {
            assert!(
                (late(k, start) - latency).abs() <= 300,
                "{}",
                late(k, start)
            );
        }
        // Straight from one note to the next: about half a window, once the
        // old note has left the analysis.
        let change = late(1, 2 * quarter);
        assert!((latency..=1024 + 2 * 256).contains(&change), "{change}");
    }

    #[test]
    fn range_and_threshold_are_sanitized() {
        let x = sine(440.0, 9600);
        let mut h = Harness::new::<PitchDetect>(json!({}));
        h.set_audio("in", vec![x.clone()])
            .set("min_freq", 2000.0)
            .set("max_freq", 300.0)
            .set("threshold", f64::NAN);
        let out = h.run(9600);
        assert!(cents(f64::from(*out.float("freq").last().unwrap()), 440.0).abs() < 5.0);
        // A range above the tone finds nothing clear.
        let mut h = Harness::new::<PitchDetect>(json!({}));
        h.set_audio("in", vec![x])
            .set("min_freq", 1000.0)
            .set("max_freq", 4000.0);
        assert_eq!(*h.run(9600).float("freq").last().unwrap(), 0.0);
    }

    #[test]
    fn config_sets_window_hop_and_latency() {
        let ctx = PrepareCtx {
            sample_rate: RATE,
            max_block: 64,
            seed: 0,
            node_key: "t",
            name: "t",
            resources: &NoResources,
        };
        assert_eq!(PitchDetect::new(&json!({}), &ctx).latency(), 768);
        let p = PitchDetect::new(&json!({ "window": 5000, "hop": 1e9 }), &ctx);
        assert_eq!((p.yin.size(), p.hop), (4096, 4096));
        let p = PitchDetect::new(&json!({ "window": 1, "hop": -4 }), &ctx);
        assert_eq!((p.yin.size(), p.hop), (256, 16));
    }

    #[test]
    fn block_size_only_changes_when_block_outputs_update() {
        let mut x = saw(196.0, 6000);
        x.extend(saw(293.66, 6000));
        let one = detect(&x, 1);
        for block in [64, 512] {
            let other = detect(&x, block);
            assert_eq!(one.triggers("onset"), other.triggers("onset"));
            // At the end of every 512-frame block all sizes agree.
            for t in (511..x.len()).step_by(512) {
                assert_eq!(one.float("freq")[t], other.float("freq")[t]);
                assert_eq!(one.float("confidence")[t], other.float("confidence")[t]);
                assert_eq!(one.int("note")[t], other.int("note")[t]);
            }
        }
    }

    #[test]
    fn nan_input_and_reset() {
        let mut x = sine(440.0, 9600);
        x[3000] = f32::NAN;
        x[3001] = f32::INFINITY;
        let mut h = Harness::new::<PitchDetect>(json!({}));
        h.set_audio("in", vec![x]);
        let out = h.run(9600);
        assert!(out.float("freq").iter().all(|v| v.is_finite()));
        assert!(out.float("confidence").iter().all(|v| v.is_finite()));
        h.reset();
        let out = h.run(64);
        assert_eq!(out.float("freq")[0], 0.0);
    }
}
