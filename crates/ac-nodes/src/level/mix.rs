use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit,
    node,
};
use ac_dsp::util::db_to_gain;

node! {
    kind: "mix",
    name: "Mix",
    category: Level,
    keywords: ["mixer", "sum", "bus", "add", "combine", "merge"],
    /// Sums several audio signals into one, each with its own gain in
    /// decibels.
    ///
    /// The number of inputs is set by the `inputs` config (1 to 64, default
    /// 4); each `in_N` has a matching `gain_N`. The output has as many
    /// channels as the widest input, and mono inputs are copied to every
    /// channel, so a mono and a stereo track mix to stereo. At -60 dB and
    /// below an input is silent. Nothing limits the sum, so lower the gains
    /// when many loud inputs add up.
    ///
    /// **Try this:** select four nodes and add a Mix to auto-wire them, or
    /// put a `Pan` before each input to place tracks in a stereo mix.
    pub struct Mix {
        inputs: usize,
    }
}

const OUT: usize = 0;
const MAX_INPUTS: u64 = 64;

fn input_count(config: &Config) -> usize {
    config.get_u64_or("inputs", 4).clamp(1, MAX_INPUTS) as usize
}

/// Linear gain for a dB value; non-finite values are silent and the boost
/// is capped at +48 dB so a wild wire can't overflow the sum.
#[inline]
fn gain(db: f32) -> f32 {
    if db.is_nan() {
        0.0
    } else {
        db_to_gain(db.min(48.0))
    }
}

impl Node for Mix {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let n = input_count(config);
        let mut sig = Signature::new();
        for i in 0..n {
            sig = sig.input(SocketSpec::audio(format!("in_{i}")).doc("A signal to add."));
        }
        for i in 0..n {
            sig = sig.input(
                SocketSpec::float(format!("gain_{i}"), Unit::Db, 0.0)
                    .range(-60.0, 12.0)
                    .doc("Gain for the matching input."),
            );
        }
        sig.output(SocketSpec::audio("out").doc("The sum of all inputs."))
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        Mix {
            inputs: input_count(config),
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let mut out = io.outputs.audio(OUT);
        out.fill(0.0);
        for k in 0..self.inputs {
            if !io.inputs.is_wired(k) {
                continue;
            }
            let input = io.inputs.audio(k);
            let g = io.inputs.float(self.inputs + k);
            for c in 0..out.channels() {
                let src = input.channel(c);
                let dst = out.channel(c);
                if g.is_varying() {
                    for (i, (o, s)) in dst.iter_mut().zip(src).enumerate() {
                        *o += s * gain(g.at(i));
                    }
                } else {
                    let gk = gain(g.first());
                    if gk == 0.0 {
                        continue;
                    }
                    for (o, s) in dst.iter_mut().zip(src) {
                        *o += s * gk;
                    }
                }
            }
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn sums_inputs() {
        let mut h = Harness::new::<Mix>(json!({}));
        h.set_audio("in_0", vec![vec![0.25; 64]])
            .set_audio("in_2", vec![vec![0.5; 64]]);
        let out = h.run(64);
        assert_eq!(out.audio("out").len(), 1);
        assert_eq!(out.audio("out")[0][7], 0.75);
    }

    #[test]
    fn per_input_gain() {
        let mut h = Harness::new::<Mix>(json!({ "inputs": 2 }));
        h.set_audio("in_0", vec![vec![0.25; 64]])
            .set_audio("in_1", vec![vec![0.5; 64]])
            .set("gain_1", -6.0206)
            .set("gain_0", -60.0);
        let out = h.run(64);
        assert!((out.audio("out")[0][3] - 0.25).abs() < 1e-4);
    }

    #[test]
    fn stereo_plus_mono_is_stereo() {
        let mut h = Harness::new::<Mix>(json!({ "inputs": 2 }));
        h.set_audio("in_0", vec![vec![1.0; 64], vec![-1.0; 64]])
            .set_audio("in_1", vec![vec![0.5; 64]]);
        let out = h.run(64);
        let o = out.audio("out");
        assert_eq!(o.len(), 2);
        assert_eq!((o[0][0], o[1][0]), (1.5, -0.5));
    }

    #[test]
    fn config_sets_the_socket_count() {
        let names = |c| {
            Mix::describe(&c)
                .inputs
                .iter()
                .map(|s| s.name.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(json!({ "inputs": 2 })),
            ["in_0", "in_1", "gain_0", "gain_1"]
        );
        assert_eq!(names(json!({})).len(), 8);
        assert_eq!(names(json!({ "inputs": 0 })).len(), 2);
        assert_eq!(names(json!({ "inputs": 1000 })).len(), 128);
        assert_eq!(names(json!({ "inputs": "lots" })).len(), 8);
    }

    #[test]
    fn sample_rate_gain_modulation() {
        let mut h = Harness::new::<Mix>(json!({ "inputs": 1 }));
        let g: Vec<f32> = (0..64).map(|i| if i < 32 { 0.0 } else { -60.0 }).collect();
        h.set_audio("in_0", vec![vec![1.0; 64]])
            .set_samples("gain_0", g);
        let out = h.run(64);
        assert_eq!(out.audio("out")[0][31], 1.0);
        assert_eq!(out.audio("out")[0][32], 0.0);
    }

    #[test]
    fn wild_gains_stay_finite() {
        let mut h = Harness::new::<Mix>(json!({ "inputs": 1 }));
        h.set_audio("in_0", vec![vec![1.0; 4]]).set_samples(
            "gain_0",
            vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30],
        );
        let out = h.run(4);
        assert!(out.audio("out")[0].iter().all(|v| v.is_finite()));
    }

    #[test]
    fn nothing_wired_is_silent() {
        let mut h = Harness::new::<Mix>(json!({}));
        assert!(h.run(64).audio("out")[0].iter().all(|v| *v == 0.0));
    }
}
