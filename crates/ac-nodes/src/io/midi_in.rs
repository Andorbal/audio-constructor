use ac_core::{
    Config, Determinism, Io, MidiEvent, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature,
    SocketSpec, node,
};

node! {
    kind: "midi_in",
    name: "MIDI In",
    category: Io,
    keywords: ["keyboard", "controller", "notes", "device", "input"],
    /// Brings MIDI from the connected devices into the patch.
    ///
    /// Every message keeps its timing within the block, so notes start on
    /// the sample they arrived. `channel` filters by MIDI channel: 0 passes
    /// all channels (omni), 1–16 passes only that one.
    ///
    /// MIDI In is a live source: nodes that need their whole input before
    /// they can play are disabled downstream of it.
    ///
    /// **Try this:** wire `midi` into a `MIDI Note` and its `freq` into an
    /// `Oscillator` for a one-voice synth; use two MIDI In nodes on
    /// different channels to split a keyboard between two sounds.
    pub struct MidiIn {}
}

const CHANNEL: usize = 0;
const MIDI: usize = 0;

/// True if `ev` passes a channel filter (0 = omni, 1..=16 = one channel).
pub(crate) fn channel_matches(filter: i32, ev: &MidiEvent) -> bool {
    filter <= 0 || i32::from(ev.msg.channel()) + 1 == filter.min(16)
}

impl Node for MidiIn {
    fn meta() -> NodeMeta {
        Self::base_meta().live()
    }

    fn describe(_: &Config) -> Signature {
        Signature::new()
            .input(
                SocketSpec::int("channel", 0)
                    .range(0.0, 16.0)
                    .doc("MIDI channel to pass, 1–16; 0 passes all."),
            )
            .output(SocketSpec::midi("midi").doc("Messages from the devices."))
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        MidiIn {}
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let filter = io.inputs.int(CHANNEL);
        let last = ctx.frames.saturating_sub(1) as u32;
        let mut out = io.outputs.midi(MIDI);
        for ev in io.host.midi().iter().filter(|e| channel_matches(filter, e)) {
            // Keep a late event inside the block rather than drop it: a lost
            // note-off would leave a note hanging.
            out.push(MidiEvent::new(ev.offset.min(last), ev.msg));
        }
    }

    fn determinism(&self) -> Determinism {
        Determinism::Live
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use ac_core::{MidiMsg, NoResources};
    use serde_json::json;

    fn on(channel: u8, note: u8) -> MidiMsg {
        MidiMsg::NoteOn {
            channel,
            note,
            velocity: 100,
        }
    }

    #[test]
    fn passes_device_midi_with_offsets() {
        let mut h = Harness::new::<MidiIn>(json!({}));
        let evs = [
            (3, on(0, 60)),
            (70, on(5, 62)),
            (
                71,
                MidiMsg::ControlChange {
                    channel: 15,
                    controller: 1,
                    value: 64,
                },
            ),
            (200, on(0, 64)),
        ];
        h.host_midi(&evs);
        let out = h.run(256);
        assert_eq!(out.midi("midi"), &evs);
    }

    #[test]
    fn channel_filter_passes_one_channel() {
        let mut h = Harness::new::<MidiIn>(json!({}));
        h.set("channel", 6);
        h.host_midi(&[(1, on(0, 60)), (2, on(5, 61)), (3, on(6, 62))]);
        let out = h.run(64);
        assert_eq!(out.midi("midi"), &[(2, on(5, 61))]);
    }

    #[test]
    fn out_of_range_filters_are_clamped() {
        let ev = MidiEvent::new(0, on(15, 60));
        assert!(channel_matches(-3, &ev));
        assert!(channel_matches(0, &ev));
        assert!(channel_matches(16, &ev));
        assert!(channel_matches(99, &ev));
        assert!(!channel_matches(1, &ev));
    }

    #[test]
    fn is_a_live_source() {
        assert!(MidiIn::meta().live);
        let ctx = PrepareCtx {
            sample_rate: 48_000.0,
            max_block: 64,
            seed: 0,
            node_key: "t",
            name: "MIDI In",
            resources: &NoResources,
        };
        assert_eq!(
            MidiIn::new(&json!({}), &ctx).determinism(),
            Determinism::Live
        );
    }
}
