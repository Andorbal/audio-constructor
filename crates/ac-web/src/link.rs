//! The page's side of the link to the audio worklet (DECISIONS D150,
//! D151).
//!
//! The editor runs a mirror engine on the page. Once a frame,
//! [`PageLink::sync`] looks at what it did: when the mirror compiled a new
//! project, the project (and, when they changed, its audio files) is
//! posted to the worklet with a barrier in the ring; then everything the
//! mirror's handle sent is relayed through the ring. MIDI and control
//! events go straight into the ring as they arrive ([`PageLink::send_midi`],
//! [`PageLink::send_control`]) and are kept for the mirror too, so the
//! editor can learn bindings from them. [`PageLink::import`] copies the
//! worklet's newest telemetry into the mirror, so every meter, scope, and
//! the transport readout show what the worklet plays.

use std::collections::HashMap;
use std::sync::Arc;

use ac_core::{AudioData, Control, ControlEvent, MidiEvent, MidiMsg, NodePath};
use ac_engine::{EngineHandle, Processor, STATUS_WORDS, WholeState};

use crate::ring::{Consumer, MAX_RECORD, Producer, SharedMem};
use crate::snapshot::SnapshotReader;
use crate::wire::{FromAudio, Posted, RECORD_MAX, ToAudio};

/// The audio files a project plays, as the page decoded them.
pub type Assets = [(String, Arc<AudioData>)];

/// The page's end of the link.
pub struct PageLink<M> {
    to_audio: Producer<M>,
    from_audio: Consumer<M>,
    snapshot: SnapshotReader<M>,
    /// The sequence number of the last project posted.
    seq: u32,
    /// The mirror generation of the last project posted.
    sent: Option<u64>,
    assets_revision: u32,
    sent_assets: Vec<(String, Arc<AudioData>)>,
    /// The key of the bake last posted for each node.
    sent_bakes: HashMap<NodePath, u64>,
    learn_midi: Vec<MidiEvent>,
    learn_controls: Vec<ControlEvent>,
    record: [u8; MAX_RECORD],
    /// Records the ring had no room for.
    pub dropped: u64,
    /// Whether the last snapshot imported was for the project the mirror
    /// runs.
    pub in_step: bool,
}

impl<M> std::fmt::Debug for PageLink<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageLink")
            .field("seq", &self.seq)
            .field("sent", &self.sent)
            .field("dropped", &self.dropped)
            .finish_non_exhaustive()
    }
}

impl<M: SharedMem> PageLink<M> {
    /// The link over three shared buffers: the ring to the worklet, the
    /// ring back, and the telemetry snapshot. `None` if a ring isn't set
    /// up.
    pub fn new(to_audio: M, from_audio: M, snapshot: M) -> Option<PageLink<M>> {
        Some(PageLink {
            to_audio: Producer::new(to_audio)?,
            from_audio: Consumer::new(from_audio)?,
            snapshot: SnapshotReader::new(snapshot),
            seq: 0,
            sent: None,
            assets_revision: 0,
            sent_assets: Vec::new(),
            sent_bakes: HashMap::new(),
            learn_midi: Vec::new(),
            learn_controls: Vec::new(),
            record: [0; MAX_RECORD],
            dropped: 0,
            in_step: false,
        })
    }

    fn push(&mut self, m: ToAudio) -> bool {
        let mut rec = [0u8; RECORD_MAX];
        let n = m.encode(&mut rec);
        let ok = self.to_audio.push(&rec[..n]);
        if !ok {
            self.dropped += 1;
        }
        ok
    }

    /// Send MIDI from an input to the worklet now (it plays at the start
    /// of the next render quantum).
    pub fn send_midi(&mut self, msg: MidiMsg) -> bool {
        if self.learn_midi.len() < 1024 {
            self.learn_midi.push(MidiEvent::new(0, msg));
        }
        self.push(ToAudio::Midi(msg))
    }

