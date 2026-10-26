use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};

node! {
    kind: "tempo",
    name: "Tempo",
    category: Generators,
    keywords: ["bpm", "tap tempo", "tap", "speed", "tempo map", "beats per minute", "clock"],
    /// Sets the song's tempo: turn `bpm`, or tap the tempo on `tap`.
    ///
    /// The Tempo node doesn't put out a signal: the editor changes the
    /// project's tempo map, so every clock, pattern, and placed clip
    /// follows, and the saved project renders at the new tempo. A change to
    /// `bpm` (typed, turned, or arriving on a wire) sets the tempo where the
    /// playhead is; the value it has when the project opens changes
    /// nothing. Each `tap` after the first, within two seconds of the one
    /// before, sets the tempo from the average of the last few gaps between
    /// taps, and `bpm` shows it. Tempos are kept between 20 and 300 BPM.
    /// Each change is an undo step on the Project tab's tempo. Outside the
    /// editor (a render on its own, `audioc run`) nothing applies them.
    ///
    /// **Try this:** wire a `Button` (or a gamepad button bound to one) into
    /// `tap` and tap along with a record playing: the song's clocks lock to
    /// your tempo.
    pub struct Tempo {
        /// The `bpm` input as last seen, NaN before the first block.
        seen: f32,
        /// Sample times of the last taps, newest last.
        taps: [u64; TAPS],
        tapped: usize,
        /// The node's own sample clock.
        now: u64,
        /// The tempo asked for, and how many times one has been asked for.
        asked: f32,
        serial: u32,
    }
}

const BPM: usize = 0;
const TAP: usize = 1;

/// Taps averaged.
const TAPS: usize = 4;
/// Longest gap between taps that still counts, in seconds.
const TAP_GAP: f64 = 2.0;
pub const MIN_BPM: f64 = 20.0;
pub const MAX_BPM: f64 = 300.0;

/// Telemetry: the tempo asked for (BPM), and a count that goes up with each
/// new request, for the editor to apply.
pub const TELEMETRY_BPM: usize = 0;
pub const TELEMETRY_SERIAL: usize = 1;

impl Tempo {
    fn ask(&mut self, bpm: f64) {
        if bpm.is_finite() {
            self.asked = bpm.clamp(MIN_BPM, MAX_BPM) as f32;
            self.serial = self.serial.wrapping_add(1);
        }
    }

    /// A tap at sample `at`: once there are two in reach, the tempo.
    fn tap(&mut self, at: u64, rate: f64) {
        let gap = (TAP_GAP * rate) as u64;
        if self.tapped > 0 && at.saturating_sub(self.taps[TAPS - 1]) > gap {
            self.tapped = 0;
        }
        self.taps.rotate_left(1);
        self.taps[TAPS - 1] = at;
        self.tapped = (self.tapped + 1).min(TAPS);
        if self.tapped >= 2 {
            let first = self.taps[TAPS - self.tapped];
            let gaps = (self.tapped - 1) as f64;
            let mean = (at - first) as f64 / gaps;
            if mean > 0.0 {
                self.ask(60.0 * rate / mean);
            }
        }
    }
}

impl Node for Tempo {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(2)
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(
                SocketSpec::float("bpm", Unit::Bpm, 120.0)
                    .range(MIN_BPM, MAX_BPM)
                    .no_smooth()
                    .doc("The tempo; a change sets the song's tempo."),
            )
            .input(SocketSpec::trigger("tap").doc("Tap the tempo, two taps or more."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Tempo {
            seen: f32::NAN,
            taps: [0; TAPS],
            tapped: 0,
            now: 0,
            asked: 0.0,
            serial: 0,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let bpm = io.inputs.float(BPM).first();
        if bpm.is_finite() {
            if !self.seen.is_nan() && (bpm - self.seen).abs() > 1e-3 {
                self.ask(f64::from(bpm));
            }
            self.seen = bpm;
        }
        let rate = if ctx.sample_rate > 0.0 {
            ctx.sample_rate
        } else {
            48_000.0
        };
        for &t in io.inputs.triggers(TAP) {
            self.tap(self.now + u64::from(t), rate);
        }
        self.now = self.now.saturating_add(ctx.frames as u64);
        io.telemetry.set(TELEMETRY_BPM, self.asked);
        io.telemetry.set(TELEMETRY_SERIAL, self.serial as f32);
    }

    /// Taps and requests outlive a restart: they're the person's.
    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn taps_ask_for_their_tempo() {
        let mut h = Harness::new::<Tempo>(json!({}));
        // Taps every half second at 48 kHz: 120 BPM.
        h.triggers("tap", &[0, 24_000, 48_000, 72_000]);
        h.run(16);
        assert_eq!(h.telemetry(TELEMETRY_SERIAL), 0.0, "one tap is no tempo");
        h.run(80_000);
        assert_eq!(h.telemetry(TELEMETRY_SERIAL), 3.0);
        assert!((h.telemetry(TELEMETRY_BPM) - 120.0).abs() < 0.01);
        // A long pause starts over.
        let t = 80_016 + 200_000;
        h.triggers("tap", &[t, t + 32_000]);
        h.run(200_000 + 40_000);
        assert!((h.telemetry(TELEMETRY_BPM) - 90.0).abs() < 0.01);
    }

    #[test]
    fn a_change_to_bpm_asks_but_the_first_value_doesnt() {
        let mut h = Harness::new::<Tempo>(json!({}));
        h.set("bpm", 128.0);
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_SERIAL), 0.0);
        h.set("bpm", 140.0);
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_SERIAL), 1.0);
        assert_eq!(h.telemetry(TELEMETRY_BPM), 140.0);
        h.set("bpm", 1_000.0);
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_BPM), 300.0, "kept in range");
    }
}
