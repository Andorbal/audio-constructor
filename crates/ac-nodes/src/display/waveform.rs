use std::sync::Arc;

use ac_core::{
    AudioData, Config, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate, Signature, SocketSpec,
    Unit, node,
};

node! {
    kind: "waveform",
    name: "Waveform",
    category: Display,
    keywords: ["wave", "sample", "buffer", "display", "start", "end", "loop points", "editor"],
    /// Draws a sound, with where it's playing and handles for the points
    /// that pick which part plays.
    ///
    /// Wire a sound into `buffer` (a `Sample`, a `Record` node's take) and
    /// the card shows its waveform. Wire a Sampler's or a Clip's `position`
    /// into `position` (seconds) for a playhead.
    ///
    /// The four handles, `start_point`, `end_point`, `loop_start`, and
    /// `loop_end` (fractions of the sound, 0 to 1), are dragged on the card
    /// (or on a panel) and come out of the outputs of the same names: wire
    /// them into a `Sampler`'s points and set its start and end by eye. The
    /// end handles can't cross the start ones.
    ///
    /// **Try this:** wire a `Sample` into Waveform and into a `Sampler`'s
    /// `buffer`, Waveform's four outputs into the Sampler's four points,
    /// and the Sampler's `position` back into Waveform's `position`; drag
    /// the handles while a pad plays it.
    pub struct Waveform {
        sound: Option<Arc<AudioData>>,
        seconds: f32,
        /// Lowest and highest sample in each column.
        overview: Vec<(f32, f32)>,
    }
}

const BUFFER: usize = 0;
const POSITION: usize = 1;
/// The handles' inputs, and their outputs in the same order.
const HANDLES: usize = 2;
const HANDLE_NAMES: [&str; 4] = ["start_point", "end_point", "loop_start", "loop_end"];

/// Columns in the overview.
pub const COLUMNS: usize = 128;
/// Telemetry: the sound's length in seconds, the playhead as a fraction
/// (negative when there's none), then each column's lowest and highest.
pub const TELEMETRY_SECONDS: usize = 0;
pub const TELEMETRY_PLAYHEAD: usize = 1;
pub const TELEMETRY_COLUMNS: usize = 2;

/// Lowest and highest sample of every channel in each of `columns` equal
/// parts of `data`.
pub fn overview(data: &AudioData, out: &mut [(f32, f32)]) {
    let len = data.frames();
    let n = out.len().max(1);
    for (k, col) in out.iter_mut().enumerate() {
        let (a, b) = (
            len * k / n,
            (len * (k + 1) / n).max(len * k / n + 1).min(len),
        );
        let mut lo = 0.0f32;
        let mut hi = 0.0f32;
        for ch in &data.channels {
            for &x in ch.get(a..b).unwrap_or(&[]) {
                if x.is_finite() {
                    lo = lo.min(x);
                    hi = hi.max(x);
                }
            }
        }
        *col = (lo, hi);
    }
}

impl Node for Waveform {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(TELEMETRY_COLUMNS + 2 * COLUMNS)
    }

    fn describe(_: &Config) -> Signature {
        let mut sig = Signature::new()
            .input(SocketSpec::buffer("buffer").doc("The sound to draw."))
            .input(
                SocketSpec::float("position", Unit::Seconds, -1.0)
                    .range(0.0, 60.0)
                    .doc("Where it's playing, for the playhead."),
            );
        let defaults = [0.0, 1.0, 0.0, 1.0];
        for (name, d) in HANDLE_NAMES.iter().zip(defaults) {
            sig = sig.input(
                SocketSpec::float(*name, Unit::Generic, d)
                    .range(0.0, 1.0)
                    .no_smooth()
                    .doc("A handle, dragged on the card (fraction of the sound)."),
            );
        }
        for name in HANDLE_NAMES {
            sig = sig.output(
                SocketSpec::float(name, Unit::Generic, 0.0)
                    .rate(Rate::Block)
                    .doc("The handle's place, for a Sampler's point."),
            );
        }
        sig
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Waveform {
            sound: None,
            seconds: 0.0,
            overview: vec![(0.0, 0.0); COLUMNS],
        }
    }

    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let buffer = io.inputs.buffer(BUFFER);
        let same = match (buffer, self.sound.as_ref()) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.sound = buffer.cloned();
            match self.sound.as_deref() {
                Some(d) => {
                    overview(d, &mut self.overview);
                    self.seconds = if d.sample_rate > 0 {
                        d.frames() as f32 / d.sample_rate as f32
                    } else {
                        0.0
                    };
                }
                None => {
                    self.overview.fill((0.0, 0.0));
                    self.seconds = 0.0;
                }
            }
            for (k, &(lo, hi)) in self.overview.iter().enumerate() {
                io.telemetry.set(TELEMETRY_COLUMNS + 2 * k, lo);
                io.telemetry.set(TELEMETRY_COLUMNS + 2 * k + 1, hi);
            }
        }
        io.telemetry.set(TELEMETRY_SECONDS, self.seconds);
        let pos = io.inputs.float(POSITION).first();
        let head = if pos.is_finite() && pos >= 0.0 && self.seconds > 0.0 {
            (pos / self.seconds).min(1.0)
        } else {
            -1.0
        };
        io.telemetry.set(TELEMETRY_PLAYHEAD, head);
        // The handles, kept in range, the ends after the starts.
        let h = |k: usize| {
            let v = io.inputs.float(HANDLES + k).first();
            if v.is_finite() {
                v.clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        let (start, loop_start) = (h(0), h(2));
        let (end, loop_end) = (h(1).max(start), h(3).max(loop_start));
        for (k, v) in [start, end, loop_start, loop_end].into_iter().enumerate() {
            io.outputs.set_float(k, v);
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
    fn the_overview_holds_each_columns_extremes() {
        let mut ch = vec![0.0f32; 1_280];
        ch[5] = 0.9;
        ch[1_279] = -0.4;
        let d = AudioData {
            sample_rate: 48_000,
            channels: vec![ch],
        };
        let mut cols = vec![(0.0, 0.0); COLUMNS];
        overview(&d, &mut cols);
        assert_eq!(cols[0], (0.0, 0.9));
        assert_eq!(cols[COLUMNS - 1], (-0.4, 0.0));
        assert_eq!(cols[50], (0.0, 0.0));
    }

    #[test]
    fn telemetry_shows_the_sound_and_playhead_and_handles_pass_through() {
        let sound = Arc::new(AudioData {
            sample_rate: 48_000,
            channels: vec![vec![0.5; 48_000]],
        });
        let mut h = Harness::new::<Waveform>(json!({}));
        h.buffer("buffer", sound)
            .set("position", 0.25)
            .set("start_point", 0.2)
            .set("end_point", 0.1)
            .set("loop_end", 0.7);
        let out = h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_SECONDS), 1.0);
        assert_eq!(h.telemetry(TELEMETRY_PLAYHEAD), 0.25);
        assert_eq!(h.telemetry(TELEMETRY_COLUMNS + 1), 0.5);
        assert_eq!(out.float("start_point")[0], 0.2);
        assert_eq!(out.float("end_point")[0], 0.2, "the end can't come first");
        assert_eq!(out.float("loop_end")[0], 0.7);
    }

    #[test]
    fn no_sound_no_playhead() {
        let mut h = Harness::new::<Waveform>(json!({}));
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_SECONDS), 0.0);
        assert_eq!(h.telemetry(TELEMETRY_PLAYHEAD), -1.0);
    }
}
