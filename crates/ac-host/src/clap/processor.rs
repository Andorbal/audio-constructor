//! The audio half of a CLAP instance: what the `Plugin` node calls every
//! block, on whichever audio thread runs it (ARCHITECTURE §8).
//!
//! Everything it needs is allocated when it's built: the plugin's audio
//! buffers (shaped like the plugin's own ports, which a node's stored
//! description may no longer match), the event lists, and the ring that
//! carries parameter moves made in the plugin's window back to the main
//! thread. A block copies the node's buffers in, builds events, calls the
//! plugin, and copies back, without allocating, locking, or panicking.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::ThreadId;

use ac_core::{MidiEvent, MidiMsg, PluginBlock, PluginEvent, PluginProcessor};
use clack_host::events::event_types::{
    MidiEvent as ClapMidiEvent, NoteOffEvent, NoteOnEvent, ParamValueEvent, TransportEvent,
    TransportFlags,
};
use clack_host::events::io::{OutputEventBuffer, TryPushError};
use clack_host::events::spaces::CoreEventSpace;
use clack_host::events::{EventFlags, EventHeader, Match, Pckn, UnknownEvent};
use clack_host::prelude::*;
use clack_host::utils::{BeatTime, SecondsTime};

use super::host::{AcClap, Requests};

/// Most events one block sends a plugin (the node sends at most
/// `PLUGIN_EVENT_CAPACITY` too).
const EVENTS_IN: usize = ac_core::plugin::PLUGIN_EVENT_CAPACITY;
/// Parameter moves one block can carry back from the plugin.
const PARAMS_OUT: usize = 512;
/// Parameter moves waiting for the main thread.
pub(crate) const PARAM_RING: usize = 4096;

/// How the plugin wants notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Notes {
    /// No note input: notes are dropped.
    None,
    /// CLAP note events, with other MIDI as raw MIDI events when the port
    /// takes them.
    Clap { midi: bool },
    /// Everything as raw MIDI events.
    Midi,
}

/// The audio half's state machine: started on its first block, on the
/// audio thread, as CLAP requires.
enum Audio {
    Stopped(StoppedPluginAudioProcessor<AcClap>),
    Started(StartedPluginAudioProcessor<AcClap>),
    /// Starting failed; the node plays silence.
    Failed,
}

pub(crate) struct ClapProcessor {
    audio: Audio,
    owner: ThreadId,
    pub(crate) instance_id: u64,
    requests: Arc<Requests>,
    /// Buffers shaped like the plugin's ports: per port, per channel.
    ins: Vec<Vec<Vec<f32>>>,
    outs: Vec<Vec<Vec<f32>>>,
    in_ports: AudioPorts,
    out_ports: AudioPorts,
    events: EventBuffer,
    out_events: OutEvents,
    /// The plugin's id for each parameter of the node's description.
    param_ids: Vec<Option<ClapId>>,
    /// (plugin id, description index), sorted, for parameters the plugin
    /// moves itself.
    by_id: Vec<(u32, u32)>,
    notes: Notes,
    latency: u32,
    steady: u64,
    to_main: rtrb::Producer<(u32, f64)>,
}

/// Events the plugin sends back, parsed on arrival into fixed room.
pub(crate) struct OutEvents {
    midi: Vec<MidiEvent>,
    params: Vec<(u32, f64)>,
}

impl OutEvents {
    fn new() -> OutEvents {
        OutEvents {
            midi: Vec::with_capacity(ac_core::MIDI_CAPACITY),
            params: Vec::with_capacity(PARAMS_OUT),
        }
    }

    fn clear(&mut self) {
        self.midi.clear();
        self.params.clear();
    }

    fn midi(&mut self, offset: u32, msg: Option<MidiMsg>) {
        if let Some(msg) = msg
            && self.midi.len() < self.midi.capacity()
        {
            self.midi.push(MidiEvent::new(offset, msg));
        }
    }
}

fn clamp7(v: f64) -> u8 {
    (v * 127.0).round().clamp(0.0, 127.0) as u8
}

