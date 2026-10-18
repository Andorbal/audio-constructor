//! Real-time hygiene for sequencing nodes in configurations the default
//! sweep in `all_nodes.rs` doesn't reach: a filled Pattern grid, a MIDI
//! Pattern with notes, the MIDI type of Scale Quantize, custom chords, a
//! Merge fed the same notes on every input, and floods of MIDI. Each runs
//! at block sizes 1, 64, and 1024 with every input driven, under an
//! allocation guard, and must never send more note-offs than note-ons for
//! any note.

use ac_core::testing::Harness;
use ac_core::{MidiMsg, NoResources, SignalType, TempoMap};
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

/// Dense MIDI over `total` frames: notes on several channels, controllers,
/// bend, pressure, and an all-notes-off.
fn busy_midi(total: usize) -> Vec<(u64, MidiMsg)> {
    let mut evs = Vec::new();
    for k in 0..total as u64 / 3 {
        let channel = (k % 3) as u8;
        let note = 30 + (k * 7 % 70) as u8;
        let msg = match k % 6 {
            0 | 2 => MidiMsg::NoteOn {
                channel,
                note,
                velocity: 1 + (k % 127) as u8,
            },
            1 | 3 => MidiMsg::NoteOff {
                channel,
                note: 30 + ((k - 1) * 7 % 70) as u8,
                velocity: 0,
            },
            4 => MidiMsg::ControlChange {
                channel,
                controller: (k % 128) as u8,
                value: (k % 128) as u8,
            },
            _ => MidiMsg::PolyPressure {
                channel,
                note,
                value: 3,
            },
        };
        evs.push((k * 3, msg));
    }
    evs.push((
        total as u64 / 2,
        MidiMsg::ControlChange {
            channel: 0,
            controller: 123,
            value: 0,
        },
    ));
    evs.push((
        total as u64 / 3,
        MidiMsg::PitchBend {
            channel: 1,
            value: -100,
        },
    ));
    evs
}

/// More note-ons in one block than a MIDI output holds.
fn flood() -> Vec<(u64, MidiMsg)> {
    (0..1_500u64)
        .map(|k| {
            (
                0,
                MidiMsg::NoteOn {
                    channel: (k % 16) as u8,
                    note: (k % 128) as u8,
                    velocity: 100,
                },
            )
        })
        .collect()
}

/// No note gets more note-offs from a node than note-ons, per channel.
fn no_extra_note_offs(kind: &str, out: &[(u64, MidiMsg)], passes_input: bool) {
    if passes_input {
        return;
    }
    let mut held = [[0i32; 128]; 16];
    for (_, m) in out {
        match *m {
            MidiMsg::NoteOn { channel, note, .. } => {
                held[usize::from(channel)][usize::from(note)] += 1
            }
            MidiMsg::NoteOff { channel, note, .. } => {
                let h = &mut held[usize::from(channel)][usize::from(note)];
                *h -= 1;
                assert!(*h >= 0, "{kind}: note-off without a note-on");
            }
            _ => {}
        }
    }
    assert!(
        held.iter().flatten().all(|&h| h == 0),
        "{kind}: a note was left hanging"
    );
}

#[test]
fn configured_sequencing_nodes_process_without_allocating() {
    let grid = vec!["x.x.xx..".repeat(8); 16];
    let many: Vec<serde_json::Value> = (0..400)
        .map(|k| json!([f64::from(k) * 0.005, 0.02, 20 + k % 100, 100]))
        .collect();
    let cases = [
        ("pattern", json!({ "steps": 64, "rows": 16, "cells": grid })),
        (
            "midi_pattern",
            json!({
                "length": 0.25,
                "notes": [[0, 0.05, 60, 100], [0.01, 0.3, 61, 90], [0.1, 0.1, 60, 5]]
            }),
        ),
        ("midi_pattern", json!({ "length": 2, "notes": many })),
        ("scale_quantize", json!({ "type": "midi" })),
        (
            "chord",
            json!({ "intervals": [0, 3, 7, 10, 14, 17, 21, 24, 99] }),
        ),
        ("arpeggiator", json!({})),
        ("note_to_midi", json!({})),
        ("midi_transpose", json!({})),
        ("midi_filter", json!({})),
        ("midi_cc", json!({ "initial": 0.5 })),
        // The same notes from every input at once, and floods of them.
        ("merge", json!({})),
        ("merge", json!({ "inputs": 16 })),
    ];
    let reg = ac_nodes::registry();
    for (kind, config) in cases {
        let Some(entry) = reg.get(kind) else {
            panic!("{kind} is not registered");
        };
        // MIDI Filter passes the input's own unmatched note-offs on; every
        // other node only ends notes it started.
        let passes_input = kind == "midi_filter";
        for block in [1usize, 64, 1024] {
            for midi in [busy_midi(block * 4), flood()] {
                let total = block * 4;
                let mut h =
                    Harness::from_entry(entry, config.clone(), 48_000.0, block, &NoResources);
                h.tempo(TempoMap::constant(48_000.0, 2_000.0));
                h.guard(|f| assert_no_alloc(f));
                let inputs = h.signature().inputs.clone();
                for (k, spec) in inputs.iter().enumerate() {
                    let name = spec.name.to_string();
                    match spec.ty {
                        SignalType::Float | SignalType::Bool if spec.choices.is_empty() => {
                            let lo = spec.range.map_or(-1.0, |r| r.0) as f32;
                            let hi = spec.range.map_or(1.0, |r| r.1) as f32;
                            let v = noise(total, k as u32 + 7)
                                .into_iter()
                                .map(|x| lo + (x * 0.5 + 0.5) * (hi - lo))
                                .collect();
                            h.set_samples(&name, v);
                        }
                        SignalType::Trigger if name == "clock" => {
                            let at: Vec<u64> = (0..total as u64).step_by(5).collect();
                            h.triggers(&name, &at);
                        }
                        SignalType::Trigger if name == "trigger" => {
                            let at: Vec<u64> = (0..total as u64).step_by(7).collect();
                            h.triggers(&name, &at);
                        }
                        SignalType::Trigger if name == "start" => {
                            h.triggers(&name, &[0, total as u64 / 2]);
                        }
                        SignalType::Trigger => {
                            let at: Vec<u64> = (0..total as u64)
                                .step_by(block.max(3) / 3 + 11)
                                .skip(1)
                                .collect();
                            h.triggers(&name, &at);
                        }
                        SignalType::Midi => {
                            h.midi(&name, &midi);
                        }
                        _ => {}
                    }
                }
                // Merge's output is `out`; every other node's is `midi`.
                let out = if kind == "merge" { "out" } else { "midi" };
                let mut all = h.run(total).midi(out).to_vec();
                h.stop();
                all.extend_from_slice(h.run(block).midi(out));
                h.reset();
                all.extend_from_slice(h.run(block).midi(out));
                assert!(
                    all.windows(2).all(|w| w[0].0 <= w[1].0),
                    "{kind}: events out of order"
                );
                no_extra_note_offs(kind, &all, passes_input);
            }
        }
    }
}
