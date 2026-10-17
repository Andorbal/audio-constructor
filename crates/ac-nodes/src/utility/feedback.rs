use ac_core::{
    Channels, Config, Io, Node, NodeMeta, Phase, PrepareCtx, ProcessCtx, Signature, SocketSpec,
    node,
};

use super::{channel_count, copy_into};

node! {
    kind: "feedback",
    name: "Feedback",
    category: Utility,
    keywords: ["loop", "cycle", "recursion"],
    /// Sends a signal back to an earlier point in the patch. It is the only
    /// way to make a loop: a wire path that leads back to where it started.
    ///
    /// What comes out of `out` is what went into `in` one block earlier. The
    /// delay is exactly one processing block (the engine's block size, shown
    /// in the node's header), so **results depend on the block size**: a
    /// feedback patch sounds slightly different at a different buffer size,
    /// and offline renders match live playback only at the same block size.
    /// For a loop with a delay you choose, put a `Delay` inside it; the loop
    /// then takes the `Delay` time plus one block.
    ///
    /// The `channels` setting (1 to 32, default 1) sets how many channels the
    /// loop carries; a mono input is copied to every channel. `out` is silent
    /// for the first block and after a transport restart. NaN or infinite
    /// samples are stored as 0, so one bad value can't circulate forever.
    ///
    /// **Try this:** build an echo from parts: `Mix` → `Delay` → `Gain`
    /// (−6 dB) → `Feedback` → back into a second input of the `Mix`. Each
    /// trip round the loop comes back 6 dB quieter. Keep the gain around a
    /// loop below 0 dB, or it grows until it distorts.
    pub struct Feedback {
        /// The block stored for the next read, one buffer per channel.
        store: Vec<Vec<f32>>,
        /// Frames of `store` holding real samples.
        stored: usize,
    }
}

const IN: usize = 0;
const OUT: usize = 0;

impl Node for Feedback {
    fn meta() -> NodeMeta {
        Self::base_meta().feedback()
    }

    fn describe(config: &Config) -> Signature {
        let ch = Channels::Fixed(channel_count(config, 1) as u32);
        Signature::new()
            .input(
                SocketSpec::audio("in")
                    .channels(ch)
                    .doc("Signal to send back round the loop."),
            )
            .output(
                SocketSpec::audio("out")
                    .channels(ch)
                    .doc("What went into `in` one block ago."),
            )
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let channels = channel_count(config, 1);
        Feedback {
            store: vec![vec![0.0; ctx.max_block.max(1)]; channels],
            stored: 0,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        // The compiler splits this node into a read step (scheduled first) and
        // a write step (scheduled after `in` is computed), which is what cuts
        // the cycle. A single unsplit call does both, in that order.
        match ctx.phase {
            Phase::FeedbackRead => self.read(io),
            Phase::FeedbackWrite => self.write(io),
            Phase::Main => {
                self.read(io);
                self.write(io);
            }
        }
    }

    fn reset(&mut self) {
        for buf in &mut self.store {
            buf.fill(0.0);
        }
        self.stored = 0;
    }
}

impl Feedback {
    /// Last block's input to `out`; frames beyond what was stored are silent.
    fn read(&self, io: &mut Io) {
        let mut out = io.outputs.audio(OUT);
        for (c, dst) in out.iter_mut().enumerate() {
            let src = self
                .store
                .get(c)
                .and_then(|buf| buf.get(..self.stored))
                .unwrap_or(&[]);
            copy_into(dst, src);
        }
    }