impl OutputEventBuffer for OutEvents {
    fn try_push(&mut self, event: &UnknownEvent) -> Result<(), TryPushError> {
        let time = event.header().time();
        match event.as_core_event() {
            Some(CoreEventSpace::NoteOn(e)) => {
                let raw = e.as_raw();
                let (Ok(channel), Ok(note)) = (u8::try_from(raw.channel), u8::try_from(raw.key))
                else {
                    return Ok(());
                };
                self.midi(
                    time,
                    Some(MidiMsg::NoteOn {
                        channel: channel & 0x0f,
                        note: note & 0x7f,
                        velocity: clamp7(raw.velocity).max(1),
                    }),
                );
            }
            Some(CoreEventSpace::NoteOff(e)) => {
                let raw = e.as_raw();
                let (Ok(channel), Ok(note)) = (u8::try_from(raw.channel), u8::try_from(raw.key))
                else {
                    return Ok(());
                };
                self.midi(
                    time,
                    Some(MidiMsg::NoteOff {
                        channel: channel & 0x0f,
                        note: note & 0x7f,
                        velocity: clamp7(raw.velocity),
                    }),
                );
            }
            Some(CoreEventSpace::Midi(e)) => self.midi(time, MidiMsg::from_bytes(&e.data())),
            Some(CoreEventSpace::ParamValue(e)) => {
                if let Some(id) = e.param_id()
                    && self.params.len() < self.params.capacity()
                {
                    self.params.push((id.get(), e.value()));
                }
            }
            _ => {}
        }
        Ok(())
    }
}

pub(crate) struct Layout {
    /// Channels per audio input and output port of the plugin.
    pub ins: Vec<u32>,
    pub outs: Vec<u32>,
    pub notes: Notes,
}

impl ClapProcessor {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        audio: StoppedPluginAudioProcessor<AcClap>,
        instance_id: u64,
        requests: Arc<Requests>,
        layout: &Layout,
        param_ids: Vec<Option<ClapId>>,
        latency: u32,
        max_block: usize,
        to_main: rtrb::Producer<(u32, f64)>,
    ) -> ClapProcessor {
        let block = max_block.max(1);
        let buffers = |ports: &[u32]| -> Vec<Vec<Vec<f32>>> {
            ports
                .iter()
                .map(|&c| (0..c).map(|_| vec![0.0; block]).collect())
                .collect()
        };
        let channels = |ports: &[u32]| ports.iter().map(|&c| c as usize).sum::<usize>();
        let mut by_id: Vec<(u32, u32)> = param_ids
            .iter()
            .enumerate()
            .filter_map(|(i, id)| id.map(|id| (id.get(), i as u32)))
            .collect();
        by_id.sort_unstable();
        ClapProcessor {
            audio: Audio::Stopped(audio),
            owner: std::thread::current().id(),
            instance_id,
            requests,
            ins: buffers(&layout.ins),
            outs: buffers(&layout.outs),
            in_ports: AudioPorts::with_capacity(channels(&layout.ins), layout.ins.len()),
            out_ports: AudioPorts::with_capacity(channels(&layout.outs), layout.outs.len()),
            events: EventBuffer::with_capacity(EVENTS_IN),
            out_events: OutEvents::new(),
            param_ids,
            by_id,
            notes: layout.notes,
            latency,
            steady: 0,
            to_main,
        }
    }

    fn push_events(&mut self, events: &[PluginEvent]) {
        self.events.clear();
        let mut n = 0usize;
        for e in events {
            if n >= EVENTS_IN {
                break;
            }
            match *e {
                PluginEvent::Param {
                    offset,
                    index,
                    value,
                } => {
                    if let Some(Some(id)) = self.param_ids.get(index as usize) {
                        self.events.push(&ParamValueEvent::new(
                            offset,
                            *id,
                            Pckn::match_all(),
                            value,
                        ));
                        n += 1;
                    }
                }
                PluginEvent::Midi(m) => {
                    if push_midi(&mut self.events, self.notes, m) {
                        n += 1;
                    }
                }
            }
        }
    }
}

/// One MIDI message as the plugin wants it; false if it was dropped.
fn push_midi(events: &mut EventBuffer, notes: Notes, m: MidiEvent) -> bool {
    let t = m.offset;
    let raw = |events: &mut EventBuffer| {
        let (bytes, _) = m.msg.to_bytes();
        events.push(&ClapMidiEvent::new(t, 0, bytes));
        true
    };
    match notes {
        Notes::None => false,
        Notes::Midi => raw(events),
        Notes::Clap { midi } => match m.msg {
            MidiMsg::NoteOn {
                channel,
                note,
                velocity,
            } => {
                events.push(&NoteOnEvent::new(
                    t,
                    Pckn::new(0u16, u16::from(channel), u16::from(note), Match::All),
                    f64::from(velocity) / 127.0,
                ));
                true
            }
            MidiMsg::NoteOff {
                channel,
                note,
                velocity,
            } => {
                events.push(&NoteOffEvent::new(
                    t,
                    Pckn::new(0u16, u16::from(channel), u16::from(note), Match::All),
                    f64::from(velocity) / 127.0,
                ));
                true
            }
            _ if midi => raw(events),
            _ => false,
        },
    }
}

