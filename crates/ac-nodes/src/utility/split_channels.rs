use ac_core::{
    Channels, Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node,
};

use super::{channel_count, copy_into};

node! {
    kind: "split_channels",
    name: "Split Channels",
    category: Utility,
    keywords: ["stereo", "mono", "channels"],
    /// Splits a multichannel signal into one mono output per channel.
    ///
    /// `out_0` is the first channel (left, for stereo), `out_1` the second,
    /// and so on. The `channels` setting (1 to 32, default 2) sets how many
    /// outputs there are. A mono input is copied to every output.
    ///
    /// **Try this:** split a stereo signal, give each side a different
    /// `Delay` time, and join them again with `Merge Channels` for a wide
    /// echo. Or split a stereo `AudioIn` to process a guitar on the left
    /// input and a microphone on the right separately.
    pub struct SplitChannels {}
}

const IN: usize = 0;

/// Default `channels`: stereo.
const DEFAULT_CHANNELS: u64 = 2;

impl Node for SplitChannels {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let n = channel_count(config, DEFAULT_CHANNELS);
        let mut sig = Signature::new().input(
            SocketSpec::audio("in")
                .channels(Channels::Fixed(n as u32))
                .doc("The signal to split."),
        );
        for c in 0..n {
            sig = sig.output(
                SocketSpec::audio(format!("out_{c}"))
                    .channels(Channels::Fixed(1))
                    .doc("One channel."),
            );
        }
        sig
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        SplitChannels {}
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(IN);
        // Output `c` carries channel `c`; the output count bounds the loop.
        for c in 0..io.outputs.slots.len() {
            let mut out = io.outputs.audio(c);
            copy_into(out.channel(0), input.channel(c));
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
        let sig = SplitChannels::describe(&json!({}));
        let names: Vec<_> = sig.outputs.iter().map(|s| s.name.as_ref()).collect();
        assert_eq!(names, ["out_0", "out_1"]);
        assert_eq!(sig.inputs[0].channels, Channels::Fixed(2));
        assert!(sig.outputs.iter().all(|s| s.channels == Channels::Fixed(1)));
    }

    #[test]
    fn splits_each_channel() {
        let mut h = Harness::new::<SplitChannels>(json!({"channels": 3}));
        let chans = vec![vec![1.0; 100], vec![2.0; 100], vec![3.0; 100]];
        h.set_audio("in", chans.clone());
        let out = h.run(100);
        for (c, want) in chans.iter().enumerate() {
            let got = out.audio(&format!("out_{c}"));
            assert_eq!(got.len(), 1);
            assert_eq!(&got[0], want);
        }
    }

    #[test]
    fn mono_input_goes_to_every_output() {
        let mut h = Harness::new::<SplitChannels>(json!({}));
        let x: Vec<f32> = (0..64).map(|i| i as f32).collect();
        h.set_audio("in", vec![x.clone()]);
        let out = h.run(64);
        assert_eq!(out.audio("out_0")[0], x);
        assert_eq!(out.audio("out_1")[0], x);
    }

    #[test]
    fn channel_config_is_clamped() {
        assert_eq!(
            SplitChannels::describe(&json!({"channels": 0}))
                .outputs
                .len(),
            1
        );
        assert_eq!(
            SplitChannels::describe(&json!({"channels": 99}))
                .outputs
                .len(),
            32
        );
    }
}
