use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node,
};
use ac_dsp::util::snap_samples;

node! {
    kind: "lookahead",
    name: "Lookahead",
    category: Level,
    keywords: ["look ahead", "limiter", "delay", "latency", "transient", "brickwall", "dynamics"],
    /// Lets a level detector see the signal before it is heard: `out` is
    /// the input delayed by `time`, and `early` is the input as it comes
    /// in, `time` ahead of `out`.
    ///
    /// Feed the detector (`Envelope Follower` → `Gain Computer`) from
    /// `early` and the `Gain` that does the turning down from `out`. The
    /// gain change then starts `time` before the transient that caused it
    /// reaches the Gain, so a limiter can catch a peak completely instead
    /// of letting its first few milliseconds through.
    ///
    /// Lookahead reports `time` as its latency, so the engine delays other
    /// paths to stay in step with `out`: a dry signal mixed with `out`
    /// lines up. Both outputs come from the same node, so the engine treats
    /// them as equally late and puts no delay between them; that is what
    /// keeps `early` early. (It also means `early` mixed with other audio
    /// arrives `time` ahead of it.)
    ///
    /// `time` is a config (seconds, default 0.005, at most 0.1) because the
    /// engine needs the latency before playing. Both outputs have as many
    /// channels as the input; up to 64 channels are delayed, and `out` is
    /// silent on any beyond that.
    ///
    /// **Try this:** build a limiter: `in` → Lookahead; `early` →
    /// Envelope Follower (`attack` about the lookahead `time`, `release`
    /// 0.1) → Gain Computer (`ratio` 100, `knee` 0, `threshold` −1) →
    /// `gain_reduction` into a `Gain` whose `in` is Lookahead's `out`.
    pub struct Lookahead {
        delay: usize,
        /// `LINES` delay lines of `delay` samples, one after another.
        lines: Vec<f32>,
        write: usize,
        /// Samples written since the last reset (up to `delay`); slots
        /// older than that read as silence, so resets take no time.
        filled: usize,
    }
}

const IN: usize = 0;
const OUT: usize = 0;
const EARLY: usize = 1;

/// Channels with their own delay line. The node isn't told its channel
/// count before `process`; lines are short, so this covers any patch.
const LINES: usize = 64;
const MAX_TIME: f64 = 0.1;
const DEFAULT_TIME: f64 = 0.005;

/// The lookahead in seconds, repaired into `0..=0.1`.
fn time(config: &Config) -> f64 {
    let t = config.get_f64_or("time", DEFAULT_TIME);
    if t.is_finite() {
        t.clamp(0.0, MAX_TIME)
    } else {
        DEFAULT_TIME
    }
}

/// The lookahead in whole samples at `rate`.
fn samples(config: &Config, rate: f64) -> usize {
    let n = snap_samples(time(config) * rate).round();
    if n.is_finite() && n > 0.0 {
        n as usize
    } else {
        0
    }
}

impl Node for Lookahead {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let _ = time(config);
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to look ahead into."))
            .output(SocketSpec::audio("out").doc("The input, `time` later: feed the Gain."))
            .output(
                SocketSpec::audio("early")
                    .doc("The input, `time` ahead of `out`: feed the detector."),
            )
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let delay = samples(config, ctx.sample_rate);
        Lookahead {
            delay,
            lines: vec![0.0; LINES * delay],
            write: 0,
            filled: 0,
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(IN);
        let frames = io.frames();
        let (delay, start, filled) = (self.delay, self.write, self.filled);
        {
            let mut out = io.outputs.audio(OUT);
            for c in 0..out.channels() {
                let src = input.channel(c);
                let dst = out.channel(c);
                if delay == 0 {
                    let n = src.len().min(dst.len());
                    dst[..n].copy_from_slice(&src[..n]);
                    continue;
                }
                let Some(line) = self.lines.get_mut(c * delay..(c + 1) * delay) else {
                    dst.fill(0.0);
                    continue;
                };
                let mut w = start;
                for (i, o) in dst.iter_mut().enumerate() {
                    *o = if filled + i >= delay { line[w] } else { 0.0 };
                    line[w] = src.get(i).copied().unwrap_or(0.0);
                    w += 1;
                    if w == delay {
                        w = 0;
                    }
                }
            }
        }
        let mut early = io.outputs.audio(EARLY);
        for c in 0..early.channels() {
            let src = input.channel(c);
            let dst = early.channel(c);
            let n = src.len().min(dst.len());
            dst[..n].copy_from_slice(&src[..n]);
        }
        if delay > 0 {
            self.write = (start + frames) % delay;
            self.filled = (filled + frames).min(delay);
        }
    }

