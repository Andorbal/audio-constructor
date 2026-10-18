//! Real-time hygiene for node configurations the default sweep in
//! `all_nodes.rs` doesn't reach: a Clip with a loaded (and resampled) file,
//! tempo-synced Delay, a wide Mix, a pure Oscillator, beat-timed trigger
//! nodes, a full-size Keyboard, typed controls, the delay-based filters and
//! time effects at their buffer-size extremes, Mid/Side decoding, and the
//! control-surface nodes under a flood of device events. Each
//! runs at block sizes 1, 64, and 1024 with every input driven, under an
//! allocation guard.

use std::sync::Arc;

use ac_core::testing::Harness;
use ac_core::{
    AudioData, Control, GamepadAxis, GamepadButton, Key, NoResources, Resources, SignalType,
    osc_address_hash,
};
use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use serde_json::json;

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOCATOR: AllocDisabler = AllocDisabler;

struct OneFile(Arc<AudioData>);

impl Resources for OneFile {
    fn audio_asset(&self, _path: &str) -> Option<Arc<AudioData>> {
        Some(self.0.clone())
    }
}

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
fn configured_nodes_process_without_allocating() {
    let file = OneFile(Arc::new(AudioData {
        sample_rate: 44_100,
        channels: vec![noise(3_000, 5), noise(3_000, 6)],
    }));
    let cases = [
        ("clip", json!({ "file": "assets/x.wav", "channels": 2 })),
        ("clip", json!({ "file": "assets/x.wav", "channels": 5 })),
        ("delay", json!({ "unit": "beats", "max_time": 0.5 })),
        ("mix", json!({ "inputs": 64 })),
        ("oscillator", json!({ "pure": true })),
        ("trigger_delay", json!({ "unit": "beats" })),
        ("trigger_to_gate", json!({ "unit": "beats" })),
        ("keyboard", json!({ "low": 21, "keys": 88, "channel": 16 })),
        ("number", json!({ "type": "int", "min": -5, "max": 5 })),
        ("dropdown", json!({ "choices": ["a", "b", "c", "d", "e"] })),
        (
            "xy_pad",
            json!({ "x_min": 20, "x_max": 20000, "y_min": -1, "y_max": 1 }),
        ),
        ("comb", json!({ "max_delay": 2.0 })),
        ("allpass", json!({ "max_delay": 0.001 })),
        ("freeze", json!({ "max_time": 0.05 })),
        ("pitch_shift", json!({ "max_grain": 0.2 })),
        ("pitch_shift", json!({ "max_grain": 0.01 })),
        ("mid_side", json!({ "mode": "decode" })),
    ];
    let reg = ac_nodes::registry();
    for (kind, config) in cases {
        let Some(entry) = reg.get(kind) else {
            panic!("{kind} is not registered");
        };
        for block in [1usize, 64, 1024] {
            let total = block * 4;
            let mut h = Harness::from_entry(entry, config.clone(), 48_000.0, block, &file);
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
                        let at: Vec<u64> = (0..total as u64).step_by(block / 3 + 1).collect();
                        h.triggers(&name, &at);
                    }
                    _ => {}
                }
            }
            let out = h.run(block * 2);
            for (name, chans) in &out.audio {
                assert!(
                    chans.iter().flatten().all(|v| v.is_finite()),
                    "{kind} {config}: output {name} has non-finite samples"
                );
            }
            h.stop();
            h.run(block);
            h.reset();
            h.run(block);
        }
    }
}

/// Control-surface events of every kind for `total` frames, `per_frame` of
/// them on each frame: keys, both pads' buttons and axes, and OSC.
fn control_flood(total: usize, per_frame: usize) -> Vec<(u64, Control, f32)> {
    let keys: Vec<Key> = Key::all().collect();
    let osc = [osc_address_hash("/fader/1"), osc_address_hash("/x")];
    let mut s = 0x9e37_79b9u32;
    let mut out = Vec::new();
    for frame in 0..total as u64 {
        for _ in 0..per_frame {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let pick = s as usize >> 4;
            let control = match s % 4 {
                0 => Control::Key(keys[pick % keys.len()]),
                1 => Control::GamepadButton {
                    pad: (s >> 2) as u8 % 2,
                    button: GamepadButton::ALL[pick % GamepadButton::ALL.len()],
                },
                2 => Control::GamepadAxis {
                    pad: (s >> 2) as u8 % 2,
                    axis: GamepadAxis::ALL[pick % GamepadAxis::ALL.len()],
                },
                _ => Control::Osc {
                    address: osc[pick % 2],
                    arg: (s >> 8) as u8 % 5,
                },
            };
            let value = match s % 5 {
                0 => f32::NAN,
                1 => (s % 2) as f32,
                _ => (s >> 9) as f32 / (1 << 23) as f32 * 2.0 - 1.0,
            };
            out.push((frame, control, value));
        }
    }
    out
}

/// The control-surface nodes under a flood of events (more per block than
/// the engine passes on), with every input driven.
#[test]
fn control_surface_nodes_process_without_allocating() {
    let cases = [
        ("gamepad", json!({})),
        ("gamepad", json!({ "pad": 2, "releases": true })),
        ("keyboard_in", json!({})),
        (
            "keyboard_in",
            json!({ "keys": "A, S, Z, X, Space, 1, F12, Quote", "low": 0, "channel": 16 }),
        ),
        ("osc_in", json!({})),
        ("osc_in", json!({ "address": "/x", "args": 4 })),
        ("osc_in", json!({ "address": "bad" })),
        ("osc_out", json!({})),
        (
            "osc_out",
            json!({ "host": "::1", "port": 9, "address": "/x/y", "args": 4 }),
        ),
        ("osc_out", json!({ "host": "nowhere" })),
    ];
    let reg = ac_nodes::registry();
    for (kind, config) in cases {
        let Some(entry) = reg.get(kind) else {
            panic!("{kind} is not registered");
        };
        for block in [1usize, 64, 1024] {
            let total = block * 4;
            let mut h = Harness::from_entry(entry, config.clone(), 48_000.0, block, &NoResources);
            h.guard(|f| assert_no_alloc(f));
            let inputs = h.signature().inputs.clone();
            for (k, spec) in inputs.iter().enumerate() {
                let name = spec.name.to_string();
                match spec.ty {
                    SignalType::Float | SignalType::Bool => {
                        let lo = spec.range.map_or(-1.0, |r| r.0) as f32;
                        let hi = spec.range.map_or(1.0, |r| r.1) as f32;
                        let v = noise(total, k as u32 + 7)
                            .into_iter()
                            .map(|x| lo + (x * 0.5 + 0.5) * (hi - lo))
                            .collect();
                        h.set_samples(&name, v);
                    }
                    SignalType::Trigger => {
                        let at: Vec<u64> = (0..total as u64).step_by(block / 3 + 1).collect();
                        h.triggers(&name, &at);
                    }
                    _ => {}
                }
            }
            h.host_controls(&control_flood(total, 2_048 / block + 1));
            let out = h.run(block * 2);
            for (name, v) in &out.floats {
                assert!(
                    v.iter().all(|x| x.is_finite()),
                    "{kind} {config}: output {name} has non-finite values"
                );
            }
            h.stop();
            h.run(block);
            h.reset();
            h.run(block);
        }
    }
}
