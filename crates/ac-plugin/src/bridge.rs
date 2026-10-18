//! The audio side of a plugin instance: the host's buffers, notes,
//! transport, and macro values in, the engine's [`HostIo`] out.
//!
//! The host hands the plugin per-channel buffers it processes in place
//! (an effect's input is already in them); the engine takes channel-major
//! input and output slices. The bridge copies between the two through
//! scratch space sized at activation, so `process` never allocates.
//! Host notes become MIDI events at their sample offsets; macros that
//! moved become control events at offset 0 of the (framework-split)
//! sub-block, which the engine's bindings apply on that sample.
//!
//! A mono host bus still meets the engine as a pair, so stereo patches
//! (the default ones included) work in every layout: a mono input feeds
//! both of the engine's first two inputs, and a mono output gets the
//! average of its first two outputs.

use ac_core::{
    CONTROL_CAPACITY, Control, ControlEvent, MACRO_COUNT, MIDI_CAPACITY, MidiEvent, MidiMsg,
};
use ac_engine::{HostIo, Processor};
use nice_plug::prelude::NoteEvent;

/// How far, in samples, a playing host's position may differ from the
/// engine's before it counts as a jump (a loop, a click on the timeline)
/// rather than rounding in the host's position.
pub const TRANSPORT_TOLERANCE: u64 = 64;

/// What the host says about its transport this block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HostTransport {
    pub playing: bool,
    /// Samples at the engine rate since the host's timeline start.
    pub position: Option<u64>,
}

/// Scratch space and bookkeeping between the host and the processor.
#[derive(Debug)]
pub struct Bridge {
    /// The host's channels.
    host_in: usize,
    host_out: usize,
    /// The engine's: the host's, with mono widened to a pair.
    in_channels: usize,
    out_channels: usize,
    /// Most frames one engine call takes; longer host blocks are split.
    capacity: usize,
    input: Vec<f32>,
    output: Vec<f32>,
    midi: Vec<MidiEvent>,
    chunk_midi: Vec<MidiEvent>,
    controls: Vec<ControlEvent>,
    midi_out: Vec<MidiEvent>,
    /// Send MIDI Out nodes' messages to the host.
    midi_out_enabled: bool,
    /// The macro values the running graph last got (NaN: send again).
    sent: [f32; MACRO_COUNT],
    generation: Option<u64>,
    dropped: u64,
}

impl Bridge {
    /// Scratch for `in_channels` → `out_channels` at up to `max_frames`
    /// frames per call (the host's maximum block).
    pub fn new(
        in_channels: usize,
        out_channels: usize,
        max_frames: usize,
        midi_out: bool,
    ) -> Bridge {
        let capacity = max_frames.max(1);
        let widen = |n: usize| if n == 1 { 2 } else { n };
        let (host_in, host_out) = (in_channels, out_channels);
        let (in_channels, out_channels) = (widen(host_in), widen(host_out));
        Bridge {
            host_in,
            host_out,
            in_channels,
            out_channels,
            capacity,
            input: vec![0.0; in_channels * capacity],
            output: vec![0.0; out_channels * capacity],
            midi: Vec::with_capacity(MIDI_CAPACITY),
            chunk_midi: Vec::with_capacity(MIDI_CAPACITY),
            controls: Vec::with_capacity(CONTROL_CAPACITY),
            midi_out: Vec::with_capacity(MIDI_CAPACITY),
            midi_out_enabled: midi_out,
            sent: [f32::NAN; MACRO_COUNT],
            generation: None,
            dropped: 0,
        }
    }

    /// Queue a host note or MIDI message for the next [`Bridge::process`],
    /// `offset` frames into its block. Messages past the engine's MIDI
    /// capacity are dropped (and counted).
    pub fn push_midi(&mut self, offset: u32, msg: MidiMsg) {
        if self.midi.len() < self.midi.capacity() {
            self.midi.push(MidiEvent::new(offset, msg));
        } else {
            self.dropped += 1;
        }
    }

