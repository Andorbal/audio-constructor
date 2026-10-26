use std::sync::Arc;

use ac_core::{
    AudioData, Channels, Config, ConfigExt, Io, MidiMsg, Node, NodeMeta, PrepareCtx, ProcessCtx,
    Rate, Signature, SocketSpec, Unit, node,
};
use ac_dsp::interp::hermite4;
use ac_dsp::util::db_to_gain;

node! {
    kind: "slicer",
    name: "Slicer",
    category: Generators,
    keywords: [
        "slice", "chop", "loop", "break", "beat", "transient", "pads", "drum loop", "cut",
        "sampler"
    ],
    /// Cuts a sound into slices and plays them one at a time: a drum loop
    /// becomes a kit on a pad grid.
    ///
    /// Wire a sound into `buffer` (a `Sample`, or a `Record` node's take).
    /// The `mode` config says where the slices are:
    ///
    /// - `equal` (the default): `count` slices of the same length;
    /// - `transients`: a slice at every hit the sound has, as many as
    ///   `sensitivity` (0 to 1) finds, up to `count`;
    /// - `markers`: at the `markers` config, fractions of the sound from 0
    ///   to 1, set by dragging in the Node tab's waveform.
    ///
    /// A `trigger` plays slice `slice` (from 0) at `velocity` (0 to 1).
    /// MIDI notes play slices too: note `low` (config, default 36) is
    /// slice 0, the next note slice 1, and so on, at the note's velocity,
    /// so a `Pads` grid plays the slices directly. One slice sounds at a
    /// time: a new one cuts the last. `rate` sets the speed (and pitch) and
    /// `gain` the level in dB. `playing` is high while a slice plays,
    /// `end` fires when one finishes, `slice` says which played last, and
    /// `slices` how many there are.
    ///
    /// The slices are found again when the sound, `count`, or
    /// `sensitivity` change; finding transients reads the whole sound, so
    /// it costs a moment the first time.
    ///
    /// **Try this:** wire a `Sample` of a drum loop into `buffer`, set
    /// `count` to 8, and wire a 4 × 2 `Pads` grid's `midi` into `midi`:
    /// eight pads, each a hit from the loop, played as hard as you hit
    /// them. Switch `mode` to `transients` for slices that start on each
    /// hit.
    pub struct Slicer {
        mode: Mode,
        markers: Vec<f32>,
        low: u8,
        channels: usize,
        rate: f64,
        /// The sound the slices were found in, and the settings used.
        sound: Option<Arc<AudioData>>,
        found: (i32, f32),
        /// Slice starts in frames, and the sound's length after them.
        starts: Vec<usize>,
        /// The slice playing: its read position, end, and gain.
        playing: Option<Playing>,
        last: i32,
        /// This block's events: (frame, slice, velocity).
        events: Vec<(usize, i32, f32)>,
    }
}

const BUFFER: usize = 0;
const MIDI: usize = 1;
const TRIGGER: usize = 2;
const SLICE: usize = 3;
const VELOCITY: usize = 4;
const COUNT: usize = 5;
const SENSITIVITY: usize = 6;
const RATE: usize = 7;
const GAIN: usize = 8;

const OUT: usize = 0;
const SLICE_OUT: usize = 1;
const SLICES: usize = 2;
const PLAYING: usize = 3;
const END: usize = 4;

/// Most slices one sound is cut into.
pub const MAX_SLICES: usize = 64;
/// Telemetry: how many slices, then each one's start as a fraction of the
/// sound (for the editor's waveform).
pub const TELEMETRY_COUNT: usize = 0;
pub const TELEMETRY_STARTS: usize = 1;
/// Shortest gap between two transients.
const MIN_GAP_SECONDS: f64 = 0.05;
/// Samples per energy window when looking for transients.
const HOP: usize = 256;
const MAX_CHANNELS: u64 = 64;
/// Most events a block holds.
const MAX_EVENTS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Equal,
    Transients,
    Markers,
}

