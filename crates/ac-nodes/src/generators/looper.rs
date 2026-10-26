use std::sync::Arc;

use ac_core::{
    AudioData, Channels, Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate,
    Signature, SocketSpec, TransportView, Unit, node,
};

node! {
    kind: "looper",
    name: "Looper",
    category: Generators,
    keywords: [
        "loop", "loop pedal", "overdub", "layer", "record", "phrase", "live looping", "undo",
        "sound on sound"
    ],
    /// A loop pedal: records what comes into `in`, plays it round and round,
    /// and layers more on top.
    ///
    /// `record` starts a new loop (the old one is gone). With `length` at 0
    /// the first pass lasts until the next trigger, `record` again (or
    /// `play`) to play it back, `overdub` to play it back and keep
    /// recording on top, `stop` to keep it stopped; with `length` set, it
    /// records that many beats and plays on its own. `overdub` toggles
    /// layering while it plays (or starts playing and layering), `play`
    /// plays from the top or ends a layer, `stop` stops. `undo` takes the
    /// last layer off; `undo` again puts it back. A loop is at most
    /// `seconds` long (config, 1–120, default 30), with `channels` (config,
    /// 1 or 2, default 2).
    ///
    /// While the transport plays, every trigger waits for the next multiple
    /// of `quantize` beats in the song (default 1; 0 acts at once), so
    /// loops start and end on the beat and a free loop's length comes out a
    /// whole number of beats. Stopped, triggers act at once. The transport
    /// stopping stops the loop too, and so do a restart, a seek, and
    /// Panic, but none of them clears it.
    ///
    /// `out` is the loop alone (wire `in` to the mix as well to hear
    /// yourself), `playing` and `recording` say what it's doing, and
    /// `position` is the seconds into the loop. `buffer` is the loop as it
    /// stood after the last recording, layer, or undo, a moment after it
    /// ends: wire it into a `Waveform` (with `position`) to see the loop, or
    /// a `Sampler` to play it.
    ///
    /// **Try this:** wire an `Audio In` into a Looper and into the `Audio
    /// Out`, the Looper's `out` into the Audio Out too, and three Buttons
    /// into `record`, `overdub`, and `undo`. Record a bar, then press
    /// overdub and play over it; undo if it went wrong.
    pub struct Looper {
        channels: usize,
        max: usize,
        rate: u32,
        state: State,
        /// The loop, and the samples under the current layer before it was
        /// added.
        audio: Vec<Vec<f32>>,
        undo: Vec<Vec<f32>>,
        /// Frames in the loop (or recorded so far on the first pass).
        len: usize,
        /// Frames the first pass stops at.
        target: usize,
        pos: usize,
        /// The layer: where it started and how many frames it has covered,
        /// and whether there is one to undo or redo.
        layer_start: usize,
        layer_len: usize,
        has_layer: bool,
        pending: Option<(Action, u64)>,
        /// Copies of the loop handed out on `buffer`, made a slice per block
        /// so a long loop doesn't cost one block all at once.
        pool: [Arc<AudioData>; 2],
        shown: Option<usize>,
        copying: Option<(usize, usize)>,
        want_publish: bool,
    }
}

const IN: usize = 0;
const RECORD: usize = 1;
const PLAY: usize = 2;
const STOP: usize = 3;
const OVERDUB: usize = 4;
const UNDO: usize = 5;
const LENGTH: usize = 6;
const QUANTIZE: usize = 7;

const OUT: usize = 0;
const BUFFER: usize = 1;
const PLAYING: usize = 2;
const RECORDING: usize = 3;
const POSITION: usize = 4;

/// Frames copied into `buffer` per block.
const CHUNK: usize = 16_384;
const MAX_SECONDS: f64 = 120.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Empty,
    /// The first pass.
    Recording,
    Playing,
    Overdubbing,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Record,
    Play,
    Stop,
    Overdub,
    Undo,
}

fn channel_count(config: &Config) -> usize {
    config.get_u64_or("channels", 2).clamp(1, 2) as usize
}

fn max_frames(config: &Config, rate: f64) -> usize {
    let s = config.get_f64_or("seconds", 30.0);
    let s = if s.is_finite() {
        s.clamp(1.0, MAX_SECONDS)
    } else {
        30.0
    };
    (s * rate.max(1.0)).round() as usize
}

impl Looper {
    fn finish_first_pass(&mut self, then: State) {
        self.len = self.len.min(self.max);
        self.pos = 0;
        self.has_layer = false;
        if self.len == 0 {
            self.state = State::Empty;
            return;
        }
        self.state = then;
        if then == State::Overdubbing {
            self.begin_layer();
        }
        self.want_publish = true;
    }

