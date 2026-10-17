//! Sweeps every registered node: docs, socket naming, and real-time hygiene
//! (no allocation in `process`, at block sizes 1, 64, and 1024).

use ac_core::testing::Harness;
use ac_core::{MidiMsg, NoResources, SignalType, signal::is_snake_case};
use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use serde_json::json;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

/// Every node marked P1 in docs/NODES.md, by registry kind. `group` is handled
/// by the compiler but registered for docs and the palette.
const P1_KINDS: &[&str] = &[
    "audio_in",
    "audio_out",
    "midi_in",
    "record",
    "oscillator",
    "clip",
    "transport",
    "gain",
    "pan",
    "mix",
    "biquad",
    "delay",
    "constant",
    "math",
    "unary",
    "compare",
    "map",
    "adsr",
    "clock",
    "midi_note",
    "button",
    "toggle",
    "slider",
    "meter",
    "scope",
    "readout",
    "feedback",
    "reroute",
    "group",
    "split_channels",
    "merge_channels",
    "comment",
];

fn noise(n: usize, seed: u32) -> Vec<f32> {
    let mut s = seed.wrapping_mul(2_654_435_761).max(1);
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32) * 2.0 - 1.0
        })
        .collect()
}

#[test]
fn every_node_processes_without_allocating() {
    let reg = ac_nodes::registry();
    for entry in reg.iter() {
        for block in [1usize, 64, 1024] {
            let total = block * 4;
            let mut h = Harness::from_entry(entry, json!({}), 48_000.0, block, &NoResources);
            h.guard(|f| assert_no_alloc(f));
            let inputs = h.signature().inputs.clone();
            for (k, spec) in inputs.iter().enumerate() {
                let name = spec.name.to_string();
                match spec.ty {
                    SignalType::Audio => {
                        h.set_audio(&name, vec![noise(total, k as u32), noise(total, 99)]);
                    }
                    SignalType::Float | SignalType::Bool if spec.choices.is_empty() => {
                        let lo = spec.range.map_or(-1.0, |r| r.0) as f32;
                        let hi = spec.range.map_or(1.0, |r| r.1) as f32;
                        let v = noise(total, k as u32 + 7)
                            .into_iter()
                            .map(|x| lo + (x * 0.5 + 0.5) * (hi - lo))
                            .collect();
                        h.set_samples(&name, v);
                    }
                    SignalType::Trigger => {
                        let at: Vec<u64> =
                            (0..total as u64).step_by(block.max(3) / 3 + 1).collect();
                        h.triggers(&name, &at);
                    }
                    SignalType::Midi => {
                        let evs: Vec<(u64, MidiMsg)> = (0..8)
                            .map(|i| {
                                let at = (i * total / 8) as u64;
                                let msg = if i % 2 == 0 {
                                    MidiMsg::NoteOn {
                                        channel: 0,
                                        note: 60 + i as u8,
                                        velocity: 100,
                                    }
                                } else {
                                    MidiMsg::NoteOff {
                                        channel: 0,
                                        note: 60 + i as u8 - 1,
                                        velocity: 0,
                                    }
                                };
                                (at, msg)
                            })
                            .collect();
                        h.midi(&name, &evs);
                    }
                    _ => {}
                }
            }
            h.host_input(vec![noise(total, 1), noise(total, 2)]);
            h.host_outputs(2);
            h.host_midi(&[(
                0,
                MidiMsg::NoteOn {
                    channel: 0,
                    note: 64,
                    velocity: 90,
                },
            )]);
            let out = h.run(block * 2);
            for (name, chans) in &out.audio {
                for c in chans {
                    assert!(
                        c.iter().all(|v| v.is_finite()),
                        "{} output {name} has non-finite samples",
                        entry.meta.kind
                    );
                }
            }
            h.stop();
            h.run(block);
            h.reset();
            h.run(block);
        }
    }
}

#[test]
fn every_node_documents_itself_with_a_tip() {
    for entry in ac_nodes::registry().iter() {
        let doc = entry.meta.doc_text();
        assert!(
            !entry.meta.summary().is_empty(),
            "{} has no summary",
            entry.meta.kind
        );
        assert!(
            doc.to_lowercase().contains("try this"),
            "{} docs need a \"Try this\" tip",
            entry.meta.kind
        );
    }
}

#[test]
fn socket_names_are_unique_snake_case() {
    for entry in ac_nodes::registry().iter() {
        let sig = (entry.describe)(&json!({}));
        for list in [&sig.inputs, &sig.outputs] {
            let mut seen = std::collections::BTreeSet::new();
            for s in list {
                assert!(
                    is_snake_case(&s.name),
                    "{}: `{}` is not snake_case",
                    entry.meta.kind,
                    s.name
                );
                assert!(
                    seen.insert(s.name.clone()),
                    "{}: duplicate socket `{}`",
                    entry.meta.kind,
                    s.name
                );
            }
        }
        assert!(
            is_snake_case(entry.meta.kind),
            "kind `{}` is not snake_case",
            entry.meta.kind
        );
    }
}

#[test]
#[ignore = "enabled once every P1 node lands (M1.4)"]
fn every_phase1_node_is_registered() {
    let reg = ac_nodes::registry();
    let missing: Vec<_> = P1_KINDS.iter().filter(|k| !reg.contains(k)).collect();
    assert!(missing.is_empty(), "missing P1 nodes: {missing:?}");
}
