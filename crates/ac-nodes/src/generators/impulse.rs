use ac_core::{
    AutoTrigger, Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec,
    Unit, node,
};

node! {
    kind: "impulse",
    name: "Impulse",
    category: Generators,
    keywords: ["click", "dirac", "spike", "ping", "test signal", "impulse response"],
    /// A single-sample click on each `trigger`, as a sample-rate `Float`.
    ///
    /// The output is `gain` (default 1) on the exact frame of each trigger
    /// and 0 everywhere else; several triggers on one frame make one click.
    /// Negative `gain` clicks downwards. Unwired, `trigger` fires when the
    /// transport starts (or on load, or never: pick in the dropdown), so
    /// pressing play sends one click; its button fires one by hand.
    ///
    /// A click contains every frequency at once, which makes it the
    /// standard way to hear (or measure) what a filter, delay, or reverb
    /// does: what comes out is the patch's impulse response.
    ///
    /// **Try this:** wire it into a `Biquad` with high `q` for a pitched
    /// "ping", or into a `Delay` with high `feedback` to hear the echo
    /// pattern; drive `trigger` from a `Clock` for a metronome click.
    pub struct Impulse {}
}

const TRIGGER: usize = 0;
const GAIN: usize = 1;
const OUT: usize = 0;

impl Node for Impulse {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::trigger("trigger").auto(AutoTrigger::Start).doc(
                "Each trigger makes one click. Unwired: on transport play, on load, or never.",
            ))
            .input(
                SocketSpec::float("gain", Unit::Generic, 1.0)
                    .range(-1.0, 1.0)
                    .doc("Height of the click."),
            )
            .output(
                SocketSpec::float("out", Unit::Generic, 0.0)
                    .rate(Rate::Sample)
                    .doc("The clicks; 0 between them."),
            )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Impulse {}
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let frames = io.frames();
        let gain = io.inputs.float(GAIN);
        let triggers = io.inputs.triggers(TRIGGER);
        let mut out = io.outputs.float(OUT);
        out.fill(0.0);
        for &t in triggers {
            let i = (t as usize).min(frames.saturating_sub(1));
            let g = gain.at(i);
            out.set(i, if g.is_finite() { g } else { 0.0 });
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NoResources;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn harness(block: usize) -> Harness {
        Harness::with_resources::<Impulse>(json!({}), 48_000.0, block, &NoResources)
    }

    #[test]
    fn clicks_on_transport_play_by_default() {
        let mut h = harness(64);
        let o = h.run(256).float("out").to_vec();
        assert_eq!(o[0], 1.0);
        assert_eq!(o[1..].iter().map(|v| v.abs()).sum::<f32>(), 0.0);
        // Stopped: nothing.
        let mut h = harness(64);
        h.stopped();
        assert!(h.run(256).float("out").iter().all(|&v| v == 0.0));
    }

    #[test]
    fn wired_triggers_are_sample_accurate_with_gain() {
        for block in [1, 64, 512] {
            let mut h = harness(block);
            h.triggers("trigger", &[5, 5, 300, 1_000]).set("gain", -0.5);
            let o = h.run(1_200).float("out").to_vec();
            let clicks: Vec<(usize, f32)> = o
                .iter()
                .enumerate()
                .filter(|(_, v)| **v != 0.0)
                .map(|(i, v)| (i, *v))
                .collect();
            assert_eq!(
                clicks,
                vec![(5, -0.5), (300, -0.5), (1_000, -0.5)],
                "{block}"
            );
        }
    }

    #[test]
    fn gain_follows_the_frame_and_stays_finite() {
        let mut h = harness(64);
        let gain: Vec<f32> = (0..128).map(|i| i as f32).collect();
        h.triggers("trigger", &[10, 100]).set_samples("gain", gain);
        let o = h.run(128).float("out").to_vec();
        assert_eq!((o[10], o[100]), (10.0, 100.0));
        let mut h = harness(64);
        h.triggers("trigger", &[3]).set("gain", f64::NAN);
        assert!(h.run(64).float("out").iter().all(|v| v.is_finite()));
        let mut h = harness(64);
        h.triggers("trigger", &[3])
            .set_samples("gain", vec![f32::INFINITY; 64]);
        assert!(h.run(64).float("out").iter().all(|v| v.is_finite()));
    }

    #[test]
    fn inline_button_fires_once() {
        let mut h = harness(64);
        h.stopped().fire("trigger", 70);
        let o = h.run(192).float("out").to_vec();
        // The button fires at the start of the block it lands in.
        assert_eq!(o[64], 1.0);
        assert_eq!(o.iter().filter(|v| **v != 0.0).count(), 1);
        h.reset();
    }
}
