use ac_core::{
    Channels, Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec,
    node,
};

node! {
    kind: "audio_out",
    name: "Audio Out",
    category: Io,
    keywords: ["output", "speakers", "headphones", "monitor", "main", "device"],
    /// Sends audio to the device outputs.
    ///
    /// The `channel` config picks the first device output (counting from 0)
    /// and `channels` chooses mono (1) or a stereo pair (2, the default). A
    /// mono signal wired into a stereo Audio Out plays on both channels.
    ///
    /// Several Audio Out nodes can share outputs: their signals add up, so a
    /// main mix and a click track can both reach the speakers. Outputs the
    /// device doesn't have are skipped. Samples that aren't finite numbers
    /// (the result of a runaway feedback loop or a division by zero) are
    /// sent as silence rather than reaching the speakers.
    ///
    /// The node's meter shows the peak level of its first two channels.
    ///
    /// **Try this:** add a second Audio Out on outputs 3–4 (`channel` = 2)
    /// for a headphone cue mix, or wire an `Oscillator` straight in at a low
    /// `Gain` to check that sound comes out at all.
    pub struct AudioOut {
        first: usize,
    }
}

const IN: usize = 0;

/// A node spans one device output or a stereo pair.
const MAX_CHANNELS: u64 = 2;

fn channel_count(config: &Config) -> u32 {
    config.get_u64_or("channels", 2).clamp(1, MAX_CHANNELS) as u32
}

