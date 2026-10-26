use ac_core::{
    BoolIn, Config, ConfigExt, Io, MidiMsg, Node, NodeMeta, PrepareCtx, ProcessCtx, Rate,
    Signature, SocketSpec, SongLock, TempoMap, Unit, Value, node,
};

use super::notes::{RELEASE_VELOCITY, Sender, note_off};
use crate::segments::Runs;
use crate::song;

node! {
    kind: "midi_pattern",
    name: "MIDI Pattern",
    category: Sequencing,
    keywords: [
        "piano roll", "clip", "sequence", "riff", "melody", "notes", "song", "loop",
        "sequencer", "midi clip"
    ],
    /// A piano-roll clip of notes in beats that plays as MIDI, from a one-bar riff to a whole song.
    ///
    /// The clip is `length` beats long (config, 0.25–1024, default 4; a
    /// beat is a quarter note). Its notes are the `notes` config, edited in
    /// the piano roll: each is `[start, length, note, velocity]`, with
    /// `start` and `length` in beats from the top of the clip, `note`
    /// 0–127, and `velocity` 1–127 (100 if left out), such as
    /// `[[0, 1, 60, 100], [1, 0.5, 64, 90]]`. Notes are cut off at the end
    /// of the clip (and at its start); notes that start past the end, have
    /// no length, or hold something other than numbers are skipped. Up to
    /// 4096 notes, played on MIDI channel `channel` (config, 1–16,
    /// default 1).
    ///
    /// **Banks:** a clip can hold several sets of notes of the same length
    /// (up to 16), added as tabs in the piano roll; `notes` is then a list
    /// of note lists. The `bank` input picks which plays, counting from 0.
    /// A new bank starts with the next pass, so the riff changes on its
    /// downbeat; while stopped it applies at once.
    ///
    /// **Swing:** `swing` (0 to 1) moves the notes on every other
    /// sixteenth (or eighth, with the `swing_grid` config) later, by up to
    /// half a sixteenth, stretching the time between so nothing changes
    /// order: at about 0.67 a straight riff shuffles in triplets.
    ///
    /// Timing follows the song's tempo map from wherever the clip started:
    /// a note `n` beats into the clip plays `n / rate` song beats after the
    /// start, on the exact sample the tempo map gives for that beat, so
    /// tempo changes are followed and nothing drifts, however long it
    /// loops. The clip keeps its own count of samples since it started, so
    /// it also plays when started while the transport is stopped. `rate`
    /// scales the speed (2 is double time; 0 pauses, holding any sounding
    /// notes).
    ///
    /// **Placed in the song:** set `start` to *At beat…* and the clip
    /// starts at song beat `at` and stops at `until` (unless that's 0). Start the transport anywhere and it
    /// carries on from where the song is: around again if it loops,
    /// finished if it doesn't, waiting before its beat. Notes already
    /// under way where the transport starts stay silent; the next ones
    /// play.
    ///
    /// Generator contract: `start` (on transport play by default) plays
    /// from the top: note-offs for anything sounding, then the notes at
    /// beat 0 on the same sample. `stop` ends sounding notes and fires
    /// `end`; `reset` jumps back to the top without stopping. At the end of
    /// the clip it wraps around while `loop` is on (the default), ending
    /// every sounding note before the first notes of the next pass; with
    /// `loop` off it ends them, stops, and fires `end`. A note overlapping
    /// an earlier one of the same pitch ends it where it starts. Events on
    /// the same sample apply in the order stop, start, reset. `position`
    /// is how many beats into the clip it is (held while stopped). A
    /// transport reset releases sounding notes at the start of the next
    /// block.
    ///
    /// **Try this:** wire MIDI Pattern into a `MIDI Note` driving an
    /// `Oscillator`, with the gate into an `ADSR` on a `Gain`, for a
    /// looping riff; feed a `Voices` synth instead for chords, and chain
    /// clips by turning `loop` off and wiring one's `end` into the next
    /// one's `start`.
    pub struct MidiPattern {
        /// Each bank's notes.
        banks: Vec<Vec<ClipNote>>,
        /// The bank playing, and the one the `bank` input asks for.
        bank: usize,
        wanted: usize,
        /// Swing amount, and the grid it swings, in beats.
        swing: f64,
        swing_grid: f64,
        length: f64,
        channel: u8,
        playing: bool,
        /// The anchor: clip position `anchor_pos` (beats counted over every
        /// pass) sits at song beat `anchor_beat`, and the clip has moved at
        /// `rate` since. Set at start, reset, and rate changes only.
        anchor_beat: f64,
        anchor_pos: f64,
        rate: f64,
        /// Passes completed since the start.
        passes: u64,
        /// The next note to start in this pass.
        cursor: usize,
        /// Sounding notes by pitch, with the bits in `mask`.
        sounding: [Sounding; 128],
        mask: u128,
        /// The song sample of the next block's first frame while playing:
        /// this node's own clock, which keeps counting while the transport
        /// is stopped.
        next_sample: u64,
        /// The `position` output, beats into the clip.
        position: f64,
        /// A reset happened: release sounding notes at the next block.
        flush: bool,
        /// Taken over from the clip before an edit while playing: carry on
        /// this many beats into the clip from the next block, which has the
        /// tempo map to anchor there.
        resume: Option<f64>,
        ends: Vec<u32>,
    }
}

