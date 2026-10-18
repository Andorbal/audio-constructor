use ac_core::{
    Config, ConfigExt, Io, MidiEvent, MidiMsg, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature,
    SocketSpec, node,
};

use crate::sequencing::notes::{
    RELEASE_VELOCITY, Sender, is_all_notes_off, note_off_of, note_on_of,
};

node! {
    kind: "merge",
    name: "Merge",
    category: Math,
    keywords: ["combine", "join", "midi merge", "trigger merge", "together", "union", "funnel"],
    /// Combines several MIDI or trigger streams into one, so two sources can
    /// play one instrument at once.
    ///
    /// The `inputs` config sets how many inputs there are (2 to 16, default
    /// 2), named `in_0`, `in_1`, …, and the `type` config what they carry:
    /// `midi` (the default) or `trigger`. `out` has the same type.
    ///
    /// - `trigger`: `out` fires on every sample where any input fires; two
    ///   triggers on one sample are one.
    /// - `midi`: every input's events in time order; on one sample, a lower
    ///   input's events come first. A note held from several inputs at once
    ///   (same channel and note) sounds once: the first note-on passes, and
    ///   the note-off passes when the last input holding the note lets go.
    ///   A note-off for a note its input isn't holding is dropped. A note-on
    ///   for a note that only its own input holds retriggers it: a note-off,
    ///   then the note-on.
    ///
    /// The sustain pedal (CC 64) is merged the same way: it stays down while
    /// any input holds it down, and a pedal message passes only when no
    /// other input holds the pedal on its channel. An all-notes-off or
    /// all-sound-off ends the notes its input holds on that channel, and
    /// passes on itself only when no other input holds a note or the pedal
    /// there, so another input's notes keep sounding. Pitch bend, pressure,
    /// program changes, and other controllers pass as they come.
    ///
    /// When a block brings more MIDI than an output can carry, note-ons and
    /// other messages give way so note-offs and pedal releases always get
    /// through. Restarting the transport releases every note and pedal still
    /// held, at the start of the next block.
    ///
    /// **Try this:** wire `MIDI In` into `in_0` and a `MIDI Pattern` into
    /// `in_1`, and `out` into a `Voices` synth: play along with the pattern
    /// on your MIDI keyboard. With `type` `trigger`, merge two `Euclidean`
    /// rhythms into one `Sampler`'s `start` for a busier beat.
    pub struct Merge {
        ty: MergeType,
        inputs: usize,
        /// Per MIDI channel and note, a bit for each input holding it.
        holders: [[u16; 128]; 16],
        /// Per MIDI channel, a bit for each input holding the sustain pedal.
        pedals: [u16; 16],
        /// Notes sounding on `out`, each owing a note-off.
        sounding: usize,
        /// Release everything at the start of the next block (after reset).
        flush: bool,
    }
}

/// What a Merge carries, from its `type` config.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MergeType {
    Midi,
    Trigger,
}

impl MergeType {
    fn from_config(config: &Config) -> MergeType {
        match config.get_str_or("type", "midi") {
            "trigger" => MergeType::Trigger,
            // Unknown types fall back to the default rather than failing to load.
            _ => MergeType::Midi,
        }
    }
}

/// Most inputs: one bit each in a `u16` holder mask.
const MAX_INPUTS: usize = 16;
const OUT: usize = 0;
const SUSTAIN_PEDAL: u8 = 64;

fn input_count(config: &Config) -> usize {
    config.get_u64_or("inputs", 2).clamp(2, MAX_INPUTS as u64) as usize
}

impl Merge {
    /// Messages `out` owes: a note-off per sounding note and a pedal
    /// release per channel whose pedal is down.
    fn owed(&self) -> usize {
        self.sounding + self.pedals.iter().filter(|&&m| m != 0).count()
    }