    fn begin_layer(&mut self) {
        self.layer_start = self.pos;
        self.layer_len = 0;
        self.has_layer = true;
        self.state = State::Overdubbing;
    }

    fn end_layer(&mut self) {
        self.state = State::Playing;
        self.want_publish = true;
    }

    /// Swap the layer's frames with what was under them: undo, then redo.
    fn swap_layer(&mut self) {
        if self.len == 0 {
            return;
        }
        for k in 0..self.layer_len.min(self.len) {
            let p = (self.layer_start + k) % self.len;
            for (a, u) in self.audio.iter_mut().zip(self.undo.iter_mut()) {
                if let (Some(x), Some(y)) = (a.get_mut(p), u.get_mut(p)) {
                    std::mem::swap(x, y);
                }
            }
        }
        self.want_publish = true;
    }

    fn apply(&mut self, action: Action, target: usize) {
        use State::*;
        match (action, self.state) {
            (Action::Record, Recording) => self.finish_first_pass(Playing),
            (Action::Record, _) | (Action::Overdub, Empty) => {
                self.state = Recording;
                self.len = 0;
                self.pos = 0;
                self.target = target;
                self.has_layer = false;
            }
            (Action::Overdub, Recording) => self.finish_first_pass(Overdubbing),
            (Action::Overdub, Playing) => self.begin_layer(),
            (Action::Overdub, Overdubbing) => self.end_layer(),
            (Action::Overdub, Stopped) => {
                self.pos = 0;
                self.begin_layer();
            }
            (Action::Play, Recording) => self.finish_first_pass(Playing),
            (Action::Play, Overdubbing) => self.end_layer(),
            (Action::Play, Playing | Stopped) => {
                self.state = Playing;
                self.pos = 0;
            }
            (Action::Play, Empty) | (Action::Stop, Empty | Stopped) => {}
            (Action::Stop, Recording) => self.finish_first_pass(Stopped),
            (Action::Stop, Overdubbing) => {
                self.end_layer();
                self.state = Stopped;
                self.pos = 0;
            }
            (Action::Stop, Playing) => {
                self.state = Stopped;
                self.pos = 0;
            }
            (Action::Undo, _) => {
                if self.has_layer {
                    if self.state == Overdubbing {
                        self.end_layer();
                    }
                    self.swap_layer();
                }
            }
        }
    }

    /// One frame of the loop for each channel into `out`.
    fn step(&mut self, input: &[&[f32]; 2], i: usize, out: &mut [f32; 2]) {
        *out = [0.0; 2];
        match self.state {
            State::Recording => {
                if self.len < self.max {
                    for (c, a) in self.audio.iter_mut().enumerate() {
                        let x = input[c].get(i).copied().unwrap_or(0.0);
                        if let Some(s) = a.get_mut(self.len) {
                            *s = if x.is_finite() { x } else { 0.0 };
                        }
                    }
                    self.len += 1;
                }
                if self.len >= self.target.min(self.max) {
                    self.finish_first_pass(State::Playing);
                }
            }
            State::Playing | State::Overdubbing => {
                let dub = self.state == State::Overdubbing;
                let p = self.pos;
                // A layer covers each frame once: the first time round, keep
                // what was there for undo.
                let first = dub && self.layer_len < self.len;
                for c in 0..self.channels {
                    let (Some(a), Some(u)) = (self.audio.get_mut(c), self.undo.get_mut(c)) else {
                        continue;
                    };
                    let Some(s) = a.get_mut(p) else {
                        continue;
                    };
                    out[c] = *s;
                    if dub {
                        if first && let Some(saved) = u.get_mut(p) {
                            *saved = *s;
                        }
                        let x = input[c].get(i).copied().unwrap_or(0.0);
                        if x.is_finite() {
                            *s += x;
                        }
                    }
                }
                if first {
                    self.layer_len += 1;
                }
                self.pos = (p + 1) % self.len.max(1);
            }
            State::Empty | State::Stopped => {}
        }
    }

    /// The song sample at which an action asked for at `sample` happens:
    /// the next multiple of `quantize` beats, or at once.
    fn due(t: &TransportView, sample: u64, quantize: f32) -> u64 {
        let q = f64::from(quantize);
        // NaN counts as 0: at once.
        if !t.playing || q.is_nan() || q <= 0.0 {
            return sample;
        }
        let beat = t.tempo.beat_at(sample);
        // A trigger a hair after the beat still counts as on it.
        let next = ((beat - 1e-6) / q).ceil() * q;
        t.tempo.sample_at(next).max(sample)
    }