#[derive(Clone, Copy, Debug)]
struct Playing {
    pos: f64,
    end: f64,
    level: f32,
}

fn mode(config: &Config) -> Mode {
    match config.get_str_or("mode", "equal") {
        "transients" => Mode::Transients,
        "markers" => Mode::Markers,
        _ => Mode::Equal,
    }
}

/// The `markers` config: fractions strictly inside 0..1, in order.
pub fn marker_list(config: &Config) -> Vec<f32> {
    let mut m: Vec<f32> = config
        .get("markers")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(serde_json::Value::as_f64)
                .filter(|x| x.is_finite() && *x > 0.0 && *x < 1.0)
                .map(|x| x as f32)
                .collect()
        })
        .unwrap_or_default();
    m.sort_by(f32::total_cmp);
    m.dedup();
    m.truncate(MAX_SLICES - 1);
    m
}

/// Slice starts (frames) in a sound of `len` frames cut into `count` equal
/// parts.
pub fn equal_starts(len: usize, count: usize, out: &mut Vec<usize>) {
    out.clear();
    let count = count.clamp(1, MAX_SLICES);
    for k in 0..count {
        out.push(len * k / count);
    }
}

/// Slice starts at the sound's hits: windows whose energy jumps well above
/// the last one, at least [`MIN_GAP_SECONDS`] apart, the first at 0.
/// `sensitivity` 1 finds the faintest; at most `count`.
pub fn transient_starts(data: &AudioData, sensitivity: f32, count: usize, out: &mut Vec<usize>) {
    out.clear();
    out.push(0);
    let len = data.frames();
    let rate = f64::from(data.sample_rate.max(1));
    let gap = (MIN_GAP_SECONDS * rate) as usize;
    // A jump of `threshold` in log energy counts as a hit: 6 at the least
    // sensitive, 0.5 at the most.
    let s = if sensitivity.is_finite() {
        sensitivity.clamp(0.0, 1.0)
    } else {
        0.5
    };
    let threshold = 6.0 - 5.5 * s;
    let mut prev = f32::NEG_INFINITY;
    let mut k = 0;
    while k + HOP <= len && out.len() < count.clamp(1, MAX_SLICES) {
        let mut e = 0.0f32;
        for ch in &data.channels {
            for x in ch.get(k..k + HOP).unwrap_or(&[]) {
                if x.is_finite() {
                    e += x * x;
                }
            }
        }
        let level = (e / HOP as f32 + 1e-9).ln();
        if level - prev > threshold
            && level > (1e-6f32).ln()
            && out.last().is_some_and(|&l| k >= l + gap)
        {
            out.push(k);
        }
        prev = level;
        k += HOP;
    }
}

impl Slicer {
    /// The slices in `sound` with these settings.
    fn find(&mut self, data: &AudioData, count: usize, sensitivity: f32) {
        let len = data.frames();
        match self.mode {
            Mode::Equal => equal_starts(len, count, &mut self.starts),
            Mode::Transients => transient_starts(data, sensitivity, count, &mut self.starts),
            Mode::Markers => {
                self.starts.clear();
                self.starts.push(0);
                for &m in &self.markers {
                    self.starts.push((m as f64 * len as f64) as usize);
                }
            }
        }
    }

    /// Slice `k`'s frames: start and end.
    fn bounds(&self, k: i32, len: usize) -> Option<(usize, usize)> {
        let k = usize::try_from(k).ok()?;
        let start = *self.starts.get(k)?;
        let end = self.starts.get(k + 1).copied().unwrap_or(len);
        (end > start).then_some((start, end))
    }

    fn read(ch: &[f32], p: f64) -> f32 {
        let whole = p.floor();
        let t = (p - whole) as f32;
        let k = whole as isize;
        let at = |i: isize| {
            if i < 0 {
                0.0
            } else {
                ch.get(i as usize).copied().unwrap_or(0.0)
            }
        };
        hermite4(at(k - 1), at(k), at(k + 1), at(k + 2), t)
    }
}

