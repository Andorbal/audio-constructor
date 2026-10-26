use ac_core::{
    BoolIn, Config, ConfigExt, FloatOut, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature,
    SocketSpec, node,
};

node! {
    kind: "logic",
    name: "Logic",
    category: Math,
    keywords: ["and", "or", "xor", "not", "nand", "nor", "boolean", "gate"],
    /// Combines gates, `a`, `b`, and more, with a logic operation.
    ///
    /// | `op` | `out` is true when |
    /// | --- | --- |
    /// | `and` | all are true |
    /// | `or` | any is true |
    /// | `xor` | an odd number are true (with two: exactly one) |
    /// | `not` | `a` is false (the others are ignored) |
    /// | `nand` | not all are true |
    /// | `nor` | none is true |
    ///
    /// The `inputs` config sets how many gates there are (2 to 16, default
    /// 2), named `a`, `b`, `c`, and on; the Node tab sets it.
    ///
    /// A `Float` wired into `a` or `b` counts as true above 0.5. The output
    /// runs at the fastest rate of the inputs, so two sample-rate gates
    /// combine sample by sample.
    ///
    /// **Try this:** `and` a `Clock` gate with a `Toggle` to mute a rhythm
    /// without stopping the clock, or `xor` two clocks at different
    /// intervals for a syncopated gate to drive an `ADSR`.
    pub struct Logic {
        inputs: usize,
    }
}

const OUT: usize = 0;
pub const MAX_INPUTS: usize = 16;

/// Gate inputs from config, clamped; `op` follows them.
pub fn input_count(config: &Config) -> usize {
    config.get_u64_or("inputs", 2).clamp(2, MAX_INPUTS as u64) as usize
}

/// The name of input `k`: `a`, `b`, `c`, ….
pub fn input_name(k: usize) -> String {
    char::from(b'a' + (k.min(25)) as u8).to_string()
}

const OPS: &[&str] = &["and", "or", "xor", "not", "nand", "nor"];

impl Node for Logic {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let mut sig = Signature::new()
            .input(SocketSpec::boolean("a", false).doc("First gate."))
            .input(SocketSpec::boolean("b", false).doc("Second gate (ignored by `not`)."));
        for k in 2..input_count(config) {
            sig = sig.input(
                SocketSpec::boolean(input_name(k), false).doc("Another gate (ignored by `not`)."),
            );
        }
        sig.input(SocketSpec::choice("op", OPS, "and").doc("The operation."))
            .output(SocketSpec::boolean("out", false).poly().doc("The result."))
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        Logic {
            inputs: input_count(config),
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let n = self.inputs;
        let op = io.inputs.choice(n, OPS.len());
        let mut out = io.outputs.float(OUT);
        if n > 2 && op != 3 {
            let ins: [BoolIn; MAX_INPUTS] =
                std::array::from_fn(|k| io.inputs.boolean(k.min(n - 1)));
            let ins = &ins[..n];
            let count = |i: usize| ins.iter().filter(|g| g.at(i)).count();
            match op {
                0 => out.fill_with(|i| bit(count(i) == n)),
                1 => out.fill_with(|i| bit(count(i) > 0)),
                2 => out.fill_with(|i| bit(count(i) % 2 == 1)),
                4 => out.fill_with(|i| bit(count(i) < n)),
                _ => out.fill_with(|i| bit(count(i) == 0)),
            }
            return;
        }
        let (a, b) = (io.inputs.boolean(0), io.inputs.boolean(1));
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
fn bit(on: bool) -> f32 {
    if on { 1.0 } else { 0.0 }
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
    fn more_inputs_combine_together() {
        let sig = Logic::describe(&json!({ "inputs": 4 }));
        let names: Vec<_> = sig.inputs.iter().map(|s| s.name.to_string()).collect();
        assert_eq!(names, ["a", "b", "c", "d", "op"]);
        assert_eq!(Logic::describe(&json!({ "inputs": 99 })).inputs.len(), 17);
        let run = |op: &str| {
            let mut h = Harness::new::<Logic>(json!({ "inputs": 3 }));
            h.set_samples("a", vec![0.0, 1.0, 1.0, 1.0])
                .set_samples("b", vec![0.0, 0.0, 1.0, 1.0])
                .set_samples("c", vec![0.0, 0.0, 0.0, 1.0])
                .set("op", op);
            h.run(4).float("out").to_vec()
        };
        assert_eq!(run("and"), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(run("or"), [0.0, 1.0, 1.0, 1.0]);
        assert_eq!(run("xor"), [0.0, 1.0, 0.0, 1.0], "odd parity");
        assert_eq!(run("nand"), [1.0, 1.0, 1.0, 0.0]);
        assert_eq!(run("nor"), [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(run("not"), [1.0, 0.0, 0.0, 0.0], "a alone");
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

    #[test]
    fn same_output_at_any_block_size() {
        let a: Vec<f32> = (0..1_000).map(|i| ((i / 7) % 2) as f32).collect();
        let b: Vec<f32> = (0..1_000).map(|i| ((i / 13) % 2) as f32).collect();
        let render = |block: usize| {
            let mut h =
                Harness::with_resources::<Logic>(json!({}), 48_000.0, block, &ac_core::NoResources);
            h.set_samples("a", a.clone())
                .set_samples("b", b.clone())
                .set("op", "xor");
            h.run(1_000).float("out").to_vec()
        };
        let one = render(1);
        assert_eq!(one[7], 1.0);
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }
}