    fn latency(&self) -> u32 {
        self.delay as u32
    }

    fn reset(&mut self) {
        self.write = 0;
        self.filled = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn impulse(n: usize, at: usize) -> Vec<f32> {
        let mut x = vec![0.0; n];
        x[at] = 1.0;
        x
    }

    fn ctx() -> PrepareCtx<'static> {
        PrepareCtx {
            sample_rate: 48_000.0,
            max_block: 64,
            seed: 0,
            node_key: "t",
            name: "Lookahead",
            resources: &NoResources,
        }
    }

    #[test]
    fn early_is_time_ahead_of_out() {
        for block in [1, 64, 512] {
            let mut h = Harness::with_resources::<Lookahead>(
                json!({ "time": 0.001 }),
                48_000.0,
                block,
                &NoResources,
            );
            h.set_audio("in", vec![impulse(1_000, 10)]);
            let out = h.run(1_000);
            let (o, e) = (&out.audio("out")[0], &out.audio("early")[0]);
            assert_eq!(e[10], 1.0, "block {block}");
            assert_eq!(o[58], 1.0, "block {block}");
            assert_eq!(o.iter().sum::<f32>(), 1.0);
            assert_eq!(e.iter().sum::<f32>(), 1.0);
        }
    }

    #[test]
    fn reports_its_latency_and_repairs_the_config() {
        assert_eq!(Lookahead::new(&json!({}), &ctx()).latency(), 240);
        assert_eq!(
            Lookahead::new(&json!({ "time": 0.01 }), &ctx()).latency(),
            480
        );
        assert_eq!(
            Lookahead::new(&json!({ "time": 5.0 }), &ctx()).latency(),
            4_800
        );
        assert_eq!(
            Lookahead::new(&json!({ "time": -1.0 }), &ctx()).latency(),
            0
        );
        assert_eq!(
            Lookahead::new(&json!({ "time": "x" }), &ctx()).latency(),
            240
        );
    }

    #[test]
    fn stereo_and_zero_time() {
        let mut h = Harness::new::<Lookahead>(json!({ "time": 0.0005 }));
        let x: Vec<f32> = (0..200).map(|i| i as f32).collect();
        let y: Vec<f32> = x.iter().map(|v| -v).collect();
        h.set_audio("in", vec![x.clone(), y.clone()]);
        let out = h.run(200);
        let o = out.audio("out");
        assert_eq!(o.len(), 2);
        assert_eq!(&o[0][24..], &x[..176]);
        assert_eq!(&o[1][24..], &y[..176]);
        assert_eq!(out.audio("early")[1], y);
        let mut h = Harness::new::<Lookahead>(json!({ "time": 0 }));
        h.set_audio("in", vec![x.clone()]);
        let out = h.run(200);
        assert_eq!(out.audio("out")[0], x);
    }

    #[test]
    fn reset_silences_the_line() {
        let mut h = Harness::new::<Lookahead>(json!({}));
        h.set_audio("in", vec![vec![1.0; 100]]);
        h.run(100);
        h.reset();
        let out = h.run(300);
        assert!(out.audio("out")[0].iter().all(|&v| v == 0.0));
    }
}
