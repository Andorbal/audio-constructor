# Benchmarks

`audioc bench` renders eight stress projects (they are generated in code, see
`crates/ac-cli/src/bench.rs`) and reports how many seconds of audio one CPU
core renders per second (the real-time ratio), plus the share of time per
node kind. `--threads N` runs each project again on N threads (the audio
thread plus real-time workers) and reports the speedup:

| Project | What it stresses |
| --- | --- |
| `oscillators-1000` | 1000 band-limited saw oscillators through a tree of 8-input mixers |
| `effect-chain-200` | 200 × (Biquad → Delay → Gain) in series |
| `mono-synths-16` | 16 clock-driven subtractive voices with per-sample filter sweeps |
| `parallel-chains-8` | 8 independent chains of 12 × (Biquad → Delay), mixed: what the multi-core scheduler spreads across cores |
| `poly-synth-16` | a 16-voice `Voices` synth (saw, ADSR-swept filter, ADSR gain) playing a held 16-note chord |
| `fft-chains-16` | 16 × (FFT 2048 → SpectralGate → IFFT), overlap 4 |
| `pitch-shift-16` | 16 × PitchShift, +7 semitones with formant correction |
| `diffuser-16` | 16 × Diffuser (mono in, stereo out) |

- `baseline.json` is the reference from the machine that last updated it
  (128-frame blocks, 10 s per project, best of 3):
  `cargo run --release -p ac-cli -- bench --repeat 3 --json --history bench/history.jsonl > bench/baseline.json`
- `history.jsonl` gets one line per recorded run so trends stay visible.
- CI benches every merge request against its base branch on the same runner
  and fails on a slowdown of more than 20 % (`--compare`, `--threshold`).
  Absolute numbers differ between machines; compare runs from one machine.