const START: usize = 0;
const STOP: usize = 3;
const RESET: usize = 4;
const LOOP: usize = 5;
const RATE: usize = 6;
const SWING: usize = 7;
const BANK: usize = 8;

const MIDI: usize = 0;
const POSITION: usize = 1;
const PLAYING: usize = 2;
const END: usize = 3;

/// Most notes a clip (each bank) holds; the rest are ignored.
pub const MAX_NOTES: usize = 4096;
pub const MAX_BANKS: usize = 16;
pub const SWING_GRIDS: [&str; 2] = ["sixteenths", "eighths"];
/// Telemetry, for the piano roll's playhead: beats into the clip, the bank
/// playing, and whether it plays (1 or 0).
pub const TELEMETRY_POSITION: usize = 0;
pub const TELEMETRY_BANK: usize = 1;
pub const TELEMETRY_PLAYING: usize = 2;
const MIN_LENGTH: f64 = 0.25;
const MAX_LENGTH: f64 = 1024.0;
/// Fastest `rate`, beyond the input's soft range.
const MAX_RATE: f64 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct ClipNote {
    /// Beats from the top of the clip; `start < end <= length`.
    start: f64,
    end: f64,
    note: u8,
    velocity: u8,
}

#[derive(Clone, Copy, Debug, Default)]
struct Sounding {
    /// Where it ends, in clip beats counted over every pass.
    end: f64,
    /// The song sample it started on.
    started: u64,
}

/// The clip length from config, clamped.
pub fn clip_length(config: &Config) -> f64 {
    let l = config.get_f64_or("length", 4.0);
    if l.is_finite() {
        l.clamp(MIN_LENGTH, MAX_LENGTH)
    } else {
        4.0
    }
}

/// The swing grid from config, in beats.
fn swing_grid(config: &Config) -> f64 {
    if config.get_str_or("swing_grid", SWING_GRIDS[0]) == SWING_GRIDS[1] {
        0.5
    } else {
        0.25
    }
}

/// The banks in `notes`: a list of notes is one bank, a list of note lists
/// several (an empty list counts as a bank). At least one, at most
/// [`MAX_BANKS`].
pub fn note_banks(config: &Config) -> Vec<&[serde_json::Value]> {
    let list = config
        .get("notes")
        .and_then(|v| v.as_array())
        .map_or(&[][..], Vec::as_slice);
    let is_bank = |v: &serde_json::Value| {
        v.as_array()
            .is_some_and(|a| a.first().is_none_or(serde_json::Value::is_array))
    };
    if !list.is_empty() && list.iter().all(is_bank) {
        list.iter()
            .take(MAX_BANKS)
            .map(|b| b.as_array().map_or(&[][..], Vec::as_slice))
            .collect()
    } else {
        vec![list]
    }
}

/// Swung clip position: within each pair of `grid` steps, the second
/// starts `swing × grid / 2` late and both stretch to meet it.
pub fn swung(x: f64, swing: f64, grid: f64) -> f64 {
    if swing <= 0.0 || grid <= 0.0 {
        return x;
    }
    let pair = 2.0 * grid;
    let base = (x / pair).floor() * pair;
    let t = x - base;
    let mid = grid * (1.0 + swing / 2.0);
    let warped = if t < grid {
        t * mid / grid
    } else {
        mid + (t - grid) * (pair - mid) / grid
    };
    base + warped
}

/// One bank's notes, repaired and sorted by start.
fn parse_notes(list: &[serde_json::Value], length: f64) -> Vec<ClipNote> {
    let mut notes = Vec::new();
    for item in list {
        if notes.len() >= MAX_NOTES {
            break;
        }
        let Some(fields) = item.as_array() else {
            continue;
        };
        let num = |i: usize| {
            fields
                .get(i)
                .and_then(serde_json::Value::as_f64)
                .filter(|x| x.is_finite())
        };
        let (Some(start), Some(len), Some(note)) = (num(0), num(1), num(2)) else {
            continue;
        };
        let velocity = match fields.get(3) {
            None => Some(100.0),
            Some(_) => num(3),
        };
        let Some(velocity) = velocity else {
            continue;
        };
        let note = note.round();
        let end = (start + len).min(length);
        let start = start.max(0.0);
        if len <= 0.0 || start >= length || end <= start || !(0.0..=127.0).contains(&note) {
            continue;
        }
        notes.push(ClipNote {
            start,
            end,
            note: note as u8,
            velocity: velocity.round().clamp(1.0, 127.0) as u8,
        });
    }
    notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.note.cmp(&b.note)));
    // The same pitch twice at the same start plays once, as the longer note.
    notes.dedup_by(|later, earlier| {
        let same = later.start == earlier.start && later.note == earlier.note;
        if same && later.end > earlier.end {
            *earlier = *later;
        }
        same
    });
    notes
}

fn rate_of(v: f32) -> f64 {
    if v.is_finite() {
        f64::from(v).clamp(0.0, MAX_RATE)
    } else {
        1.0
    }
}

impl MidiPattern {
    /// The notes of the bank playing.
    fn notes(&self) -> &[ClipNote] {
        self.banks.get(self.bank).map_or(&[], Vec::as_slice)
    }

