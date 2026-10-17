//! Real-time hygiene for node configurations the default sweep in
//! `all_nodes.rs` doesn't reach: a Clip with a loaded (and resampled) file,
//! tempo-synced Delay, a wide Mix, a pure Oscillator. Each runs at block
//! sizes 1, 64, and 1024 with every input driven, under an allocation guard.

use std::sync::Arc;

use ac_core::testing::Harness;
use ac_core::{AudioData, Resources, SignalType};
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
