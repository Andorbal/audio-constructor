//! Real-time hygiene for generator and dynamics configurations the default
//! sweep in `all_nodes.rs` doesn't reach: Buffer inputs (a Sampler playing
//! a buffer, a Wavetable preparing frames from one on the audio thread),
//! loaded files, custom curves, and long lookaheads. Each runs at block
//! sizes 1, 64, and 1024 with every input driven, under an allocation
//! guard.

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
fn buffer_and_configured_nodes_process_without_allocating() {
    let file = OneFile(Arc::new(AudioData {
        sample_rate: 44_100,
        channels: vec![noise(3_000, 5), noise(3_000, 6)],
    }));
    // Eight 2048-sample frames, so preparing them spans many blocks.
    let buffer = Arc::new(AudioData {
        sample_rate: 48_000,
        channels: vec![noise(8 * 2048 + 100, 7)],
    });
    let cases = [
        ("sampler", json!({ "file": "assets/x.wav", "channels": 2 })),
        ("sampler", json!({ "channels": 5, "root": 48 })),
        ("wavetable", json!({ "table": "buffer" })),
        (
            "wavetable",
            json!({ "table": "buffer", "frame_size": 64, "max_frames": 3 }),
        ),
        (
            "graph",
            json!({ "unit": "beats", "points": [[0, 1, "step"], [0.5, -1, "smooth"], [1, 0, 0.7]] }),
        ),
        ("noise", json!({ "seed": 42 })),
        (
            "shaper",
            json!({ "points": [[-1, 1], [0, 0, "smooth"], [1, 1]] }),
        ),
        ("lookahead", json!({ "time": 0.05 })),
        ("normalize", json!({ "mode": "lufs" })),
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
                    SignalType::Buffer => {
                        h.buffer(&name, buffer.clone());
                    }
                    _ => {}
                }
            }
            // Longer than the default sweep so a buffer table finishes
            // preparing at every block size.
            let out = h.run((block * 2).max(2_048 * 12));
            for (name, chans) in &out.audio {
                assert!(
                    chans.iter().flatten().all(|v| v.is_finite()),
                    "{kind} {config}: output {name} has non-finite samples"
                );
            }
            for (name, v) in &out.floats {
                assert!(
                    v.iter().all(|v| v.is_finite()),
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
