use ac_core::{
    BoolIn, Config, FloatOut, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec,
    node,
};

node! {
    kind: "logic",
    name: "Logic",
    category: Math,
    keywords: ["and", "or", "xor", "not", "nand", "nor", "boolean", "gate"],
    /// Combines two gates, `a` and `b`, with a logic operation.
    ///
    /// | `op` | `out` is true when |
    /// | --- | --- |
    /// | `and` | both are true |
    /// | `or` | either is true |
    /// | `xor` | exactly one is true |
    /// | `not` | `a` is false (`b` is ignored) |
    /// | `nand` | not both are true |
    /// | `nor` | neither is true |
    ///
    /// A `Float` wired into `a` or `b` counts as true above 0.5. The output
    /// runs at the fastest rate of the inputs, so two sample-rate gates
    /// combine sample by sample.
    ///
    /// **Try this:** `and` a `Clock` gate with a `Toggle` to mute a rhythm
    /// without stopping the clock, or `xor` two clocks at different
    /// intervals for a syncopated gate to drive an `ADSR`.
    pub struct Logic {}
}

const A: usize = 0;
const B: usize = 1;
const OP: usize = 2;
const OUT: usize = 0;

const OPS: &[&str] = &["and", "or", "xor", "not", "nand", "nor"];

impl Node for Logic {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::boolean("a", false).doc("First gate."))
            .input(SocketSpec::boolean("b", false).doc("Second gate (ignored by `not`)."))
            .input(SocketSpec::choice("op", OPS, "and").doc("The operation."))
            .output(SocketSpec::boolean("out", false).poly().doc("The result."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Logic {}
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let a = io.inputs.boolean(A);
        let b = io.inputs.boolean(B);
        let op = io.inputs.choice(OP, OPS.len());
        let mut out = io.outputs.float(OUT);
        match op {
            0 => apply(&mut out, a, b, |x, y| x && y),
            1 => apply(&mut out, a, b, |x, y| x || y),
            2 => apply(&mut out, a, b, |x, y| x != y),
            3 => apply(&mut out, a, b, |x, _| !x),
            4 => apply(&mut out, a, b, |x, y| !(x && y)),
            _ => apply(&mut out, a, b, |x, y| !(x || y)),
        }
    }

    fn reset(&mut self) {}
}

#[inline]
fn apply(out: &mut FloatOut, a: BoolIn, b: BoolIn, f: impl Fn(bool, bool) -> bool) {
    out.fill_with(|i| if f(a.at(i), b.at(i)) { 1.0 } else { 0.0 });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    /// `out` for (a, b) = (F, F), (F, T), (T, F), (T, T).
    fn table(op: &str) -> Vec<f32> {
        let mut h = Harness::new::<Logic>(json!({}));
        h.set_samples("a", vec![0.0, 0.0, 1.0, 1.0])
            .set_samples("b", vec![0.0, 1.0, 0.0, 1.0])
            .set("op", op);
        h.run(4).float("out").to_vec()
    }

    #[test]
    fn truth_tables() {
        assert_eq!(table("and"), vec![0.0, 0.0, 0.0, 1.0]);
        assert_eq!(table("or"), vec![0.0, 1.0, 1.0, 1.0]);
        assert_eq!(table("xor"), vec![0.0, 1.0, 1.0, 0.0]);
        assert_eq!(table("not"), vec![1.0, 1.0, 0.0, 0.0]);
        assert_eq!(table("nand"), vec![1.0, 1.0, 1.0, 0.0]);
        assert_eq!(table("nor"), vec![1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn default_is_and_and_floats_threshold_at_half() {
        let mut h = Harness::new::<Logic>(json!({}));
        h.set_samples("a", vec![0.4, 0.6, 0.6, f32::NAN])
            .set_samples("b", vec![1.0, 1.0, 0.5, 1.0]);
        assert_eq!(h.run(4).float("out"), vec![0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn rate_follows_the_inputs() {
        let mut h = Harness::new::<Logic>(json!({}));
        h.set("a", true).set("b", true);
        let out = h.run(64);
        assert_eq!(out.float("out"), vec![1.0; 64]);

        let gate: Vec<f32> = (0..128)
            .map(|i| {
                if (5..20).contains(&i) || i >= 64 {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        let mut h = Harness::new::<Logic>(json!({}));
        h.set_block("a", gate.clone()).set("b", true);
        let out = h.run(128);
        // Block rate: one value per block, read at its first frame.
        assert!(out.float("out")[..64].iter().all(|&v| v == 0.0));
        assert!(out.float("out")[64..].iter().all(|&v| v == 1.0));

        let mut h = Harness::new::<Logic>(json!({}));
        h.set_samples("a", gate.clone()).set("op", "not");
        let out = h.run(128);
        let want: Vec<f32> = gate.iter().map(|&g| 1.0 - g).collect();
        assert_eq!(out.float("out"), want.as_slice());
    }

    #[test]
    fn op_can_be_wired_as_an_int() {
        let mut h = Harness::new::<Logic>(json!({}));
        h.set("a", true).set("b", false).set_int("op", 2);
        assert_eq!(h.run(1).float("out")[0], 1.0);
        h.set_int("op", 42);
        // Clamps to the last op, `nor`.
        assert_eq!(h.run(1).float("out")[0], 0.0);
    }
}