impl Node for Slicer {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(1 + MAX_SLICES)
    }

    fn describe(config: &Config) -> Signature {
        let channels = config.get_u64_or("channels", 2).clamp(1, MAX_CHANNELS) as u32;
        Signature::new()
            .input(SocketSpec::buffer("buffer").doc("The sound to slice."))
            .input(SocketSpec::midi("midi").doc("Notes from `low` up play the slices."))
            .input(SocketSpec::trigger("trigger").doc("Plays slice `slice`."))
            .input(
                SocketSpec::int("slice", 0)
                    .range(0.0, (MAX_SLICES - 1) as f64)
                    .doc("Which slice a trigger plays, from 0."),
            )
            .input(
                SocketSpec::float("velocity", Unit::Generic, 1.0)
                    .range(0.0, 1.0)
                    .no_smooth()
                    .doc("How hard a trigger plays, 0 to 1."),
            )
            .input(
                SocketSpec::int("count", 8)
                    .range(1.0, MAX_SLICES as f64)
                    .doc("Slices (equal), or the most to find (transients)."),
            )
            .input(
                SocketSpec::float("sensitivity", Unit::Generic, 0.5)
                    .range(0.0, 1.0)
                    .no_smooth()
                    .doc("How faint a hit still starts a slice (transients)."),
            )
            .input(
                SocketSpec::float("rate", Unit::Ratio, 1.0)
                    .range(0.25, 4.0)
                    .doc("Playback speed."),
            )
            .input(
                SocketSpec::float("gain", Unit::Db, 0.0)
                    .range(-60.0, 12.0)
                    .doc("Level in decibels."),
            )
            .output(
                SocketSpec::audio("out")
                    .channels(Channels::Fixed(channels))
                    .doc("The slice playing."),
            )
            .output(
                SocketSpec::int("slice", -1)
                    .rate(Rate::Block)
                    .doc("The slice played last, from 0."),
            )
            .output(
                SocketSpec::int("slices", 0)
                    .rate(Rate::Block)
                    .doc("How many slices there are."),
            )
            .output(
                SocketSpec::boolean("playing", false)
                    .rate(Rate::Block)
                    .doc("True while a slice plays."),
            )
            .output(SocketSpec::trigger("end").doc("Fires when a slice finishes."))
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        Slicer {
            mode: mode(config),
            markers: marker_list(config),
            low: config.get_u64_or("low", 36).min(127) as u8,
            channels: config.get_u64_or("channels", 2).clamp(1, MAX_CHANNELS) as usize,
            rate: ctx.sample_rate,
            sound: None,
            found: (-1, f32::NAN),
            starts: Vec::with_capacity(MAX_SLICES),
            playing: None,
            last: -1,
            events: Vec::with_capacity(MAX_EVENTS),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let frames = ctx.frames;
        let buffer = io.inputs.buffer(BUFFER);
        let count = io.inputs.int(COUNT).clamp(1, MAX_SLICES as i32);
        let sensitivity = io.inputs.float(SENSITIVITY).first();
        let same = match (buffer, self.sound.as_ref()) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        let settings = (count, sensitivity);
        let changed_settings = match self.mode {
            Mode::Equal => settings.0 != self.found.0,
            Mode::Transients => {
                settings.0 != self.found.0 || (settings.1 - self.found.1).abs() > 1e-3
            }
            Mode::Markers => false,
        };
        if !same || changed_settings || self.found.0 < 0 {
            if !same {
                // Whatever played came from the old sound.
                self.playing = None;
                self.sound = buffer.cloned();
            }
            match self.sound.clone() {
                Some(data) => self.find(&data, count as usize, sensitivity),
                None => self.starts.clear(),
            }
            self.found = settings;
        }
        let len = self.sound.as_ref().map_or(0, |d| d.frames());

        // This block's slice starts, in order.
        self.events.clear();
        let slice = io.inputs.int(SLICE);
        let velocity = io.inputs.float(VELOCITY);
        for &at in io.inputs.triggers(TRIGGER) {
            if self.events.len() < self.events.capacity() {
                let v = velocity.at(at as usize);
                self.events.push((
                    at as usize,
                    slice,
                    if v.is_finite() {
                        v.clamp(0.0, 1.0)
                    } else {
                        1.0
                    },
                ));
            }
        }
        for e in io.inputs.midi(MIDI) {
            if let MidiMsg::NoteOn { note, velocity, .. } = e.msg
                && velocity > 0
                && note >= self.low
                && self.events.len() < self.events.capacity()
            {
                self.events.push((
                    e.offset as usize,
                    i32::from(note - self.low),
                    f32::from(velocity.min(127)) / 127.0,
                ));
            }
        }
        // In place: a stable sort would allocate.
        self.events.sort_unstable_by_key(|e| (e.0, e.1));

        let speed = match self.sound.as_ref() {
            Some(d) if d.sample_rate > 0 && self.rate > 0.0 => f64::from(d.sample_rate) / self.rate,
            _ => 1.0,
        };
        let rate = io.inputs.float(RATE);
        let gain = io.inputs.float(GAIN);
        let channels = self.channels;
        let sound = self.sound.clone();
        let mut next = 0;
        let mut out = io.outputs.audio(OUT);
        let mut ended: [u32; 4] = [0; 4];
        let mut n_ended = 0;
        for i in 0..frames {
            while let Some(&(at, k, v)) = self.events.get(next) {
                if at > i {
                    break;
                }
                next += 1;
                if let Some((start, end)) = self.bounds(k, len) {
                    self.playing = Some(Playing {
                        pos: start as f64,
                        end: end as f64,
                        level: v,
                    });
                    self.last = k;
                }
            }
            if let (Some(p), Some(data)) = (self.playing.as_mut(), sound.as_deref()) {
                let db = gain.at(i);
                let g = if db.is_finite() {
                    db_to_gain(db.min(48.0))
                } else {
                    0.0
                } * p.level;
                for c in 0..channels.min(out.channels()) {
                    let src = data
                        .channels
                        .get(c)
                        .or_else(|| data.channels.first())
                        .map_or(&[][..], Vec::as_slice);
                    if let Some(o) = out.channel(c).get_mut(i) {
                        *o = Self::read(src, p.pos) * g;
                    }
                }
                let r = rate.at(i);
                let r = if r.is_finite() {
                    r.clamp(0.0, 16.0)
                } else {
                    1.0
                };
                p.pos += f64::from(r) * speed;
                if p.pos >= p.end {
                    self.playing = None;
                    if n_ended < ended.len() {
                        ended[n_ended] = i as u32;
                        n_ended += 1;
                    }
                }
            } else {
                for c in 0..out.channels() {
                    if let Some(o) = out.channel(c).get_mut(i) {
                        *o = 0.0;
                    }
                }
            }
        }
        let mut end = io.outputs.trigger(END);
        for &at in &ended[..n_ended] {
            end.push(at);
        }
        io.outputs.set_int(SLICE_OUT, self.last);
        io.outputs.set_int(SLICES, self.starts.len() as i32);
        io.telemetry.set(TELEMETRY_COUNT, self.starts.len() as f32);
        for (k, &start) in self.starts.iter().enumerate() {
            let f = if len > 0 {
                start as f32 / len as f32
            } else {
                0.0
            };
            io.telemetry.set(TELEMETRY_STARTS + k, f);
        }
        io.outputs.set_bool(PLAYING, self.playing.is_some());
    }

    fn reset(&mut self) {
        self.playing = None;
        self.last = -1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    /// A mono sound of `n` frames: a short click of 1.0 every `every`
    /// frames, else silence, with a value equal to the frame index /1e6
    /// so reads can be told apart.
    fn clicks(n: usize, every: usize) -> Arc<AudioData> {
        let mut ch = vec![0.0f32; n];
        for (i, s) in ch.iter_mut().enumerate() {
            if i % every < 50 {
                *s = 0.8;
            }
        }
        Arc::new(AudioData {
            sample_rate: 48_000,
            channels: vec![ch],
        })
    }

    fn ramp(n: usize) -> Arc<AudioData> {
        Arc::new(AudioData {
            sample_rate: 48_000,
            channels: vec![(0..n).map(|i| i as f32 / n as f32).collect()],
        })
    }

    #[test]
    fn equal_slices_play_by_index_and_end() {
        let mut h = Harness::new::<Slicer>(json!({ "channels": 1 }));
        h.buffer("buffer", ramp(800))
            .set("count", 4)
            .set("slice", 2)
            .triggers("trigger", &[10]);
        let out = h.run(512);
        let o = &out.audio("out")[0];
        assert_eq!(o[9], 0.0);
        assert!(
            (o[10] - 0.5).abs() < 1e-3,
            "slice 2 starts halfway: {}",
            o[10]
        );
        assert_eq!(out.triggers("end"), &[209]);
        assert_eq!(out.int("slices")[0], 4);
        assert_eq!(out.int("slice")[0], 2);
    }

    #[test]
    fn midi_notes_from_low_play_slices_at_their_velocity() {
        let mut h = Harness::new::<Slicer>(json!({ "channels": 1, "low": 36 }));
        let on = MidiMsg::NoteOn {
            channel: 0,
            note: 37,
            velocity: 64,
        };
        h.buffer("buffer", ramp(800))
            .set("count", 8)
            .midi("midi", &[(0, on)]);
        let out = h.run(64);
        let o = &out.audio("out")[0];
        // Slice 1 starts at frame 100: 0.125, at about half level.
        assert!((o[0] - 0.125 * 64.0 / 127.0).abs() < 1e-3, "{}", o[0]);
    }

    #[test]
    fn transients_start_a_slice_at_each_hit() {
        let mut starts = Vec::new();
        transient_starts(&clicks(48_000, 9_600), 0.5, 64, &mut starts);
        assert_eq!(starts.len(), 5, "{starts:?}");
        for (k, s) in starts.iter().enumerate().skip(1) {
            assert!(
                (*s as i64 - (k * 9_600) as i64).abs() <= HOP as i64,
                "{starts:?}"
            );
        }
        // At most `count`.
        transient_starts(&clicks(48_000, 9_600), 0.5, 3, &mut starts);
        assert_eq!(starts.len(), 3);
    }

    #[test]
    fn markers_cut_where_they_are_and_a_new_slice_cuts_the_last() {
        let mut h = Harness::new::<Slicer>(
            json!({ "channels": 1, "mode": "markers", "markers": [0.5, 0.25, 7, -1] }),
        );
        h.buffer("buffer", ramp(800))
            .triggers("trigger", &[0, 20])
            .set("slice", 1);
        let out = h.run(64);
        assert_eq!(out.int("slices")[0], 3);
        let o = &out.audio("out")[0];
        assert!((o[0] - 0.25).abs() < 1e-3);
        // Restarted at frame 20.
        assert!((o[20] - 0.25).abs() < 1e-3);
        assert_eq!(
            marker_list(&json!({ "markers": [0.5, 0.25, 0.5] })),
            vec![0.25, 0.5]
        );
    }

    #[test]
    fn no_sound_is_silence_and_a_slice_past_the_end_plays_nothing() {
        let mut h = Harness::new::<Slicer>(json!({ "channels": 1 }));
        h.triggers("trigger", &[0]);
        let out = h.run(64);
        assert!(out.audio("out")[0].iter().all(|&s| s == 0.0));
        assert_eq!(out.int("slices")[0], 0);
        let mut h = Harness::new::<Slicer>(json!({ "channels": 1 }));
        h.buffer("buffer", ramp(800))
            .set("count", 2)
            .set("slice", 5)
            .triggers("trigger", &[0]);
        assert!(!h.run(64).float("playing").iter().any(|&p| p > 0.0));
    }
}
