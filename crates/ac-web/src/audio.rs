//! The audio worklet's side (DECISIONS D150): a whole engine, both halves,
//! fed by what the page posts and relays.
//!
//! [`AudioSide::message`] handles a posted message between render quanta:
//! it compiles a project the page sent (with the audio files it plays), so
//! no compile, allocation, or freeing happens inside `process()`.
//! [`AudioSide::process`] runs one render quantum: it takes the ring's
//! records up to a barrier for a project not compiled yet (relayed
//! transport commands and parameter changes, MIDI, control events), runs
//! the processor, sends MIDI Out messages back, and now and then publishes
//! telemetry for the page. It never allocates.

use std::collections::HashMap;
use std::sync::Arc;

use ac_core::{
    AudioData, CONTROL_CAPACITY, ControlEvent, MIDI_CAPACITY, MidiEvent, Project, Registry,
    Resources,
};
use ac_engine::{
    BindingValues, Engine, EngineHandle, EngineOptions, GraphEdit, HostIo, Processor,
    TelemetryReader,
};

use crate::ring::{Consumer, MAX_RECORD, Producer, SharedMem};
use crate::snapshot::SnapshotWriter;
use crate::wire::{FromAudio, Posted, RECORD_MAX, ToAudio};

/// The longest render quantum the worklet takes.
pub const MAX_FRAMES: usize = 4096;
/// Telemetry is published every this many quanta (128 frames each: about
/// every 10 ms at 48 kHz, more often than the page draws).
pub const PUBLISH_EVERY: u64 = 4;
/// Records taken from the ring in one quantum at most; the rest wait.
const RECORDS_PER_QUANTUM: usize = 4096;
/// Quanta a barrier holds the ring for at most (about two seconds at
/// 48 kHz), in case its project never arrives.
const BARRIER_PATIENCE: u32 = 750;

/// Audio files from the page, decoded there (the worklet decodes nothing).
#[derive(Debug, Default)]
struct MemoryAssets {
    files: HashMap<String, Arc<AudioData>>,
}

impl Resources for MemoryAssets {
    fn audio_asset(&self, path: &str) -> Option<Arc<AudioData>> {
        self.files.get(path).cloned()
    }
}

/// What a posted message did.
#[derive(Clone, Debug, PartialEq)]
pub enum Handled {
    /// Compiled project `seq`; `errors` diagnostics were errors.
    Compiled {
        seq: u32,
        errors: usize,
    },
    /// Audio files kept for the project that asks for them.
    Assets,
    Collected,
}

/// The engine inside the audio worklet.
pub struct AudioSide {
    handle: EngineHandle,
    processor: Processor,
    /// The mirror generation of the project compiled last: relayed
    /// parameter changes carry it.
    mirror_generation: u64,
    /// The sequence number of the last project compiled.
    compiled: u32,
    assets_revision: u32,
    /// Files posted for a revision no project has asked for yet.
    next_assets: Option<(u32, MemoryAssets)>,
    in_channels: usize,
    out_channels: usize,
    input: Vec<f32>,
    output: Vec<f32>,
    midi: Vec<MidiEvent>,
    controls: Vec<ControlEvent>,
    midi_out: Vec<MidiEvent>,
    record: [u8; MAX_RECORD],
    reader: TelemetryReader,
    bindings: Arc<BindingValues>,
    export: Vec<u32>,
    binding_words: Vec<u32>,
    quanta: u64,
    /// Quanta the ring has waited at a barrier.
    waited: u32,
    /// Records that didn't decode, and messages that couldn't be sent.
    pub dropped: u64,
}

impl std::fmt::Debug for AudioSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioSide")
            .field("mirror_generation", &self.mirror_generation)
            .field("compiled", &self.compiled)
            .finish_non_exhaustive()
    }
}

impl AudioSide {
    /// An engine at the page's rate and block size, running an empty
    /// project until the page sends one.
    pub fn new(
        registry: Arc<Registry>,
        sample_rate: f64,
        max_block: usize,
        in_channels: usize,
        out_channels: usize,
    ) -> AudioSide {
        let options = EngineOptions {
            sample_rate,
            max_block: max_block.clamp(1, MAX_FRAMES),
            socket_telemetry: true,
            offline: false,
        };
        let (engine, _) = Engine::new(
            Project::default(),
            registry,
            Arc::new(MemoryAssets::default()),
            options,
        );
        let (handle, processor) = engine.split();
        let in_channels = in_channels.min(32);
        let out_channels = out_channels.clamp(1, 32);
        let mut me = AudioSide {
            reader: handle.telemetry(),
            bindings: handle.binding_values(),
            handle,
            processor,
            mirror_generation: u64::MAX,
            compiled: 0,
            assets_revision: 0,
            next_assets: None,
            in_channels,
            out_channels,
            input: vec![0.0; in_channels * MAX_FRAMES],
            output: vec![0.0; out_channels * MAX_FRAMES],
            midi: Vec::with_capacity(MIDI_CAPACITY),
            controls: Vec::with_capacity(CONTROL_CAPACITY),
            midi_out: Vec::with_capacity(MIDI_CAPACITY),
            record: [0; MAX_RECORD],
            export: Vec::new(),
            binding_words: Vec::new(),
            quanta: 0,
            waited: 0,
            dropped: 0,
        };
        me.size_buffers();
        me
    }

    pub fn in_channels(&self) -> usize {
        self.in_channels
    }

    pub fn out_channels(&self) -> usize {
        self.out_channels
    }

    /// Where the page writes this quantum's input: channel `c` at
    /// `c * frames`, for up to [`MAX_FRAMES`] frames.
    pub fn input_mut(&mut self) -> &mut [f32] {
        &mut self.input
    }

