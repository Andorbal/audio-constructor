//! The audio half of a VST3 instance, and the event and parameter lists
//! `process` hands the plugin, all allocated when the instance is made
//! (ARCHITECTURE §8).
//!
//! The lists are COM objects the plugin reads (and, for the output lists,
//! writes) only during `process`, on the thread calling it, so their
//! contents live in `UnsafeCell`s; see each `SAFETY` note.

#![allow(unsafe_code)]

use std::cell::{Cell, UnsafeCell};
use std::mem::ManuallyDrop;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::ThreadId;

use ac_core::{MidiEvent, MidiMsg, PluginBlock, PluginEvent, PluginProcessor};
use vst3::Steinberg::Vst::{
    AudioBusBuffers, AudioBusBuffers__type0, Event, Event_, IAudioProcessor, IAudioProcessorTrait,
    IEventList, IEventListTrait, IParamValueQueue, IParamValueQueueTrait, IParameterChanges,
    IParameterChangesTrait, NoteOffEvent, NoteOnEvent, PolyPressureEvent, ProcessContext,
    ProcessContext_::StatesAndFlags_, ProcessData, ProcessModes_, SymbolicSampleSizes_,
};
use vst3::Steinberg::{kInvalidArgument, kResultFalse, kResultOk, tresult};
use vst3::{Class, ComPtr, ComWrapper};

use crate::clap::host::Requests;

/// Points one parameter can get in a block (a moving input sends one
/// every 32 samples, so 1024-frame blocks need 32).
const POINTS: usize = 128;
/// Parameters that can change in one block.
const QUEUES: usize = 320;
/// Events in or out per block.
const EVENTS: usize = 1024;

/// The changes of one parameter in a block.
pub(crate) struct ParamQueue {
    id: Cell<u32>,
    points: UnsafeCell<Vec<(i32, f64)>>,
}

// SAFETY: a queue is only touched by the thread running the processor
// (filling it before `process`, the plugin reading or writing it during,
// reading it after), never by two threads at once.
unsafe impl Sync for ParamQueue {}
// SAFETY: as above; moving it between audio threads between blocks is
// what the engine does with every node.
unsafe impl Send for ParamQueue {}

impl Class for ParamQueue {
    type Interfaces = (IParamValueQueue,);
}

impl ParamQueue {
    fn new() -> ParamQueue {
        ParamQueue {
            id: Cell::new(0),
            points: UnsafeCell::new(Vec::with_capacity(POINTS)),
        }
    }

    /// Run `f` on the points. The borrow ends with `f`, and nothing in
    /// `f` re-enters the queue.
    fn with<R>(&self, f: impl FnOnce(&mut Vec<(i32, f64)>) -> R) -> R {
        // SAFETY: see the type's Sync note: one thread, and the only
        // reference into the vector is `f`'s, which ends with the call.
        f(unsafe { &mut *self.points.get() })
    }
}

impl IParamValueQueueTrait for ParamQueue {
    unsafe fn getParameterId(&self) -> u32 {
        self.id.get()
    }

    unsafe fn getPointCount(&self) -> i32 {
        self.with(|p| p.len() as i32)
    }

    unsafe fn getPoint(&self, index: i32, offset: *mut i32, value: *mut f64) -> tresult {
        let Some((o, v)) = usize::try_from(index)
            .ok()
            .and_then(|i| self.with(|p| p.get(i).copied()))
        else {
            return kInvalidArgument;
        };
        if offset.is_null() || value.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: non-null out pointers from the plugin.
        unsafe {
            *offset = o;
            *value = v;
        }
        kResultOk
    }

    unsafe fn addPoint(&self, offset: i32, value: f64, index: *mut i32) -> tresult {
        let added = self.with(|p| {
            if p.len() >= p.capacity() {
                return None;
            }
            p.push((offset, value));
            Some(p.len() as i32 - 1)
        });
        let Some(i) = added else {
            return kResultFalse;
        };
        if !index.is_null() {
            // SAFETY: a non-null out pointer.
            unsafe { *index = i };
        }
        kResultOk
    }
}