    /// Frames the first pass records: `length` beats from `sample`, or as
    /// many as fit when it's 0.
    fn target(&self, t: &TransportView, sample: u64, length: f32) -> usize {
        let l = f64::from(length);
        if l.is_nan() || l <= 0.0 {
            return self.max;
        }
        let frames = if t.playing {
            let beat = t.tempo.beat_at(sample);
            t.tempo.sample_at(beat + l).saturating_sub(sample) as f64
        } else {
            l * t.tempo.samples_per_beat_at(sample)
        };
        (frames.round() as usize).clamp(1, self.max)
    }

    /// Copy the next slice of the loop into a free copy, and show it when
    /// it's whole.
    fn publish(&mut self) {
        if matches!(self.state, State::Recording | State::Overdubbing) {
            self.copying = None;
            return;
        }
        if std::mem::take(&mut self.want_publish) {
            let free = (0..self.pool.len())
                .find(|&k| Some(k) != self.shown && Arc::strong_count(&self.pool[k]) == 1);
            match free {
                Some(k) => self.copying = Some((k, 0)),
                // Every copy is still held downstream: try next block.
                None => self.want_publish = true,
            }
        }
        let Some((k, done)) = self.copying else {
            return;
        };
        let len = self.len;
        let Some(data) = Arc::get_mut(&mut self.pool[k]) else {
            self.copying = None;
            self.want_publish = true;
            return;
        };
        let to = (done + CHUNK).min(len);
        for (dst, src) in data.channels.iter_mut().zip(&self.audio) {
            // Within the capacity reserved at creation: no allocation.
            dst.resize(len, 0.0);
            if let (Some(d), Some(s)) = (dst.get_mut(done..to), src.get(done..to)) {
                d.copy_from_slice(s);
            }
        }
        if to >= len {
            self.copying = None;
            self.shown = (len > 0).then_some(k);
        } else {
            self.copying = Some((k, to));
        }
    }
}

impl Node for Looper {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let n = channel_count(config) as u32;
        Signature::new()
            .input(
                SocketSpec::audio("in")
                    .channels(Channels::Fixed(n))
                    .doc("The sound to record."),
            )
            .input(SocketSpec::trigger("record").doc("Starts a new loop, or ends the first pass."))
            .input(SocketSpec::trigger("play").doc("Plays from the top, or ends a layer."))
            .input(SocketSpec::trigger("stop").doc("Stops, keeping the loop."))
            .input(SocketSpec::trigger("overdub").doc("Starts or ends a layer on top."))
            .input(SocketSpec::trigger("undo").doc("Takes the last layer off, or puts it back."))
            .input(
                SocketSpec::float("length", Unit::Beats, 0.0)
                    .range(0.0, 64.0)
                    .no_smooth()
                    .doc("Beats the first pass records; 0 until the next trigger."),
            )
            .input(
                SocketSpec::float("quantize", Unit::Beats, 1.0)
                    .range(0.0, 16.0)
                    .no_smooth()
                    .doc("Triggers wait for the next multiple of this in the song; 0 at once."),
            )
            .output(
                SocketSpec::audio("out")
                    .channels(Channels::Fixed(n))
                    .doc("The loop."),
            )
            .output(SocketSpec::buffer("buffer").doc("The loop, after each recording or layer."))
            .output(SocketSpec::boolean("playing", false).doc("Playing the loop (or layering)."))
            .output(SocketSpec::boolean("recording", false).doc("Recording a first pass or layer."))
            .output(
                SocketSpec::float("position", Unit::Seconds, 0.0)
                    .rate(Rate::Block)
                    .doc("Seconds into the loop."),
            )
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let channels = channel_count(config);
        let max = max_frames(config, ctx.sample_rate);
        let rate = ctx.sample_rate.round().max(1.0) as u32;
        let copy = || {
            Arc::new(AudioData {
                sample_rate: rate,
                channels: (0..channels).map(|_| Vec::with_capacity(max)).collect(),
            })
        };
        Looper {
            channels,
            max,
            rate,
            state: State::Empty,
            audio: vec![vec![0.0; max]; channels],
            undo: vec![vec![0.0; max]; channels],
            len: 0,
            target: max,
            pos: 0,
            layer_start: 0,
            layer_len: 0,
            has_layer: false,
            pending: None,
            pool: [copy(), copy()],
            shown: None,
            copying: None,
            want_publish: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let t = ctx.transport;
        let frames = io.frames();
        let input = io.inputs.audio(IN);
        let chans = [input.channel(0), input.channel(1)];
        let (length, quantize) = (io.inputs.float(LENGTH), io.inputs.float(QUANTIZE));
        let lists = [
            (Action::Record, io.inputs.triggers(RECORD)),
            (Action::Play, io.inputs.triggers(PLAY)),
            (Action::Stop, io.inputs.triggers(STOP)),
            (Action::Overdub, io.inputs.triggers(OVERDUB)),
            (Action::Undo, io.inputs.triggers(UNDO)),
        ];
        if t.stopped {
            self.pending = None;
            self.apply(Action::Stop, self.max);
        }
        let mut out = io.outputs.audio(OUT);
        let mut next = [0usize; 5];
        let mut frame = [0.0f32; 2];
        for i in 0..frames {
            let sample = t.position + if t.playing { i as u64 } else { 0 };
            // Triggers at this frame, in the order of the inputs.
            for (k, (action, list)) in lists.iter().enumerate() {
                while next[k] < list.len() && list[next[k]] as usize <= i {
                    next[k] += 1;
                    let due = Self::due(&t, sample, quantize.at(i));
                    self.pending = Some((*action, due));
                }
            }
            if let Some((action, due)) = self.pending
                && due <= sample
            {
                self.pending = None;
                let target = self.target(&t, sample, length.at(i));
                self.apply(action, target);
            }
            self.step(&chans, i, &mut frame);
            for (c, &v) in frame
                .iter()
                .enumerate()
                .take(out.channels().min(self.channels))
            {
                if let Some(s) = out.channel(c).get_mut(i) {
                    *s = v;
                }
            }
        }
        self.publish();
        io.outputs
            .set_buffer(BUFFER, self.shown.map(|k| &self.pool[k]));
        io.outputs.set_bool(
            PLAYING,
            matches!(self.state, State::Playing | State::Overdubbing),
        );
        io.outputs.set_bool(
            RECORDING,
            matches!(self.state, State::Recording | State::Overdubbing),
        );
        let at = if self.state == State::Recording {
            self.len
        } else {
            self.pos
        };
        io.outputs
            .set_float(POSITION, at as f32 / self.rate.max(1) as f32);
    }

