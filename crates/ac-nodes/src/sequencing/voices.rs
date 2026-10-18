use ac_core::{
    Config, ConfigExt, Io, MidiEvent, MidiMsg, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature,
    SocketSpec, node,
};

node! {
    kind: "voices",
    name: "Voices",
    category: Sequencing,
    keywords: ["polyphony", "polyphonic", "poly", "voice allocation", "chords", "synth"],
    /// Makes any patch polyphonic: its body (a group) runs once per voice,
    /// each voice plays one note at a time, and the voices' outputs are
    /// added together.
    ///
    /// Notes arriving at `midi` are shared out among `voices` copies of the
    /// body (config, 1 to 64, default 8). Inside the body, a `Voice` node
    /// gives each copy its own note: `freq`, `gate`, `velocity`, `note`,
    /// `trigger`, and the voice's `index`. Build a mono voice with a `Voice`
    /// node, an Oscillator, a filter, and an envelope, expose its output, and
    /// it becomes a synth. The body's other inputs appear on the Voices node
    /// and reach every voice; its Audio and Float outputs are summed over
    /// the voices (other types come from the first voice).
    ///
    /// A new note goes to a free voice, preferring the one released longest
    /// ago so release tails can ring out. When every voice is busy, `steal`
    /// (config) chooses which note gives way: `oldest` (default), `newest`,
    /// `lowest`, `highest`, or `none` (the new note is ignored). A stolen or
    /// repeated note retriggers its voice. The sustain pedal (CC 64) holds
    /// released notes; pitch bend, pressure, and controllers reach every
    /// voice. Double-click the node to edit its body.
    ///
    /// **Try this:** select the Oscillator → Biquad → Gain voice of the
    /// Subtractive Synth example, group it, swap its MIDI Note for a `Voice`
    /// node, and change the group into a Voices node: play chords.
    pub struct Voices {
        voices: Vec<VoiceState>,
        steal: Steal,
        /// Sustain pedal per MIDI channel.
        pedal: [bool; 16],
        /// Counts note-ons and releases, to order voices by age.
        clock: u64,
        /// Notes ignored under `steal: none`, so their note-offs are too.
        dropped: [u128; 16],
    }
}

const MIDI: usize = 0;

/// Most voices a config can ask for (DECISIONS D29).
pub const MAX_VOICES: u64 = 64;

/// The voice count from config, clamped.
pub fn voice_count(config: &Config) -> usize {
    config.get_u64_or("voices", 8).clamp(1, MAX_VOICES) as usize
}