    fn merge_triggers(&self, io: &mut Io) {
        let n = self.inputs;
        let mut lists: [&[u32]; MAX_INPUTS] = [&[]; MAX_INPUTS];
        for (k, list) in lists.iter_mut().enumerate().take(n) {
            *list = io.inputs.triggers(k);
        }
        let mut next = [0usize; MAX_INPUTS];
        let mut out = io.outputs.trigger(OUT);
        let mut last = None;
        while let Some((k, at)) = earliest(&lists[..n], &next, |&at| at) {
            next[k] += 1;
            if last != Some(at) {
                out.push(at);
                last = Some(at);
            }
        }
    }

    fn merge_midi(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let n = self.inputs;
        let mut lists: [&[MidiEvent]; MAX_INPUTS] = [&[]; MAX_INPUTS];
        for (k, list) in lists.iter_mut().enumerate().take(n) {
            *list = io.inputs.midi(k);
        }
        let last = ctx.frames.saturating_sub(1) as u32;
        let mut tx = Sender::new(io.outputs.midi(OUT));
        if std::mem::take(&mut self.flush) {
            self.release_all(&mut tx);
        }
        let mut next = [0usize; MAX_INPUTS];
        while let Some((k, ev)) = earliest(&lists[..n], &next, |e| e.offset) {
            next[k] += 1;
            // Malformed offsets past the block land on its last sample.
            self.route(&mut tx, k, ev.offset.min(last), ev.msg);
        }
    }

    /// Pass on (or hold back) one message from input `k`.
    fn route(&mut self, tx: &mut Sender, k: usize, at: u32, msg: MidiMsg) {
        let bit = 1u16 << k;
        let owed = self.owed();
        if let Some((ch, note, _)) = note_on_of(&msg) {
            let Some(held) = self.holder(ch, note) else {
                return;
            };
            if held & !bit != 0 {
                // Another input already sounds it.
                self.set_holder(ch, note, held | bit);
                return;
            }
            if held & bit != 0 {
                // Its own input strikes it again: end it, then strike it.
                self.sounding = self.sounding.saturating_sub(1);
                self.set_holder(ch, note, 0);
                tx.send(at, note_off(ch, note));
            }
            if tx.send_if_room(at, msg, self.owed() + 1) {
                self.sounding += 1;
                self.set_holder(ch, note, bit);
            }
        } else if let Some((ch, note, _)) = note_off_of(&msg) {
            let Some(held) = self.holder(ch, note) else {
                return;
            };
            if held & bit == 0 {
                return;
            }
            self.set_holder(ch, note, held & !bit);
            if held == bit {
                self.sounding = self.sounding.saturating_sub(1);
                tx.send(at, msg);
            }
        } else if let MidiMsg::ControlChange {
            channel,
            controller: SUSTAIN_PEDAL,
            value,
        } = msg
        {
            let Some(mask) = self.pedals.get_mut(usize::from(channel & 15)) else {
                return;
            };
            let others = *mask & !bit;
            let mine = *mask & bit != 0;
            let down = value >= 64;
            if others != 0 {
                // Another input holds the pedal down; this one's changes
                // only count for when that one lets go.
                *mask = if down { *mask | bit } else { others };
            } else if !down && mine {
                *mask = 0;
                tx.send(at, msg);
            } else if !down || mine {
                // Already up, or a new depth for a pedal this input holds.
                tx.send_if_room(at, msg, owed);
            } else if tx.send_if_room(at, msg, owed + 1) {
                *mask = bit;
            }
        } else if is_all_notes_off(&msg) {
            let ch = msg.channel() & 15;
            for note in 0..128u8 {
                let Some(held) = self.holder(ch, note) else {
                    continue;
                };
                if held & bit == 0 {
                    continue;
                }
                self.set_holder(ch, note, held & !bit);
                if held == bit {
                    self.sounding = self.sounding.saturating_sub(1);
                    tx.send(at, note_off(ch, note));
                }
            }
            let ch = usize::from(ch);
            let pedal = self.pedals.get(ch).copied().unwrap_or(0);
            let others_hold = pedal & !bit != 0
                || self
                    .holders
                    .get(ch)
                    .is_some_and(|notes| notes.iter().any(|&m| m != 0));
            // It ends everything downstream (pedal included), so it passes
            // only when that leaves no other input's notes cut short.
            if !others_hold
                && tx.send_if_room(at, msg, self.owed())
                && let Some(p) = self.pedals.get_mut(ch)
            {
                *p = 0;
            }
        } else {
            tx.send_if_room(at, msg, owed);
        }
    }

