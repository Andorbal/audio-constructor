use ac_core::{
    Causality, Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature,
    SocketSpec, Unit, WholeCtx, WholeInputs, WholeOutputs, WholeValue, node,
};
use ac_dsp::loudness;

node! {
    kind: "normalize",
    name: "Normalize",
    category: Level,
    keywords: ["loudness", "lufs", "peak", "maximize", "volume", "level", "master", "whole input"],
    /// Turns a whole signal up or down so its peak, or its loudness,
    /// lands on `target`.
    ///
    /// The `mode` config picks what is measured:
    /// - `"peak"` (default): the highest sample over the whole signal,
    ///   across all channels; `target` is in dBFS (default −1).
    /// - `"lufs"`: integrated loudness as broadcast and streaming services
    ///   measure it (ITU-R BS.1770-4: K-weighted, with the standard's −70
    ///   LUFS and relative gates, so silence and quiet passages don't drag
    ///   the figure down); `target` is in LUFS (default −14). Material
    ///   quieter than −70 LUFS throughout is measured without the gates.
    ///   This mode doesn't limit, so a loud target can push peaks past 0
    ///   dBFS; follow it with a clipping `Shaper` or lower the target.
    ///
    /// One gain applies to the whole signal and every channel, so the
    /// dynamics and the stereo image are untouched. Silence stays silent,
    /// and the gain never rises above +60 dB, so a nearly silent signal
    /// isn't turned into a wall of noise. If `target` is wired, its value
    /// at the start is used.
    ///
    /// Normalize needs to hear all of its input before it knows the gain,
    /// so it works like `Reverse`: when everything feeding it is known
    /// ahead of time, the engine renders that part of the patch over the
    /// project length in the background (**baking**) and Normalize plays
    /// the result in time with the transport, baking again after any
    /// change upstream. Fed by a live source such as Audio In it is
    /// **disabled** and outputs silence. Offline renders always work.
    ///
    /// **Try this:** put Normalize (`lufs`, −14) just before Audio Out so
    /// a render comes out at streaming loudness whatever you changed
    /// upstream, or normalize a quiet `Clip` to −1 dBFS peak before a
    /// `Shaper` so the drive behaves the same for any file.
    pub struct Normalize {}
}

const IN: usize = 0;
const TARGET: usize = 1;
const OUT: usize = 0;

/// Most the gain may rise, in dB.
const MAX_BOOST_DB: f64 = 60.0;

fn is_lufs(config: &Config) -> bool {
    config.get_str_or("mode", "peak") == "lufs"
}

fn default_target(config: &Config) -> f64 {
    if is_lufs(config) { -14.0 } else { -1.0 }
}

/// The level that `target` is compared with, in dB, or `None` for silence.
fn measure(chans: &[Vec<f32>], lufs: bool, rate: f64) -> Option<f64> {
    if lufs {
        loudness::integrated(chans, rate).or_else(|| loudness::ungated(chans, rate))
    } else {
        let peak = chans
            .iter()
            .flatten()
            .filter(|v| v.is_finite())
            .fold(0.0f32, |m, v| m.max(v.abs()));
        (peak > 0.0).then(|| 20.0 * f64::from(peak).log10())
    }
}

impl Node for Normalize {
    fn meta() -> NodeMeta {
        Self::base_meta().whole_input()
    }