    /// A clip position, swung, kept inside the clip.
    fn swing(&self, x: f64) -> f64 {
        swung(x, self.swing, self.swing_grid).min(self.length)
    }

    /// The first note starting at or after `x` beats into the clip.
    fn cursor_at(&self, x: f64) -> usize {
        self.notes().partition_point(|n| self.swing(n.start) < x)
    }

    /// The song sample where clip position `pos` falls (a position at or
    /// before the anchor falls on the anchor; with the clip paused, later
    /// positions never come).
    fn sample_of(&self, tempo: &TempoMap, pos: f64) -> u64 {
        if pos <= self.anchor_pos {
            return tempo.sample_at(self.anchor_beat);
        }
        if self.rate <= 0.0 {
            return u64::MAX;
        }
        let beat = self.anchor_beat + (pos - self.anchor_pos) / self.rate;
        if beat.is_finite() {
            tempo.sample_at(beat)
        } else {
            u64::MAX
        }
    }

    /// Clip position (over every pass) at song sample `sample`.
    fn pos_at(&self, tempo: &TempoMap, sample: u64) -> f64 {
        let moved = self.rate * (tempo.beat_at(sample) - self.anchor_beat);
        self.anchor_pos + moved.max(0.0)
    }

    fn pass_start(&self) -> f64 {
        self.passes as f64 * self.length
    }

    fn in_clip(&self, pos: f64) -> f64 {
        (pos - self.pass_start()).clamp(0.0, self.length)
    }

    /// Plays from the top at song sample `sample`.
    fn restart(&mut self, tempo: &TempoMap, sample: u64) {
        self.anchor_beat = tempo.beat_at(sample);
        self.anchor_pos = 0.0;
        self.passes = 0;
        self.cursor = 0;
        self.position = 0.0;
        self.bank = self.wanted;
    }

    /// When a sounding note's note-off is due: its end, but never on the
    /// sample it started.
    fn due(&self, tempo: &TempoMap, s: &Sounding) -> u64 {
        self.sample_of(tempo, s.end)
            .max(s.started.saturating_add(1))
    }

    fn next_off(&self, tempo: &TempoMap) -> u64 {
        let mut next = u64::MAX;
        let mut m = self.mask;
        while m != 0 {
            let p = m.trailing_zeros() as usize;
            m &= m - 1;
            next = next.min(self.due(tempo, &self.sounding[p]));
        }
        next
    }

    fn release_due(&mut self, tx: &mut Sender, tempo: &TempoMap, sample: u64, at: u32) {
        let mut m = self.mask;
        while m != 0 {
            let p = m.trailing_zeros() as usize;
            m &= m - 1;
            if self.due(tempo, &self.sounding[p]) <= sample {
                self.mask &= !(1u128 << p);
                note_off(tx, at, self.channel, p as u8, RELEASE_VELOCITY);
            }
        }
    }

    fn release_all(&mut self, tx: &mut Sender, at: u32) {
        let mut m = self.mask;
        while m != 0 {
            let p = m.trailing_zeros() as u8;
            m &= m - 1;
            note_off(tx, at, self.channel, p, RELEASE_VELOCITY);
        }
        self.mask = 0;
    }

    fn start_note(&mut self, tx: &mut Sender, at: u32, sample: u64, n: ClipNote) {
        let p = usize::from(n.note & 127);
        let bit = 1u128 << p;
        if self.mask & bit != 0 {
            // Overlapping the same pitch: the earlier note ends here.
            self.mask &= !bit;
            note_off(tx, at, self.channel, n.note, RELEASE_VELOCITY);
        }
        let msg = MidiMsg::NoteOn {
            channel: self.channel,
            note: n.note,
            velocity: n.velocity,
        };
        // Keep room in the block for the note-offs owed, this one's too.
        let reserve = self.mask.count_ones() as usize + 1;
        if tx.send_if_room(at, msg, reserve) {
            self.mask |= bit;
            self.sounding[p] = Sounding {
                end: self.pass_start() + self.swing(n.end),
                started: sample,
            };
        }
    }

    /// Plays song samples `from..to` (frames `from - first..to - first`)
    /// with the anchor fixed.
    fn render(
        &mut self,
        tx: &mut Sender,
        tempo: &TempoMap,
        looping: &BoolIn,
        first: u64,
        (from, to): (u64, u64),
    ) {
        // Each pass handles at least one event; this only guards against
        // a clip shorter than a sample.
        let mut budget = 4 * (to - from) as usize + 2 * self.notes().len() + 64;
        while self.playing && budget > 0 {
            budget -= 1;
            let wrap = self.sample_of(tempo, (self.passes + 1) as f64 * self.length);
            let next_on = self.notes().get(self.cursor).map_or(u64::MAX, |n| {
                self.sample_of(tempo, self.pass_start() + self.swing(n.start))
            });
            let now = self.next_off(tempo).min(wrap).min(next_on).max(from);
            if now >= to {
                break;
            }
            let at = (now - first) as u32;
            self.release_due(tx, tempo, now, at);
            if wrap <= now {
                self.release_all(tx, at);
                if looping.at(at as usize) {
                    self.passes += 1;
                    self.cursor = 0;
                    // A new bank starts with the pass.
                    self.bank = self.wanted;
                } else {
                    self.playing = false;
                    self.position = self.length;
                    if self.ends.len() < self.ends.capacity() {
                        self.ends.push(at);
                    }
                    break;
                }
            }
            while let Some(&n) = self.notes().get(self.cursor) {
                let on = self.pass_start() + self.swing(n.start);
                if self.sample_of(tempo, on).max(from) > now {
                    break;
                }
                self.cursor += 1;
                self.start_note(tx, at, now, n);
            }
        }
    }
}

