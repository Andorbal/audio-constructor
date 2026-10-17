use ac_core::{
    Channels, Config, ConfigExt, Determinism, Io, Node, NodeMeta, PrepareCtx, ProcessCtx,
    Signature, SocketSpec, node,
};

node! {
    kind: "audio_in",
    name: "Audio In",
    category: Io,
    keywords: ["input", "microphone", "mic", "line in", "interface", "device"],
    /// Brings a device input channel, or a pair of them, into the patch.
    ///
    /// The `channel` config picks the first device input (counting from 0)
    /// and `channels` chooses mono (1) or a stereo pair (2). Inputs the
    /// device doesn't have read as silence, so a project opened on a smaller
    /// interface still runs.
    ///
    /// Audio In is a live source: nodes that need their whole input before
    /// they can play are disabled downstream of it.
    ///
    /// **Try this:** wire `out` through a `Gain` into an `Audio Out` to
    /// monitor a microphone, or into a `Record` node to capture takes.
    pub struct AudioIn {
        first: usize,
    }
}

const OUT: usize = 0;

/// A node spans one device input or a stereo pair.
const MAX_CHANNELS: u64 = 2;

fn channel_count(config: &Config) -> u32 {
    config.get_u64_or("channels", 1).clamp(1, MAX_CHANNELS) as u32
}

impl Node for AudioIn {
    fn meta() -> NodeMeta {
        Self::base_meta().live()
    }

    fn describe(config: &Config) -> Signature {
        Signature::new().output(
            SocketSpec::audio("out")
                .channels(Channels::Fixed(channel_count(config)))
                .doc("The device input."),
        )
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        AudioIn {
            first: usize::try_from(config.get_u64_or("channel", 0)).unwrap_or(usize::MAX),
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let mut out = io.outputs.audio(OUT);
        for (c, dst) in out.iter_mut().enumerate() {
            let src = io.host.input_channel(self.first.saturating_add(c));
            let padded = src.iter().copied().chain(std::iter::repeat(0.0));
            for (o, s) in dst.iter_mut().zip(padded) {
                *o = s;
            }
        }
    }

    fn determinism(&self) -> Determinism {
        Determinism::Live
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn ramp(n: usize, scale: f32) -> Vec<f32> {
        (0..n).map(|i| i as f32 * scale).collect()
    }

    #[test]
    fn mono_copies_the_chosen_device_channel() {
        let mut h = Harness::new::<AudioIn>(json!({"channel": 1}));
        h.host_input(vec![ramp(200, 1.0), ramp(200, -1.0)]);
        let out = h.run(200);
        assert_eq!(out.audio("out").len(), 1);
        assert_eq!(out.audio("out")[0], ramp(200, -1.0));
    }

    #[test]
    fn stereo_pair_copies_two_channels() {
        let mut h = Harness::new::<AudioIn>(json!({"channels": 2}));
        h.host_input(vec![ramp(130, 1.0), ramp(130, 2.0), ramp(130, 3.0)]);
        let out = h.run(130);
        assert_eq!(out.audio("out").len(), 2);
        assert_eq!(out.audio("out")[0], ramp(130, 1.0));
        assert_eq!(out.audio("out")[1], ramp(130, 2.0));
    }

    #[test]
    fn channels_the_device_lacks_are_silent() {
        let mut h = Harness::new::<AudioIn>(json!({"channel": 1, "channels": 2}));
        h.host_input(vec![vec![0.5; 64], vec![0.25; 64]]);
        let out = h.run(64);
        assert_eq!(out.audio("out")[0], vec![0.25; 64]);
        assert_eq!(out.audio("out")[1], vec![0.0; 64]);

        let mut h = Harness::new::<AudioIn>(json!({"channel": u64::MAX}));
        let out = h.run(64);
        assert_eq!(out.audio("out")[0], vec![0.0; 64]);
    }

    #[test]
    fn channel_count_is_mono_or_stereo() {
        let fixed = |c| AudioIn::describe(&c).outputs[0].channels;
        assert_eq!(fixed(json!({})), Channels::Fixed(1));
        assert_eq!(fixed(json!({"channels": 0})), Channels::Fixed(1));
        assert_eq!(fixed(json!({"channels": 2})), Channels::Fixed(2));
        assert_eq!(fixed(json!({"channels": 8})), Channels::Fixed(2));
        assert_eq!(fixed(json!({"channels": "two"})), Channels::Fixed(1));
    }

    #[test]
    fn is_a_live_source() {
        assert!(AudioIn::meta().live);
        let ctx = PrepareCtx {
            sample_rate: 48_000.0,
            max_block: 64,
            seed: 0,
            node_key: "t",
            name: "Audio In",
            resources: &NoResources,
        };
        assert_eq!(
            AudioIn::new(&json!({}), &ctx).determinism(),
            Determinism::Live
        );
    }
}