fn transport_event(t: &ac_core::PluginTransport) -> TransportEvent {
    let mut flags = TransportFlags::HAS_TEMPO
        | TransportFlags::HAS_BEATS_TIMELINE
        | TransportFlags::HAS_SECONDS_TIMELINE
        | TransportFlags::HAS_TIME_SIGNATURE;
    if t.playing {
        flags |= TransportFlags::IS_PLAYING;
    }
    if t.recording {
        flags |= TransportFlags::IS_RECORDING;
    }
    TransportEvent {
        header: EventHeader::new_core(0, EventFlags::empty()),
        flags,
        song_pos_beats: BeatTime::from_float(t.beats),
        song_pos_seconds: SecondsTime::from_float(t.seconds),
        tempo: t.bpm,
        tempo_inc: 0.0,
        loop_start_beats: BeatTime::from_int(0),
        loop_end_beats: BeatTime::from_int(0),
        loop_start_seconds: SecondsTime::from_int(0),
        loop_end_seconds: SecondsTime::from_int(0),
        bar_start: BeatTime::from_float(t.bar_start),
        bar_number: t.bar.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        time_signature_numerator: u16::from(t.time_sig[0]),
        time_signature_denominator: u16::from(t.time_sig[1]),
    }
}

/// Copy the overlap of `src` into `dst` (ports, then channels), zeroing
/// what `src` doesn't cover.
fn copy_ports(dst: &mut [Vec<Vec<f32>>], src: &[Vec<Vec<f32>>], frames: usize) {
    for (p, port) in dst.iter_mut().enumerate() {
        for (c, ch) in port.iter_mut().enumerate() {
            let out = &mut ch[..frames];
            match src.get(p).and_then(|s| s.get(c)) {
                Some(s) => out.copy_from_slice(&s[..frames]),
                None => out.fill(0.0),
            }
        }
    }
}

impl PluginProcessor for ClapProcessor {
    fn process(&mut self, block: &mut PluginBlock<'_>) -> bool {
        let frames = block.frames;
        if frames == 0 {
            return true;
        }
        if let Audio::Stopped(_) = self.audio {
            let Audio::Stopped(stopped) = std::mem::replace(&mut self.audio, Audio::Failed) else {
                return false;
            };
            self.audio = match stopped.start_processing() {
                Ok(started) => Audio::Started(started),
                Err(_) => Audio::Failed,
            };
        }
        copy_ports(&mut self.ins, block.audio_in, frames);
        self.push_events(block.events);
        let transport = transport_event(&block.transport);
        self.out_events.clear();
        let Audio::Started(audio) = &mut self.audio else {
            return false;
        };
        let inputs = self
            .in_ports
            .with_input_buffers(self.ins.iter_mut().map(|port| {
                AudioPortBuffer {
                    latency: 0,
                    channels: AudioPortBufferType::f32_input_only(
                        port.iter_mut()
                            .map(|c| InputChannel::variable(&mut c[..frames])),
                    ),
                }
            }));
        let mut outputs = self
            .out_ports
            .with_output_buffers(self.outs.iter_mut().map(|port| AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_output_only(
                    port.iter_mut().map(|c| &mut c[..frames]),
                ),
            }));
        let events = InputEvents::from_buffer(&self.events);
        let mut out_events = OutputEvents::from_buffer(&mut self.out_events);
        // The plugin's own code is not ours to police: it has its own
        // allocator, and may allocate in its process call. The guard stays
        // on for everything around it (DECISIONS D133).
        let result = assert_no_alloc::permit_alloc(|| {
            audio.process(
                &inputs,
                &mut outputs,
                &events,
                &mut out_events,
                Some(self.steady),
                Some(&transport),
            )
        });
        self.steady = self.steady.wrapping_add(frames as u64);
        let ok = result.is_ok();
        if ok {
            copy_ports(block.audio_out, &self.outs, frames);
        }
        for ev in &self.out_events.midi {
            block.midi_out.push(*ev);
        }
        for &(id, value) in &self.out_events.params {
            if let Ok(i) = self.by_id.binary_search_by_key(&id, |e| e.0) {
                let _ = self.to_main.push((self.by_id[i].1, value));
            }
        }
        ok
    }

    fn latency(&self) -> u32 {
        self.latency
    }

    fn reset(&mut self) {
        if let Audio::Started(a) = &mut self.audio {
            a.reset();
        }
    }
}

impl Drop for ClapProcessor {
    fn drop(&mut self) {
        // Stop processing here, on the thread retiring the graph, then let
        // the main half go: at once when that's this thread (the usual
        // case: garbage is collected on the main thread), else at the
        // owner's next poll.
        match std::mem::replace(&mut self.audio, Audio::Failed) {
            Audio::Started(a) => drop(a.stop_processing()),
            other => drop(other),
        }
        if std::thread::current().id() == self.owner {
            crate::registry::release(self.instance_id);
        } else {
            self.requests.orphaned.store(true, Ordering::Release);
        }
    }
}