    /// This block's input into the store, made finite.
    fn write(&mut self, io: &mut Io) {
        let input = io.inputs.audio(IN);
        let capacity = self.store.first().map_or(0, Vec::len);
        let frames = io.frames().min(capacity);
        for (c, buf) in self.store.iter_mut().enumerate() {
            let Some(dst) = buf.get_mut(..frames) else {
                continue;
            };
            copy_into(dst, input.channel(c));
            for s in dst.iter_mut() {
                if !s.is_finite() {
                    *s = 0.0;
                }
            }
        }
        self.stored = frames;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    const SPLIT: &[Phase] = &[Phase::FeedbackRead, Phase::FeedbackWrite];

    fn ramp(n: usize) -> Vec<f32> {
        (1..=n).map(|i| i as f32).collect()
    }

    #[test]
    fn is_marked_as_the_feedback_node() {
        assert!(Feedback::meta().feedback);
    }

    #[test]
    fn delays_by_one_block_when_split() {
        let mut h = Harness::new::<Feedback>(json!({}));
        h.phases(SPLIT);
        let x = ramp(256);
        h.set_audio("in", vec![x.clone()]);
        let out = h.run(256);
        let y = &out.audio("out")[0];
        assert_eq!(&y[..64], &[0.0; 64]);
        assert_eq!(&y[64..], &x[..192]);
    }

    #[test]
    fn main_phase_matches_split_phases() {
        let x = ramp(300);
        let mut split = Harness::new::<Feedback>(json!({}));
        split.phases(SPLIT).set_audio("in", vec![x.clone()]);
        let mut main = Harness::new::<Feedback>(json!({}));
        main.set_audio("in", vec![x]);
        assert_eq!(split.run(300).audio("out"), main.run(300).audio("out"));
    }

    #[test]
    fn delay_is_one_block_at_any_block_size() {
        for block in [1, 7, 64, 512] {
            let mut h = Harness::with_resources::<Feedback>(
                json!({}),
                48_000.0,
                block,
                &ac_core::NoResources,
            );
            h.phases(SPLIT);
            let x = ramp(1024);
            h.set_audio("in", vec![x.clone()]);
            let y = h.run(1024).audio("out")[0].clone();
            assert_eq!(&y[block..], &x[..1024 - block], "block {block}");
        }
    }

    #[test]
    fn varying_block_sizes_pad_with_silence() {
        // The harness splits runs into blocks of at most 64 frames.
        let mut h = Harness::new::<Feedback>(json!({}));
        h.phases(SPLIT);
        let x = ramp(200);
        h.set_audio("in", vec![x.clone()]);
        h.run(64);
        let short = h.run(30).audio("out")[0].clone();
        assert_eq!(short, &x[..30]);
        let long = h.run(64).audio("out")[0].clone();
        assert_eq!(&long[..30], &x[64..94]);
        assert_eq!(&long[30..], &[0.0; 34]);
    }

    #[test]
    fn carries_several_channels_and_upmixes_mono() {
        let mut h = Harness::new::<Feedback>(json!({"channels": 2}));
        h.phases(SPLIT);
        let (l, r) = (ramp(128), vec![-1.0; 128]);
        h.set_audio("in", vec![l.clone(), r.clone()]);
        let out = h.run(128);
        assert_eq!(out.audio("out").len(), 2);
        assert_eq!(&out.audio("out")[0][64..], &l[..64]);
        assert_eq!(&out.audio("out")[1][64..], &r[..64]);

        let mut h = Harness::new::<Feedback>(json!({"channels": 2}));
        h.phases(SPLIT);
        h.set_audio("in", vec![l.clone()]);
        let out = h.run(128);
        assert_eq!(&out.audio("out")[1][64..], &l[..64]);
    }

    #[test]
    fn channel_config_is_clamped() {
        let ch = |c: serde_json::Value| match Feedback::describe(&c).outputs[0].channels {
            Channels::Fixed(n) => n,
            other => panic!("{other:?}"),
        };
        assert_eq!(ch(json!({})), 1);
        assert_eq!(ch(json!({"channels": 0})), 1);
        assert_eq!(ch(json!({"channels": 1000})), 32);
        assert_eq!(ch(json!({"channels": "two"})), 1);
    }

    #[test]
    fn reset_clears_the_stored_block() {
        let mut h = Harness::new::<Feedback>(json!({}));
        h.phases(SPLIT);
        h.set_audio("in", vec![vec![0.5; 256]]);
        h.run(64);
        h.reset();
        let out = h.run(64);
        assert_eq!(out.audio("out")[0], vec![0.0; 64]);
    }

    #[test]
    fn non_finite_samples_are_stored_as_zero() {
        let mut h = Harness::new::<Feedback>(json!({}));
        h.phases(SPLIT);
        let mut x = vec![0.5; 128];
        x[3] = f32::NAN;
        x[4] = f32::INFINITY;
        h.set_audio("in", vec![x]);
        let y = h.run(128).audio("out")[0].clone();
        assert_eq!(&y[64..70], &[0.5, 0.5, 0.5, 0.0, 0.0, 0.5]);
    }
}