impl Node for AudioOut {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(2).host_output()
    }

    fn describe(config: &Config) -> Signature {
        Signature::new().input(
            SocketSpec::audio("in")
                .channels(Channels::Fixed(channel_count(config)))
                .doc("Signal to play."),
        )
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        AudioOut {
            first: usize::try_from(config.get_u64_or("channel", 0)).unwrap_or(usize::MAX),
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.audio(IN);
        let mut peaks = [0.0f32; 2];
        for c in 0..input.channels() {
            let src = input.channel(c);
            if let Some(p) = peaks.get_mut(c) {
                *p = src.iter().fold(0.0, |m, s| m.max(s.abs()));
            }
            if let Some(dst) = io.host.output_channel(self.first.saturating_add(c)) {
                for (o, s) in dst.iter_mut().zip(src) {
                    if s.is_finite() {
                        *o += s;
                    }
                }
            }
        }
        // A mono node shows the same level on both meter bars.
        if input.channels() == 1 {
            peaks[1] = peaks[0];
        }
        io.telemetry.set(0, peaks[0]);
        io.telemetry.set(1, peaks[1]);
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{
        AutoFlags, HostAccess, InputSlot, Inputs, NoResources, Outputs, Phase, TelemetryAccess,
        TempoMap, TransportView,
    };
    use serde_json::json;
    use std::sync::atomic::AtomicU32;

    fn ramp(n: usize, scale: f32) -> Vec<f32> {
        (0..n).map(|i| i as f32 * scale).collect()
    }

    fn node(config: serde_json::Value) -> AudioOut {
        let ctx = PrepareCtx {
            sample_rate: 48_000.0,
            max_block: 64,
            seed: 0,
            node_key: "t",
            name: "Audio Out",
            resources: &NoResources,
        };
        AudioOut::new(&config, &ctx)
    }

    /// Runs one block into a caller-owned device buffer (channel-major,
    /// `frames` per channel), which the harness can't do: it starts every
    /// block from silence.
    fn process_into(
        node: &mut AudioOut,
        input: &[Vec<f32>],
        node_channels: usize,
        device: &mut [f32],
        device_channels: usize,
    ) {
        let frames = input[0].len();
        let chan_table: Vec<u32> = (0..node_channels)
            .map(|c| (c % input.len()) as u32)
            .collect();
        let slots = [InputSlot::Audio {
            start: 0,
            count: node_channels as u32,
        }];
        let telemetry: Vec<AtomicU32> = (0..2).map(|_| AtomicU32::new(0)).collect();
        let tempo = TempoMap::default();
        let ctx = ProcessCtx {
            frames,
            sample_rate: 48_000.0,
            transport: TransportView::stopped_at_zero(&tempo),
            phase: Phase::Main,
            engine: Default::default(),
        };
        let mut io = Io {
            inputs: Inputs {
                slots: &slots,
                frames,
                params: &[],
                scalars: &[],
                ints: &[],
                samples: input,
                chan_table: &chan_table,
                triggers: &[],
                midi: &[],
                spectra: &[],
                buffers: &[],
                texts: &[],
                auto: AutoFlags::default(),
                local: Default::default(),
            },
            outputs: Outputs {
                slots: &[],
                frames,
                samples: &mut [],
                scalars: &mut [],
                ints: &mut [],
                triggers: &mut [],
                midi: &mut [],
                spectra: &mut [],
                buffers: &mut [],
                texts: &mut [],
            },
            host: HostAccess {
                input: &[],
                in_channels: 0,
                output: device,
                out_channels: device_channels,
                stride: frames,
                offset: 0,
                frames,
                midi: &[],
                midi_out: None,
                controls: &[],
                osc_out: None,
            },
            telemetry: TelemetryAccess {
                values: &telemetry,
                scope: &[],
                scope_pos: None,
            },
        };
        node.process(&ctx, &mut io);
    }

    #[test]
    fn writes_stereo_to_the_first_two_outputs() {
        let mut h = Harness::new::<AudioOut>(json!({}));
        h.host_outputs(2);
        h.set_audio("in", vec![ramp(200, 1.0), ramp(200, -1.0)]);
        let out = h.run(200);
        assert_eq!(out.host_out[0], ramp(200, 1.0));
        assert_eq!(out.host_out[1], ramp(200, -1.0));
    }

    #[test]
    fn mono_signal_is_upmixed_to_both_outputs() {
        let mut h = Harness::new::<AudioOut>(json!({}));
        h.host_outputs(2);
        h.set_audio("in", vec![vec![0.5; 64]]);
        let out = h.run(64);
        assert_eq!(out.host_out[0], vec![0.5; 64]);
        assert_eq!(out.host_out[1], vec![0.5; 64]);
    }

    #[test]
    fn channel_offset_and_mono_output() {
        let mut h = Harness::new::<AudioOut>(json!({"channel": 2, "channels": 1}));
        h.host_outputs(4);
        h.set_audio("in", vec![vec![0.25; 64]]);
        let out = h.run(64);
        assert_eq!(out.host_out[0], vec![0.0; 64]);
        assert_eq!(out.host_out[1], vec![0.0; 64]);
        assert_eq!(out.host_out[2], vec![0.25; 64]);
        assert_eq!(out.host_out[3], vec![0.0; 64]);
    }

    #[test]
    fn out_of_range_channels_are_ignored() {
        let mut h = Harness::new::<AudioOut>(json!({"channel": 1}));
        h.host_outputs(2);
        h.set_audio("in", vec![vec![0.5; 64], vec![0.75; 64]]);
        let out = h.run(64);
        assert_eq!(out.host_out[0], vec![0.0; 64]);
        assert_eq!(out.host_out[1], vec![0.5; 64]);

        let mut h = Harness::new::<AudioOut>(json!({"channel": u64::MAX}));
        h.host_outputs(2);
        h.set_audio("in", vec![vec![0.5; 64]]);
        let out = h.run(64);
        assert_eq!(out.host_out[0], vec![0.0; 64]);

        let mut h = Harness::new::<AudioOut>(json!({}));
        h.set_audio("in", vec![vec![0.5; 64]]);
        assert!(h.run(64).host_out.is_empty());
    }

    #[test]
    fn adds_into_outputs_shared_with_other_nodes() {
        let mut device = vec![0.0f32; 2 * 4];
        let mut main = node(json!({}));
        let mut click = node(json!({"channels": 1}));
        process_into(&mut main, &[vec![0.5; 4], vec![0.25; 4]], 2, &mut device, 2);
        process_into(&mut click, &[vec![0.125; 4]], 1, &mut device, 2);
        assert_eq!(&device[..4], &[0.625; 4]);
        assert_eq!(&device[4..], &[0.25; 4]);
    }

    #[test]
    fn non_finite_samples_become_silence() {
        let mut device = vec![0.1f32; 4];
        let mut n = node(json!({"channels": 1}));
        let input = vec![vec![f32::NAN, f32::INFINITY, 0.5, f32::NEG_INFINITY]];
        process_into(&mut n, &input, 1, &mut device, 1);
        assert_eq!(device, vec![0.1, 0.1, 0.6, 0.1]);
    }

    #[test]
    fn meters_report_block_peaks() {
        let mut h = Harness::new::<AudioOut>(json!({}));
        h.host_outputs(2);
        h.set_audio("in", vec![vec![-0.8; 64], vec![0.3; 64]]);
        h.run(64);
        assert!((h.telemetry(0) - 0.8).abs() < 1e-6);
        assert!((h.telemetry(1) - 0.3).abs() < 1e-6);

        let mut h = Harness::new::<AudioOut>(json!({"channels": 1}));
        h.set_audio("in", vec![vec![0.4; 64]]);
        h.run(64);
        assert!((h.telemetry(0) - 0.4).abs() < 1e-6);
        assert!((h.telemetry(1) - 0.4).abs() < 1e-6);
    }
}