    fn describe(config: &Config) -> Signature {
        let (range, doc) = if is_lufs(config) {
            ((-36.0, -6.0), "Loudness to reach, in LUFS.")
        } else {
            ((-24.0, 0.0), "Peak level to reach, in dBFS.")
        };
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to normalize."))
            .input(
                SocketSpec::float("target", Unit::Db, default_target(config))
                    .range(range.0, range.1)
                    .doc(doc),
            )
            .output(SocketSpec::audio("out").doc("The input at the target level."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Normalize {}
    }

    // The engine streams the bake; called directly (outside an engine) the
    // node is silent.
    fn process(&mut self, _ctx: &ProcessCtx, _io: &mut Io) {}

    fn causality(&self) -> Causality {
        Causality::WholeInput
    }

    fn render_whole(config: &Config, ctx: &WholeCtx, inputs: &WholeInputs) -> WholeOutputs {
        let chans: Vec<Vec<f32>> = inputs
            .audio(IN)
            .iter()
            .map(|c| {
                let mut c: Vec<f32> = c
                    .iter()
                    .map(|v| if v.is_finite() { *v } else { 0.0 })
                    .collect();
                c.resize(ctx.frames, 0.0);
                c
            })
            .collect();
        let default = default_target(config);
        let target = f64::from(inputs.float_at(TARGET, 0, default as f32));
        let target = if target.is_finite() {
            target.clamp(-100.0, 24.0)
        } else {
            default
        };
        let gain = match measure(&chans, is_lufs(config), ctx.sample_rate) {
            Some(level) => 10f64.powf((target - level).min(MAX_BOOST_DB) / 20.0) as f32,
            None => 0.0,
        };
        let chans = chans
            .into_iter()
            .map(|c| c.into_iter().map(|v| v * gain).collect())
            .collect();
        let mut out = WholeOutputs::with_outputs(1);
        out.set(OUT, WholeValue::Audio(chans));
        out
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::{Harness, peak};
    use serde_json::json;

    fn sine(amp: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (std::f32::consts::TAU * 997.0 * i as f32 / 48_000.0).sin())
            .collect()
    }

    fn normalize(config: Config, chans: Vec<Vec<f32>>, target: Option<f32>) -> Vec<Vec<f32>> {
        let n = chans.first().map_or(0, Vec::len);
        let target = match target {
            Some(t) => WholeValue::Float(vec![t; n]),
            None => WholeValue::None,
        };
        let out = Harness::render_whole::<Normalize>(
            &config,
            48_000.0,
            vec![WholeValue::Audio(chans), target],
        );
        match out.get(OUT) {
            WholeValue::Audio(c) => c.clone(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn peak_mode_hits_the_target_on_every_channel() {
        let out = normalize(json!({}), vec![sine(0.25, 4_800), sine(0.1, 4_800)], None);
        let want = 10f32.powf(-1.0 / 20.0);
        assert!((peak(&out[0]) - want).abs() < 1e-4, "{}", peak(&out[0]));
        // The quieter channel keeps its relative level.
        assert!((peak(&out[1]) - want * 0.4).abs() < 1e-4);
        let out = normalize(json!({}), vec![sine(0.25, 4_800)], Some(-6.0));
        assert!((peak(&out[0]) - 0.501).abs() < 1e-3);
    }

    #[test]
    fn lufs_mode_hits_the_target_loudness() {
        let out = normalize(json!({ "mode": "lufs" }), vec![sine(0.1, 96_000)], None);
        let l = loudness::integrated(&out, 48_000.0).unwrap();
        assert!((l + 14.0).abs() < 0.01, "{l}");
        let out = normalize(
            json!({ "mode": "lufs" }),
            vec![sine(0.5, 96_000)],
            Some(-23.0),
        );
        let l = loudness::integrated(&out, 48_000.0).unwrap();
        assert!((l + 23.0).abs() < 0.01, "{l}");
    }

    #[test]
    fn silence_stays_silent_and_boost_is_limited() {
        for config in [json!({}), json!({ "mode": "lufs" })] {
            let out = normalize(config.clone(), vec![vec![0.0; 4_800]], None);
            assert!(out[0].iter().all(|&v| v == 0.0), "{config}");
        }
        let out = normalize(json!({}), vec![sine(1e-6, 4_800)], None);
        assert!((peak(&out[0]) - 1e-3).abs() < 1e-5, "{}", peak(&out[0]));
        // Non-finite samples become silence, not a measurement.
        let mut x = sine(0.5, 4_800);
        x[7] = f32::NAN;
        x[8] = f32::INFINITY;
        let out = normalize(json!({}), vec![x], Some(f32::NAN));
        assert!(out[0].iter().all(|v| v.is_finite()));
        assert!((peak(&out[0]) - 10f32.powf(-1.0 / 20.0)).abs() < 1e-3);
    }

    #[test]
    fn config_sets_the_target_default_and_bad_modes_are_peak() {
        let sig = Normalize::describe(&json!({ "mode": "lufs" }));
        assert_eq!(sig.inputs[TARGET].default, ac_core::Value::Float(-14.0));
        let sig = Normalize::describe(&json!({ "mode": 3 }));
        assert_eq!(sig.inputs[TARGET].default, ac_core::Value::Float(-1.0));
        assert!(Normalize::meta().whole_input);
    }

    #[test]
    fn unwired_bakes_nothing_and_process_is_silent() {
        let out = Harness::render_whole::<Normalize>(&json!({}), 48_000.0, vec![WholeValue::None]);
        assert_eq!(out.get(OUT), &WholeValue::Audio(vec![]));
        let mut h = Harness::new::<Normalize>(json!({}));
        h.set_audio("in", vec![vec![1.0; 64]]);
        assert!(h.run(64).audio("out")[0].iter().all(|&v| v == 0.0));
        h.reset();
    }
}
