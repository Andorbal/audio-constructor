use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, WholeCtx,
    WholeInputs, WholeOutputs, WholeValue, node,
};

node! {
    kind: "reverse",
    name: "Reverse",
    category: Time,
    keywords: ["backwards", "reversed", "flip", "palindrome", "whole input"],
    /// Plays its input backwards.
    ///
    /// Reverse needs to hear all of its input before it can play the end
    /// first, so it doesn't run live like other nodes. When everything
    /// feeding it is known ahead of time (a Clip, an Oscillator, a Pattern,
    /// a Slider), the engine renders that part of the patch over the project
    /// length in the background (**baking**, shown on the node) and Reverse
    /// plays the result in time with the transport. Change anything upstream
    /// and it bakes again; the old bake plays until the new one is ready.
    /// When a live source such as Audio In feeds it, Reverse is **disabled**
    /// and outputs silence, since it can't know what you'll play next. An
    /// offline render (`audioc render`, Export WAV) always works, because the
    /// whole input is known.
    ///
    /// The `segment` config reverses the input in slices of that many
    /// seconds instead of all at once (0, the default, reverses the whole
    /// project): each slice plays backwards in its own place, which keeps
    /// the music where it was.
    ///
    /// **Try this:** feed it a drum Clip with `segment` set to one beat's
    /// length (0.5 s at 120 BPM) for a reversed groove that stays in time,
    /// or put a `Delay` after it for the classic reverse-echo swell.
    pub struct Reverse {}
}

const IN: usize = 0;
const OUT: usize = 0;

/// Slice length in seconds; 0 reverses everything.
fn segment(config: &Config) -> f64 {
    let s = config.get_f64_or("segment", 0.0);
    if s.is_finite() {
        s.clamp(0.0, 3600.0)
    } else {
        0.0
    }
}

/// Reverse `x` within consecutive slices of `len` frames (0 = all of it).
fn reverse_in_slices(x: &[f32], len: usize) -> Vec<f32> {
    if len == 0 || len >= x.len() {
        return x.iter().rev().copied().collect();
    }
    let mut out = Vec::with_capacity(x.len());
    for chunk in x.chunks(len) {
        out.extend(chunk.iter().rev());
    }
    out
}

impl Node for Reverse {
    fn meta() -> NodeMeta {
        Self::base_meta().whole_input()
    }

    fn describe(config: &Config) -> Signature {
        let _ = segment(config);
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to reverse."))
            .output(SocketSpec::audio("out").doc("The input, backwards."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Reverse {}
    }

    // The engine streams the bake; called directly (outside an engine) the
    // node is silent.
    fn process(&mut self, _ctx: &ProcessCtx, _io: &mut Io) {}

    fn causality(&self) -> ac_core::Causality {
        ac_core::Causality::WholeInput
    }

    fn render_whole(config: &Config, ctx: &WholeCtx, inputs: &WholeInputs) -> WholeOutputs {
        let len = (segment(config) * ctx.sample_rate).round() as usize;
        let chans: Vec<Vec<f32>> = inputs
            .audio(IN)
            .iter()
            .map(|c| {
                let mut c = c.clone();
                c.resize(ctx.frames, 0.0);
                reverse_in_slices(&c, len)
            })
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
    use ac_core::testing::Harness;
    use serde_json::json;

    fn ramp(n: usize) -> Vec<f32> {
        (0..n).map(|i| i as f32).collect()
    }

    #[test]
    fn whole_input_plays_backwards() {
        let out = Harness::render_whole::<Reverse>(
            &json!({}),
            48_000.0,
            vec![WholeValue::Audio(vec![
                ramp(5),
                vec![1.0, 0.0, 0.0, 0.0, 0.0],
            ])],
        );
        match out.get(OUT) {
            WholeValue::Audio(c) => {
                assert_eq!(c[0], [4.0, 3.0, 2.0, 1.0, 0.0]);
                assert_eq!(c[1], [0.0, 0.0, 0.0, 0.0, 1.0]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn segments_reverse_in_place() {
        // 2-frame slices at 2 Hz: segment = 1 s.
        let out = Harness::render_whole::<Reverse>(
            &json!({ "segment": 1.0 }),
            2.0,
            vec![WholeValue::Audio(vec![ramp(5)])],
        );
        match out.get(OUT) {
            WholeValue::Audio(c) => assert_eq!(c[0], [1.0, 0.0, 3.0, 2.0, 4.0]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unwired_input_bakes_nothing_and_process_is_silent() {
        let out = Harness::render_whole::<Reverse>(&json!({}), 48_000.0, vec![WholeValue::None]);
        assert_eq!(out.get(OUT), &WholeValue::Audio(vec![]));
        let mut h = Harness::new::<Reverse>(json!({}));
        h.set_audio("in", vec![vec![1.0; 64]]);
        assert!(h.run(64).audio("out")[0].iter().all(|&v| v == 0.0));
        assert!(Reverse::meta().whole_input);
    }

    #[test]
    fn bad_segments_are_repaired() {
        assert_eq!(segment(&json!({ "segment": -3.0 })), 0.0);
        assert_eq!(segment(&json!({ "segment": "x" })), 0.0);
        assert_eq!(segment(&json!({ "segment": 1e9 })), 3600.0);
    }
}
