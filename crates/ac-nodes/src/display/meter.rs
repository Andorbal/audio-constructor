use ac_core::{Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node};

use super::pass_through;

node! {
    kind: "meter",
    name: "Meter",
    category: Display,
    keywords: ["level", "vu", "peak", "rms"],
    /// Shows the peak and RMS level of a signal, and passes it through
    /// unchanged so it can sit inline anywhere in a chain.
    ///
    /// The node measures each block it processes and the editor draws the
    /// readings, adding peak hold and meter ballistics from successive
    /// blocks. Stereo signals get two bars; a mono signal shows the same
    /// reading on both. With more than two channels only the first two are
    /// metered, but all of them pass through.
    ///
    /// Non-finite samples (NaN or infinity) count as silence in the readings;
    /// `out` is still an exact copy of `in`.
    ///
    /// **Try this:** drop a `Meter` on the wire just before `AudioOut` to
    /// watch your headroom, or put one either side of a `Gain` to see
    /// exactly how much it changes the level.
    pub struct Meter {}
}

const IN: usize = 0;
const OUT: usize = 0;

/// Telemetry slots: peak and RMS for the first two channels.
const PEAK_0: usize = 0;
const RMS_0: usize = 1;
const PEAK_1: usize = 2;
const RMS_1: usize = 3;

impl Node for Meter {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(4)
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::audio("in").doc("Signal to measure."))
            .output(SocketSpec::audio("out").doc("The same signal, unchanged."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Meter {}
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(IN);
        pass_through(&input, &mut io.outputs.audio(OUT));
        let (p0, r0) = levels(input.channel(0));
        let (p1, r1) = if input.channels() > 1 {
            levels(input.channel(1))
        } else {
            (p0, r0)
        };
        io.telemetry.set(PEAK_0, p0);
        io.telemetry.set(RMS_0, r0);
        io.telemetry.set(PEAK_1, p1);
        io.telemetry.set(RMS_1, r1);
    }

    fn reset(&mut self) {}
}

/// Peak absolute value and RMS of one block, ignoring non-finite samples.
fn levels(x: &[f32]) -> (f32, f32) {
    let mut peak = 0.0f32;
    let mut sum = 0.0f64;
    for &v in x {
        let v = if v.is_finite() { v } else { 0.0 };
        peak = peak.max(v.abs());
        sum += f64::from(v) * f64::from(v);
    }
    let rms = if x.is_empty() {
        0.0
    } else {
        (sum / x.len() as f64).sqrt() as f32
    };
    (peak, rms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn passes_through_unchanged() {
        let mut h = Harness::new::<Meter>(json!({}));
        let l: Vec<f32> = (0..128).map(|i| (i as f32 * 0.37).sin()).collect();
        let r: Vec<f32> = l.iter().map(|v| -v * 0.5).collect();
        h.set_audio("in", vec![l.clone(), r.clone()]);
        let out = h.run(128);
        assert_eq!(out.audio("out"), &[l, r]);
    }

    #[test]
    fn reports_peak_and_rms_per_channel() {
        let mut h = Harness::new::<Meter>(json!({}));
        let square: Vec<f32> = (0..64)
            .map(|i| if i % 2 == 0 { 0.5 } else { -0.5 })
            .collect();
        let quiet = vec![0.1; 64];
        h.set_audio("in", vec![square, quiet]);
        h.run(64);
        assert_eq!(h.telemetry(PEAK_0), 0.5);
        assert!((h.telemetry(RMS_0) - 0.5).abs() < 1e-6);
        assert!((h.telemetry(PEAK_1) - 0.1).abs() < 1e-6);
        assert!((h.telemetry(RMS_1) - 0.1).abs() < 1e-6);
    }

    #[test]
    fn mono_shows_on_both_channels() {
        let mut h = Harness::new::<Meter>(json!({}));
        h.set_audio("in", vec![vec![0.25; 64]]);
        h.run(64);
        assert_eq!(h.telemetry(PEAK_1), h.telemetry(PEAK_0));
        assert_eq!(h.telemetry(RMS_1), h.telemetry(RMS_0));
        assert_eq!(h.telemetry(PEAK_0), 0.25);
    }

    #[test]
    fn readings_are_for_the_latest_block() {
        let mut h = Harness::new::<Meter>(json!({}));
        let mut x = vec![0.9; 64];
        x.extend(vec![0.2; 64]);
        h.set_audio("in", vec![x]);
        h.run(128);
        assert!((h.telemetry(PEAK_0) - 0.2).abs() < 1e-6);
    }

    #[test]
    fn non_finite_samples_read_as_silence() {
        let mut h = Harness::new::<Meter>(json!({}));
        let mut x = vec![0.5; 64];
        x[3] = f32::INFINITY;
        x[4] = f32::NAN;
        h.set_audio("in", vec![x]);
        h.run(64);
        assert_eq!(h.telemetry(PEAK_0), 0.5);
        assert!(h.telemetry(RMS_0).is_finite());
    }
}
