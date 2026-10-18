//! Real-time hygiene for math and modulation configurations the default
//! sweep in `all_nodes.rs` doesn't reach: Select and Switch for every
//! `type` (with the index moving, so audio crossfades and MIDI releases
//! run), a beats-mode LFO, a looping Envelope, and a 64-step sequencer.
//! Each runs at block sizes 1, 64, and 1024 with every input driven, under
//! an allocation guard.

use ac_core::testing::Harness;
use ac_core::{MidiMsg, NoResources, SignalType};
use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use serde_json::json;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

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

fn midi(total: usize, base: u8) -> Vec<(u64, MidiMsg)> {
    (0..16)
        .map(|i| {
            let at = (i * total / 16) as u64;
            let note = base + (i / 2) as u8;
            let msg = match i % 4 {
                0 => MidiMsg::NoteOn {
                    channel: (i % 3) as u8,
                    note,
                    velocity: 100,
                },
                1 => MidiMsg::ControlChange {
                    channel: 0,
                    controller: 64,
                    value: 127,
                },
                2 => MidiMsg::NoteOff {
                    channel: (i % 3) as u8,
                    note: note.saturating_sub(1),
                    velocity: 0,
                },
                _ => MidiMsg::PitchBend {
                    channel: 1,
                    value: 100,
                },
            };
            (at, msg)
        })
        .collect()
}

#[test]
fn configured_nodes_process_without_allocating() {
    let mut cases = Vec::new();
    for ty in ["float", "audio", "int", "bool", "trigger", "midi"] {
        cases.push(("select", json!({ "type": ty, "inputs": 64 })));
        cases.push(("select", json!({ "type": ty, "inputs": 3 })));
        cases.push(("switch", json!({ "type": ty })));
    }
    cases.extend([
        ("lfo", json!({ "unit": "beats" })),
        (
            "envelope",
            json!({
                "points": [[0, 0], [0.001, 1, "smooth"], [0.002, 0.5, -0.3], [0.004, 0.2], [0.006, 0]],
                "sustain": 3, "loop_start": 1, "loop_end": 3
            }),
        ),
        ("step_sequencer", json!({ "steps": 64, "values": [1, 2, 3] })),
        ("random_walk", json!({ "seed": -7 })),
    ]);
    let reg = ac_nodes::registry();
    for (kind, config) in cases {
        let Some(entry) = reg.get(kind) else {
            panic!("{kind} is not registered");
        };
        for block in [1usize, 64, 1024] {
            let total = block * 6;
            let mut h = Harness::from_entry(entry, config.clone(), 48_000.0, block, &NoResources);
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
                    SignalType::Int if spec.choices.is_empty() => {
                        // A block-rate Int that moves around (Select's index,
                        // Switch's int inputs, the sequencer's length).
                        let v = noise(total, k as u32 + 3)
                            .into_iter()
                            .map(|x| (x * 40.0).round())
                            .collect();
                        h.set_block(&name, v);
                    }
                    SignalType::Trigger => {
                        let at: Vec<u64> = (0..total as u64).step_by(block / 3 + 1).collect();
                        h.triggers(&name, &at);
                    }
                    SignalType::Midi => {
                        h.midi(&name, &midi(total, 40 + (k % 40) as u8));
                    }
                    _ => {}
                }
            }
            let out = h.run(block * 4);
            for (name, chans) in &out.audio {
                assert!(
                    chans.iter().flatten().all(|v| v.is_finite()),
                    "{kind} {config}: output {name} has non-finite samples"
                );
            }
            for (name, v) in &out.floats {
                assert!(
                    v.iter().all(|v| v.is_finite()),
                    "{kind} {config}: output {name} is not finite"
                );
            }
            h.stop();
            h.run(block);
            h.reset();
            h.run(block);
        }
    }
}