/// Every parameter's changes in a block.
pub(crate) struct ParamChanges {
    queues: Vec<ComWrapper<ParamQueue>>,
    used: Cell<usize>,
}

// SAFETY: as for `ParamQueue`: one thread at a time.
unsafe impl Sync for ParamChanges {}
// SAFETY: as above.
unsafe impl Send for ParamChanges {}

impl Class for ParamChanges {
    type Interfaces = (IParameterChanges,);
}

impl ParamChanges {
    pub fn new() -> ParamChanges {
        ParamChanges {
            queues: (0..QUEUES)
                .map(|_| ComWrapper::new(ParamQueue::new()))
                .collect(),
            used: Cell::new(0),
        }
    }

    pub fn clear(&self) {
        for q in &self.queues[..self.used.get()] {
            q.with(Vec::clear);
        }
        self.used.set(0);
    }

    fn queue(&self, id: u32) -> Option<&ComWrapper<ParamQueue>> {
        let used = self.used.get();
        if let Some(q) = self.queues[..used].iter().find(|q| q.id.get() == id) {
            return Some(q);
        }
        let q = self.queues.get(used)?;
        q.id.set(id);
        q.with(Vec::clear);
        self.used.set(used + 1);
        Some(q)
    }

    /// Add a point (in time order per parameter); dropped when full.
    pub fn add(&self, id: u32, offset: i32, value: f64) {
        if let Some(q) = self.queue(id) {
            q.with(|p| {
                if p.len() < p.capacity() {
                    p.push((offset, value));
                }
            });
        }
    }

    /// The last value each parameter got, as `(id, value)`.
    pub fn last_values(&self) -> impl Iterator<Item = (u32, f64)> + '_ {
        self.queues[..self.used.get()]
            .iter()
            .filter_map(|q| q.with(|p| p.last().map(|p| (q.id.get(), p.1))))
    }

    fn as_raw(&self, i: usize) -> *mut IParamValueQueue {
        self.queues
            .get(i)
            .and_then(|q| q.as_com_ref::<IParamValueQueue>())
            .map_or(std::ptr::null_mut(), |r| r.as_ptr())
    }
}

impl IParameterChangesTrait for ParamChanges {
    unsafe fn getParameterCount(&self) -> i32 {
        self.used.get() as i32
    }

    unsafe fn getParameterData(&self, index: i32) -> *mut IParamValueQueue {
        match usize::try_from(index) {
            Ok(i) if i < self.used.get() => self.as_raw(i),
            _ => std::ptr::null_mut(),
        }
    }

    unsafe fn addParameterData(&self, id: *const u32, index: *mut i32) -> *mut IParamValueQueue {
        if id.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: a non-null id pointer from the plugin.
        let id = unsafe { *id };
        if self.queue(id).is_none() {
            return std::ptr::null_mut();
        }
        let i = self.queues[..self.used.get()]
            .iter()
            .position(|q| q.id.get() == id)
            .unwrap_or(0);
        if !index.is_null() {
            // SAFETY: a non-null out pointer.
            unsafe { *index = i as i32 };
        }
        self.as_raw(i)
    }
}

/// Events in or out of a block.
pub(crate) struct EventList {
    events: UnsafeCell<Vec<Event>>,
}

// SAFETY: as for `ParamQueue`: one thread at a time.
unsafe impl Sync for EventList {}
// SAFETY: as above.
unsafe impl Send for EventList {}

impl Class for EventList {
    type Interfaces = (IEventList,);
}

impl EventList {
    pub fn new() -> EventList {
        EventList {
            events: UnsafeCell::new(Vec::with_capacity(EVENTS)),
        }
    }