    fn holder(&self, ch: u8, note: u8) -> Option<u16> {
        self.holders
            .get(usize::from(ch))
            .and_then(|notes| notes.get(usize::from(note)))
            .copied()
    }

    fn set_holder(&mut self, ch: u8, note: u8, mask: u16) {
        if let Some(m) = self
            .holders
            .get_mut(usize::from(ch))
            .and_then(|notes| notes.get_mut(usize::from(note)))
        {
            *m = mask;
        }
    }

    /// Lift every pedal and end every note still sounding, at offset 0.
    fn release_all(&mut self, tx: &mut Sender) {
        for (ch, (notes, pedal)) in (0u8..).zip(self.holders.iter_mut().zip(&mut self.pedals)) {
            if *pedal != 0 {
                tx.send(
                    0,
                    MidiMsg::ControlChange {
                        channel: ch,
                        controller: SUSTAIN_PEDAL,
                        value: 0,
                    },
                );
                *pedal = 0;
            }
            for (note, held) in (0u8..).zip(notes.iter_mut()) {
                if *held != 0 {
                    tx.send(0, note_off(ch, note));
                    *held = 0;
                }
            }
        }
        self.sounding = 0;
    }
}

/// The input whose next event (at `next[k]` in `lists[k]`) comes first,
/// and that event; on a tie the lower input wins.
fn earliest<T: Copy>(
    lists: &[&[T]],
    next: &[usize],
    offset: impl Fn(&T) -> u32,
) -> Option<(usize, T)> {
    let mut pick: Option<(usize, T)> = None;
    for (k, (list, &i)) in lists.iter().zip(next).enumerate() {
        if let Some(&ev) = list.get(i)
            && pick.as_ref().is_none_or(|(_, p)| offset(&ev) < offset(p))
        {
            pick = Some((k, ev));
        }
    }
    pick
}

fn note_off(channel: u8, note: u8) -> MidiMsg {
    MidiMsg::NoteOff {
        channel,
        note,
        velocity: RELEASE_VELOCITY,
    }
}

impl Node for Merge {
    fn meta() -> NodeMeta {
        Self::base_meta()
    }

    fn describe(config: &Config) -> Signature {
        let n = input_count(config);
        let mut sig = Signature::new();
        match MergeType::from_config(config) {
            MergeType::Midi => {
                for i in 0..n {
                    sig = sig.input(SocketSpec::midi(format!("in_{i}")).doc("MIDI to merge."));
                }
                sig.output(
                    SocketSpec::midi("out")
                        .doc("Every input's MIDI in time order, each held note once."),
                )
            }
            MergeType::Trigger => {
                for i in 0..n {
                    sig =
                        sig.input(SocketSpec::trigger(format!("in_{i}")).doc("Triggers to merge."));
                }
                sig.output(SocketSpec::trigger("out").doc("Fires wherever any input fires."))
            }
        }
    }

    fn new(config: &Config, _: &PrepareCtx) -> Self {
        Merge {
            ty: MergeType::from_config(config),
            inputs: input_count(config),
            holders: [[0; 128]; 16],
            pedals: [0; 16],
            sounding: 0,
            flush: false,
        }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        match self.ty {
            MergeType::Trigger => self.merge_triggers(io),
            MergeType::Midi => self.merge_midi(ctx, io),
        }
    }

