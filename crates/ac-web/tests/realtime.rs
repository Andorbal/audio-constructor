//! The worklet's render quantum is real-time safe (ARCHITECTURE §8): with
//! the allocation guard as the allocator, a quantum that takes every kind
//! of ring record, runs a patch with MIDI Out, sends its messages back,
//! and publishes telemetry aborts the test if anything allocates.

use std::sync::Arc;

use ac_core::{Control, GamepadButton, MidiMsg, Project};
use ac_engine::Relayed;
use ac_web::audio::{AudioSide, PUBLISH_EVERY};
use ac_web::ring::{Consumer, HeapShared, MAX_RECORD, Producer, init_ring, ring_bytes};
use ac_web::snapshot::{SnapshotReader, SnapshotWriter};
use ac_web::wire::{FromAudio, Posted, RECORD_MAX, ToAudio};
use assert_no_alloc::{AllocDisabler, assert_no_alloc};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

fn example(name: &str) -> Project {
    let path = format!(
        "{}/../../examples/{name}.acproj",
        env!("CARGO_MANIFEST_DIR")
    );
    ac_io::load_project(std::path::Path::new(&path)).unwrap()
}

#[test]
fn a_render_quantum_never_allocates() {
    let to_audio = HeapShared::new(ring_bytes(1 << 16));
    let from_audio = HeapShared::new(ring_bytes(1 << 14));
    let snapshot = HeapShared::new(1 << 20);
    init_ring(&to_audio).unwrap();
    init_ring(&from_audio).unwrap();
    let mut side = AudioSide::new(Arc::new(ac_nodes::registry()), 48_000.0, 128, 2, 2);
    // A patch with MIDI in and out, meters, and bindings.
    let project = example("build-a-keyboard");
    let post = Posted::Project {
        seq: 1,
        generation: 1,
        assets: 0,
        json: project.to_json(),
    };
    side.message(&post.encode()).unwrap();
    let mut tx = Producer::new(&to_audio).unwrap();
    let mut inbox = Consumer::new(&to_audio).unwrap();
    let mut outbox = Producer::new(&from_audio).unwrap();
    let mut back = Consumer::new(&from_audio).unwrap();
    let mut snap = SnapshotWriter::new(&snapshot);
    let mut reader = SnapshotReader::new(&snapshot);
    let mut rec = [0u8; RECORD_MAX];
    let mut push = |m: ToAudio| {
        let n = m.encode(&mut rec);
        assert!(tx.push(&rec[..n]));
    };
    push(ToAudio::Barrier(1));
    push(ToAudio::Relayed(Relayed::Play));
    // Warm up outside the guard (the first graph installs, lazy buffers).
    side.process(128, &mut inbox, &mut outbox, &mut snap);
    for k in 0..(4 * PUBLISH_EVERY) {
        push(ToAudio::Midi(MidiMsg::NoteOn {
            channel: 0,
            note: 60 + k as u8,
            velocity: 100,
        }));
        push(ToAudio::Control(
            Control::GamepadButton {
                pad: 0,
                button: GamepadButton::South,
            },
            1.0,
        ));
        push(ToAudio::Relayed(Relayed::Param {
            generation: 1,
            slot: 0,
            value: 0.5,
        }));
        push(ToAudio::Relayed(Relayed::Seek(4_800)));
        push(ToAudio::Barrier(1));
        assert_no_alloc(|| side.process(128, &mut inbox, &mut outbox, &mut snap));
    }
    assert!(side.handle().is_playing());
    assert!(reader.read().is_some(), "telemetry was published");
    let mut out = [0u8; MAX_RECORD];
    while let Some(n) = back.pop(&mut out) {
        assert!(FromAudio::decode(&out[..n]).is_some());
    }
    assert_eq!(side.dropped, 0);
}