    /// Run `f` on the events. The borrow ends with `f`, and nothing in
    /// `f` re-enters the list.
    pub fn with<R>(&self, f: impl FnOnce(&mut Vec<Event>) -> R) -> R {
        // SAFETY: see the type's Sync note: one thread, and the only
        // reference into the vector is `f`'s, which ends with the call.
        f(unsafe { &mut *self.events.get() })
    }

    pub fn clear(&self) {
        self.with(Vec::clear);
    }

    pub fn push(&self, e: Event) {
        self.with(|l| {
            if l.len() < l.capacity() {
                l.push(e);
            }
        });
    }
}

impl IEventListTrait for EventList {
    unsafe fn getEventCount(&self) -> i32 {
        self.with(|l| l.len() as i32)
    }

    unsafe fn getEvent(&self, index: i32, e: *mut Event) -> tresult {
        let Some(ev) = usize::try_from(index)
            .ok()
            .and_then(|i| self.with(|l| l.get(i).copied()))
        else {
            return kInvalidArgument;
        };
        if e.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: a non-null out pointer to an Event.
        unsafe { *e = ev };
        kResultOk
    }

    unsafe fn addEvent(&self, e: *mut Event) -> tresult {
        if e.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: a non-null Event from the plugin.
        let ev = unsafe { *e };
        if self.with(|l| {
            let room = l.len() < l.capacity();
            if room {
                l.push(ev);
            }
            room
        }) {
            kResultOk
        } else {
            kResultFalse
        }
    }
}