    /// The loop is what was played into it, so a restart, a seek, or a
    /// panic stops it but keeps it; what was waiting for its beat is
    /// dropped.
    fn reset(&mut self) {
        self.pending = None;
        self.apply(Action::Stop, self.max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{NoResources, TempoMap};
    use serde_json::json;

    /// 1 kHz so beats are short: 120 BPM is 500 frames a beat.
    const RATE: f64 = 1_000.0;
    const BEAT: u64 = 500;

    fn harness(config: serde_json::Value, block: usize) -> Harness {
        let mut h = Harness::with_resources::<Looper>(config, RATE, block, &NoResources);
        h.tempo(TempoMap::constant(RATE, 120.0));
        h
    }

    /// A ramp counting frames from 1, so recorded frames are recognizable.
    fn ramp(n: usize) -> Vec<f32> {
        (1..=n).map(|k| k as f32).collect()
    }

    #[test]
    fn records_a_free_loop_on_the_beat_and_plays_it_round() {
        let mut h = harness(json!({ "channels": 1 }), 64);
        h.set_audio("in", vec![ramp(4 * BEAT as usize)]);
        // Pressed a little after beat 0 and a little before beat 2: the
        // loop runs from beat 1 to beat 2.
        h.triggers("record", &[30, 2 * BEAT - 20]);
        let out = h.run(4 * BEAT as usize);
        let o = &out.audio("out")[0];
        assert!(
            o[..2 * BEAT as usize].iter().all(|&x| x == 0.0),
            "silent while recording"
        );
        // Recorded frames BEAT..2*BEAT of the ramp, played from beat 2.
        assert_eq!(o[2 * BEAT as usize], (BEAT + 1) as f32);
        assert_eq!(o[3 * BEAT as usize - 1], (2 * BEAT) as f32);
        assert_eq!(o[3 * BEAT as usize], (BEAT + 1) as f32, "round again");
        assert_eq!(*out.float("playing").last().unwrap(), 1.0);
        assert_eq!(*out.float("recording").last().unwrap(), 0.0);
    }

    #[test]
    fn a_set_length_ends_the_first_pass_on_its_own() {
        let mut h = harness(json!({ "channels": 1 }), 37);
        h.set_audio("in", vec![ramp(2_000)]);
        h.set("length", 2.0).set("quantize", 0.0);
        h.triggers("record", &[100]);
        let out = h.run(2_000);
        let o = &out.audio("out")[0];
        // Recorded 100..1100, playing from 1100.
        assert_eq!(o[1_099], 0.0);
        assert_eq!(o[1_100], 101.0);
        assert_eq!(o[1_999], 1_000.0);
    }

    #[test]
    fn overdub_layers_and_undo_takes_it_off_and_puts_it_back() {
        let mut h = harness(json!({ "channels": 1 }), 64);
        h.set("quantize", 0.0).set("length", 0.2);
        // A 100-frame loop of ones, then a pass of twos layered on top.
        let input = [vec![1.0; 100], vec![2.0; 100], vec![0.0; 300]].concat();
        h.set_audio("in", vec![input]);
        h.triggers("record", &[0]);
        h.run(100);
        h.triggers("overdub", &[100]);
        let out = h.run(100);
        assert!(
            out.audio("out")[0].iter().all(|&x| x == 1.0),
            "hears the loop under it"
        );
        assert_eq!(out.float("recording")[50], 1.0);
        h.triggers("play", &[200]);
        let out = h.run(100);
        assert!(
            out.audio("out")[0].iter().all(|&x| x == 3.0),
            "{:?}",
            &out.audio("out")[0][..4]
        );
        h.triggers("undo", &[300]);
        let out = h.run(100);
        assert!(out.audio("out")[0].iter().all(|&x| x == 1.0));
        h.triggers("undo", &[400]);
        let out = h.run(100);
        assert!(out.audio("out")[0].iter().all(|&x| x == 3.0), "redo");
    }

    #[test]
    fn stop_and_play_and_the_transport() {
        let mut h = harness(json!({ "channels": 1 }), 64);
        h.set("quantize", 0.0).set("length", 0.2);
        h.set_audio("in", vec![ramp(100)]);
        h.triggers("record", &[0]).triggers("stop", &[150]);
        let out = h.run(300);
        let o = &out.audio("out")[0];
        assert_eq!(o[100], 1.0);
        assert!(o[150..].iter().all(|&x| x == 0.0));
        assert_eq!(out.float("playing")[299], 0.0);
        // Play starts from the top.
        h.triggers("play", &[310]);
        let out = h.run(100);
        assert_eq!(out.audio("out")[0][10], 1.0);
        // The transport stopping stops it; a restart keeps the loop.
        h.stop();
        let out = h.run(64);
        assert_eq!(out.float("playing")[0], 0.0);
        h.reset();
        h.play();
        h.triggers("play", &[464 + 5]);
        let out = h.run(64);
        assert_eq!(out.audio("out")[0][5], 1.0, "kept across a restart");
    }

    #[test]
    fn buffer_shows_the_loop_after_it_is_recorded() {
        let mut h = harness(json!({ "channels": 2 }), 64);
        h.set("quantize", 0.0).set("length", 0.1);
        h.set_audio("in", vec![ramp(64), vec![0.5; 64]]);
        h.triggers("record", &[0]);
        assert!(h.run(32).buffer("buffer").is_none(), "not while recording");
        let out = h.run(64);
        let b = out.buffer("buffer").expect("published");
        assert_eq!(b.frames(), 50);
        assert_eq!(b.channels[0][..3], [1.0, 2.0, 3.0]);
        assert_eq!(b.channels[1][49], 0.5);
        assert_eq!(b.sample_rate, 1_000);
    }

    #[test]
    fn a_long_loop_is_published_a_slice_at_a_time() {
        let mut h = Harness::with_resources::<Looper>(
            json!({ "channels": 1, "seconds": 2 }),
            48_000.0,
            512,
            &NoResources,
        );
        h.set("quantize", 0.0).set("length", 2.0);
        h.set_audio("in", vec![vec![0.25; 48_000]]);
        h.triggers("record", &[0]);
        // 48 000 frames at 16 384 a block: published in the third block
        // after the loop ends.
        assert!(h.run(48_000 + 512).buffer("buffer").is_none());
        let out = h.run(3 * 512);
        assert_eq!(out.buffer("buffer").map(|b| b.frames()), Some(48_000));
    }

    #[test]
    fn a_full_loop_stops_recording_at_its_limit() {
        let mut h = harness(json!({ "channels": 1, "seconds": 1 }), 64);
        h.set("quantize", 0.0);
        h.set_audio("in", vec![ramp(1_500)]);
        h.triggers("record", &[0]);
        let out = h.run(1_500);
        assert_eq!(out.audio("out")[0][1_000], 1.0, "a second, then plays");
    }
}