    /// Host messages dropped for want of room since the bridge was made.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Send every macro's value again with the next block (after a reset).
    pub fn resend_macros(&mut self) {
        self.sent = [f32::NAN; MACRO_COUNT];
    }

    /// Run one host block. `channels` are the host's buffers, all the same
    /// length, processed in place: the first `in_channels` hold the input
    /// and every one gets output. `macros` are the macro parameters'
    /// current values; `transport` is `None` when the host gives none, and
    /// the engine then runs as if playing. `send` gets what MIDI Out nodes
    /// produced, offsets relative to the block. Real-time safe.
    pub fn process(
        &mut self,
        processor: &mut Processor,
        channels: &mut [&mut [f32]],
        macros: &[f32],
        transport: Option<HostTransport>,
        mut send: impl FnMut(MidiEvent),
    ) {
        let frames = channels.iter().map(|c| c.len()).min().unwrap_or(0);
        // A new graph (an edit, a state load) starts with every macro's
        // value, in the block it starts.
        let generation = processor.install_pending();
        if self.generation != Some(generation) {
            self.generation = Some(generation);
            self.resend_macros();
        }
        self.controls.clear();
        for (i, (&v, sent)) in macros.iter().zip(self.sent.iter_mut()).enumerate() {
            if v.to_bits() != sent.to_bits() && self.controls.len() < self.controls.capacity() {
                self.controls
                    .push(ControlEvent::new(0, Control::Macro { index: i as u8 }, v));
                *sent = v;
            }
        }
        match transport {
            Some(t) => processor.follow_transport(t.playing, t.position, TRANSPORT_TOLERANCE),
            None => processor.follow_transport(true, None, 0),
        }
        let mut start = 0;
        while start < frames {
            let n = (frames - start).min(self.capacity);
            self.run_chunk(processor, channels, start, n, frames, &mut send);
            start += n;
        }
        if frames == 0 {
            // Still let commands and graphs through, and let the macros land.
            self.run_chunk(processor, channels, 0, 0, 0, &mut send);
        }
        self.midi.clear();
    }

    fn run_chunk(
        &mut self,
        processor: &mut Processor,
        channels: &mut [&mut [f32]],
        start: usize,
        n: usize,
        frames: usize,
        send: &mut impl FnMut(MidiEvent),
    ) {
        let (ic, oc) = (self.in_channels, self.out_channels);
        for c in 0..ic {
            let Some(dst) = self.input.get_mut(c * n..(c + 1) * n) else {
                continue;
            };
            let from = if self.host_in == 1 { 0 } else { c };
            match channels
                .get(from)
                .filter(|_| from < self.host_in)
                .and_then(|ch| ch.get(start..start + n))
            {
                Some(src) => dst.copy_from_slice(src),
                None => dst.fill(0.0),
            }
        }
        // This chunk's MIDI, rebased; events past the block (a host's
        // rounding) land on its last frame.
        self.chunk_midi.clear();
        let last = frames.saturating_sub(1);
        for e in &self.midi {
            let off = (e.offset as usize).min(last);
            if off >= start && off < start + n.max(1) {
                self.chunk_midi
                    .push(MidiEvent::new((off - start) as u32, e.msg));
            }
        }
        self.midi_out.clear();
        let controls: &[ControlEvent] = if start == 0 { &self.controls } else { &[] };
        let mut io = HostIo {
            frames: n,
            input: self.input.get(..ic * n).unwrap_or(&[]),
            in_channels: ic,
            output: self.output.get_mut(..oc * n).unwrap_or(&mut []),
            out_channels: oc,
            midi: &self.chunk_midi,
            midi_out: self.midi_out_enabled.then_some(&mut self.midi_out),
            controls,
            osc_out: None,
        };
        processor.process(&mut io);
        for (c, ch) in channels.iter_mut().enumerate() {
            let Some(dst) = ch.get_mut(start..start + n) else {
                continue;
            };
            if self.host_out == 1 && c == 0 {
                let l = self.output.get(..n).unwrap_or(&[]);
                let r = self.output.get(n..2 * n).unwrap_or(&[]);
                for ((d, a), b) in dst.iter_mut().zip(l).zip(r) {
                    *d = 0.5 * (a + b);
                }
                continue;
            }
            match self
                .output
                .get(c * n..(c + 1) * n)
                .filter(|_| c < oc && c < self.host_out)
            {
                Some(src) => dst.copy_from_slice(src),
                None => dst.fill(0.0),
            }
        }
        for e in &self.midi_out {
            send(MidiEvent::new(e.offset + start as u32, e.msg));
        }
    }
}

