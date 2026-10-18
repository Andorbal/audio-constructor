use ac_core::{Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node};

node! {
    kind: "gate_to_trigger",
    name: "Gate to Trigger",
    category: Triggers,
    keywords: ["edge", "rising", "falling", "detect", "change", "bool to trigger", "onset"],
    /// Fires a trigger where a gate turns on, turns off, or both.
    ///
    /// `edge` chooses which changes of `in` fire: `rising` (off to on, the
    /// default), `falling` (on to off), or `both`. The trigger lands on the
    /// exact sample the gate changes when `in` is wired at audio rate, and
    /// at the start of the block for slower signals. The gate counts as off
    /// before the first block and after a transport restart, so a gate that
    /// is already on fires a rising edge straight away.
    ///
    /// **Try this:** wire a `Compare` into `in` to fire a trigger whenever a
    /// signal crosses a level, or set `edge` to falling and wire a `Button`'s
    /// `gate` in to fire on release.
    pub struct GateToTrigger {
        /// The gate at the end of the last block.
        prev: bool,
    }
}

const IN: usize = 0;
const EDGE: usize = 1;

const OUT: usize = 0;

const EDGES: &[&str] = &["rising", "falling", "both"];

impl Node for GateToTrigger {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::boolean("in", false).doc("The gate to watch."))
            .input(SocketSpec::choice("edge", EDGES, "rising").doc("Which changes fire."))
            .output(SocketSpec::trigger("out").doc("Fires on each chosen edge."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        GateToTrigger { prev: false }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let gate = io.inputs.boolean(IN);
        let edge = io.inputs.choice(EDGE, EDGES.len());
        let (rising, falling) = (edge != 1, edge != 0);
        let frames = if gate.is_varying() {
            ctx.frames
        } else {
            ctx.frames.min(1)
        };
        let mut out = io.outputs.trigger(OUT);
        for i in 0..frames {
            let g = gate.at(i);
            if g != self.prev && (if g { rising } else { falling }) {
                out.push(i as u32);
            }
            self.prev = g;
        }
    }

    fn reset(&mut self) {
        self.prev = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn gate(len: usize, spans: &[(usize, usize)]) -> Vec<f32> {
        let mut v = vec![0.0; len];
        for &(a, b) in spans {
            v[a..b].fill(1.0);
        }
        v
    }

    fn run(block: usize, g: &[f32], edge: &str) -> Vec<u64> {
        let mut h = Harness::with_resources::<GateToTrigger>(
            json!({}),
            48_000.0,
            block,
            &ac_core::NoResources,
        );
        h.set_samples("in", g.to_vec());
        h.set("edge", edge);
        h.run(g.len()).triggers("out").to_vec()
    }

    #[test]
    fn fires_on_the_chosen_edges() {
        let g = gate(300, &[(10, 70), (130, 131), (200, 300)]);
        assert_eq!(run(64, &g, "rising"), &[10, 130, 200]);
        assert_eq!(run(64, &g, "falling"), &[70, 131]);
        assert_eq!(run(64, &g, "both"), &[10, 70, 130, 131, 200]);
    }

    #[test]
    fn block_size_does_not_matter() {
        let spans: Vec<(usize, usize)> = (0..50).map(|k| (k * 97, k * 97 + 1 + k % 40)).collect();
        let g = gate(5_000, &spans);
        for edge in EDGES {
            let a = run(1, &g, edge);
            assert_eq!(a, run(64, &g, edge));
            assert_eq!(a, run(512, &g, edge));
            assert!(!a.is_empty());
        }
    }

    #[test]
    fn a_gate_already_on_fires_at_once_and_after_reset() {
        let mut h = Harness::new::<GateToTrigger>(json!({}));
        h.set("in", true);
        assert_eq!(h.run(256).triggers("out"), &[0]);
        h.reset();
        assert_eq!(h.run(64).triggers("out"), &[256]);
        h.set("in", false).set("edge", "falling");
        assert_eq!(h.run(64).triggers("out"), &[320]);
        // Out-of-range choices clamp.
        let mut h = Harness::new::<GateToTrigger>(json!({}));
        h.set_samples("in", gate(64, &[(5, 9)]));
        h.set("edge", 9);
        assert_eq!(h.run(64).triggers("out"), &[5, 9]);
    }
}
