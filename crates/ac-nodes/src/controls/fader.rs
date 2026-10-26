use ac_core::{
    Channels, Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit, node,
};

use super::bounds;

node! {
    kind: "fader",
    name: "Fader",
    category: Controls,
    keywords: ["level", "channel", "mixer", "slider", "db", "strip", "console"],
    /// A mixer fader: a vertical level in decibels, with a meter beside it.
    ///
    /// Drag the fader up and down in the node (or on a panel); double-click
    /// it to go back to 0 dB. The `min` and `max` config set its range in
    /// dB (default −60 to +6). `out` is `value` kept within the range, and
    /// `gain` is the same level as a factor to multiply by, 0 at the bottom
    /// of the range, so the fader's lowest point is silence. Wire the audio
    /// it controls (or anything else) into `meter` and the fader shows its
    /// level beside the handle, so a channel's level and its reading are
    /// one card. `meter` reads its wire one block late, so it can come from
    /// after the gain the fader sets without making a cycle. Right-click
    /// `value` to bind it to a MIDI fader.
    ///
    /// **Try this:** wire `out` into a `Gain`'s `gain` (both in dB) on a
    /// channel, and the `Gain`'s output into `meter`; pin the fader to a
    /// panel, and the channel's level and its reading are one card.
    pub struct Fader {
        min: f32,
        max: f32,
    }
}

const VALUE: usize = 0;
const METER: usize = 1;

const OUT: usize = 0;
const GAIN: usize = 1;

/// Telemetry: the `meter` input's peak, left and right.
pub const TELEMETRY_METER: usize = 0;

impl Node for Fader {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(2)
    }

    fn describe(config: &Config) -> Signature {
        let (min, max) = bounds(config, "min", "max", (-60.0, 6.0));
        Signature::new()
            .input(
                SocketSpec::float("value", Unit::Db, 0.0f64.clamp(min, max))
                    .range(min, max)
                    .doc("The fader's position, in dB."),
            )
            .input(
                SocketSpec::audio("meter")
                    .channels(Channels::UpTo(2))
                    .late()
                    .doc("Audio to show beside the fader."),
            )
            .output(
                SocketSpec::float("out", Unit::Db, 0.0)
                    .poly()
                    .doc("`value`, kept within the range."),
            )
            .output(
                SocketSpec::float("gain", Unit::Generic, 1.0)
                    .poly()
                    .doc("The level as a factor: 0 at the bottom of the range."),
            )
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        let (min, max) = bounds(config, "min", "max", (-60.0, 6.0));
        Fader {
            min: min as f32,
            max: max as f32,
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let value = io.inputs.float(VALUE);
        let (min, max) = (self.min, self.max);
        // NaN reads as the bottom of the range: silence.
        let db = |i: usize| value.at(i).max(min).min(max);
        io.outputs.float(OUT).fill_with(db);
        io.outputs.float(GAIN).fill_with(|i| {
            let d = db(i);
            if d <= min { 0.0 } else { 10f32.powf(d / 20.0) }
        });
        let meter = io.inputs.audio(METER);
        let mut peaks = [0.0f32; 2];
        for (c, p) in peaks.iter_mut().enumerate().take(meter.channels()) {
            *p = meter
                .channel(c)
                .iter()
                .filter(|v| v.is_finite())
                .fold(0.0, |m, v| m.max(v.abs()));
        }
        if meter.channels() == 1 {
            peaks[1] = peaks[0];
        }
        io.telemetry.set(TELEMETRY_METER, peaks[0]);
        io.telemetry.set(TELEMETRY_METER + 1, peaks[1]);
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn out_is_the_level_and_gain_its_factor() {
        let mut h = Harness::new::<Fader>(json!({}));
        let out = h.run(32);
        assert_eq!(out.float("out")[0], 0.0);
        assert_eq!(out.float("gain")[0], 1.0);
        h.set("value", -6.0);
        let out = h.run(32);
        assert!((out.float("gain")[0] - 0.501_187).abs() < 1e-5);
        // The bottom of the range is silence, and out of range clamps.
        h.set("value", -100.0);
        let out = h.run(32);
        assert_eq!(out.float("out")[0], -60.0);
        assert_eq!(out.float("gain")[0], 0.0);
        h.set("value", 20.0);
        assert_eq!(h.run(32).float("out")[0], 6.0);
    }

    #[test]
    fn a_range_of_its_own_and_nan_is_silence() {
        let mut h = Harness::new::<Fader>(json!({"min": 10.0, "max": -30.0}));
        assert_eq!(h.signature().inputs[0].range, Some((-30.0, 10.0)));
        h.set_samples("value", vec![f32::NAN; 16]);
        let out = h.run(16);
        assert_eq!(out.float("out")[0], -30.0);
        assert_eq!(out.float("gain")[0], 0.0);
    }

    #[test]
    fn the_meter_shows_its_input_per_channel() {
        let mut h = Harness::new::<Fader>(json!({}));
        h.set_audio("meter", vec![vec![0.5; 64], vec![-0.25; 64]]);
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_METER), 0.5);
        assert_eq!(h.telemetry(TELEMETRY_METER + 1), 0.25);
        // Mono shows on both bars.
        let mut h = Harness::new::<Fader>(json!({}));
        h.set_audio("meter", vec![vec![0.75; 64]]);
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_METER + 1), 0.75);
    }
}
