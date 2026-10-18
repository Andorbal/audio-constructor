use ac_core::{
    Channels, Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec,
    node,
};

use super::channel_count;

node! {
    kind: "latency",
    name: "Latency",
    category: Utility,
    keywords: ["pdc", "delay compensation", "samples", "align", "debug"],
    /// Delays audio by a fixed number of samples and tells the engine so,
    /// which makes it a handy way to see latency compensation at work.
    ///
    /// Some nodes (FFT, Lookahead, PitchShift) delay what passes through
    /// them and report it. Wherever paths with different delays meet, the
    /// engine delays the faster inputs so everything lines up: a dry signal
    /// mixed with an FFT-processed copy stays in time. Latency does the same
    /// as those nodes, by exactly `samples` (config, up to 10 seconds' worth
    /// at 96 kHz), with `channels` channels (config, 1 to 32, default 2).
    ///
    /// `path_latency` reports how far behind the sources this point runs, in
    /// samples: the slowest path into the node plus its own delay. That is
    /// the figure the engine lines other paths up to where they meet. With
    /// `samples` at 0, Latency is a pure probe: drop it anywhere to read the
    /// latency at that point. The project's total output latency shows in
    /// `audioc validate` and in the editor.
    ///
    /// **Try this:** split an Oscillator into two paths, one through a
    /// Latency of 480 samples, and mix them: they stay in phase instead of
    /// comb-filtering, because the engine delays the direct path too.
    /// `audioc validate` lists the delay it added to the Mix input.
    pub struct Latency {
        delay: usize,
        lines: Vec<Vec<f32>>,
        write: usize,
        path_latency: u32,
    }
}

const IN: usize = 0;
const OUT: usize = 0;
const PATH_LATENCY: usize = 1;

/// 10 seconds at 96 kHz.
const MAX_SAMPLES: u64 = 960_000;

fn samples(config: &Config) -> usize {
    config.get_u64_or("samples", 0).min(MAX_SAMPLES) as usize
}

impl Node for Latency {
    fn meta() -> NodeMeta {
        Self::base_meta().latency_probe()
    }

    fn describe(config: &Config) -> Signature {
        let ch = Channels::Fixed(channel_count(config, 2) as u32);
        Signature::new()
            .input(SocketSpec::audio("in").channels(ch).doc("Signal to delay."))
            .output(
                SocketSpec::audio("out")
                    .channels(ch)
                    .doc("The input, `samples` later."),
            )
            .output(
                SocketSpec::int("path_latency", 0)
                    .doc("How far behind the sources this point runs, in samples."),
            )
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        let n = samples(config);
        Latency {
            delay: n,
            lines: vec![vec![0.0; n.max(1)]; channel_count(config, 2)],
            write: 0,
            path_latency: 0,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(IN);
        let delay = self.delay;
        {
            let mut out = io.outputs.audio(OUT);
            for (c, line) in self.lines.iter_mut().enumerate() {
                let src = input.channel(c);
                let dst = out.channel(c);
                if delay == 0 {
                    let n = src.len().min(dst.len());
                    dst[..n].copy_from_slice(&src[..n]);
                    continue;
                }
                let len = line.len();
                let mut w = self.write;
                for (i, o) in dst.iter_mut().enumerate() {
                    *o = line[w];
                    line[w] = src.get(i).copied().unwrap_or(0.0);
                    w = (w + 1) % len;
                }
            }
        }
        if delay > 0 {
            self.write = (self.write + ctx.frames) % delay;
        }
        io.outputs
            .set_int(PATH_LATENCY, self.path_latency.min(i32::MAX as u32) as i32);
    }

    fn latency(&self) -> u32 {
        self.delay as u32
    }

    fn path_latency(&mut self, samples: u32) {
        self.path_latency = samples;
    }

    fn reset(&mut self) {
        for l in &mut self.lines {
            l.fill(0.0);
        }
        self.write = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn impulse(n: usize) -> Vec<f32> {
        let mut x = vec![0.0; n];
        x[0] = 1.0;
        x
    }

    #[test]
    fn delays_by_exactly_the_configured_samples_and_reports_it() {
        for block in [1, 64, 1000] {
            let mut h = Harness::with_resources::<Latency>(
                json!({ "samples": 100, "channels": 1 }),
                48_000.0,
                block,
                &ac_core::NoResources,
            );
            h.set_audio("in", vec![impulse(300)]);
            let out = h.run(300);
            let o = &out.audio("out")[0];
            assert_eq!(o[100], 1.0, "block {block}");
            assert_eq!(o.iter().sum::<f32>(), 1.0);
        }
        let ctx = PrepareCtx {
            sample_rate: 48_000.0,
            max_block: 64,
            seed: 0,
            node_key: "t",
            name: "Latency",
            resources: &ac_core::NoResources,
        };
        let n = Latency::new(&json!({ "samples": 480 }), &ctx);
        assert_eq!(n.latency(), 480);
        assert_eq!(
            Latency::new(&json!({ "samples": 1e12 }), &ctx).latency(),
            960_000
        );
        assert!(Latency::meta().latency_probe);
    }

    #[test]
    fn zero_samples_passes_through_and_path_latency_is_reported() {
        let mut h = Harness::new::<Latency>(json!({}));
        let x: Vec<f32> = (0..128).map(|i| i as f32).collect();
        h.set_audio("in", vec![x.clone(), x.clone()]);
        let out = h.run(128);
        assert_eq!(out.audio("out")[1], x);
        assert_eq!(out.int("path_latency")[0], 0);
    }

    #[test]
    fn reset_clears_the_line() {
        let mut h = Harness::new::<Latency>(json!({ "samples": 50 }));
        h.set_audio("in", vec![vec![1.0; 40]]);
        h.run(40);
        h.reset();
        let out = h.run(100);
        assert!(out.audio("out")[0].iter().all(|&v| v == 0.0));
    }
}