/// A host note event as a MIDI message the engine reads, with its offset.
/// Note expressions and voice bookkeeping have no MIDI form and are
/// skipped; a choke ends its note.
pub fn note_to_midi(event: &NoteEvent<()>) -> Option<(u32, MidiMsg)> {
    if let NoteEvent::Choke {
        timing,
        channel,
        key,
        ..
    } = *event
    {
        return Some((
            timing,
            MidiMsg::NoteOff {
                channel: channel.number().unwrap_or(0) & 0x0f,
                note: key.number()?,
                velocity: 0,
            },
        ));
    }
    let timing = event.timing();
    match event.as_midi()? {
        nice_plug::midi::MidiResult::Basic(bytes) => {
            MidiMsg::from_bytes(&bytes).map(|m| (timing, m))
        }
        nice_plug::midi::MidiResult::SysEx(..) => None,
    }
}

/// A MIDI message from the engine as a host note event.
pub fn midi_to_note(event: MidiEvent) -> Option<NoteEvent<()>> {
    let (bytes, len) = event.msg.to_bytes();
    NoteEvent::from_midi(event.offset, bytes.get(..len)?).ok()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use ac_core::*;
    use ac_engine::{Engine, EngineOptions};
    use nice_plug::midi::{Channel, Key as NoteKey, VoiceID};

    /// Audio In (stereo) → Gain (node 2) → Audio Out, Macro 1 on the gain
    /// over −60..0 dB; MIDI In → a MIDI Out so notes come back.
    fn patch() -> Project {
        let mut p = Project::default();
        let n = |id, kind: &str| NodeInstance::new(NodeId(id), kind);
        p.graph.nodes = vec![
            n(1, "audio_in").with_config(serde_json::json!({ "channels": 2 })),
            n(2, "gain"),
            n(3, "audio_out"),
            n(4, "midi_in"),
            n(5, "midi_out"),
        ];
        let w = |a, o: &str, b, i: &str| {
            Wire::new(Endpoint::new(NodeId(a), o), Endpoint::new(NodeId(b), i))
        };
        p.graph.wires = vec![
            w(1, "out", 2, "in"),
            w(2, "out", 3, "in"),
            w(4, "midi", 5, "midi"),
        ];
        p.bindings.push(Binding {
            min: Some(-60.0),
            max: Some(0.0),
            ..Binding::input(
                Source::Macro { number: 1 },
                NodePath::root(NodeId(2)),
                "gain",
            )
        });
        p
    }

    fn processor(max: usize) -> (ac_engine::EngineHandle, Processor) {
        let (e, report) = Engine::new(
            patch(),
            Arc::new(ac_nodes::registry()),
            Arc::new(NoResources),
            EngineOptions {
                sample_rate: 48_000.0,
                max_block: max,
                socket_telemetry: false,
                offline: false,
            },
        );
        assert!(
            report
                .diagnostics
                .iter()
                .all(|d| d.severity != ac_engine::Severity::Error),
            "{report}"
        );
        e.split()
    }

    fn run(
        bridge: &mut Bridge,
        p: &mut Processor,
        l: &mut [f32],
        r: &mut [f32],
        macros: &[f32],
    ) -> Vec<MidiEvent> {
        // Reserved up front: the whole call runs under the allocation guard.
        let mut out = Vec::with_capacity(64);
        let mut ch: [&mut [f32]; 2] = [l, r];
        assert_no_alloc::assert_no_alloc(|| {
            bridge.process(p, &mut ch, macros, None, |e| {
                if out.len() < out.capacity() {
                    out.push(e);
                }
            })
        });
        out
    }

    #[test]
    fn stereo_passes_through_in_place_at_any_block_size() {
        let (_h, mut p) = processor(256);
        let mut b = Bridge::new(2, 2, 256, true);
        let macros = [1.0; MACRO_COUNT]; // Macro 1 at the top: 0 dB.
        for frames in [256, 1, 7, 100, 256, 3] {
            let mut l: Vec<f32> = (0..frames).map(|i| i as f32 / 512.0).collect();
            let mut r: Vec<f32> = l.iter().map(|v| -v).collect();
            let (l0, r0) = (l.clone(), r.clone());
            run(&mut b, &mut p, &mut l, &mut r, &macros);
            assert_eq!(l, l0, "{frames} frames, left");
            assert_eq!(r, r0, "{frames} frames, right");
        }
    }

    #[test]
    fn blocks_longer_than_the_maximum_are_split() {
        let (_h, mut p) = processor(64);
        let mut b = Bridge::new(2, 2, 64, true);
        let macros = [1.0; MACRO_COUNT];
        let mut l: Vec<f32> = (0..200).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut r = l.clone();
        let l0 = l.clone();
        b.push_midi(
            150,
            MidiMsg::NoteOn {
                channel: 0,
                note: 60,
                velocity: 100,
            },
        );
        let sent = run(&mut b, &mut p, &mut l, &mut r, &macros);
        assert_eq!(l, l0);
        assert_eq!(
            sent,
            vec![MidiEvent::new(
                150,
                MidiMsg::NoteOn {
                    channel: 0,
                    note: 60,
                    velocity: 100
                }
            )]
        );
    }

    #[test]
    fn notes_keep_their_offsets_and_late_ones_land_on_the_last_frame() {
        let (_h, mut p) = processor(128);
        let mut b = Bridge::new(2, 2, 128, true);
        let macros = [1.0; MACRO_COUNT];
        let on = MidiMsg::NoteOn {
            channel: 2,
            note: 64,
            velocity: 90,
        };
        let off = MidiMsg::NoteOff {
            channel: 2,
            note: 64,
            velocity: 0,
        };
        b.push_midi(0, on);
        b.push_midi(17, off);
        b.push_midi(500, on);
        let (mut l, mut r) = (vec![0.0; 32], vec![0.0; 32]);
        let sent = run(&mut b, &mut p, &mut l, &mut r, &macros);
        assert_eq!(
            sent,
            vec![
                MidiEvent::new(0, on),
                MidiEvent::new(17, off),
                MidiEvent::new(31, on)
            ]
        );
        // The queue is per block.
        let sent = run(&mut b, &mut p, &mut l, &mut r, &macros);
        assert!(sent.is_empty());
        // MIDI Out disabled: nothing is sent.
        let mut quiet = Bridge::new(2, 2, 128, false);
        quiet.push_midi(0, on);
        assert!(run(&mut quiet, &mut p, &mut l, &mut r, &macros).is_empty());
    }

    #[test]
    fn macros_move_bound_inputs_and_reach_a_new_graph() {
        let (mut h, mut p) = processor(256);
        let mut b = Bridge::new(2, 2, 256, false);
        let level = |b: &mut Bridge, p: &mut Processor, m: f32| {
            let mut macros = [0.5; MACRO_COUNT];
            macros[0] = m;
            let mut out = 0.0;
            // Long enough for the engine's smoothing to settle.
            for _ in 0..8 {
                let (mut l, mut r) = (vec![1.0; 256], vec![1.0; 256]);
                run(b, p, &mut l, &mut r, &macros);
                out = l[255];
            }
            20.0 * out.log10()
        };
        assert!((level(&mut b, &mut p, 1.0) - 0.0).abs() < 1e-3);
        assert!((level(&mut b, &mut p, 0.5) + 30.0).abs() < 1e-3);
        // A recompile with the gain stored at 0 dB: the new graph gets the
        // macro's value in its first block, not the stored one.
        let mut project = h.project().clone();
        project.graph.nodes[1]
            .params
            .insert("gain".into(), Value::Float(0.0));
        h.edit(ac_engine::GraphEdit::Replace(Box::new(project)));
        let mut macros = [0.5; MACRO_COUNT];
        macros[0] = 0.5;
        let (mut l, mut r) = (vec![1.0; 64], vec![1.0; 64]);
        run(&mut b, &mut p, &mut l, &mut r, &macros);
        assert!((20.0 * l[0].log10() + 30.0).abs() < 0.5, "{}", l[0]);
    }

    #[test]
    fn mono_buses_meet_the_engine_as_a_pair() {
        let (_h, mut p) = processor(64);
        let macros = [1.0; MACRO_COUNT];
        // Mono in and out: the stereo patch's average is the input.
        let mut mono = Bridge::new(1, 1, 64, false);
        let x: Vec<f32> = (0..64).map(|i| (i as f32 * 0.1).sin()).collect();
        let mut buf = x.clone();
        {
            let mut ch: [&mut [f32]; 1] = [&mut buf];
            mono.process(&mut p, &mut ch, &macros, None, |_| {});
        }
        assert_eq!(buf, x);
        // Mono in, stereo out: both sides get it.
        let mut wide = Bridge::new(1, 2, 64, false);
        let (mut l, mut r) = (x.clone(), vec![9.0; 64]);
        {
            let mut ch: [&mut [f32]; 2] = [&mut l, &mut r];
            wide.process(&mut p, &mut ch, &macros, None, |_| {});
        }
        assert_eq!(l, x);
        assert_eq!(r, x);
    }

    #[test]
    fn the_instrument_layout_has_no_input() {
        let (_h, mut p) = processor(64);
        let mut b = Bridge::new(0, 2, 64, false);
        let (mut l, mut r) = (vec![0.7; 64], vec![0.7; 64]);
        run(&mut b, &mut p, &mut l, &mut r, &[1.0; MACRO_COUNT]);
        assert!(
            l.iter().chain(&r).all(|v| *v == 0.0),
            "silent: Audio In reads nothing"
        );
    }

    #[test]
    fn host_notes_convert_to_midi_and_back() {
        let on = NoteEvent::NoteOn {
            timing: 5,
            voice_id: VoiceID::Wildcard,
            channel: Channel::Number(3),
            key: NoteKey::Number(61),
            velocity: 0.5,
        };
        assert_eq!(
            note_to_midi(&on),
            Some((
                5,
                MidiMsg::NoteOn {
                    channel: 3,
                    note: 61,
                    velocity: 64
                }
            ))
        );
        let choke = NoteEvent::Choke {
            timing: 9,
            voice_id: VoiceID::Wildcard,
            channel: Channel::Wildcard,
            key: NoteKey::Number(40),
        };
        assert_eq!(
            note_to_midi(&choke),
            Some((
                9,
                MidiMsg::NoteOff {
                    channel: 0,
                    note: 40,
                    velocity: 0
                }
            ))
        );
        let cc = NoteEvent::MidiCC {
            timing: 2,
            channel: 1,
            cc: 74,
            value: 1.0,
        };
        assert_eq!(
            note_to_midi(&cc),
            Some((
                2,
                MidiMsg::ControlChange {
                    channel: 1,
                    controller: 74,
                    value: 127
                }
            ))
        );
        let back = midi_to_note(MidiEvent::new(
            4,
            MidiMsg::NoteOff {
                channel: 0,
                note: 61,
                velocity: 0,
            },
        ));
        assert!(
            matches!(back, Some(NoteEvent::NoteOff { timing: 4, .. })),
            "{back:?}"
        );
    }
}
