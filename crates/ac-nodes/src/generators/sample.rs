use std::sync::Arc;

use ac_core::{
    AudioData, Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature,
    SocketSpec, Unit, node,
};

use super::clip::prepare;

node! {
    kind: "sample",
    name: "Sample",
    category: Generators,
    keywords: ["file", "audio file", "wav", "buffer", "sound", "load", "drum", "one shot"],
    /// An audio file as a sound to wire: its `buffer` output holds the
    /// whole file, for a `Sampler`, a `Slicer`, or anything else that
    /// plays a Buffer.
    ///
    /// Assign the file in the side panel (the `file` config, a path
    /// relative to the project). It's converted to the engine's sample rate
    /// once, when the project loads, and keeps all its channels. `length`
    /// is how long it is, in seconds. A missing file gives no sound (the
    /// Sampler it feeds plays silence) and is flagged in the editor.
    ///
    /// A Sample is how a sound gets into a block from outside: a pad or
    /// keyboard block can take its sound as a Buffer input, so one block
    /// plays any file.
    ///
    /// **Try this:** wire Sample into a `Sampler`'s `buffer` and a pad's
    /// trigger into the Sampler's `start`; then wire it into a `Slicer`
    /// too, set to eight slices, and play the slices from a `Pads` grid.
    pub struct Sample {
        data: Option<Arc<AudioData>>,
        missing: bool,
        seconds: f32,
    }
}

const BUFFER: usize = 0;
const LENGTH: usize = 1;

/// Telemetry slot: 1.0 when the file couldn't be loaded.
pub const TELEMETRY_MISSING: usize = 0;

/// Most channels kept from a file.
const MAX_CHANNELS: usize = 64;

impl Node for Sample {
    fn meta() -> NodeMeta {
        Self::base_meta().assets(&["file"]).telemetry(1)
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .output(SocketSpec::buffer("buffer").doc("The whole file."))
            .output(
                SocketSpec::float("length", Unit::Seconds, 0.0)
                    .rate(Rate::Block)
                    .doc("How long the file is."),
            )
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let path = config.get_str_or("file", "");
        let asset = if path.is_empty() {
            None
        } else {
            ctx.resources.audio_asset(path)
        };
        let missing = asset.is_none() && !path.is_empty();
        let data = asset
            .map(|a| prepare(a, MAX_CHANNELS, ctx.sample_rate))
            .filter(|d| d.frames() > 0);
        let seconds = data.as_ref().map_or(0.0, |d| {
            if d.sample_rate > 0 {
                d.frames() as f32 / d.sample_rate as f32
            } else {
                0.0
            }
        });
        Sample {
            data,
            missing,
            seconds,
        }
    }

    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        io.outputs.set_buffer(BUFFER, self.data.as_ref());
        io.outputs.set_float(LENGTH, self.seconds);
        io.telemetry
            .set(TELEMETRY_MISSING, if self.missing { 1.0 } else { 0.0 });
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{NoResources, Resources};
    use serde_json::json;

    struct Assets(Arc<AudioData>);

    impl Resources for Assets {
        fn audio_asset(&self, path: &str) -> Option<Arc<AudioData>> {
            (path == "assets/hit.wav").then(|| self.0.clone())
        }
    }

    #[test]
    fn a_file_comes_out_as_a_buffer_at_the_engine_rate() {
        let res = Assets(Arc::new(AudioData {
            sample_rate: 24_000,
            channels: vec![vec![0.5; 2_400], vec![-0.5; 2_400]],
        }));
        let mut h = Harness::with_resources::<Sample>(
            json!({ "file": "assets/hit.wav" }),
            48_000.0,
            64,
            &res,
        );
        let out = h.run(64);
        let b = out.buffer("buffer").expect("a buffer");
        assert_eq!(b.sample_rate, 48_000);
        assert_eq!(b.channels.len(), 2);
        assert!((b.frames() as i64 - 4_800).abs() <= 2, "{}", b.frames());
        assert!((out.float("length")[0] - 0.1).abs() < 1e-3);
        assert_eq!(h.telemetry(TELEMETRY_MISSING), 0.0);
    }

    #[test]
    fn a_missing_file_is_no_sound_and_flagged() {
        let mut h = Harness::with_resources::<Sample>(
            json!({ "file": "assets/gone.wav" }),
            48_000.0,
            64,
            &NoResources,
        );
        let out = h.run(64);
        assert!(out.buffer("buffer").is_none());
        assert_eq!(out.float("length")[0], 0.0);
        assert_eq!(h.telemetry(TELEMETRY_MISSING), 1.0);
        // No file chosen yet isn't missing.
        let mut h = Harness::new::<Sample>(json!({}));
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_MISSING), 0.0);
    }
}