    /// Send a control event (a key, a gamepad control) to the worklet now.
    pub fn send_control(&mut self, control: Control, value: f32) -> bool {
        if self.learn_controls.len() < 1024 {
            self.learn_controls
                .push(ControlEvent::new(0, control, value));
        }
        self.push(ToAudio::Control(control, value))
    }

    /// Once a frame, after the editor ran: post the project if the mirror
    /// compiled a new one (its audio files first, if they changed), then
    /// relay what the mirror's handle sent. Returns the messages to post
    /// to the worklet, in order.
    pub fn sync(
        &mut self,
        handle: &EngineHandle,
        mirror: &mut Processor,
        assets: &Assets,
    ) -> Vec<Vec<u8>> {
        let mut posts = Vec::new();
        if self.sent != Some(handle.generation()) {
            let same_assets = assets.len() == self.sent_assets.len()
                && assets
                    .iter()
                    .zip(&self.sent_assets)
                    .all(|((p, a), (q, b))| p == q && Arc::ptr_eq(a, b));
            if !same_assets {
                self.assets_revision = self.assets_revision.wrapping_add(1);
                self.sent_assets = assets.to_vec();
                posts.push(
                    Posted::Assets {
                        revision: self.assets_revision,
                        files: assets
                            .iter()
                            .map(|(p, a)| (p.clone(), AudioData::clone(a)))
                            .collect(),
                    }
                    .encode(),
                );
            }
            self.seq = self.seq.wrapping_add(1);
            posts.push(
                Posted::Project {
                    seq: self.seq,
                    generation: handle.generation(),
                    assets: self.assets_revision,
                    json: handle.project().to_json(),
                }
                .encode(),
            );
            self.push(ToAudio::Barrier(self.seq));
            self.sent = Some(handle.generation());
        }
        // Bakes the mirror finished, after the project they belong to (the
        // worklet takes a bake for the project it runs), so it never bakes
        // on the audio thread.
        for spec in handle.bakes() {
            if spec.state != WholeState::Baked || self.sent_bakes.get(&spec.path) == Some(&spec.key)
            {
                continue;
            }
            if let Some(r) = handle.bake_result(&spec.path).filter(|r| r.key == spec.key) {
                posts.push(
                    Posted::Bake {
                        path: r.path.to_string(),
                        key: r.key,
                        outputs: (*r.outputs).clone(),
                    }
                    .encode(),
                );
                self.sent_bakes.insert(spec.path.clone(), spec.key);
            }
        }
        let midi = std::mem::take(&mut self.learn_midi);
        let controls = std::mem::take(&mut self.learn_controls);
        let mut relayed = Vec::new();
        mirror.relay(&midi, &controls, &mut |m| relayed.push(m));
        for m in relayed {
            self.push(ToAudio::Relayed(m));
        }
        self.learn_midi = midi;
        self.learn_midi.clear();
        self.learn_controls = controls;
        self.learn_controls.clear();
        posts
    }

    /// Copy the worklet's newest telemetry into the mirror. The transport
    /// and levels always go in; meters, scopes, and socket values only
    /// when the worklet runs the project the mirror runs. Returns whether
    /// there was a new snapshot.
    pub fn import(&mut self, handle: &EngineHandle) -> bool {
        let Some(s) = self.snapshot.read() else {
            return false;
        };
        let reader = handle.telemetry();
        if s.generation == handle.generation() {
            self.in_step = reader.import(s.telemetry) && handle.binding_values().import(s.bindings);
        } else {
            self.in_step = false;
            if let Some(status) = s.telemetry.first_chunk::<STATUS_WORDS>() {
                reader.status().import_published(status);
            }
        }
        true
    }

    /// MIDI the worklet's MIDI Out nodes sent since the last call.
    pub fn take_midi_out(&mut self, mut out: impl FnMut(MidiMsg)) {
        while let Some(n) = self.from_audio.pop(&mut self.record) {
            match FromAudio::decode(&self.record[..n]) {
                Some(FromAudio::MidiOut(m)) => out(m),
                None => self.dropped += 1,
            }
        }
    }
}