/// A note the plugin sent, onto the node's `midi_out`.
fn out_note(e: &Event, last: u32, midi_out: &mut MidiSink) {
    use Event_::EventTypes_::{kNoteOffEvent, kNoteOnEvent};
    let t = (e.sampleOffset.max(0) as u32).min(last);
    // The type constants are a C enum, signed on Windows only.
    let kind = i64::from(e.r#type);
    // SAFETY: the union field read matches the event's type tag.
    let msg = unsafe {
        if kind == i64::from(kNoteOnEvent) {
            let n = e.__field0.noteOn;
            Some(MidiMsg::NoteOn {
                channel: (n.channel.clamp(0, 15)) as u8,
                note: (n.pitch.clamp(0, 127)) as u8,
                velocity: ((n.velocity * 127.0).round().clamp(1.0, 127.0)) as u8,
            })
        } else if kind == i64::from(kNoteOffEvent) {
            let n = e.__field0.noteOff;
            Some(MidiMsg::NoteOff {
                channel: (n.channel.clamp(0, 15)) as u8,
                note: (n.pitch.clamp(0, 127)) as u8,
                velocity: ((n.velocity * 127.0).round().clamp(0.0, 127.0)) as u8,
            })
        } else {
            None
        }
    };
    if let Some(msg) = msg {
        midi_out.push(MidiEvent::new(t, msg));
    }
}

fn note_event(offset: u32, kind: u16, body: vst3::Steinberg::Vst::Event__type0) -> Event {
    Event {
        busIndex: 0,
        sampleOffset: offset as i32,
        ppqPosition: 0.0,
        flags: 0,
        r#type: kind,
        __field0: body,
    }
}

/// MIDI controllers a plugin maps to parameters (`IMidiMapping`), by
/// channel and controller (128: channel pressure, 129: pitch bend).
pub(crate) type MidiMap = Box<[[Option<u32>; 130]; 16]>;

pub(crate) struct Vst3Processor {
    /// Released before the main half (see `Drop`).
    processor: ManuallyDrop<ComPtr<IAudioProcessor>>,
    started: bool,
    owner: ThreadId,
    pub(crate) instance_id: u64,
    requests: Arc<Requests>,
    ins: Vec<Vec<Vec<f32>>>,
    outs: Vec<Vec<Vec<f32>>>,
    in_ptrs: Vec<Vec<*mut f32>>,
    out_ptrs: Vec<Vec<*mut f32>>,
    in_bus: Vec<AudioBusBuffers>,
    out_bus: Vec<AudioBusBuffers>,
    in_params: ComWrapper<ParamChanges>,
    out_params: ComWrapper<ParamChanges>,
    in_events: ComWrapper<EventList>,
    out_events: ComWrapper<EventList>,
    context: ProcessContext,
    param_ids: Vec<Option<u32>>,
    midi_map: Option<MidiMap>,
    edits: rtrb::Consumer<(u32, f64)>,
    to_main: rtrb::Producer<(u32, f64)>,
    latency: u32,
    sample_rate: f64,
}

// SAFETY: the raw pointers point into this value's own buffers and are
// rewritten before each `process`; the COM pointers' objects are used by
// one thread at a time (VST3 calls the processor from one audio thread at
// a time), which is how the engine uses every node.
unsafe impl Send for Vst3Processor {}

pub(crate) struct Setup {
    pub ins: Vec<u32>,
    pub outs: Vec<u32>,
    pub param_ids: Vec<Option<u32>>,
    pub midi_map: Option<MidiMap>,
    pub latency: u32,
    pub max_block: usize,
    pub sample_rate: f64,
}

impl Vst3Processor {
    pub(crate) fn new(
        processor: ComPtr<IAudioProcessor>,
        instance_id: u64,
        requests: Arc<Requests>,
        setup: Setup,
        edits: rtrb::Consumer<(u32, f64)>,
        to_main: rtrb::Producer<(u32, f64)>,
    ) -> Vst3Processor {
        let block = setup.max_block.max(1);
        let buffers = |ports: &[u32]| -> Vec<Vec<Vec<f32>>> {
            ports
                .iter()
                .map(|&c| (0..c).map(|_| vec![0.0; block]).collect())
                .collect()
        };
        let ptrs = |ports: &[u32]| -> Vec<Vec<*mut f32>> {
            ports
                .iter()
                .map(|&c| vec![std::ptr::null_mut(); c as usize])
                .collect()
        };
        let bus = |ports: &[u32]| -> Vec<AudioBusBuffers> {
            ports
                .iter()
                .map(|&c| AudioBusBuffers {
                    numChannels: c as i32,
                    silenceFlags: 0,
                    __field0: AudioBusBuffers__type0 {
                        channelBuffers32: std::ptr::null_mut(),
                    },
                })
                .collect()
        };
        // SAFETY: ProcessContext is plain data; all zeroes is a valid,
        // empty context.
        let context: ProcessContext = unsafe { std::mem::zeroed() };
        Vst3Processor {
            processor: ManuallyDrop::new(processor),
            started: false,
            owner: std::thread::current().id(),
            instance_id,
            requests,
            ins: buffers(&setup.ins),
            outs: buffers(&setup.outs),
            in_ptrs: ptrs(&setup.ins),
            out_ptrs: ptrs(&setup.outs),
            in_bus: bus(&setup.ins),
            out_bus: bus(&setup.outs),
            in_params: ComWrapper::new(ParamChanges::new()),
            out_params: ComWrapper::new(ParamChanges::new()),
            in_events: ComWrapper::new(EventList::new()),
            out_events: ComWrapper::new(EventList::new()),
            context,
            param_ids: setup.param_ids,
            midi_map: setup.midi_map,
            edits,
            to_main,
            latency: setup.latency,
            sample_rate: setup.sample_rate,
        }
    }

    fn fill_events(&mut self, events: &[PluginEvent]) {
        self.in_params.clear();
        self.in_events.clear();
        // Edits from the plugin's window first, at the block's start.
        while let Ok((id, v)) = self.edits.pop() {
            self.in_params.add(id, 0, v);
        }
        for e in events {
            match *e {
                PluginEvent::Param {
                    offset,
                    index,
                    value,
                } => {
                    if let Some(Some(id)) = self.param_ids.get(index as usize) {
                        self.in_params
                            .add(*id, offset as i32, value.clamp(0.0, 1.0));
                    }
                }
                PluginEvent::Midi(m) => self.midi(m),
            }
        }
    }

    fn midi(&mut self, m: MidiEvent) {
        use vst3::Steinberg::Vst::Event__type0 as Body;
        let t = m.offset;
        let controller = |channel: u8, number: usize| -> Option<u32> {
            self.midi_map
                .as_ref()
                .and_then(|map| map.get(channel as usize)?.get(number).copied().flatten())
        };
        match m.msg {
            MidiMsg::NoteOn {
                channel,
                note,
                velocity,
            } => self.in_events.push(note_event(
                t,
                Event_::EventTypes_::kNoteOnEvent as u16,
                Body {
                    noteOn: NoteOnEvent {
                        channel: i16::from(channel),
                        pitch: i16::from(note),
                        tuning: 0.0,
                        velocity: f32::from(velocity) / 127.0,
                        length: 0,
                        noteId: -1,
                    },
                },
            )),
            MidiMsg::NoteOff {
                channel,
                note,
                velocity,
            } => self.in_events.push(note_event(
                t,
                Event_::EventTypes_::kNoteOffEvent as u16,
                Body {
                    noteOff: NoteOffEvent {
                        channel: i16::from(channel),
                        pitch: i16::from(note),
                        velocity: f32::from(velocity) / 127.0,
                        noteId: -1,
                        tuning: 0.0,
                    },
                },
            )),
            MidiMsg::PolyPressure {
                channel,
                note,
                value,
            } => self.in_events.push(note_event(
                t,
                Event_::EventTypes_::kPolyPressureEvent as u16,
                Body {
                    polyPressure: PolyPressureEvent {
                        channel: i16::from(channel),
                        pitch: i16::from(note),
                        pressure: f32::from(value) / 127.0,
                        noteId: -1,
                    },
                },
            )),
            MidiMsg::ControlChange {
                channel,
                controller: cc,
                value,
            } => {
                if let Some(id) = controller(channel, cc as usize) {
                    self.in_params.add(id, t as i32, f64::from(value) / 127.0);
                }
            }
            MidiMsg::ChannelPressure { channel, value } => {
                if let Some(id) = controller(channel, 128) {
                    self.in_params.add(id, t as i32, f64::from(value) / 127.0);
                }
            }
            MidiMsg::PitchBend { channel, value } => {
                if let Some(id) = controller(channel, 129) {
                    let v = (f64::from(value) + 8192.0) / 16383.0;
                    self.in_params.add(id, t as i32, v.clamp(0.0, 1.0));
                }
            }
            MidiMsg::ProgramChange { .. } => {}
        }
    }

    fn fill_context(&mut self, t: &ac_core::PluginTransport) {
        use StatesAndFlags_::*;
        let c = &mut self.context;
        let mut state = kTempoValid | kTimeSigValid | kProjectTimeMusicValid | kBarPositionValid;
        if t.playing {
            state |= kPlaying;
        }
        if t.recording {
            state |= kRecording;
        }
        // `as _`: the flags are a C enum, signed on Windows only.
        c.state = state as _;
        c.sampleRate = self.sample_rate;
        c.projectTimeSamples = t.sample as i64;
        c.projectTimeMusic = t.beats;
        c.barPositionMusic = t.bar_start;
        c.tempo = t.bpm;
        c.timeSigNumerator = i32::from(t.time_sig[0]);
        c.timeSigDenominator = i32::from(t.time_sig[1]);
    }
}

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

impl PluginProcessor for Vst3Processor {
    fn process(&mut self, block: &mut PluginBlock<'_>) -> bool {
        let frames = block.frames;
        if frames == 0 {
            return true;
        }
        if !self.started {
            // SAFETY: the processor is active; VST3 starts processing on
            // the audio thread, which this is.
            unsafe { self.processor.setProcessing(1) };
            self.started = true;
        }
        copy_ports(&mut self.ins, block.audio_in, frames);
        for port in &mut self.outs {
            for ch in port {
                ch[..frames].fill(0.0);
            }
        }
        self.fill_events(block.events);
        self.fill_context(&block.transport);
        self.out_params.clear();
        self.out_events.clear();
        for (b, port) in self.ins.iter_mut().enumerate() {
            for (c, ch) in port.iter_mut().enumerate() {
                self.in_ptrs[b][c] = ch.as_mut_ptr();
            }
            self.in_bus[b].__field0.channelBuffers32 = self.in_ptrs[b].as_mut_ptr();
            self.in_bus[b].silenceFlags = 0;
        }
        for (b, port) in self.outs.iter_mut().enumerate() {
            for (c, ch) in port.iter_mut().enumerate() {
                self.out_ptrs[b][c] = ch.as_mut_ptr();
            }
            self.out_bus[b].__field0.channelBuffers32 = self.out_ptrs[b].as_mut_ptr();
            self.out_bus[b].silenceFlags = 0;
        }
        let raw = |w: Option<vst3::ComRef<'_, IParameterChanges>>| {
            w.map_or(std::ptr::null_mut(), |r| r.as_ptr())
        };
        let mut data = ProcessData {
            processMode: ProcessModes_::kRealtime as i32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            numSamples: frames as i32,
            numInputs: self.in_bus.len() as i32,
            numOutputs: self.out_bus.len() as i32,
            inputs: if self.in_bus.is_empty() {
                std::ptr::null_mut()
            } else {
                self.in_bus.as_mut_ptr()
            },
            outputs: if self.out_bus.is_empty() {
                std::ptr::null_mut()
            } else {
                self.out_bus.as_mut_ptr()
            },
            inputParameterChanges: raw(self.in_params.as_com_ref::<IParameterChanges>()),
            outputParameterChanges: raw(self.out_params.as_com_ref::<IParameterChanges>()),
            inputEvents: self
                .in_events
                .as_com_ref::<IEventList>()
                .map_or(std::ptr::null_mut(), |r| r.as_ptr()),
            outputEvents: self
                .out_events
                .as_com_ref::<IEventList>()
                .map_or(std::ptr::null_mut(), |r| r.as_ptr()),
            processContext: &mut self.context,
        };
        // The plugin's own code is not ours to police (DECISIONS D133).
        // SAFETY: `data` points at buffers, lists, and a context this value
        // owns, all valid and sized for `frames` until the call returns.
        let r = assert_no_alloc::permit_alloc(|| unsafe { self.processor.process(&mut data) });
        let ok = r == kResultOk;
        if ok {
            copy_ports(block.audio_out, &self.outs, frames);
        }
        let last = frames.saturating_sub(1) as u32;
        let midi_out = &mut *block.midi_out;
        self.out_events.with(|events| {
            for e in events.iter() {
                out_note(e, last, midi_out);
            }
        });
        for (id, v) in self.out_params.last_values() {
            let _ = self.to_main.push((id, v));
        }
        ok
    }

    fn latency(&self) -> u32 {
        self.latency
    }

    fn reset(&mut self) {
        // VST3 has no reset: stopping and starting processing clears a
        // plugin's voices and tails.
        if self.started {
            // SAFETY: as in `process`.
            unsafe {
                self.processor.setProcessing(0);
                self.processor.setProcessing(1);
            }
        }
    }
}

impl Drop for Vst3Processor {
    fn drop(&mut self) {
        if self.started {
            // SAFETY: the graph is retired; this thread stands in for the
            // audio thread (as for CLAP, D132).
            unsafe { self.processor.setProcessing(0) };
        }
        // SAFETY: dropped once, here, and never used again; the main half
        // (released next) must see the processor's reference gone.
        unsafe { ManuallyDrop::drop(&mut self.processor) };
        if std::thread::current().id() == self.owner {
            crate::registry::release(self.instance_id);
        } else {
            self.requests.orphaned.store(true, Ordering::Release);
        }
    }
}
