use ac_core::{Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node};

use super::frames_of;

node! {
    kind: "clock_multiply",
    name: "Clock Multiply",
    category: Triggers,
    keywords: ["multiplier", "subdivide", "ratchet", "faster", "double time", "interpolate"],
    /// Fills the gap between incoming triggers with evenly spaced ones.
    ///
    /// Each trigger on `in` passes straight to `out`, and the time since the
    /// one before it is split into `multiply` equal parts: with `multiply`
    /// at 4 and triggers a beat apart, `out` fires on every sixteenth note.
    /// The extra triggers land on exact samples (rounded to the nearest),
    /// across block boundaries, and follow the input when it speeds up or
    /// slows down from one trigger to the next.
    ///
    /// The node has to hear two triggers before it knows the spacing, so
    /// the first trigger after loading (or a transport restart) passes alone
    /// and the extra triggers start with the second. Each new trigger starts
    /// a new set of extra triggers spaced by the time since the last one; if
    /// it comes early, the old set's remaining triggers are dropped. When
    /// the input stops, `out` stops after one more interval. A new
    /// `multiply` (1–64) takes effect at the next trigger on `in`. Triggers
    /// that would fall on the same sample (an interval shorter than
    /// `multiply` samples) come out once.
    ///
    /// **Try this:** wire a `Clock` at one beat into `in` and set `multiply`
    /// to 3 for triplets, or tap a `Button` into `in` and let the multiplier
    /// follow your tempo.
    pub struct ClockMultiply {
        /// The node's own clock: frames since loading or reset, at the
        /// start of the block.
        now: u64,
        /// When the last trigger came in.
        last_in: Option<u64>,
        /// The last trigger in, which the extra triggers count from.
        base: u64,
        /// Samples between the last two triggers in.
        interval: u64,
        /// `multiply` as it was at the last trigger in.
        mult: u64,
        /// The next extra trigger to emit (1..mult); `mult` when none.
        next: u64,
        /// The last trigger out, so none fires twice on one sample.
        last_out: Option<u64>,
    }
}

const IN: usize = 0;
const MULTIPLY: usize = 1;

const OUT: usize = 0;

const MAX_MULTIPLY: i32 = 64;

impl ClockMultiply {
    /// When the next extra trigger is due, if any.
    fn next_extra(&self) -> Option<u64> {
        if self.next >= self.mult || self.interval == 0 {
            return None;
        }
        let part = (u128::from(self.next) * u128::from(self.interval) + u128::from(self.mult / 2))
            / u128::from(self.mult);
        Some(
            self.base
                .saturating_add(u64::try_from(part).unwrap_or(u64::MAX)),
        )
    }

    fn arrive(&mut self, t: u64, multiply: u64) {
        match self.last_in {
            Some(last) if t > last => {
                self.interval = t - last;
                self.base = t;
                self.mult = multiply;
                self.next = 1;
            }
            _ => self.next = self.mult,
        }
        self.last_in = Some(t);
    }
}

