use std::sync::atomic::Ordering;

use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, TelemetryAccess,
    Unit, node,
};

use super::pass_through;

node! {
    kind: "scope",
    name: "Scope",
    category: Display,
    keywords: ["oscilloscope", "waveform"],
    /// An oscilloscope: draws the waveform of a signal over a time `window`,
    /// and passes the signal through unchanged.
    ///
    /// The scope shows the first channel of `in`. Wire a `Float` signal
    /// (an envelope, an LFO) into `in` to watch it too; it arrives as mono
    /// audio. `window` sets how much time the display spans, from 1 ms for a
    /// single cycle up to a couple of seconds for envelopes; longer windows
    /// keep every n-th sample to fit the display.
    ///
    /// A waveform drawn from free-running samples drifts across the screen.
    /// Wire `trigger` (for example from a `Clock`, or the `MidiNote` trigger
    /// that starts each note) and the display lines up on each trigger, so a
    /// repeating shape stands still.
    ///
    /// **Try this:** watch an `ADSR` with `window` at 1 s and `trigger` wired
    /// from the `MidiNote` trigger that starts it, to see the envelope's shape
    /// on every note. Or set `window` to 5 ms and scope a saw `Oscillator`
    /// after a `Biquad` to see the filter round off its corners.
    pub struct Scope {
        /// Position in the decimation cycle; the next sample is kept when 0.
        phase: usize,
        /// Ring position at the last trigger, or -1 before the first.
        trigger_pos: f32,
    }
}

const IN: usize = 0;
const TRIGGER: usize = 1;
const WINDOW: usize = 2;
const OUT: usize = 0;

/// Samples in the scope ring.
const SCOPE_LEN: usize = 1024;

/// Telemetry slots.
const TRIGGER_POS: usize = 0;
const DECIMATION: usize = 1;

/// Longest window honoured, in seconds; only guards against absurd values.
const MAX_WINDOW: f32 = 3600.0;