impl Node for MidiPattern {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(3)
    }

    fn describe(_: &Config) -> Signature {
        let mut sig = Signature::new()
            .song_generator_inputs(true, false)
            .input(
                SocketSpec::float("rate", Unit::Ratio, 1.0)
                    .range(0.0, 4.0)
                    .doc("Playback speed: 2 is double time, 0 pauses."),
            )
            .input(
                SocketSpec::float("swing", Unit::Generic, 0.0)
                    .range(0.0, 1.0)
                    .no_smooth()
                    .doc("Moves every other sixteenth later, up to half of one."),
            )
            .input(
                SocketSpec::int("bank", 0)
                    .range(0.0, (MAX_BANKS - 1) as f64)
                    .doc("Which notes play, from 0; a new bank starts with the next pass."),
            );
        if let Some(l) = sig.inputs.iter_mut().find(|s| s.name == "loop") {
            l.default = Value::Bool(true);
        }
        sig.output(SocketSpec::midi("midi").doc("The clip's notes."))
            .output(
                SocketSpec::float("position", Unit::Beats, 0.0)
                    .rate(Rate::Block)
                    .doc("Beats into the clip."),
            )
            .generator_outputs()
    }

    fn new(config: &Config, ctx: &PrepareCtx) -> Self {
        let length = clip_length(config);
        MidiPattern {
            banks: note_banks(config)
                .into_iter()
                .map(|b| parse_notes(b, length))
                .collect(),
            bank: 0,
            wanted: 0,
            swing: 0.0,
            swing_grid: swing_grid(config),
            length,
            channel: (config.get_u64_or("channel", 1).clamp(1, 16) - 1) as u8,
            playing: false,
            anchor_beat: 0.0,
            anchor_pos: 0.0,
            rate: 1.0,
            passes: 0,
            cursor: 0,
            sounding: [Sounding::default(); 128],
            mask: 0,
            next_sample: 0,
            position: 0.0,
            flush: false,
            resume: None,
            ends: Vec::with_capacity(ctx.max_block.max(1) + 1),
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let frames = io.frames();
        let transport = ctx.transport;
        let tempo = transport.tempo;
        let looping = io.inputs.boolean(LOOP);
        let rate = io.inputs.float(RATE);
        let swing = io.inputs.float(SWING).first();
        self.swing = if swing.is_finite() {
            f64::from(swing).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.wanted = (io.inputs.int(BANK).max(0) as usize).min(self.banks.len() - 1);
        if !self.playing {
            self.bank = self.wanted;
        }
        let lock = song::lock(ctx, io, START);
        let mut at = [0; 2];
        let (start, stop) = song::triggers(
            lock,
            io.inputs.triggers(START),
            io.inputs.triggers(STOP),
            &mut at,
        );
        let events = [stop, start, io.inputs.triggers(RESET)];
        // Song sample of frame 0: the transport's, or this node's own clock
        // while it plays.
        let mut first = if self.playing {
            self.next_sample
        } else {
            transport.position
        };
        self.ends.clear();
        let mut tx = Sender::new(io.outputs.midi(MIDI));
        if std::mem::take(&mut self.flush) {
            self.release_all(&mut tx, 0);
        }
        // Placed in the song: carry on from where the song is, `beats` past
        // the clip's beat. Notes that would already be sounding stay
        // silent; the next ones play.
        match lock {
            SongLock::Wait => {
                self.release_all(&mut tx, 0);
                self.playing = false;
                self.position = 0.0;
            }
            SongLock::Land { beats, .. } => {
                self.release_all(&mut tx, 0);
                self.resume = None;
                first = transport.position;
                self.rate = rate_of(rate.first());
                let pos = beats * self.rate;
                let passes = (pos / self.length).floor().max(0.0);
                self.playing = looping.first() || pos < self.length;
                if self.playing {
                    self.anchor_beat = tempo.beat_at(first);
                    self.anchor_pos = pos;
                    self.passes = passes as u64;
                    self.bank = self.wanted;
                    let in_pass = pos - self.pass_start();
                    self.cursor = self.cursor_at(in_pass);
                    self.position = in_pass;
                } else {
                    self.position = self.length;
                }
            }
            _ => {}
        }
        if let Some(at) = self.resume.take()
            && self.playing
        {
            self.anchor_beat = tempo.beat_at(first);
            self.anchor_pos = at;
            self.passes = 0;
            // Notes from here on play; a note starting exactly here hasn't.
            self.cursor = self.cursor_at(at);
        }

        for run in Runs::new(events, frames) {
            let [stop, start, reset] = run.fired;
            let at = run.at as u32;
            if stop && self.playing {
                self.position = self.in_clip(self.pos_at(tempo, first + run.at as u64));
                self.release_all(&mut tx, at);
                self.playing = false;
                if self.ends.len() < self.ends.capacity() {
                    self.ends.push(at);
                }
            }
            if start {
                self.release_all(&mut tx, at);
                // Line up with the transport again from here.
                first = transport.position;
                self.playing = true;
                self.rate = rate_of(rate.at(run.at));
                self.restart(tempo, first + run.at as u64);
            }
            if reset {
                self.release_all(&mut tx, at);
                if self.playing {
                    self.restart(tempo, first + run.at as u64);
                } else {
                    self.passes = 0;
                    self.cursor = 0;
                    self.position = 0.0;
                    self.bank = self.wanted;
                }
            }
            // Render the run in stretches of constant rate; a rate change
            // moves the anchor to where the clip is.
            let mut a = run.at;
            while a < run.until && self.playing {
                let r = rate_of(rate.at(a));
                let sample = first + a as u64;
                if r != self.rate {
                    self.anchor_pos = self.pos_at(tempo, sample);
                    self.anchor_beat = tempo.beat_at(sample);
                    self.rate = r;
                }
                let mut b = run.until;
                if rate.is_varying() {
                    b = a + 1;
                    while b < run.until && rate_of(rate.at(b)) == r {
                        b += 1;
                    }
                }
                self.render(&mut tx, tempo, &looping, first, (sample, first + b as u64));
                a = b;
            }
        }

        if self.playing {
            let end = first + frames as u64;
            self.position = self.in_clip(self.pos_at(tempo, end));
            self.next_sample = end;
        }
        io.outputs.set_float(POSITION, self.position as f32);
        io.outputs.set_bool(PLAYING, self.playing);
        io.telemetry.set(TELEMETRY_POSITION, self.position as f32);
        io.telemetry.set(TELEMETRY_BANK, self.bank as f32);
        io.telemetry
            .set(TELEMETRY_PLAYING, if self.playing { 1.0 } else { 0.0 });
        let mut end = io.outputs.trigger(END);
        for &at in &self.ends {
            end.push(at);
        }
    }

    fn reset(&mut self) {
        self.flush = self.mask != 0;
        self.playing = false;
        self.anchor_beat = 0.0;
        self.anchor_pos = 0.0;
        self.rate = 1.0;
        self.passes = 0;
        self.cursor = 0;
        self.next_sample = 0;
        self.position = 0.0;
        self.resume = None;
    }

    /// An edited clip plays on from the same place: notes already sounding
    /// end where they would have, and the new notes play from the next
    /// block. A clip now shorter than that place wraps (or ends) at once.
    fn take_over(&mut self, previous: &dyn Node) -> bool {
        let Some(old) = (previous as &dyn std::any::Any).downcast_ref::<MidiPattern>() else {
            return false;
        };
        if old.channel != self.channel {
            // Its notes would need note-offs on the old channel.
            return false;
        }
        self.playing = old.playing;
        self.rate = old.rate;
        self.bank = old.bank.min(self.banks.len() - 1);
        self.wanted = old.wanted.min(self.banks.len() - 1);
        self.swing = old.swing;
        self.next_sample = old.next_sample;
        self.flush = old.flush;
        self.position = old.position.min(self.length);
        self.sounding = old.sounding;
        self.mask = old.mask;
        if old.playing {
            // Sounding notes end as far ahead as they did, counted from
            // where the new clip resumes.
            let reached = old.pass_start() + old.position;
            for s in &mut self.sounding {
                s.end = old.position + (s.end - reached).max(0.0);
            }
            self.resume = Some(old.position);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{NoResources, TempoSegment};
    use serde_json::json;

    /// 480 BPM at 48 kHz: 6000 samples per beat.
    const BEAT: u64 = 6_000;

    fn harness(config: serde_json::Value, block: usize) -> Harness {
        let mut h = Harness::with_resources::<MidiPattern>(config, 48_000.0, block, &NoResources);
        h.tempo(TempoMap::constant(48_000.0, 480.0));
        h
    }

    fn riff() -> serde_json::Value {
        json!({
            "length": 4,
            "notes": [[0, 1, 60, 100], [1, 0.5, 64, 90], [2.5, 1, 67, 80]]
        })
    }

    /// (frame, +note for on / -note for off).
    fn notes(out: &[(u64, MidiMsg)]) -> Vec<(u64, i32)> {
        out.iter()
            .map(|(t, m)| match m {
                MidiMsg::NoteOn { note, .. } => (*t, i32::from(*note)),
                MidiMsg::NoteOff { note, .. } => (*t, -i32::from(*note)),
                _ => (*t, 0),
            })
            .collect()
    }

    fn balanced(out: &[(u64, MidiMsg)]) -> bool {
        let mut held = [0i32; 128];
        for (_, m) in out {
            match m {
                MidiMsg::NoteOn { note, .. } => held[*note as usize] += 1,
                MidiMsg::NoteOff { note, .. } => held[*note as usize] -= 1,
                _ => {}
            }
        }
        held.iter().all(|&h| h == 0)
    }

    #[test]
    fn a_placed_clip_starts_on_its_beat_and_lands_after_a_seek() {
        let mut h = harness(riff(), 64);
        h.set("start", "at").set("at", 2.0);
        let out = h.run((4 * BEAT) as usize);
        assert_eq!(
            notes(out.midi("midi")),
            vec![
                (2 * BEAT, 60),
                (3 * BEAT, -60),
                (3 * BEAT, 64),
                (BEAT * 7 / 2, -64)
            ]
        );
        // 1.25 beats in: the note under way stays silent, the next plays.
        h.seek(2 * BEAT + BEAT * 5 / 4);
        let t = 4 * BEAT;
        let out = h.run((3 * BEAT) as usize);
        assert_eq!(
            notes(out.midi("midi")),
            vec![
                (t + BEAT * 5 / 4, 67),
                (t + BEAT * 9 / 4, -67),
                (t + BEAT * 11 / 4, 60)
            ]
        );
        // Two passes and a beat in: the note still sounding ends at the
        // seek, and a note starting right there plays.
        h.seek(2 * BEAT + 9 * BEAT);
        let t = 7 * BEAT;
        let out = h.run(64);
        assert_eq!(notes(out.midi("midi")), vec![(t, -60), (t, 64)]);
        assert_eq!(out.float("position")[0], 1.0 + 64.0 / BEAT as f32);
    }

    #[test]
    fn notes_land_on_exact_samples_and_loop() {
        let mut h = harness(riff(), 64);
        let out = h.run((5 * BEAT) as usize);
        assert_eq!(
            notes(out.midi("midi")),
            vec![
                (0, 60),
                (BEAT, -60),
                (BEAT, 64),
                (BEAT * 3 / 2, -64),
                (BEAT * 5 / 2, 67),
                (BEAT * 7 / 2, -67),
                (4 * BEAT, 60),
            ]
        );
        assert_eq!(
            out.midi("midi")[1].1,
            MidiMsg::NoteOff {
                channel: 0,
                note: 60,
                velocity: RELEASE_VELOCITY
            }
        );
        assert!(out.triggers("end").is_empty());
        assert_eq!(out.float("playing")[0], 1.0);
    }

    #[test]
    fn an_edited_clip_plays_on_from_where_it_was() {
        let mut h = harness(riff(), 64);
        let first = h.run((BEAT * 5 / 4) as usize);
        assert_eq!(
            notes(first.midi("midi")),
            vec![(0, 60), (BEAT, -60), (BEAT, 64)]
        );
        // 64 is deleted and 67 moved to beat 2: the 64 already sounding
        // still ends on time, and the clip carries on in step.
        h.reconfigure::<MidiPattern>(json!({
            "length": 4,
            "notes": [[0, 1, 60, 100], [2, 1, 67, 80]]
        }));
        let out = h.run((BEAT * 4) as usize);
        assert_eq!(
            notes(out.midi("midi")),
            vec![
                (BEAT * 3 / 2, -64),
                (2 * BEAT, 67),
                (3 * BEAT, -67),
                (4 * BEAT, 60),
                (5 * BEAT, -60),
            ]
        );
        let mut all = first.midi("midi").to_vec();
        all.extend_from_slice(out.midi("midi"));
        assert!(balanced(&all));
    }

    #[test]
    fn notes_past_the_end_are_cut_at_the_wrap() {
        let mut h = harness(
            json!({ "length": 1, "notes": [[0.5, 4, 60, 100], [0, 0.25, 62, 1]] }),
            64,
        );
        let out = h.run((2 * BEAT) as usize);
        assert_eq!(
            notes(out.midi("midi")),
            vec![
                (0, 62),
                (BEAT / 4, -62),
                (BEAT / 2, 60),
                (BEAT, -60),
                (BEAT, 62),
                (BEAT * 5 / 4, -62),
                (BEAT * 3 / 2, 60),
            ]
        );
    }

    #[test]
    fn without_loop_it_ends_after_one_pass() {
        let mut h = harness(json!({ "length": 2, "notes": [[1, 5, 60, 100]] }), 64);
        h.set("loop", false);
        let out = h.run((3 * BEAT) as usize);
        assert_eq!(notes(out.midi("midi")), vec![(BEAT, 60), (2 * BEAT, -60)]);
        assert_eq!(out.triggers("end"), &[2 * BEAT]);
        assert_eq!(*out.float("playing").last().unwrap(), 0.0);
        assert_eq!(*out.float("position").last().unwrap(), 2.0);
    }

    #[test]
    fn plays_from_its_own_start() {
        let mut h = harness(riff(), 64);
        h.triggers("start", &[1_000]);
        let out = h.run((2 * BEAT) as usize);
        assert_eq!(
            notes(out.midi("midi")),
            vec![
                (1_000, 60),
                (1_000 + BEAT, -60),
                (1_000 + BEAT, 64),
                (1_000 + BEAT * 3 / 2, -64)
            ]
        );
    }

    #[test]
    fn stop_and_reset_release_sounding_notes() {
        let mut h = harness(riff(), 64);
        h.triggers("start", &[0]).triggers("stop", &[3_000]);
        let out = h.run((2 * BEAT) as usize);
        assert_eq!(notes(out.midi("midi")), vec![(0, 60), (3_000, -60)]);
        assert_eq!(out.triggers("end"), &[3_000]);
        assert!((out.float("position")[3_100] - 0.5).abs() < 1e-6);

        let mut h = harness(riff(), 64);
        h.triggers("reset", &[3_000]);
        let out = h.run((BEAT + 4_000) as usize);
        // Reset jumps back to the top: 60 again at 3000, and everything a
        // beat after that.
        assert_eq!(
            notes(out.midi("midi")),
            vec![
                (0, 60),
                (3_000, -60),
                (3_000, 60),
                (3_000 + BEAT, -60),
                (3_000 + BEAT, 64)
            ]
        );
    }

    #[test]
    fn rate_scales_the_speed() {
        let mut h = harness(riff(), 64);
        h.set("rate", 2.0);
        let out = h.run((2 * BEAT + 1) as usize);
        assert_eq!(
            notes(out.midi("midi")),
            vec![
                (0, 60),
                (BEAT / 2, -60),
                (BEAT / 2, 64),
                (BEAT * 3 / 4, -64),
                (BEAT * 5 / 4, 67),
                (BEAT * 7 / 4, -67),
                (2 * BEAT, 60),
            ]
        );
    }

    #[test]
    fn rate_zero_pauses_and_a_rate_change_keeps_the_place() {
        let mut h = harness(riff(), 64);
        let rate: Vec<f32> = (0..4 * BEAT as usize)
            .map(|i| {
                if i < 3_000 {
                    1.0
                } else if i < 9_000 {
                    0.0
                } else {
                    2.0
                }
            })
            .collect();
        h.set_samples("rate", rate);
        let out = h.run(19_000);
        // Half a beat at rate 1, paused for a beat, then double time: the
        // note at beat 1 comes a quarter beat after the pause.
        assert_eq!(
            notes(out.midi("midi")),
            vec![
                (0, 60),
                (9_000 + BEAT / 4, -60),
                (9_000 + BEAT / 4, 64),
                (9_000 + BEAT / 2, -64),
                (9_000 + BEAT, 67),
                (9_000 + BEAT * 3 / 2, -67),
            ]
        );
    }

    #[test]
    fn follows_tempo_changes() {
        let mut h = harness(
            json!({ "length": 8, "notes": [[1, 1, 60, 100], [3, 0.5, 62, 100]] }),
            64,
        );
        // 4 beats at 480 BPM, then 240 BPM (12000 samples per beat).
        h.tempo(TempoMap::new(
            48_000.0,
            &[
                TempoSegment {
                    beat: 0.0,
                    bpm: 480.0,
                    time_sig: [4, 4],
                },
                TempoSegment {
                    beat: 2.0,
                    bpm: 240.0,
                    time_sig: [4, 4],
                },
            ],
        ));
        let out = h.run(60_000);
        assert_eq!(
            notes(out.midi("midi")),
            vec![
                (6_000, 60),
                (12_000, -60),
                (12_000 + 12_000, 62),
                (24_000 + 6_000, -62)
            ]
        );
    }

    #[test]
    fn same_pitch_overlaps_end_the_earlier_note() {
        let mut h = harness(
            json!({ "length": 4, "notes": [[0, 2, 60, 100], [1, 2, 60, 50]] }),
            64,
        );
        let out = h.run((4 * BEAT) as usize);
        assert_eq!(
            notes(out.midi("midi")),
            vec![(0, 60), (BEAT, -60), (BEAT, 60), (3 * BEAT, -60)]
        );
    }

    #[test]
    fn bad_notes_are_skipped_and_repaired() {
        let config = json!({ "notes": [
                [0, 1, 60],
                [-1, 2, 61, 300],
                [1, 0, 62, 100],
                [9, 1, 63, 100],
                [1, 1, 128, 100],
                [1, 1, 64, "loud"],
                "x",
                [1, 1],
                [2, 1, 64.4, 0]
        ] });
        let n = parse_notes(note_banks(&config)[0], 4.0);
        assert_eq!(
            n,
            vec![
                ClipNote {
                    start: 0.0,
                    end: 1.0,
                    note: 60,
                    velocity: 100
                },
                ClipNote {
                    start: 0.0,
                    end: 1.0,
                    note: 61,
                    velocity: 127
                },
                ClipNote {
                    start: 2.0,
                    end: 3.0,
                    note: 64,
                    velocity: 1
                },
            ]
        );
        assert_eq!(clip_length(&json!({ "length": 0 })), MIN_LENGTH);
        assert_eq!(clip_length(&json!({ "length": "long" })), 4.0);
        let config = json!({ "notes": [[0, 1, 60, 10], [0, 2, 60, 20]] });
        let dup = parse_notes(note_banks(&config)[0], 4.0);
        assert_eq!(dup.len(), 1);
        assert_eq!(dup[0].end, 2.0);
    }

    #[test]
    fn banks_parse_from_nested_lists() {
        let one = json!({ "notes": [[0, 1, 60]] });
        assert_eq!(note_banks(&one).len(), 1);
        let none = json!({});
        assert!(matches!(note_banks(&none).as_slice(), [b] if b.is_empty()));
        let two = json!({ "notes": [[[0, 1, 60]], []] });
        let b = note_banks(&two);
        assert_eq!((b.len(), b[0].len(), b[1].len()), (2, 1, 0));
        let many: Vec<_> = (0..40).map(|_| json!([])).collect();
        assert_eq!(note_banks(&json!({ "notes": many })).len(), MAX_BANKS);
    }

    #[test]
    fn a_new_bank_starts_with_the_next_pass() {
        let config = json!({
            "length": 2,
            "notes": [[[0, 1, 60], [1, 1, 62]], [[0, 1, 70], [1, 1, 72]]]
        });
        let mut h = harness(config.clone(), 64);
        let out = h.run(BEAT as usize);
        assert_eq!(notes(out.midi("midi")), vec![(0, 60)]);
        // Asked for in the middle of the first pass: the second beat is
        // still bank 0, and the next pass is bank 1.
        h.set("bank", 1);
        let out = h.run(2 * BEAT as usize);
        assert_eq!(
            notes(out.midi("midi")),
            vec![(BEAT, -60), (BEAT, 62), (2 * BEAT, -62), (2 * BEAT, 70)]
        );
        // Stopped, it applies at once: from the start, bank 1.
        let mut h = harness(config, 64);
        h.set("bank", 1);
        assert_eq!(notes(h.run(64).midi("midi")), vec![(0, 70)]);
    }

    #[test]
    fn swing_moves_every_other_sixteenth_late() {
        assert_eq!(swung(0.25, 0.5, 0.25), 0.3125);
        assert_eq!(swung(0.5, 0.5, 0.25), 0.5, "the pair's end stays");
        assert_eq!(swung(0.125, 0.5, 0.25), 0.15625, "stretched to meet it");
        assert_eq!(swung(0.375, 0.5, 0.25), 0.40625);
        assert_eq!(swung(1.25, 0.0, 0.25), 1.25);
        // Four sixteenths; the second and fourth land 1/16 beat late.
        let config = json!({
            "length": 1,
            "notes": [[0, 0.25, 60], [0.25, 0.25, 61], [0.5, 0.25, 62], [0.75, 0.25, 63]]
        });
        let mut h = harness(config.clone(), 64);
        h.set("swing", 0.5);
        let ons: Vec<u64> = h
            .run(BEAT as usize)
            .midi("midi")
            .iter()
            .filter(|(_, m)| matches!(m, MidiMsg::NoteOn { .. }))
            .map(|(t, _)| *t)
            .collect();
        let s = BEAT / 16;
        assert_eq!(ons, vec![0, 4 * s + s, 8 * s, 12 * s + s]);
        // Eighths: the swung grid is twice as wide, so the first sixteenth
        // stretches to 5/16 of a beat.
        let mut c = config;
        c["swing_grid"] = json!("eighths");
        let mut h = harness(c, 64);
        h.set("swing", 0.5);
        let out = h.run(BEAT as usize);
        let first_off = out
            .midi("midi")
            .iter()
            .find(|(_, m)| matches!(m, MidiMsg::NoteOff { .. }));
        assert_eq!(first_off.map(|(t, _)| *t), Some(5 * s));
    }

    #[test]
    fn channel_config() {
        let mut h = harness(json!({ "channel": 10, "notes": [[0, 1, 36, 100]] }), 64);
        let out = h.run(64);
        assert_eq!(out.midi("midi")[0].1.channel(), 9);
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let config = json!({
            "length": 3,
            "notes": [[0, 0.75, 48, 100], [0.5, 1, 55, 90], [1.25, 2, 60, 80], [2, 0.1, 48, 70]]
        });
        let tempo = TempoMap::new(
            48_000.0,
            &[
                TempoSegment {
                    beat: 0.0,
                    bpm: 480.0,
                    time_sig: [4, 4],
                },
                TempoSegment {
                    beat: 5.5,
                    bpm: 333.0,
                    time_sig: [4, 4],
                },
            ],
        );
        let render = |block| {
            let mut h = harness(config.clone(), block);
            h.tempo(tempo.clone());
            h.triggers("start", &[0, 20_011])
                .triggers("stop", &[41_000])
                .triggers("reset", &[30_500]);
            let out = h.run(48_000);
            // `playing` and `position` are block-rate, so they're left out.
            (out.midi("midi").to_vec(), out.triggers("end").to_vec())
        };
        let one = render(1);
        assert!(one.0.len() > 20);
        assert!(balanced(&one.0));
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));
    }

    #[test]
    fn no_hanging_notes_after_a_stop_or_a_reset() {
        let mut h = harness(riff(), 64);
        let mut all = h.run(8_000).midi("midi").to_vec();
        h.stop();
        all.extend_from_slice(h.run(64).midi("midi"));
        assert!(balanced(&all));

        let mut h = harness(riff(), 64);
        let mut all = h.run(8_000).midi("midi").to_vec();
        h.reset();
        all.extend_from_slice(h.run(64).midi("midi"));
        assert!(balanced(&all));
    }

    #[test]
    fn keeps_playing_while_the_transport_is_stopped() {
        let mut h = harness(riff(), 64);
        h.stopped().set("start", "load");
        let out = h.run((BEAT + 64) as usize);
        assert_eq!(
            notes(out.midi("midi")),
            vec![(0, 60), (BEAT, -60), (BEAT, 64)]
        );
    }
}
