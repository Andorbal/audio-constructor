//! Bookkeeping shared by the nodes that send notes of their own: a MIDI
//! output that keeps room for the note-offs a node owes, and a table of the
//! output notes each held input note started, so its note-off releases
//! exactly those (Chord, MIDI Transpose, Scale Quantize).
//!
//! Nothing here allocates after construction.

use ac_core::{Io, MidiEvent, MidiMsg, MidiOut};
use ac_dsp::scale::{SCALE_NAMES, Scale};

/// Events a MIDI output holds per block (see [`ac_core::MIDI_CAPACITY`]).
pub(crate) const MIDI_LIMIT: usize = ac_core::MIDI_CAPACITY;

/// Velocity for note-offs a node sends on its own (releases, flushes).
pub(crate) const RELEASE_VELOCITY: u8 = 64;

/// A MIDI output that counts what it sends this block.
pub(crate) struct Sender<'a> {
    out: MidiOut<'a>,
    sent: usize,
}

impl<'a> Sender<'a> {
    pub fn new(out: MidiOut<'a>) -> Self {
        Sender { out, sent: 0 }
    }

    /// Sends a message that must not be lost, such as a note-off.
    pub fn send(&mut self, offset: u32, msg: MidiMsg) {
        self.out.push(MidiEvent::new(offset, msg));
        self.sent += 1;
    }

    /// Sends `msg` only if that still leaves room for `reserve` more events
    /// this block; returns whether it was sent.
    pub fn send_if_room(&mut self, offset: u32, msg: MidiMsg, reserve: usize) -> bool {
        if self.sent + 1 + reserve > MIDI_LIMIT {
            return false;
        }
        self.send(offset, msg);
        true
    }
}

/// Sends a note-off for `note` on `channel`.
pub(crate) fn note_off(tx: &mut Sender, offset: u32, channel: u8, note: u8, velocity: u8) {
    tx.send(
        offset,
        MidiMsg::NoteOff {
            channel,
            note,
            velocity,
        },
    );
}

/// The major scale as a mask, the default for a `custom` scale's `mask`.
pub(crate) const MAJOR_MASK: i64 = 0b1010_1011_0101;

/// The scale picked by a `scale` choice input and a `mask` input (the
/// bits used for `custom`).
pub(crate) fn chosen_scale(io: &Io, scale: usize, mask: usize) -> Scale {
    let bits = io.inputs.int(mask).clamp(0, 0xfff) as u16;
    Scale::from_index(io.inputs.choice(scale, SCALE_NAMES.len()), bits)
}

/// True for all-sound-off and all-notes-off (controllers 120 and 123).
pub(crate) fn is_all_notes_off(msg: &MidiMsg) -> bool {
    matches!(
        msg,
        MidiMsg::ControlChange {
            controller: 120 | 123,
            ..
        }
    )
}

/// A note-on with a nonzero velocity: `(channel, note, velocity)`.
pub(crate) fn note_on_of(msg: &MidiMsg) -> Option<(u8, u8, u8)> {
    match *msg {
        MidiMsg::NoteOn {
            channel,
            note,
            velocity,
        } if velocity > 0 => Some((channel & 15, note & 127, velocity.min(127))),
        _ => None,
    }
}

/// A note-off (or a zero-velocity note-on): `(channel, note, velocity)`.
pub(crate) fn note_off_of(msg: &MidiMsg) -> Option<(u8, u8, u8)> {
    match *msg {
        MidiMsg::NoteOff {
            channel,
            note,
            velocity,
        } => Some((channel & 15, note & 127, velocity.min(127))),
        MidiMsg::NoteOn { channel, note, .. } => Some((channel & 15, note & 127, 0)),
        _ => None,
    }
}

const KEYS: usize = 16 * 128;
const NONE: u8 = u8::MAX;

fn key(channel: u8, note: u8) -> usize {
    usize::from(channel & 15) * 128 + usize::from(note & 127)
}

/// Which output notes (up to `N`) each held input note started, per MIDI
/// channel, and how many held input notes hold each output note.
///
/// An output note held by several input notes is sent once and released
/// when the last of them lets go, so every note-on gets exactly one
/// note-off.
pub(crate) struct NoteMap<const N: usize> {
    /// Indexed by `key(channel, input note)`; unused entries are `NONE`.
    started: Vec<[u8; N]>,
    /// Indexed by `key(channel, output note)`.
    count: Vec<u8>,
    /// Output notes with a nonzero count.
    sounding: usize,
}

impl<const N: usize> NoteMap<N> {
    pub fn new() -> Self {
        NoteMap {
            started: vec![[NONE; N]; KEYS],
            count: vec![0; KEYS],
            sounding: 0,
        }
    }

    /// Output notes sounding now (each needs a note-off).
    pub fn sounding(&self) -> usize {
        self.sounding
    }

    /// The output notes input `note` on `channel` started (empty when it
    /// isn't held).
    pub fn started(&self, channel: u8, note: u8) -> &[u8] {
        let list = self
            .started
            .get(key(channel, note))
            .map_or(&[][..], |l| &l[..]);
        let n = list.iter().position(|&o| o == NONE).unwrap_or(list.len());
        list.get(..n).unwrap_or(&[])
    }