impl Node for ClockMultiply {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::trigger("in").doc("The triggers to multiply."))
            .input(
                SocketSpec::int("multiply", 2)
                    .range(1.0, 16.0)
                    .doc("Triggers out per trigger in (1–64)."),
            )
            .output(SocketSpec::trigger("out").doc("The input and the triggers between."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        ClockMultiply {
            now: 0,
            last_in: None,
            base: 0,
            interval: 0,
            mult: 1,
            next: 1,
            last_out: None,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let start = self.now;
        let end = start.saturating_add(ctx.frames as u64);
        let multiply = io.inputs.int(MULTIPLY).clamp(1, MAX_MULTIPLY) as u64;
        let mut ins = frames_of(io.inputs.triggers(IN), ctx.frames)
            .map(|o| start + o)
            .peekable();
        let mut out = io.outputs.trigger(OUT);
        let mut emit = |t: u64, last_out: &mut Option<u64>| {
            if last_out.is_none_or(|l| t > l) {
                out.push((t - start) as u32);
                *last_out = Some(t);
            }
        };
        loop {
            let extra = self.next_extra();
            match (ins.peek().copied(), extra) {
                // A trigger in on the same sample as an extra one replaces it.
                (Some(t), e) if e.is_none_or(|e| t <= e) => {
                    ins.next();
                    self.arrive(t, multiply);
                    emit(t, &mut self.last_out);
                }
                (_, Some(e)) if e < end => {
                    self.next += 1;
                    emit(e, &mut self.last_out);
                }
                _ => break,
            }
        }
        self.now = end;
    }

    fn reset(&mut self) {
        self.now = 0;
        self.last_in = None;
        self.base = 0;
        self.interval = 0;
        self.mult = 1;
        self.next = 1;
        self.last_out = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn run(block: usize, at: &[u64], multiply: i32, frames: usize) -> Vec<u64> {
        let mut h = Harness::with_resources::<ClockMultiply>(
            json!({}),
            48_000.0,
            block,
            &ac_core::NoResources,
        );
        h.triggers("in", at);
        h.set("multiply", multiply);
        h.run(frames).triggers("out").to_vec()
    }

    #[test]
    fn splits_each_interval_evenly_after_the_second_trigger() {
        let out = run(64, &[100, 500, 900], 4, 1_500);
        assert_eq!(out, &[100, 500, 600, 700, 800, 900, 1_000, 1_100, 1_200]);
    }

    #[test]
    fn follows_tempo_changes_and_rounds_to_the_nearest_sample() {
        // Intervals of 300 then 100 samples, split in 3 (33.3 rounds to 33).
        let out = run(64, &[0, 300, 400], 3, 600);
        assert_eq!(out, &[0, 300, 400, 433, 467]);
    }

    #[test]
    fn an_early_trigger_drops_the_rest_of_the_set() {
        // The set after 1000 expects 1250, 1500, 1750; the trigger at 1300
        // replaces them with a new set spaced 300 / 4.
        let out = run(64, &[0, 1_000, 1_300], 4, 2_000);
        assert_eq!(out, &[0, 1_000, 1_250, 1_300, 1_375, 1_450, 1_525]);
    }

    #[test]
    fn block_size_does_not_matter() {
        let at: Vec<u64> = (0..40).map(|k| k * 997 + (k * k) % 300).collect();
        let a = run(1, &at, 7, 42_000);
        assert_eq!(a, run(64, &at, 7, 42_000));
        assert_eq!(a, run(512, &at, 7, 42_000));
        assert!(a.len() > 200);
    }

    #[test]
    fn extremes() {
        // Multiply 1 passes the input through.
        assert_eq!(run(64, &[3, 50, 90], 1, 200), &[3, 50, 90]);
        // Out-of-range multiply clamps to 1..64.
        assert_eq!(run(64, &[3, 50, 90], -5, 200), &[3, 50, 90]);
        let out = run(64, &[0, 6_400], 1_000, 13_000);
        assert_eq!(out.len(), 65);
        assert_eq!(out[1..4], [6_400, 6_500, 6_600]);
        // An interval shorter than `multiply` samples fires once per sample.
        assert_eq!(run(64, &[0, 3], 8, 64), &[0, 3, 4, 5, 6]);
        // Repeats on one sample count once.
        assert_eq!(run(64, &[10, 10, 20], 2, 64), &[10, 20, 25]);
    }

    #[test]
    fn multiply_changes_at_the_next_trigger() {
        let mut h = Harness::new::<ClockMultiply>(json!({}));
        h.triggers("in", &[0, 64, 128]);
        h.set("multiply", 2);
        assert_eq!(h.run(64).triggers("out"), &[0]);
        h.set("multiply", 4);
        // The set from 64 uses 4; at 128 the input repeats it.
        assert_eq!(
            h.run(128).triggers("out"),
            &[64, 80, 96, 112, 128, 144, 160, 176]
        );
    }

    #[test]
    fn reset_forgets_the_interval() {
        let mut h = Harness::new::<ClockMultiply>(json!({}));
        h.triggers("in", &[0, 32, 70]);
        h.set("multiply", 2);
        assert_eq!(h.run(64).triggers("out"), &[0, 32, 48]);
        h.reset();
        // 70 is the first trigger after the reset: it passes alone.
        assert_eq!(h.run(128).triggers("out"), &[70]);
    }
}
