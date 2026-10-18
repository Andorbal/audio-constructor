# Benchmarks

`audioc bench` renders nine stress projects (they are generated in code, see
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
| `subtractive-voices-16` | the shipped Subtractive Synth Voice block (two saws, a ladder filter swept by an envelope, the envelope on the volume) in a 16-voice `Voices` node, all sixteen voices held: the Phase 2 acceptance synth |
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

## Phase 2 acceptance

Measured for 0.7.0 on the development machine: a virtual machine with four
vCPUs of an Intel Xeon at 2.1 GHz, a release build, 128-frame blocks at
48 kHz (2.67 ms each), 10 s of audio per project, each figure the best of 3
or 5 renders, over six runs of the suite
(`cargo run --release -p ac-cli -- bench --threads 4 --repeat 5`, and again
with `--threads 2`). CPU is the share of one core the engine needs to keep
up, 100 % ÷ the real-time ratio. The bench renders without devices, so it
leaves out the driver and the editor.

| Project | 1 thread | CPU, 1 thread | 2 threads | 4 threads |
| --- | --- | --- | --- | --- |
| `subtractive-voices-16` | 11.9–16.6× | 6.0–8.4 % | 0.90–1.23× faster | 0.70–1.29× faster |
| `poly-synth-16` | 23–35× | 2.8–4.3 % | 0.78–0.93× | 0.61–1.29× |
| `parallel-chains-8` | 10.7–14.7× | 6.8–9.4 % | 0.68–1.47× | 0.60–1.03× |
| `diffuser-16` | 6.1–7.7× | 13–16 % | 1.29–1.85× | 0.99–2.73× (2.5× and 2.6× in quiet runs) |
| `pitch-shift-16` | 10.1–11.2× | 8.9–9.9 % | 1.15–1.52× | 1.25–2.19× |
| `oscillators-1000` | 3.4–4.3× | 23–29 % | 1.16–1.45× | 1.60–1.96× |

- **A 16-voice subtractive synth under 30 % CPU at 128-frame buffers:**
  sixteen voices of the shipped Subtractive Synth Voice block, every voice
  sounding, take 6–8.4 % of one core on one thread, about a quarter of the
  budget. Extra threads don't help it: a `Voices` node runs all its voices
  inside one step of the schedule.
- **The scheduler scaling on independent chains:** chains of heavy nodes
  scale. `diffuser-16` (sixteen independent Oscillator → Diffuser chains)
  runs 1.3–1.9× faster on two threads, 1.9–2.2× on three, and up to 2.7×
  on four; `pitch-shift-16` and `oscillators-1000` scale too. `parallel-chains-8`
  does not: its chains are 24 cheap nodes deep (a Biquad or a Delay takes
  1–2 µs per block), so each of its 27 waves holds about 8 µs of work, and
  handing a wave to the workers and waiting for them costs about as much on
  this machine. Each step also lands on whichever thread claims it first,
  so a chain's state and buffers move between cores from wave to wave.
  Keeping a step on the thread that ran it last, or running a whole linear
  chain as one job, would cut both costs.
- The threaded figures vary from run to run here because every vCPU is
  busy: when the system runs anything else, it takes a core from a worker
  that holds a job, and the wave waits for it. Four threads on four vCPUs
  vary the most; on a machine with spare cores, and with the real-time
  priority `audioc run --threads` asks for, they should be steadier.
