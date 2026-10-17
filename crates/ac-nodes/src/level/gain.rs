use ac_core::{
    Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};
use ac_dsp::util::db_to_gain;

node! {
    kind: "gain",
    name: "Gain",
    category: Level,
    keywords: ["volume", "amp", "level", "attenuate", "boost", "vca"],
    /// Scales a signal by a gain in decibels. Works on any number of channels.
    ///
    /// At -60 dB and below the output is silent.
    ///
    /// **Try this:** wire an `ADSR` through a `Map` (0..1 → -60..0 dB) into
    /// `gain` to shape a note, or drive `gain` from a slow `Oscillator` for
    /// tremolo.
    pub struct Gain {}
}

const IN: usize = 0;
const GAIN: usize = 1;
const OUT: usize = 0;

impl Node for Gain {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to scale."))
            .input(
                SocketSpec::float("gain", Unit::Db, 0.0)
                    .range(-60.0, 12.0)
                    .doc("Gain in decibels."),
            )
            .output(SocketSpec::audio("out").doc("The scaled signal."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Gain {}
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(IN);
        let gain = io.inputs.float(GAIN);
        let mut out = io.outputs.audio(OUT);
        for c in 0..out.channels() {
            let src = input.channel(c);
            let dst = out.channel(c);
            if gain.is_varying() {
                for (i, (o, s)) in dst.iter_mut().zip(src).enumerate() {
                    *o = s * db_to_gain(gain.at(i));
                }
            } else {
                let g = db_to_gain(gain.first());
                for (o, s) in dst.iter_mut().zip(src) {
                    *o = s * g;
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
    fn minus_six_db_halves_amplitude() {
        let mut h = Harness::new::<Gain>(json!({}));
        h.set_audio("in", vec![vec![1.0; 256]]);
        h.set("gain", -6.0206);
        let out = h.run(256);
        assert!((out.audio("out")[0][100] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn channel_count_follows_input() {
        let mut h = Harness::new::<Gain>(json!({}));
        h.set_audio("in", vec![vec![1.0; 64], vec![-1.0; 64]]);
        let out = h.run(64);
        assert_eq!(out.audio("out").len(), 2);
        assert_eq!(out.audio("out")[1][10], -1.0);
    }

    #[test]
    fn sample_rate_gain_modulation() {
        let mut h = Harness::new::<Gain>(json!({}));
        h.set_audio("in", vec![vec![1.0; 128]]);
        let ramp: Vec<f32> = (0..128)
            .map(|i| if i < 64 { 0.0 } else { -120.0 })
            .collect();
        h.set_samples("gain", ramp);
        let out = h.run(128);
        assert_eq!(out.audio("out")[0][10], 1.0);
        assert_eq!(out.audio("out")[0][100], 0.0);
    }

    #[test]
    fn block_size_one() {
        let mut h = Harness::with_resources::<Gain>(json!({}), 44_100.0, 1, &ac_core::NoResources);
        h.set_audio("in", vec![vec![0.25; 10]]);
        let out = h.run(10);
        assert_eq!(out.audio("out")[0], vec![0.25; 10]);
    }
}