    fn reset(&mut self) {
        self.flush = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{MIDI_CAPACITY, NoResources, SignalType};
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

    fn released(note: u8) -> MidiMsg {
        note_off(0, note)
    }

    fn cc(controller: u8, value: u8) -> MidiMsg {
        MidiMsg::ControlChange {
            channel: 0,
            controller,
            value,
        }
    }

    fn midi(inputs: usize) -> Harness {
        Harness::new::<Merge>(json!({ "inputs": inputs }))
    }

    /// Notes on and off per (channel, note) never go below zero, and end at
    /// zero: every note-on got one note-off and none is extra.
    fn balanced(evs: &[(u64, MidiMsg)]) -> bool {
        let mut held = [[0i32; 128]; 16];
        for (_, m) in evs {
            match *m {
                MidiMsg::NoteOn {
                    channel,
                    note,
                    velocity,
                } if velocity > 0 => held[usize::from(channel)][usize::from(note)] += 1,
                MidiMsg::NoteOff { channel, note, .. } => {
                    let h = &mut held[usize::from(channel)][usize::from(note)];
                    *h -= 1;
                    if *h < 0 {
                        return false;
                    }
                }
                _ => {}
            }
        }
        held.iter().flatten().all(|&h| h == 0)
    }

    #[test]
    fn config_sets_count_and_type() {
        let sig = Merge::describe(&json!({}));
        let names: Vec<_> = sig.inputs.iter().map(|s| s.name.to_string()).collect();
        assert_eq!(names, ["in_0", "in_1"]);
        assert!(sig.inputs.iter().all(|s| s.ty == SignalType::Midi));
        assert_eq!(sig.outputs.len(), 1);
        assert_eq!(sig.outputs[OUT].name, "out");
        assert_eq!(sig.outputs[OUT].ty, SignalType::Midi);

        for (ty, want) in [
            ("midi", SignalType::Midi),
            ("trigger", SignalType::Trigger),
            ("audio", SignalType::Midi),
            ("bogus", SignalType::Midi),
        ] {
            let sig = Merge::describe(&json!({ "type": ty, "inputs": 3 }));
            assert_eq!(sig.inputs.len(), 3, "{ty}");
            assert!(sig.inputs.iter().all(|s| s.ty == want), "{ty}");
            assert_eq!(sig.outputs[OUT].ty, want, "{ty}");
        }
        for (inputs, want) in [
            (json!(0), 2),
            (json!(1), 2),
            (json!(16), 16),
            (json!(500), 16),
            (json!(-4), 2),
            (json!("many"), 2),
        ] {
            let sig = Merge::describe(&json!({ "inputs": inputs }));
            assert_eq!(sig.inputs.len(), want, "inputs {inputs}");
        }
        let sig = Merge::describe(&json!({ "inputs": 99 }));
        assert_eq!(sig.inputs[15].name, "in_15");
        // A clamped config builds and runs with the sockets it describes.
        let mut h = Harness::new::<Merge>(json!({ "inputs": 99 }));
        h.midi("in_15", &[(3, on(60))]);
        assert_eq!(h.run(64).midi("out"), &[(3, on(60))]);
    }

    #[test]
    fn triggers_merge_and_one_sample_fires_once() {
        let mut h = Harness::new::<Merge>(json!({ "type": "trigger", "inputs": 3 }));
        h.triggers("in_0", &[1, 5, 10, 64, 200])
            .triggers("in_1", &[5, 7, 70, 200])
            .triggers("in_2", &[10, 10, 127]);
        assert_eq!(h.run(256).triggers("out"), &[1, 5, 7, 10, 64, 70, 127, 200]);

        // One input alone passes as it is; none, nothing.
        let mut h = Harness::new::<Merge>(json!({ "type": "trigger" }));
        h.triggers("in_1", &[0, 63, 64]);
        assert_eq!(h.run(128).triggers("out"), &[0, 63, 64]);
        let mut h = Harness::new::<Merge>(json!({ "type": "trigger" }));
        assert!(h.run(128).triggers("out").is_empty());
    }

    #[test]
    fn events_come_in_time_order_lower_input_first() {
        let bend = MidiMsg::PitchBend {
            channel: 0,
            value: 300,
        };
        let program = MidiMsg::ProgramChange {
            channel: 2,
            program: 7,
        };
        let pressure = MidiMsg::ChannelPressure {
            channel: 0,
            value: 40,
        };
        let mut h = midi(3);
        h.midi("in_0", &[(5, cc(1, 10)), (5, cc(1, 11)), (20, on(60))])
            .midi("in_1", &[(3, bend), (5, cc(7, 90)), (90, pressure)])
            .midi("in_2", &[(5, program), (20, on(72))]);
        assert_eq!(
            h.run(128).midi("out"),
            &[
                (3, bend),
                (5, cc(1, 10)),
                (5, cc(1, 11)),
                (5, cc(7, 90)),
                (5, program),
                (20, on(60)),
                (20, on(72)),
                (90, pressure),
            ]
        );
    }

    #[test]
    fn a_note_held_from_two_inputs_sounds_once() {
        // Released in the order it was struck, and the other way round.
        for (first, second) in [("in_0", "in_1"), ("in_1", "in_0")] {
            let mut h = midi(2);
            h.midi("in_0", &[(10, on(60))])
                .midi("in_1", &[(20, on(60))]);
            h.run(64);
            h.midi(first, &[(70, off(60))])
                .midi(second, &[(90, off(60))]);
            let out = h.run(64);
            assert_eq!(out.midi("out"), &[(90, off(60))], "{first} let go first");
        }
        let mut h = midi(2);
        h.midi("in_0", &[(10, on(60)), (30, off(60))])
            .midi("in_1", &[(20, on(60)), (40, off(60))]);
        assert_eq!(h.run(64).midi("out"), &[(10, on(60)), (40, off(60))]);

        // The same note number on another channel is another note.
        let on_ch2 = MidiMsg::NoteOn {
            channel: 1,
            note: 60,
            velocity: 90,
        };
        let off_ch2 = MidiMsg::NoteOff {
            channel: 1,
            note: 60,
            velocity: 0,
        };
        let mut h = midi(2);
        h.midi("in_0", &[(10, on(60)), (30, off(60))])
            .midi("in_1", &[(20, on_ch2), (40, off_ch2)]);
        assert_eq!(
            h.run(64).midi("out"),
            &[(10, on(60)), (20, on_ch2), (30, off(60)), (40, off_ch2)]
        );
    }

    #[test]
    fn a_zero_velocity_note_on_lets_go_too() {
        let quiet = MidiMsg::NoteOn {
            channel: 0,
            note: 60,
            velocity: 0,
        };
        let mut h = midi(2);
        h.midi("in_0", &[(10, on(60)), (30, quiet)])
            .midi("in_1", &[(20, on(60)), (40, quiet)]);
        assert_eq!(h.run(64).midi("out"), &[(10, on(60)), (40, quiet)]);
    }

    #[test]
    fn a_repeated_note_retriggers_unless_another_input_holds_it() {
        let mut h = midi(2);
        h.midi("in_0", &[(0, on(60)), (10, on(60)), (20, off(60))]);
        assert_eq!(
            h.run(64).midi("out"),
            &[(0, on(60)), (10, released(60)), (10, on(60)), (20, off(60))]
        );

        let mut h = midi(2);
        h.midi("in_0", &[(0, on(60)), (10, on(60)), (20, off(60))])
            .midi("in_1", &[(5, on(60)), (30, off(60))]);
        assert_eq!(h.run(64).midi("out"), &[(0, on(60)), (30, off(60))]);
    }

    #[test]
    fn note_offs_for_notes_not_held_are_dropped() {
        let mut h = midi(2);
        h.midi("in_1", &[(5, off(60))]);
        assert!(h.run(64).midi("out").is_empty(), "nothing was held");

        // in_1 never held 62: its note-off doesn't cut in_0's note short.
        let mut h = midi(2);
        h.midi("in_0", &[(0, on(62)), (50, off(62))])
            .midi("in_1", &[(10, off(62))]);
        assert_eq!(h.run(64).midi("out"), &[(0, on(62)), (50, off(62))]);
    }

    #[test]
    fn the_sustain_pedal_stays_down_while_any_input_holds_it() {
        let mut h = midi(3);
        h.midi("in_0", &[(0, cc(64, 127)), (10, cc(64, 0))])
            .midi("in_1", &[(5, cc(64, 127)), (15, cc(64, 0))]);
        assert_eq!(h.run(64).midi("out"), &[(0, cc(64, 127)), (15, cc(64, 0))]);

        // Half-pedal depths from the only input holding it pass; a pedal
        // already up passes its own "up".
        let mut h = midi(2);
        h.midi(
            "in_0",
            &[(0, cc(64, 100)), (4, cc(64, 127)), (8, cc(64, 0))],
        )
        .midi("in_1", &[(12, cc(64, 0))]);
        assert_eq!(
            h.run(64).midi("out"),
            &[
                (0, cc(64, 100)),
                (4, cc(64, 127)),
                (8, cc(64, 0)),
                (12, cc(64, 0))
            ]
        );
    }

    #[test]
    fn all_notes_off_ends_only_its_inputs_notes() {
        let mut h = midi(2);
        h.midi("in_0", &[(0, on(60)), (1, on(62)), (10, cc(123, 0))])
            .midi("in_1", &[(2, on(62)), (3, on(64)), (20, cc(123, 0))]);
        assert_eq!(
            h.run(64).midi("out"),
            &[
                (0, on(60)),
                (1, on(62)),
                (3, on(64)),
                // in_1 still plays 62 and 64: in_0's all-notes-off ends 60.
                (10, released(60)),
                (20, released(62)),
                (20, released(64)),
                (20, cc(123, 0)),
            ]
        );
        // in_0's later note-offs for its ended notes are dropped.
        h.midi("in_0", &[(70, off(60)), (71, off(62))]);
        assert!(h.run(64).midi("out").is_empty());

        // A pedal held by another input keeps it back too; all-sound-off
        // from the only input playing passes.
        let mut h = midi(2);
        h.midi("in_0", &[(0, on(60)), (10, cc(120, 0)), (30, cc(120, 0))])
            .midi("in_1", &[(5, cc(64, 127)), (20, cc(64, 0))]);
        assert_eq!(
            h.run(64).midi("out"),
            &[
                (0, on(60)),
                (5, cc(64, 127)),
                (10, released(60)),
                (20, cc(64, 0)),
                (30, cc(120, 0)),
            ]
        );
    }

    #[test]
    fn reset_releases_held_notes_and_pedals() {
        let mut h = midi(2);
        h.midi("in_0", &[(0, on(60)), (1, cc(64, 127))])
            .midi("in_1", &[(2, on(60)), (3, on(67))]);
        assert_eq!(h.run(64).midi("out").len(), 3);
        h.reset();
        // The sources let go after the reset; those note-offs are dropped,
        // and a new note plays.
        h.midi("in_0", &[(64, off(60)), (100, on(72))])
            .midi("in_1", &[(64, off(60)), (64, off(67))]);
        assert_eq!(
            h.run(64).midi("out"),
            &[
                (64, cc(64, 0)),
                (64, released(60)),
                (64, released(67)),
                (100, on(72)),
            ]
        );
        h.reset();
        assert_eq!(h.run(64).midi("out"), &[(128, released(72))]);
        h.reset();
        assert!(h.run(64).midi("out").is_empty(), "nothing left to release");
    }

    #[test]
    fn a_flood_never_loses_a_note_off() {
        let block = 512;
        let mut h =
            Harness::with_resources::<Merge>(json!({ "inputs": 3 }), 48_000.0, block, &NoResources);
        // Block 1: 2048 different notes from in_0 and in_1 (all of them,
        // twice over), more than an output holds.
        let all: Vec<(u64, MidiMsg)> = (0..2_048u64)
            .map(|k| {
                let msg = MidiMsg::NoteOn {
                    channel: (k / 128) as u8,
                    note: (k % 128) as u8,
                    velocity: 100,
                };
                (k / 8, msg)
            })
            .collect();
        // Block 2: a wall of controllers from in_2 before every note-off.
        let wall: Vec<(u64, MidiMsg)> = (0..3_000).map(|_| (block as u64, cc(7, 64))).collect();
        let offs: Vec<(u64, MidiMsg)> = all
            .iter()
            .map(|&(_, m)| {
                let (channel, note, _) = note_on_of(&m).expect("a note-on");
                let msg = MidiMsg::NoteOff {
                    channel,
                    note,
                    velocity: 0,
                };
                (block as u64 + 1, msg)
            })
            .collect();
        let both: Vec<_> = all.iter().chain(&offs).copied().collect();
        h.midi("in_0", &both)
            .midi("in_1", &both)
            .midi("in_2", &wall);
        let first = h.run(block).midi("out").to_vec();
        assert!(first.len() <= MIDI_CAPACITY);
        // Each note struck may end in the same block: half the room each.
        assert_eq!(first.len(), MIDI_CAPACITY / 2);
        let second = h.run(block).midi("out").to_vec();
        assert!(second.len() <= MIDI_CAPACITY);
        let mut evs = first.clone();
        evs.extend_from_slice(&second);
        assert!(balanced(&evs), "every note that started ended");
        assert!(evs.windows(2).all(|w| w[0].0 <= w[1].0));

        // A flood held over a reset is released in one block.
        let mut h = Harness::with_resources::<Merge>(json!({}), 48_000.0, block, &NoResources);
        h.midi("in_0", &all).midi("in_1", &all);
        let mut evs = h.run(block).midi("out").to_vec();
        h.reset();
        let flushed = h.run(block).midi("out").to_vec();
        assert!(flushed.len() <= MIDI_CAPACITY);
        evs.extend_from_slice(&flushed);
        assert!(balanced(&evs), "the reset released every note");
    }

    #[test]
    fn block_size_does_not_matter() {
        // Three inputs with overlapping notes, repeats, pedals, and
        // controllers, many on shared samples.
        let stream = |seed: u64| -> Vec<(u64, MidiMsg)> {
            let mut s = seed.wrapping_mul(6_364_136_223_846_793_005) | 1;
            // All before the reset at 4000.
            (0..550u64)
                .map(|k| {
                    s ^= s << 13;
                    s ^= s >> 7;
                    s ^= s << 17;
                    let note = 60 + (s % 12) as u8;
                    let channel = ((s >> 8) % 2) as u8;
                    let msg = match (s >> 16) % 8 {
                        0..=2 => MidiMsg::NoteOn {
                            channel,
                            note,
                            velocity: 1 + ((s >> 24) % 127) as u8,
                        },
                        3..=5 => MidiMsg::NoteOff {
                            channel,
                            note,
                            velocity: 0,
                        },
                        6 => MidiMsg::ControlChange {
                            channel,
                            controller: 64,
                            value: if (s >> 32) & 1 == 0 { 127 } else { 0 },
                        },
                        _ => MidiMsg::ControlChange {
                            channel,
                            controller: [1, 123][((s >> 40) % 2) as usize],
                            value: 5,
                        },
                    };
                    (k * 7 + (s >> 48) % 5, msg)
                })
                .collect()
        };
        let render = |block: usize| {
            let mut h = Harness::with_resources::<Merge>(
                json!({ "inputs": 3 }),
                48_000.0,
                block,
                &NoResources,
            );
            h.midi("in_0", &stream(1))
                .midi("in_1", &stream(2))
                .midi("in_2", &stream(3));
            let mut evs = h.run(4_000).midi("out").to_vec();
            h.reset();
            // The reset lands on a frame every block size starts a block on.
            evs.extend_from_slice(h.run(512).midi("out"));
            evs
        };
        let one = render(1);
        assert!(one.len() > 300, "{}", one.len());
        assert!(balanced(&one));
        assert_eq!(one, render(64));
        assert_eq!(one, render(512));

        let triggers = |block: usize| {
            let mut h = Harness::with_resources::<Merge>(
                json!({ "type": "trigger", "inputs": 4 }),
                48_000.0,
                block,
                &NoResources,
            );
            for k in 0..4u64 {
                let at: Vec<u64> = (0..4_000).step_by(3 + k as usize * 4).collect();
                h.triggers(&format!("in_{k}"), &at);
            }
            h.run(4_000).triggers("out").to_vec()
        };
        let one = triggers(1);
        assert!(one.windows(2).all(|w| w[0] < w[1]), "ascending, no repeats");
        assert_eq!(one, triggers(64));
        assert_eq!(one, triggers(512));
    }
}