impl Node for Scope {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(2).scope(SCOPE_LEN)
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to display (first channel)."))
            .input(SocketSpec::trigger("trigger").doc("Lines the display up for a stable picture."))
            .input(
                SocketSpec::float("window", Unit::Seconds, 0.02)
                    .range(0.001, 2.0)
                    .log()
                    .no_smooth()
                    .doc("Time span shown."),
            )
            .output(SocketSpec::audio("out").doc("The same signal, unchanged."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Scope {
            phase: 0,
            trigger_pos: -1.0,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(IN);
        pass_through(&input, &mut io.outputs.audio(OUT));

        let dec = decimation(io.inputs.float(WINDOW).first(), ctx.sample_rate);
        if self.phase >= dec {
            self.phase = 0;
        }
        let samples = input.channel(0);
        let tele = io.telemetry;
        // Only the last trigger in a block matters: the display aligns on it.
        let split = io
            .inputs
            .triggers(TRIGGER)
            .last()
            .and_then(|&t| samples.split_at_checked((t as usize).min(samples.len())));
        match split {
            Some((before, after)) => {
                self.push(&tele, before, dec);
                self.trigger_pos = ring_position(&tele);
                self.push(&tele, after, dec);
            }
            None => self.push(&tele, samples, dec),
        }
        tele.set(TRIGGER_POS, self.trigger_pos);
        tele.set(DECIMATION, dec as f32);
    }

    fn reset(&mut self) {
        self.phase = 0;
        self.trigger_pos = -1.0;
    }
}

impl Scope {
    /// Push every `dec`-th sample, continuing the cycle from earlier blocks.
    fn push(&mut self, tele: &TelemetryAccess, samples: &[f32], dec: usize) {
        let first = (dec - self.phase) % dec;
        tele.push_scope(samples.iter().skip(first).step_by(dec).copied());
        self.phase = (self.phase + samples.len()) % dec;
    }
}

/// Keep one sample in `dec` so `window` seconds fill the ring.
fn decimation(window: f32, sample_rate: f64) -> usize {
    let window = if window.is_finite() {
        window.clamp(0.0, MAX_WINDOW)
    } else {
        0.0
    };
    let per_slot = f64::from(window) * sample_rate / SCOPE_LEN as f64;
    if per_slot.is_finite() {
        (per_slot.ceil() as usize).max(1)
    } else {
        1
    }
}

/// Where the next sample will land in the ring, or -1 if there is no ring.
fn ring_position(tele: &TelemetryAccess) -> f32 {
    let len = tele.scope_len();
    match tele.scope_pos {
        Some(pos) if len > 0 => (pos.load(Ordering::Relaxed) as usize % len) as f32,
        _ => -1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn passes_through_unchanged() {
        let mut h = Harness::new::<Scope>(json!({}));
        let x: Vec<f32> = (0..200).map(|i| (i as f32 * 0.1).sin()).collect();
        h.set_audio("in", vec![x.clone(), x.clone()]);
        let out = h.run(200);
        assert_eq!(out.audio("out")[0], x);
        assert_eq!(out.audio("out")[1], x);
    }

    #[test]
    fn decimation_follows_window_and_rate() {
        assert_eq!(decimation(0.02, 48_000.0), 1);
        assert_eq!(decimation(0.1, 48_000.0), 5);
        assert_eq!(decimation(1.0, 44_100.0), 44);
        assert_eq!(decimation(0.0, 48_000.0), 1);
        assert_eq!(decimation(f32::NAN, 48_000.0), 1);
        assert!(decimation(1e30, 48_000.0) > 1);
    }

    #[test]
    fn short_window_keeps_every_sample() {
        let mut h = Harness::new::<Scope>(json!({}));
        let x: Vec<f32> = (0..1024).map(|i| i as f32).collect();
        h.set_audio("in", vec![x.clone()]);
        h.run(1024);
        assert_eq!(h.scope(), x);
        assert_eq!(h.telemetry(DECIMATION), 1.0);
    }

    #[test]
    fn decimation_continues_across_blocks() {
        // 0.1 s at 48 kHz keeps every 5th sample; 64-frame blocks don't divide by 5.
        let mut h = Harness::new::<Scope>(json!({}));
        h.set("window", 0.1);
        let x: Vec<f32> = (0..5 * 1024).map(|i| i as f32).collect();
        h.set_audio("in", vec![x]);
        h.run(5 * 1024);
        let want: Vec<f32> = (0..1024).map(|i| (i * 5) as f32).collect();
        assert_eq!(h.scope(), want);
        assert_eq!(h.telemetry(DECIMATION), 5.0);
    }

    #[test]
    fn records_ring_position_at_the_last_trigger() {
        let mut h = Harness::new::<Scope>(json!({}));
        h.set_audio("in", vec![vec![0.0; 256]]);
        h.run(64);
        assert_eq!(h.telemetry(TRIGGER_POS), -1.0);
        h.triggers("trigger", &[70, 100]);
        h.run(64);
        assert_eq!(h.telemetry(TRIGGER_POS), 100.0);
        // Stays put through blocks without triggers.
        h.run(64);
        assert_eq!(h.telemetry(TRIGGER_POS), 100.0);
    }

    #[test]
    fn trigger_position_counts_decimated_samples() {
        let mut h = Harness::new::<Scope>(json!({}));
        h.set("window", 0.1); // keep every 5th sample
        h.set_audio("in", vec![vec![0.0; 128]]);
        h.triggers("trigger", &[101]);
        h.run(128);
        // Samples 0, 5, ..., 100 (21 of them) land before the trigger.
        assert_eq!(h.telemetry(TRIGGER_POS), 21.0);
    }

    #[test]
    fn reset_forgets_the_trigger() {
        let mut h = Harness::new::<Scope>(json!({}));
        h.triggers("trigger", &[3]);
        h.run(64);
        assert_eq!(h.telemetry(TRIGGER_POS), 3.0);
        h.reset();
        h.run(64);
        assert_eq!(h.telemetry(TRIGGER_POS), -1.0);
    }
}
