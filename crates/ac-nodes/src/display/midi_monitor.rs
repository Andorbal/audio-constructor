use ac_core::{
    Config, Io, MidiEvent, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, node,
};

node! {
    kind: "midi_monitor",
    name: "MIDI Monitor",
    category: Display,
    keywords: ["midi", "monitor", "debug", "log", "events", "notes", "inspect", "sniffer"],
    /// Shows the last MIDI messages on a wire, newest first: notes with
    /// their names and velocities, controllers, pitch bend, pressure, and
    /// program changes, each with its channel.
    ///
    /// It passes `midi` through to `out` unchanged, so it can sit inline.
    /// The card keeps the last eight messages and how many have passed.
    ///
    /// **Try this:** put a MIDI Monitor between a `MIDI In` and whatever it
    /// plays to see which notes and controllers your keyboard really sends,
    /// or after a `Note Split`'s source to check a pad's note number.
    pub struct MidiMonitor {
        /// The newest messages as raw bytes, oldest overwritten first.
        recent: [[u8; 3]; SHOWN],
        next: usize,
        count: u64,
    }
}

/// Messages the card shows.
pub const SHOWN: usize = 8;
/// Telemetry: messages seen so far, then the last [`SHOWN`] as three bytes
/// each, newest first (status 0 for an empty slot).
pub const TELEMETRY_COUNT: usize = 0;
pub const TELEMETRY_MESSAGES: usize = 1;

impl Node for MidiMonitor {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(TELEMETRY_MESSAGES + 3 * SHOWN)
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(SocketSpec::midi("midi").doc("Messages to show."))
            .output(SocketSpec::midi("out").doc("The same messages, passed on."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        MidiMonitor {
            recent: [[0; 3]; SHOWN],
            next: 0,
            count: 0,
        }
    }

    fn process(&mut self, _: &ProcessCtx, io: &mut Io) {
        let events = io.inputs.midi(0);
        let mut out = io.outputs.midi(0);
        for e in events {
            out.push(MidiEvent::new(e.offset, e.msg));
            let (bytes, _) = e.msg.to_bytes();
            self.recent[self.next] = bytes;
            self.next = (self.next + 1) % SHOWN;
            self.count += 1;
        }
        if !events.is_empty() {
            io.telemetry
                .set(TELEMETRY_COUNT, (self.count % (1 << 24)) as f32);
            for k in 0..SHOWN {
                // Newest first.
                let m = self.recent[(self.next + SHOWN - 1 - k) % SHOWN];
                for (b, v) in m.iter().enumerate() {
                    io.telemetry
                        .set(TELEMETRY_MESSAGES + 3 * k + b, f32::from(*v));
                }
            }
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::MidiMsg;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn it_passes_messages_on_and_shows_the_newest_first() {
        let mut h = Harness::new::<MidiMonitor>(json!({}));
        let on = MidiMsg::NoteOn {
            channel: 1,
            note: 60,
            velocity: 100,
        };
        let cc = MidiMsg::ControlChange {
            channel: 0,
            controller: 7,
            value: 90,
        };
        h.midi("midi", &[(2, on), (5, cc)]);
        let out = h.run(64);
        assert_eq!(out.midi("out"), &[(2, on), (5, cc)]);
        assert_eq!(h.telemetry(TELEMETRY_COUNT), 2.0);
        let msg = |k: usize| {
            let b = |i| h.telemetry(TELEMETRY_MESSAGES + 3 * k + i) as u8;
            MidiMsg::from_bytes(&[b(0), b(1), b(2)])
        };
        assert_eq!(msg(0), Some(cc));
        assert_eq!(msg(1), Some(on));
        assert_eq!(msg(2), None, "an empty slot");
    }
}