const STEAL: &[&str] = &["oldest", "newest", "lowest", "highest", "none"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Steal {
    Oldest,
    Newest,
    Lowest,
    Highest,
    None,
}

fn steal(config: &Config) -> Steal {
    match config.get_str_or("steal", "oldest") {
        "newest" => Steal::Newest,
        "lowest" => Steal::Lowest,
        "highest" => Steal::Highest,
        "none" => Steal::None,
        _ => Steal::Oldest,
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct VoiceState {
    /// (channel, note) last given to this voice.
    note: Option<(u8, u8)>,
    /// The key is down.
    held: bool,
    /// The key is up but the sustain pedal holds the note.
    sustained: bool,
    /// When the note started.
    started: u64,
    /// When the note was released (or 0 if never used).
    released: u64,
}

impl VoiceState {
    fn busy(&self) -> bool {
        self.held || self.sustained
    }
}

fn is_dropped(dropped: &[u128; 16], ch: u8, note: u8) -> bool {
    dropped[usize::from(ch & 15)] & (1u128 << (note & 127)) != 0
}

fn set_dropped(dropped: &mut [u128; 16], ch: u8, note: u8, on: bool) {
    let bit = 1u128 << (note & 127);
    let w = &mut dropped[usize::from(ch & 15)];
    if on {
        *w |= bit;
    } else {
        *w &= !bit;
    }
}

impl Voices {
    fn send(io: &mut Io, voice: usize, offset: u32, msg: MidiMsg) {
        io.outputs.midi(voice).push(MidiEvent::new(offset, msg));
    }

    fn release(&mut self, io: &mut Io, v: usize, offset: u32, velocity: u8) {
        let Some(state) = self.voices.get_mut(v) else {
            return;
        };
        if let Some((channel, note)) = state.note {
            Self::send(
                io,
                v,
                offset,
                MidiMsg::NoteOff {
                    channel,
                    note,
                    velocity,
                },
            );
        }
        state.held = false;
        state.sustained = false;
        self.clock += 1;
        state.released = self.clock;
    }

    /// The voice for a new note, or `None` to ignore it.
    fn pick(&self, channel: u8, note: u8) -> Option<usize> {
        // The same note again retriggers its voice.
        if let Some(v) = self
            .voices
            .iter()
            .position(|s| s.note == Some((channel, note)) && s.busy())
        {
            return Some(v);
        }
        // A free voice: never used first, then released longest ago.
        let free = self
            .voices
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.busy())
            .min_by_key(|(_, s)| (s.note.is_some(), s.released));
        if let Some((v, _)) = free {
            return Some(v);
        }
        let busy = self.voices.iter().enumerate();
        let pitch = |s: &VoiceState| s.note.map_or(0, |(_, n)| n);
        let victim = match self.steal {
            Steal::Oldest => busy.min_by_key(|(_, s)| s.started),
            Steal::Newest => busy.max_by_key(|(_, s)| s.started),
            Steal::Lowest => busy.min_by_key(|(_, s)| (pitch(s), s.started)),
            Steal::Highest => busy.max_by_key(|(_, s)| (pitch(s), u64::MAX - s.started)),
            Steal::None => None,
        };
        victim.map(|(v, _)| v)
    }

    fn handle(&mut self, io: &mut Io, offset: u32, msg: MidiMsg) {
        match msg {
            MidiMsg::NoteOn {
                channel,
                note,
                velocity,
            } if velocity > 0 => {
                let Some(v) = self.pick(channel, note) else {
                    set_dropped(&mut self.dropped, channel, note, true);
                    return;
                };
                if self.voices[v].busy() {
                    // Stolen or repeated: end the old note first.
                    self.release(io, v, offset, 64);
                }
                self.clock += 1;
                let s = &mut self.voices[v];
                s.note = Some((channel, note));
                s.held = true;
                s.sustained = false;
                s.started = self.clock;
                Self::send(io, v, offset, msg);
            }
            MidiMsg::NoteOn { channel, note, .. } | MidiMsg::NoteOff { channel, note, .. } => {
                if is_dropped(&self.dropped, channel, note) {
                    set_dropped(&mut self.dropped, channel, note, false);
                    return;
                }
                let velocity = match msg {
                    MidiMsg::NoteOff { velocity, .. } => velocity,
                    _ => 64,
                };
                let held = self
                    .voices
                    .iter()
                    .position(|s| s.held && s.note == Some((channel, note)));
                if let Some(v) = held {
                    if self.pedal[usize::from(channel & 15)] {
                        self.voices[v].held = false;
                        self.voices[v].sustained = true;
                    } else {
                        self.release(io, v, offset, velocity);
                    }
                }
            }
            MidiMsg::ControlChange {
                channel,
                controller: 64,
                value,
            } => {
                let down = value >= 64;
                self.pedal[usize::from(channel & 15)] = down;
                if !down {
                    for v in 0..self.voices.len() {
                        let s = self.voices[v];
                        if s.sustained && s.note.is_some_and(|(c, _)| c == channel) {
                            self.release(io, v, offset, 64);
                        }
                    }
                }
                self.broadcast(io, offset, msg);
            }
            // All sound off, all notes off: end every note, then tell every voice.
            MidiMsg::ControlChange {
                controller: 120 | 123,
                ..
            } => {
                for v in 0..self.voices.len() {
                    if self.voices[v].busy() {
                        self.release(io, v, offset, 0);
                    }
                }
                self.pedal = [false; 16];
                self.dropped = [0; 16];
                self.broadcast(io, offset, msg);
            }
            MidiMsg::PolyPressure { channel, note, .. } => {
                if let Some(v) = self
                    .voices
                    .iter()
                    .position(|s| s.busy() && s.note == Some((channel, note)))
                {
                    Self::send(io, v, offset, msg);
                }
            }
            _ => self.broadcast(io, offset, msg),
        }
    }

    fn broadcast(&self, io: &mut Io, offset: u32, msg: MidiMsg) {
        for v in 0..self.voices.len() {
            Self::send(io, v, offset, msg);
        }
    }
}

impl Node for Voices {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    /// The allocator's own sockets: `midi` in and one hidden MIDI stream
    /// per voice, which the compiler wires to the `Voice` nodes of each
    /// copy of the body. The sockets people see come from the body
    /// (`voices_signature`).
    fn describe(config: &Config) -> Signature {
        let mut sig = Signature::new()
            .input(SocketSpec::midi("midi").doc("Notes to share out among the voices."));
        for v in 0..voice_count(config) {
            sig = sig.output(SocketSpec::midi(format!("voice_{v}")).hidden());
        }
        sig
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        Voices {
            voices: vec![VoiceState::default(); voice_count(config)],
            steal: steal(config),
            pedal: [false; 16],
            clock: 0,
            dropped: [0; 16],
        }
    }

    fn process(&mut self, _ctx: &ProcessCtx, io: &mut Io) {
        let events = io.inputs.midi(MIDI);
        for e in events {
            self.handle(io, e.offset, e.msg);
        }
    }

    fn reset(&mut self) {
        for s in &mut self.voices {
            *s = VoiceState::default();
        }
        self.pedal = [false; 16];
        self.clock = 0;
        self.dropped = [0; 16];
    }
}

/// Choices for the `steal` config, for the editor.
pub const STEAL_CHOICES: &[&str] = STEAL;

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn on(note: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel: 0,
            note,
            velocity: 100,
        }
    }
    fn off(note: u8) -> MidiMsg {
        MidiMsg::NoteOff {
            channel: 0,
            note,
            velocity: 0,
        }
    }
    fn cc(controller: u8, value: u8) -> MidiMsg {
        MidiMsg::ControlChange {
            channel: 0,
            controller,
            value,
        }
    }

    /// Note numbers each voice hears (on as +note, off as -note), in order.
    fn heard(config: serde_json::Value, events: &[(u64, MidiMsg)]) -> Vec<Vec<i32>> {
        let mut h = Harness::new::<Voices>(config.clone());
        h.midi("midi", events);
        let out = h.run(256);
        (0..voice_count(&config))
            .map(|v| {
                out.midi(&format!("voice_{v}"))
                    .iter()
                    .filter_map(|(_, m)| match m {
                        MidiMsg::NoteOn { note, .. } => Some(i32::from(*note)),
                        MidiMsg::NoteOff { note, .. } => Some(-i32::from(*note)),
                        _ => None,
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn chords_spread_over_voices_and_releases_follow_their_notes() {
        let v = heard(
            json!({ "voices": 4 }),
            &[
                (0, on(60)),
                (0, on(64)),
                (0, on(67)),
                (10, off(64)),
                (20, off(60)),
            ],
        );
        assert_eq!(v, vec![vec![60, -60], vec![64, -64], vec![67], vec![]]);
    }

    #[test]
    fn free_voices_prefer_the_longest_released() {
        let v = heard(
            json!({ "voices": 2 }),
            &[
                (0, on(60)),
                (1, on(62)),
                (2, off(60)),
                (3, off(62)),
                (4, on(64)),
            ],
        );
        // Voice 0 was released first, so it takes the new note.
        assert_eq!(v, vec![vec![60, -60, 64], vec![62, -62]]);
    }

    #[test]
    fn steal_policies_pick_the_right_victim() {
        let events = [(0, on(60)), (1, on(72)), (2, on(48)), (3, on(65))];
        let busy = |steal: &str| heard(json!({ "voices": 3, "steal": steal }), &events);
        // oldest: 60 (voice 0) gives way.
        assert_eq!(busy("oldest")[0], vec![60, -60, 65]);
        // newest: 48 (voice 2) gives way.
        assert_eq!(busy("newest")[2], vec![48, -48, 65]);
        // lowest: 48; highest: 72 (voice 1).
        assert_eq!(busy("lowest")[2], vec![48, -48, 65]);
        assert_eq!(busy("highest")[1], vec![72, -72, 65]);
        // none: 65 is ignored, and so is its note-off.
        let mut ev = events.to_vec();
        ev.push((4, off(65)));
        let v = heard(json!({ "voices": 3, "steal": "none" }), &ev);
        assert_eq!(v, vec![vec![60], vec![72], vec![48]]);
    }

    #[test]
    fn repeated_notes_retrigger_their_voice() {
        let v = heard(json!({ "voices": 2 }), &[(0, on(60)), (5, on(60))]);
        assert_eq!(v, vec![vec![60, -60, 60], vec![]]);
    }

    #[test]
    fn the_sustain_pedal_holds_released_notes() {
        let v = heard(
            json!({ "voices": 2 }),
            &[
                (0, cc(64, 127)),
                (1, on(60)),
                (2, off(60)),
                (3, on(62)),
                (9, cc(64, 0)),
            ],
        );
        // 60 stays until the pedal lifts; 62 is still held then.
        assert_eq!(v[0], vec![60, -60]);
        assert_eq!(v[1], vec![62]);
    }

    #[test]
    fn controllers_and_bend_reach_every_voice_and_all_notes_off_ends_notes() {
        let mut h = Harness::new::<Voices>(json!({ "voices": 3 }));
        h.midi(
            "midi",
            &[
                (0, on(60)),
                (
                    1,
                    MidiMsg::PitchBend {
                        channel: 0,
                        value: 100,
                    },
                ),
                (2, cc(123, 0)),
            ],
        );
        let out = h.run(64);
        for v in 0..3 {
            let msgs: Vec<_> = out
                .midi(&format!("voice_{v}"))
                .iter()
                .map(|e| e.1)
                .collect();
            assert!(msgs.iter().any(|m| matches!(m, MidiMsg::PitchBend { .. })));
            assert!(msgs.iter().any(|m| matches!(
                m,
                MidiMsg::ControlChange {
                    controller: 123,
                    ..
                }
            )));
        }
        let v0: Vec<_> = out.midi("voice_0").iter().map(|e| e.1).collect();
        assert!(v0.contains(&MidiMsg::NoteOff {
            channel: 0,
            note: 60,
            velocity: 0
        }));
    }

    #[test]
    fn configs_are_repaired() {
        assert_eq!(voice_count(&json!({ "voices": 0 })), 1);
        assert_eq!(voice_count(&json!({ "voices": 1000 })), 64);
        assert_eq!(steal(&json!({ "steal": "bogus" })), Steal::Oldest);
        let sig = Voices::describe(&json!({ "voices": 3 }));
        assert_eq!(sig.outputs.len(), 3);
        assert!(sig.outputs.iter().all(|s| s.hidden));
    }

    #[test]
    fn reset_frees_every_voice() {
        let mut h = Harness::new::<Voices>(json!({ "voices": 1 }));
        h.midi("midi", &[(0, on(60))]);
        h.run(64);
        h.reset();
        h.midi("midi", &[(64, on(62))]);
        let out = h.run(64);
        let notes: Vec<_> = out.midi("voice_0").iter().map(|e| e.1).collect();
        assert_eq!(notes, vec![on(62)], "no note-off for 60 after a reset");
    }
}
