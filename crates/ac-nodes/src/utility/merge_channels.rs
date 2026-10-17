use ac_core::{
    Channels, Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node,
};

use super::{channel_count, copy_into};

node! {
    kind: "merge_channels",
    name: "Merge Channels",
    category: Utility,
    keywords: ["stereo", "mono", "channels", "join", "combine"],
    /// Combines mono signals into one multichannel signal.
    ///
    /// `in_0` becomes the first channel (left, for stereo), `in_1` the
    /// second, and so on. The `channels` setting (1 to 32, default 2) sets how
    /// many inputs there are. An unwired input is a silent channel.
    ///
    /// **Try this:** merge two `Oscillator`s tuned a few cents apart into a
    /// stereo pair for a wide, moving sound. Or rebuild a stereo signal after
    /// processing its sides separately with `Split Channels`.
    pub struct MergeChannels {}
}

const OUT: usize = 0;

/// Default `channels`: stereo.
const DEFAULT_CHANNELS: u64 = 2;

impl Node for MergeChannels {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let n = channel_count(config, DEFAULT_CHANNELS);
        let mut sig = Signature::new();
        for c in 0..n {
            sig = sig.input(
                SocketSpec::audio(format!("in_{c}"))
                    .channels(Channels::Fixed(1))
                    .doc("One channel."),
            );
        }
        sig.output(
            SocketSpec::audio("out")
                .channels(Channels::Fixed(n as u32))
                .doc("The merged signal."),
        )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        MergeChannels {}
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let inputs = io.inputs;
        let mut out = io.outputs.audio(OUT);
        // Channel `c` comes from input `c`; the output's channel count bounds the loop.
        for (c, dst) in out.iter_mut().enumerate() {
            copy_into(dst, inputs.audio(c).channel(0));
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
    fn stereo_by_default() {
        let sig = MergeChannels::describe(&json!({}));
        let names: Vec<_> = sig.inputs.iter().map(|s| s.name.as_ref()).collect();
        assert_eq!(names, ["in_0", "in_1"]);
        assert_eq!(sig.outputs[0].channels, Channels::Fixed(2));
    }

    #[test]
    fn merges_inputs_in_order() {
        let mut h = Harness::new::<MergeChannels>(json!({"channels": 3}));
        let a: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let b = vec![-1.0; 100];
        h.set_audio("in_0", vec![a.clone()]);
        h.set_audio("in_2", vec![b.clone()]);
        let out = h.run(100);
        let merged = out.audio("out");
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0], a);
        assert_eq!(merged[1], vec![0.0; 100]);
        assert_eq!(merged[2], b);
    }

    #[test]
    fn channel_config_is_clamped() {
        assert_eq!(
            MergeChannels::describe(&json!({"channels": 0}))
                .inputs
                .len(),
            1
        );
        assert_eq!(
            MergeChannels::describe(&json!({"channels": 64}))
                .inputs
                .len(),
            32
        );
    }
}