    /// This quantum's output, laid out like the input.
    pub fn output(&self) -> &[f32] {
        &self.output
    }

    /// The engine's control half (tests read its position and telemetry).
    pub fn handle(&self) -> &EngineHandle {
        &self.handle
    }

    /// Handle a posted message: compile a project, keep audio files, or
    /// drop retired graphs. Allocates; never call it from `process()`.
    /// A project that doesn't read still counts as compiled (the old one
    /// keeps playing), so the ring never waits for it forever.
    pub fn message(&mut self, bytes: &[u8]) -> Result<Handled, String> {
        let handled = match Posted::decode(bytes)? {
            Posted::Collect => {
                self.handle.collect_garbage();
                Handled::Collected
            }
            Posted::Assets { revision, files } => {
                let files = files
                    .into_iter()
                    .map(|(path, audio)| (path, Arc::new(audio)))
                    .collect();
                self.next_assets = Some((revision, MemoryAssets { files }));
                Handled::Assets
            }
            Posted::Project {
                seq,
                generation,
                assets,
                json,
            } => {
                self.compiled = seq;
                let project = match Project::from_json(&json) {
                    Ok(p) => p,
                    Err(e) => {
                        // Parameter changes for it would name the wrong slots.
                        self.mirror_generation = u64::MAX;
                        return Err(format!("the project didn't read: {e}"));
                    }
                };
                if assets != self.assets_revision
                    && let Some((revision, files)) = self.next_assets.take()
                {
                    if revision == assets {
                        self.handle.set_resources(Arc::new(files));
                        self.assets_revision = revision;
                    } else {
                        self.next_assets = Some((revision, files));
                    }
                }
                let report = self.handle.edit(GraphEdit::Replace(Box::new(project)));
                self.mirror_generation = generation;
                self.size_buffers();
                Handled::Compiled {
                    seq,
                    errors: report.errors().count(),
                }
            }
        };
        Ok(handled)
    }

    /// Room for the newest graph's telemetry, outside `process()`.
    fn size_buffers(&mut self) {
        self.reader = self.handle.telemetry();
        self.bindings = self.handle.binding_values();
        self.export.resize(self.reader.export_len(), 0);
        self.binding_words.resize(self.bindings.len(), 0);
    }

    /// Whether project `seq` is compiled (sequence numbers count up and
    /// wrap).
    fn has_compiled(&self, seq: u32) -> bool {
        (self.compiled.wrapping_sub(seq) as i32) >= 0
    }

    /// Take ring records for this quantum, stopping at a barrier for a
    /// project not compiled yet.
    fn take_records(&mut self, inbox: &mut Consumer<impl SharedMem>) {
        self.midi.clear();
        self.controls.clear();
        for _ in 0..RECORDS_PER_QUANTUM {
            let Some(len) = inbox.peek(&mut self.record) else {
                break;
            };
            match ToAudio::decode(&self.record[..len]) {
                Some(ToAudio::Barrier(seq)) => {
                    if !self.has_compiled(seq) && self.waited < BARRIER_PATIENCE {
                        self.waited += 1;
                        return;
                    }
                    self.waited = 0;
                }
                Some(ToAudio::Relayed(m)) => {
                    self.handle.apply_relayed(m, self.mirror_generation);
                }
                Some(ToAudio::Midi(msg)) => {
                    if self.midi.len() < self.midi.capacity() {
                        self.midi.push(MidiEvent::new(0, msg));
                    } else {
                        self.dropped += 1;
                    }
                }
                Some(ToAudio::Control(control, value)) => {
                    if self.controls.len() < self.controls.capacity() {
                        self.controls.push(ControlEvent::new(0, control, value));
                    } else {
                        self.dropped += 1;
                    }
                }
                None => self.dropped += 1,
            }
            inbox.skip(len);
        }
    }

    /// Run one render quantum of `frames` frames (at most [`MAX_FRAMES`])
    /// with the input the page wrote, leaving the output in
    /// [`AudioSide::output`]. Real-time safe: nothing allocates, locks, or
    /// waits.
    pub fn process<M: SharedMem>(
        &mut self,
        frames: usize,
        inbox: &mut Consumer<M>,
        outbox: &mut Producer<M>,
        snapshot: &mut SnapshotWriter<M>,
    ) {
        let frames = frames.min(MAX_FRAMES);
        self.take_records(inbox);
        self.midi_out.clear();
        let (ic, oc) = (self.in_channels, self.out_channels);
        let mut io = HostIo {
            frames,
            input: &self.input[..ic * frames],
            in_channels: ic,
            output: &mut self.output[..oc * frames],
            out_channels: oc,
            midi: &self.midi,
            midi_out: Some(&mut self.midi_out),
            controls: &self.controls,
            osc_out: None,
        };
        self.processor.process(&mut io);
        let mut rec = [0u8; RECORD_MAX];
        for e in &self.midi_out {
            let n = FromAudio::MidiOut(e.msg).encode(&mut rec);
            if !outbox.push(&rec[..n]) {
                self.dropped += 1;
            }
        }
        self.quanta += 1;
        if self.quanta.is_multiple_of(PUBLISH_EVERY) {
            self.publish(snapshot);
        }
    }

    /// Publish telemetry for the page now. Real-time safe.
    pub fn publish(&mut self, snapshot: &mut SnapshotWriter<impl SharedMem>) {
        let n = self.reader.export(&mut self.export);
        let b = self.bindings.export(&mut self.binding_words);
        if n == self.export.len()
            && b == self.binding_words.len()
            && !snapshot.publish(self.mirror_generation, &self.export, &self.binding_words)
        {
            self.dropped += 1;
        }
    }
}
