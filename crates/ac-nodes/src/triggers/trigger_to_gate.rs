use ac_core::{Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node};

use super::{time_samples, time_unit, uses_beats};
use crate::segments::Runs;

node! {
    kind: "trigger_to_gate",
    name: "Trigger to Gate",
    category: Triggers,
    keywords: ["gate length", "pulse", "hold", "one shot", "monostable", "note length"],
    /// Turns each trigger into a gate of a set length.
    ///
    /// `gate` goes high on the sample a trigger arrives on `in` and stays
    /// high for `length`, in seconds or, with the `unit` config set to
    /// beats, in beats at the tempo of the moment the trigger arrives. The
    /// gate is always at least one sample long, so a `length` of 0 (or a
    /// negative or NaN one) still shows every trigger.
    ///
    /// A trigger that arrives while the gate is high starts the length
    /// again from that sample, so the gate stays high without dropping in
    /// between: retriggering stretches the gate, it doesn't make a new one.
    /// Wire the trigger itself to anything that should restart on every
    /// hit. Restarting the transport drops the gate.
    ///
    /// **Try this:** wire a `Clock` into `in` and `gate` into an `ADSR` to
    /// turn clock ticks into notes of a fixed length, or set `unit` to beats
    /// and `length` to 0.5 for eighth-note gates at any tempo.
    pub struct TriggerToGate {
        beats: bool,
        /// Samples of gate left.
        remaining: u64,
    }
}

const IN: usize = 0;
const LENGTH: usize = 1;

const GATE: usize = 0;

impl Node for TriggerToGate {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let beats = uses_beats(config);
        let (default, max) = if beats { (0.25, 4.0) } else { (0.1, 2.0) };
        Signature::new()
            .input(SocketSpec::trigger("in").doc("Starts the gate."))
            .input(
                SocketSpec::float("length", time_unit(beats), default)
                    .range(0.0, max)
                    .no_smooth()
                    .doc("How long the gate stays high."),
            )
            .output(SocketSpec::boolean("gate", false).doc("High for `length` after each trigger."))
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        TriggerToGate {
            beats: uses_beats(config),
            remaining: 0,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let length = io.inputs.float(LENGTH);
        let lists = [io.inputs.triggers(IN)];
        let mut gate = io.outputs.float(GATE);
        for run in Runs::new(lists, ctx.frames) {
            if run.fired[0] {
                let len = time_samples(length.at(run.at), self.beats, ctx, run.at);
                self.remaining = len.max(1);
            }
            for i in run.at..run.until {
                let high = self.remaining > 0;
                self.remaining = self.remaining.saturating_sub(1);
                gate.set(i, if high { 1.0 } else { 0.0 });
            }
        }
    }

    fn reset(&mut self) {
        self.remaining = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{TempoMap, Unit};
    use serde_json::json;

    fn harness(config: serde_json::Value, block: usize) -> Harness {
        Harness::with_resources::<TriggerToGate>(config, 48_000.0, block, &ac_core::NoResources)
    }

    /// The (start, end) frames of each high stretch of a gate.
    fn spans(gate: &[f32]) -> Vec<(usize, usize)> {
        let mut v = Vec::new();
        let mut start = None;
        for (i, &g) in gate.iter().chain(std::iter::once(&0.0)).enumerate() {
            match (start, g > 0.5) {
                (None, true) => start = Some(i),
                (Some(s), false) => {
                    v.push((s, i));
                    start = None;
                }
                _ => {}
            }
        }
        v
    }

    #[test]
    fn gates_last_the_length_across_blocks() {
        let mut h = harness(json!({}), 64);
        h.triggers("in", &[10, 1_000]);
        h.set("length", 0.01);
        let out = h.run(2_000);
        assert_eq!(spans(out.float("gate")), vec![(10, 490), (1_000, 1_480)]);
    }

    #[test]
    fn a_retrigger_restarts_the_length() {
        let mut h = harness(json!({}), 64);
        h.triggers("in", &[10, 300]);
        h.set("length", 0.01);
        assert_eq!(spans(h.run(2_000).float("gate")), vec![(10, 780)]);
    }

    #[test]
    fn beats_follow_the_tempo() {
        let mut h = harness(json!({"unit": "beats"}), 64);
        h.tempo(TempoMap::constant(48_000.0, 60.0));
        h.triggers("in", &[5]);
        h.set("length", 0.25);
        assert_eq!(spans(h.run(20_000).float("gate")), vec![(5, 12_005)]);
        let sig = TriggerToGate::describe(&json!({"unit": "beats"}));
        assert_eq!(sig.inputs[LENGTH].unit, Unit::Beats);
    }

    #[test]
    fn zero_or_bad_lengths_give_one_sample() {
        let mut h = harness(json!({}), 64);
        h.triggers("in", &[3, 4, 9]);
        h.set("length", 0.0);
        assert_eq!(spans(h.run(64).float("gate")), vec![(3, 5), (9, 10)]);
        let mut h = harness(json!({}), 64);
        h.triggers("in", &[3]);
        h.set_samples("length", vec![f32::NAN; 64]);
        assert_eq!(spans(h.run(64).float("gate")), vec![(3, 4)]);
        let mut h = harness(json!({}), 64);
        h.triggers("in", &[3]);
        h.set("length", 1.0e30);
        assert!(h.run(256).float("gate")[3..].iter().all(|&g| g == 1.0));
    }

    #[test]
    fn block_size_does_not_matter() {
        let at: Vec<u64> = (0..100).map(|k| k * 211 + (k * 7) % 90).collect();
        let length: Vec<f32> = (0..22_000).map(|i| ((i % 150) as f32) / 48_000.0).collect();
        let run = |block| {
            let mut h = harness(json!({}), block);
            h.triggers("in", &at);
            h.set_samples("length", length.clone());
            h.run(22_000).float("gate").to_vec()
        };
        let a = run(1);
        assert_eq!(a, run(64));
        assert_eq!(a, run(512));
        assert!(spans(&a).len() > 40);
    }

    #[test]
    fn reset_drops_the_gate() {
        let mut h = harness(json!({}), 64);
        h.triggers("in", &[0]);
        h.set("length", 1.0);
        assert!(h.run(64).float("gate").iter().all(|&g| g == 1.0));
        h.reset();
        assert!(h.run(64).float("gate").iter().all(|&g| g == 0.0));
    }
}
