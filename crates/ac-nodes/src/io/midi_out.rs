use ac_core::{
    Config, Io, MidiEvent, MidiMsg, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec,
    node,
};

node! {
    kind: "midi_out",
    name: "MIDI Out",
    category: Io,
    keywords: ["send", "device", "hardware", "synth", "output", "controller", "external"],
    /// Sends MIDI to the connected devices.
    ///
    /// Every message keeps its timing within the block. `channel` re-channels
    /// everything: 0 keeps each message's own channel, 1–16 sends them all on
    /// that one.
    ///
    /// When the transport stops, the node ends every note it started and
    /// hasn't ended yet, so a hardware synth isn't left droning. A `panic`
    /// trigger does the same and also sends All Notes Off on all sixteen
    /// channels, for notes something else left hanging.
    ///
    /// The host chooses the device (`audioc run --midi-out`, or the Devices
    /// tab in the editor). An offline render collects the messages instead
    /// (`audioc render --midi-out song.mid`).
    ///
    /// **Try this:** wire a `Pattern` or an `Arpeggiator` into a MIDI Out to
    /// play a hardware synth in time with the patch, then route the synth's
    /// audio back in through an `Audio In` to process it.
    pub struct MidiOut {
        /// Notes this node has started and not ended, one bit per note, per channel.
        held: [u128; 16],
    }
}

const MIDI: usize = 0;
const CHANNEL: usize = 1;
const PANIC: usize = 2;

/// All Notes Off (MIDI 1.0 channel mode message).
const ALL_NOTES_OFF: u8 = 123;

/// `msg` moved to channel `filter - 1`, or unchanged for 0 (keep).
fn rechannel(msg: MidiMsg, filter: i32) -> MidiMsg {
    if filter <= 0 {
        return msg;
    }
    let channel = (filter.min(16) - 1) as u8;
    let (mut bytes, len) = msg.to_bytes();
    bytes[0] = (bytes[0] & 0xf0) | channel;
    MidiMsg::from_bytes(&bytes[..len]).unwrap_or(msg)
}

impl MidiOut {
    fn track(&mut self, msg: &MidiMsg) {
        match *msg {
            MidiMsg::NoteOn { channel, note, .. } => {
                self.held[usize::from(channel & 15)] |= 1 << (note & 127);
            }
            MidiMsg::NoteOff { channel, note, .. } => {
                self.held[usize::from(channel & 15)] &= !(1 << (note & 127));
            }
            _ => {}
        }
    }

    /// Note-offs for every held note, at `offset`.
    fn release_all(&mut self, io: &mut Io, offset: u32) {
        for (channel, bits) in self.held.iter_mut().enumerate() {
            while *bits != 0 {
                let note = bits.trailing_zeros() as u8;
                *bits &= !(1 << note);
                io.host.send_midi(MidiEvent::new(
                    offset,
                    MidiMsg::NoteOff {
                        channel: channel as u8,
                        note,
                        velocity: 64,
                    },
                ));
            }
        }
    }
}

