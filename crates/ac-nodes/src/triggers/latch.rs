use ac_core::{Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node};

use crate::segments::Runs;

node! {
    kind: "latch",
    name: "Latch",
    category: Triggers,
    keywords: ["flip flop", "set reset", "toggle", "memory", "hold", "sr latch", "switch"],
    /// Remembers an on/off state that triggers set, reset, and flip.
    ///
    /// A trigger on `set` turns `out` on, `reset` turns it off, and
    /// `toggle` flips it. Each change lands on the trigger's exact sample.
    /// Triggers on the same sample apply in the order reset, set, toggle:
    /// `set` and `reset` together leave it on, and all three together leave
    /// it off. Several triggers on one input on the same sample count once.
    ///
    /// `initial` is the state after loading and after a transport restart;
    /// changing it later doesn't move the latch.
    ///
    /// **Try this:** wire one `Button` into `set` and another into `reset`
    /// for start and stop pads, or wire a `Threshold Trigger` into `toggle`
    /// so each loud hit switches an effect in or out.
    pub struct Latch {
        /// `None` until the first block after loading or reset, when it
        /// takes `initial`.
        state: Option<bool>,
    }
}

const SET: usize = 0;
const RESET: usize = 1;
const TOGGLE: usize = 2;
const INITIAL: usize = 3;

const OUT: usize = 0;

impl Node for Latch {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::trigger("set").doc("Turns the latch on."))
            .input(SocketSpec::trigger("reset").doc("Turns the latch off."))
            .input(SocketSpec::trigger("toggle").doc("Flips the latch."))
            .input(
                SocketSpec::boolean("initial", false)
                    .doc("The state after loading and after a transport restart."),
            )
            .output(SocketSpec::boolean("out", false).doc("The latch's state."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Latch { state: None }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let mut state = self
            .state
            .unwrap_or_else(|| io.inputs.boolean(INITIAL).first());
        let lists = [
            io.inputs.triggers(RESET),
            io.inputs.triggers(SET),
            io.inputs.triggers(TOGGLE),
        ];
        let mut out = io.outputs.float(OUT);
        for run in Runs::new(lists, ctx.frames) {
            let [reset, set, toggle] = run.fired;
            if reset {
                state = false;
            }
            if set {
                state = true;
            }
            if toggle {
                state = !state;
            }
            let v = if state { 1.0 } else { 0.0 };
            for i in run.at..run.until {
                out.set(i, v);
            }
        }
        self.state = Some(state);
    }

    fn reset(&mut self) {
        self.state = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn harness(block: usize) -> Harness {
        Harness::with_resources::<Latch>(json!({}), 48_000.0, block, &ac_core::NoResources)
    }

    /// Frames where the output changes, with the new state.
    fn changes(out: &[f32]) -> Vec<(usize, bool)> {
        let mut prev = 0.0;
        let mut v = Vec::new();
        for (i, &x) in out.iter().enumerate() {
            if x != prev {
                v.push((i, x > 0.5));
                prev = x;
            }
        }
        v
    }

    #[test]
    fn set_reset_and_toggle_land_on_their_samples() {
        let mut h = harness(64);
        h.triggers("set", &[10, 20]);
        h.triggers("reset", &[100]);
        h.triggers("toggle", &[150, 200]);
        let out = h.run(256);
        assert_eq!(
            changes(out.float("out")),
            vec![(10, true), (100, false), (150, true), (200, false)]
        );
    }

    #[test]
    fn same_sample_order_is_reset_set_toggle() {
        let mut h = harness(64);
        h.triggers("set", &[10, 30]);
        h.triggers("reset", &[10, 30]);
        h.triggers("toggle", &[30]);
        let out = h.run(64);
        // At 10, reset then set: on. At 30, reset, set, toggle: off.
        assert_eq!(changes(out.float("out")), vec![(10, true), (30, false)]);
    }

    #[test]
    fn initial_sets_the_starting_state_and_reset_returns_to_it() {
        let mut h = harness(64);
        h.set("initial", true);
        h.triggers("toggle", &[40]);
        let out = h.run(128);
        assert_eq!(out.float("out")[0], 1.0);
        assert_eq!(out.float("out")[127], 0.0);
        h.reset();
        assert!(h.run(64).float("out").iter().all(|&v| v == 1.0));
        // Changing `initial` without a reset changes nothing.
        h.set("initial", false);
        assert!(h.run(64).float("out").iter().all(|&v| v == 1.0));
    }

    #[test]
    fn block_size_does_not_matter() {
        let set: Vec<u64> = (0..40).map(|k| k * 101).collect();
        let reset: Vec<u64> = (0..40).map(|k| k * 137 + 3).collect();
        let toggle: Vec<u64> = (0..60).map(|k| k * 71 + 1).collect();
        let run = |block| {
            let mut h = harness(block);
            h.triggers("set", &set)
                .triggers("reset", &reset)
                .triggers("toggle", &toggle);
            h.run(5_000).float("out").to_vec()
        };
        let a = run(1);
        assert_eq!(a, run(64));
        assert_eq!(a, run(512));
        assert!(changes(&a).len() > 30);
    }
}
