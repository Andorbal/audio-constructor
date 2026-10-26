//! MIDI takes from `MIDI Record` nodes (DECISIONS D218).
//!
//! A [`MidiTakes`] collector hands each node a sink that only pushes into a
//! lock-free ring; the editor drains the rings with
//! [`MidiTakes::finished`] and turns each finished take into a MIDI Pattern.
//! Nothing touches a file.

use std::sync::{Arc, Mutex};

use ac_core::{MidiMsg, MidiRecordSink, RecordRequest};
use rtrb::{Consumer, Producer, RingBuffer};

/// Events a sink can hold before the editor drains it (a few seconds of
/// dense playing between frames is plenty).
const RING: usize = 8_192;

#[derive(Clone, Copy, Debug)]
enum Cmd {
    Start(Option<u64>),
    Event(u64, MidiMsg),
    Stop(u64),
}

/// One recorded take.
#[derive(Clone, Debug, PartialEq)]
pub struct MidiTake {
    /// The MIDI Record node's name and flattened path.
    pub name: String,
    pub node_key: String,
    pub sample_rate: u32,
    /// The song position (samples) the take started at, when the transport
    /// was playing.
    pub song_start: Option<u64>,
    /// How long it ran, in samples.
    pub frames: u64,
    /// Each event and when, in samples from the take's start.
    pub events: Vec<(u64, MidiMsg)>,
    /// Events lost because the ring was full.
    pub dropped: u64,
}

struct Channel {
    name: String,
    node_key: String,
    sample_rate: u32,
    rx: Consumer<Cmd>,
    /// The take in progress.
    open: Option<MidiTake>,
}

/// Collects every MIDI Record node's takes.
#[derive(Clone, Default)]
pub struct MidiTakes {
    channels: Arc<Mutex<Vec<Channel>>>,
}

impl std::fmt::Debug for MidiTakes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MidiTakes").finish_non_exhaustive()
    }
}

impl MidiTakes {
    pub fn new() -> MidiTakes {
        MidiTakes::default()
    }

    /// A sink for one node (on the thread that builds the graph).
    pub fn open(&self, req: &RecordRequest<'_>) -> Option<Box<dyn MidiRecordSink>> {
        let (tx, rx) = RingBuffer::new(RING);
        let mut channels = self.channels.lock().ok()?;
        channels.push(Channel {
            name: req.name.to_string(),
            node_key: req.node_key.to_string(),
            sample_rate: req.sample_rate,
            rx,
            open: None,
        });
        Some(Box::new(Sink { tx, dropped: 0 }))
    }

    /// Takes finished since the last call, oldest first. Drains every ring,
    /// and forgets nodes that are gone once their last take is out.
    pub fn finished(&self) -> Vec<MidiTake> {
        let mut out = Vec::new();
        let Ok(mut channels) = self.channels.lock() else {
            return out;
        };
        for c in channels.iter_mut() {
            while let Ok(cmd) = c.rx.pop() {
                match cmd {
                    Cmd::Start(song) => {
                        c.open = Some(MidiTake {
                            name: c.name.clone(),
                            node_key: c.node_key.clone(),
                            sample_rate: c.sample_rate,
                            song_start: song,
                            frames: 0,
                            events: Vec::new(),
                            dropped: 0,
                        });
                    }
                    Cmd::Event(at, msg) => {
                        if let Some(t) = c.open.as_mut() {
                            t.events.push((at, msg));
                        }
                    }
                    Cmd::Stop(frames) => {
                        if let Some(mut t) = c.open.take() {
                            t.frames = frames;
                            out.push(t);
                        }
                    }
                }
            }
        }
        channels.retain(|c| !c.rx.is_abandoned() || c.open.is_some());
        out
    }
}

/// The recorders an editor gives its engine: audio takes to disk (when the
/// project has a folder) and MIDI takes in memory.
#[derive(Clone, Debug, Default)]
pub struct Recorders {
    pub disk: Option<Arc<crate::record::DiskRecorder>>,
    pub midi: MidiTakes,
}

impl crate::assets::RecorderFactory for Recorders {
    fn open(&self, req: &RecordRequest<'_>) -> Option<Box<dyn ac_core::RecordSink>> {
        self.disk.as_ref().and_then(|d| d.open(req))
    }

    fn open_midi(&self, req: &RecordRequest<'_>) -> Option<Box<dyn MidiRecordSink>> {
        self.midi.open(req)
    }
}

/// The audio-thread end: wait-free, allocation-free.
struct Sink {
    tx: Producer<Cmd>,
    dropped: u64,
}

impl Sink {
    fn push(&mut self, cmd: Cmd) {
        if self.tx.push(cmd).is_err() {
            self.dropped += 1;
        }
    }
}

impl MidiRecordSink for Sink {
    fn start_at(&mut self, song: Option<u64>) {
        self.push(Cmd::Start(song));
    }

    fn event(&mut self, frame: u64, msg: MidiMsg) {
        // Keep room for the stop, so a take always ends.
        if self.tx.slots() > 1 {
            self.push(Cmd::Event(frame, msg));
        } else {
            self.dropped += 1;
        }
    }

    fn stop(&mut self, frames: u64) {
        self.push(Cmd::Stop(frames));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> RecordRequest<'static> {
        RecordRequest {
            node_key: "7",
            name: "Keys take",
            channels: 0,
            sample_rate: 48_000,
        }
    }

    #[test]
    fn a_take_arrives_whole_once_it_stops() {
        let takes = MidiTakes::new();
        let mut sink = takes.open(&req()).expect("a sink");
        let on = MidiMsg::NoteOn {
            channel: 0,
            note: 60,
            velocity: 100,
        };
        sink.start_at(Some(96_000));
        sink.event(10, on);
        assert!(takes.finished().is_empty(), "not finished yet");
        sink.event(
            500,
            MidiMsg::NoteOff {
                channel: 0,
                note: 60,
                velocity: 64,
            },
        );
        sink.stop(1_000);
        let t = takes.finished();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].song_start, Some(96_000));
        assert_eq!(t[0].frames, 1_000);
        assert_eq!(t[0].events.len(), 2);
        assert_eq!(t[0].name, "Keys take");
        assert!(takes.finished().is_empty());
        // A sink that's gone is forgotten.
        drop(sink);
        takes.finished();
        assert!(takes.channels.lock().unwrap().is_empty());
    }
}