impl Node for MidiOut {
    fn meta() -> NodeMeta {
        // Writes to the device, so it runs on the device-output path.
        Self::base_meta().host_output()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::midi("midi").doc("Messages to send."))
            .input(
                SocketSpec::int("channel", 0)
                    .range(0.0, 16.0)
                    .doc("Send everything on this channel, 1–16; 0 keeps each message's channel."),
            )
            .input(SocketSpec::trigger("panic").doc("Ends every note, on every channel."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        MidiOut { held: [0; 16] }
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        if ctx.transport.stopped {
            self.release_all(io, 0);
        }
        let filter = io.inputs.int(CHANNEL);
        let panic_at = io.inputs.triggers(PANIC).first().copied();
        let midi = io.inputs.midi(MIDI);
        for ev in midi.iter() {
            if panic_at.is_some_and(|p| ev.offset >= p) {
                break;
            }
            let msg = rechannel(ev.msg, filter);
            self.track(&msg);
            io.host.send_midi(MidiEvent::new(ev.offset, msg));
        }
        if let Some(p) = panic_at {
            self.release_all(io, p);
            for channel in 0..16 {
                io.host.send_midi(MidiEvent::new(
                    p,
                    MidiMsg::ControlChange {
                        channel,
                        controller: ALL_NOTES_OFF,
                        value: 0,
                    },
                ));
            }
            // Messages after the panic in the same block still go out.
            for ev in midi.iter().filter(|e| e.offset > p) {
                let msg = rechannel(ev.msg, filter);
                self.track(&msg);
                io.host.send_midi(MidiEvent::new(ev.offset, msg));
            }
        }
    }

    fn reset(&mut self) {
        // A restart can't send; notes still sounding end at the next stop.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    fn on(channel: u8, note: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel,
            note,
            velocity: 100,
        }
    }

    fn off(channel: u8, note: u8) -> MidiMsg {
        MidiMsg::NoteOff {
            channel,
            note,
            velocity: 64,
        }
    }

    #[test]
    fn sends_input_with_its_timing() {
        let mut h = Harness::new::<MidiOut>(json!({}));
        let evs = [(3, on(0, 60)), (70, on(5, 62)), (130, off(0, 60))];
        h.midi("midi", &evs);
        let out = h.run(256);
        assert_eq!(out.host_midi_out, evs);
    }

    #[test]
    fn rechannels_when_asked() {
        let mut h = Harness::new::<MidiOut>(json!({}));
        h.set("channel", 10);
        h.midi(
            "midi",
            &[
                (0, on(0, 60)),
                (
                    1,
                    MidiMsg::PitchBend {
                        channel: 3,
                        value: -100,
                    },
                ),
            ],
        );
        let out = h.run(64);
        assert_eq!(
            out.host_midi_out,
            [
                (0, on(9, 60)),
                (
                    1,
                    MidiMsg::PitchBend {
                        channel: 9,
                        value: -100
                    }
                )
            ]
        );
    }

    #[test]
    fn stopping_the_transport_ends_held_notes() {
        let mut h = Harness::new::<MidiOut>(json!({}));
        h.midi("midi", &[(0, on(0, 60)), (1, on(2, 64)), (2, off(0, 60))]);
        h.run(64);
        h.stop();
        let out = h.run(64);
        assert_eq!(out.host_midi_out, [(64, off(2, 64))]);
        // Nothing is held any more.
        h.stop();
        assert!(h.run(64).host_midi_out.is_empty());
    }

    #[test]
    fn panic_ends_notes_and_sends_all_notes_off() {
        let mut h = Harness::new::<MidiOut>(json!({}));
        h.midi("midi", &[(0, on(0, 60)), (20, on(1, 61))]);
        h.triggers("panic", &[10]);
        let out = h.run(64);
        let sent = &out.host_midi_out;
        assert_eq!(sent[0], (0, on(0, 60)));
        assert_eq!(sent[1], (10, off(0, 60)));
        let cc = sent
            .iter()
            .filter(|(at, m)| {
                *at == 10
                    && matches!(
                        m,
                        MidiMsg::ControlChange {
                            controller: ALL_NOTES_OFF,
                            ..
                        }
                    )
            })
            .count();
        assert_eq!(cc, 16);
        // The note after the panic still plays.
        assert_eq!(sent.last(), Some(&(20, on(1, 61))));
    }

    #[test]
    fn runs_on_the_device_output_path() {
        assert!(MidiOut::meta().host_output);
    }

    #[test]
    fn rechannel_keeps_or_moves() {
        assert_eq!(rechannel(on(4, 1), 0), on(4, 1));
        assert_eq!(rechannel(on(4, 1), 1), on(0, 1));
        assert_eq!(rechannel(on(4, 1), 99), on(15, 1));
    }
}
