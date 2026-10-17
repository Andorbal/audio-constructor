# Benchmarks

`audioc bench` renders three stress projects built from Phase 1 nodes (they
are generated in code, see `crates/ac-cli/src/bench.rs`) and reports how many
seconds of audio one CPU core renders per second (the real-time ratio), plus
the share of time per node kind:

| Project | What it stresses |
| --- | --- |
| `oscillators-1000` | 1000 band-limited saw oscillators through a tree of 8-input mixers |
| `effect-chain-200` | 200 × (Biquad → Delay → Gain) in series |
| `mono-synths-16` | 16 clock-driven subtractive voices with per-sample filter sweeps |

- `baseline.json` is the reference from the machine that last updated it
  (128-frame blocks, 10 s per project, best of 3):
  `cargo run --release -p ac-cli -- bench --repeat 3 --json --history bench/history.jsonl > bench/baseline.json`
- `history.jsonl` gets one line per recorded run so trends stay visible.
- CI benches every merge request against its base branch on the same runner
  and fails on a slowdown of more than 20 % (`--compare`, `--threshold`).
  Absolute numbers differ between machines; compare runs from one machine.