    /// An input note-on: ends whatever that input note started before
    /// (a repeated note-on retriggers), then starts `outs` (note numbers;
    /// those outside 0–127 are skipped, repeats count once). Output notes
    /// another held input note already sounds aren't sent again. A note-on
    /// that would leave no room in the block for the note-offs owed is
    /// dropped (and so is its note-off).
    pub fn press(
        &mut self,
        tx: &mut Sender,
        offset: u32,
        channel: u8,
        note: u8,
        velocity: u8,
        outs: &[i32],
    ) {
        self.release(tx, offset, channel, note, RELEASE_VELOCITY);
        let k = key(channel, note);
        let mut list = [NONE; N];
        let mut len = 0;
        for &o in outs {
            if len >= N {
                break;
            }
            let Ok(out) = u8::try_from(o) else {
                continue;
            };
            if out > 127 || list[..len].contains(&out) {
                continue;
            }
            let ck = key(channel, out);
            let Some(count) = self.count.get(ck).copied() else {
                continue;
            };
            if count == 0 {
                let msg = MidiMsg::NoteOn {
                    channel: channel & 15,
                    note: out,
                    velocity: velocity.clamp(1, 127),
                };
                if !tx.send_if_room(offset, msg, self.sounding + 1) {
                    continue;
                }
                self.sounding += 1;
            }
            if let Some(c) = self.count.get_mut(ck) {
                *c = c.saturating_add(1);
            }
            list[len] = out;
            len += 1;
        }
        if let Some(slot) = self.started.get_mut(k) {
            *slot = list;
        }
    }

    /// An input note-off: releases what that input note started.
    pub fn release(&mut self, tx: &mut Sender, offset: u32, channel: u8, note: u8, velocity: u8) {
        let k = key(channel, note);
        let Some(list) = self.started.get(k).copied() else {
            return;
        };
        if list[0] == NONE {
            return;
        }
        for out in list.into_iter().take_while(|&o| o != NONE) {
            let Some(c) = self.count.get_mut(key(channel, out)) else {
                continue;
            };
            if *c == 0 {
                continue;
            }
            *c -= 1;
            if *c == 0 {
                self.sounding = self.sounding.saturating_sub(1);
                note_off(tx, offset, channel & 15, out, velocity);
            }
        }
        if let Some(slot) = self.started.get_mut(k) {
            *slot = [NONE; N];
        }
    }

    /// Releases every held input note on `channel`.
    pub fn release_channel(&mut self, tx: &mut Sender, offset: u32, channel: u8) {
        if self.sounding == 0 {
            return;
        }
        for note in 0..128u8 {
            self.release(tx, offset, channel, note, RELEASE_VELOCITY);
        }
    }

    /// Releases everything.
    pub fn release_all(&mut self, tx: &mut Sender, offset: u32) {
        for channel in 0..16u8 {
            self.release_channel(tx, offset, channel);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(f: impl FnOnce(&mut Sender)) -> Vec<MidiEvent> {
        let mut midi = vec![Vec::with_capacity(MIDI_LIMIT)];
        let slots = [ac_core::OutputSlot::Midi(0)];
        {
            let mut outputs = ac_core::Outputs {
                slots: &slots,
                frames: 64,
                samples: &mut [],
                scalars: &mut [],
                ints: &mut [],
                triggers: &mut [],
                midi: &mut midi,
                spectra: &mut [],
                buffers: &mut [],
            };
            let mut tx = Sender::new(outputs.midi(0));
            f(&mut tx);
        }
        midi.remove(0)
    }

    fn notes(evs: &[MidiEvent]) -> Vec<i32> {
        evs.iter()
            .filter_map(|e| match e.msg {
                MidiMsg::NoteOn { note, .. } => Some(i32::from(note)),
                MidiMsg::NoteOff { note, .. } => Some(-i32::from(note)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn shared_output_notes_sound_once_and_end_with_the_last_holder() {
        let mut map = NoteMap::<4>::new();
        let evs = run(|tx| {
            map.press(tx, 0, 0, 60, 100, &[60, 64, 67]);
            map.press(tx, 1, 0, 57, 100, &[57, 60, 64]);
            assert_eq!(map.sounding(), 4);
            map.release(tx, 2, 0, 60, 0);
            map.release(tx, 3, 0, 57, 0);
        });
        assert_eq!(notes(&evs), vec![60, 64, 67, 57, -67, -57, -60, -64]);
        assert_eq!(map.sounding(), 0);
    }

    #[test]
    fn out_of_range_and_repeated_outputs_are_skipped() {
        let mut map = NoteMap::<4>::new();
        let evs = run(|tx| {
            map.press(tx, 0, 3, 120, 100, &[120, 132, -1, 120, 127]);
            assert_eq!(map.started(3, 120), &[120, 127]);
            map.release(tx, 5, 3, 120, 0);
        });
        assert_eq!(notes(&evs), vec![120, 127, -120, -127]);
        assert!(evs.iter().all(|e| e.msg.channel() == 3));
    }

    #[test]
    fn a_repeated_note_on_retriggers_off_first() {
        let mut map = NoteMap::<1>::new();
        let evs = run(|tx| {
            map.press(tx, 0, 0, 60, 100, &[62]);
            map.press(tx, 7, 0, 60, 100, &[63]);
            map.release_all(tx, 9);
        });
        assert_eq!(notes(&evs), vec![62, -62, 63, -63]);
        assert_eq!(evs[1].offset, 7);
        assert_eq!(evs[3].offset, 9);
    }

    #[test]
    fn note_ons_give_way_so_note_offs_always_fit() {
        let mut map = NoteMap::<8>::new();
        let evs = run(|tx| {
            for n in 0..128u8 {
                let outs: Vec<i32> = (0..8).map(|k| i32::from(n) + k).collect();
                map.press(tx, 0, n % 16, n, 100, &outs);
            }
            map.release_all(tx, 1);
        });
        assert!(evs.len() <= MIDI_LIMIT);
        let ons = evs
            .iter()
            .filter(|e| matches!(e.msg, MidiMsg::NoteOn { .. }))
            .count();
        let offs = evs.len() - ons;
        assert_eq!(ons, offs);
        assert_eq!(map.sounding(), 0);
    }
}
