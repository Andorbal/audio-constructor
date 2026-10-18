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
renders, over six runs of the suite: three of
`cargo run --release -p ac-cli -- bench --threads 4 --repeat 3` and three
with `--threads 2`, each run alternated with the same command on a build
from before chain fusion (the last column) so both saw the same machine.
CPU is the share of one core the engine needs to keep up, 100 % ÷ the
real-time ratio. The bench renders without devices, so it leaves out the
driver and the editor.

| Project | 1 thread | CPU, 1 thread | 2 threads | 4 threads | 4 threads before chains |
| --- | --- | --- | --- | --- | --- |
| `subtractive-voices-16` | 11.6–14.0× | 7.2–8.6 % | 1.33–1.57× faster | 1.86–2.44× faster | 1.12–1.18× |
| `poly-synth-16` | 28–33× | 3.0–3.6 % | 1.31–1.38× | 1.38–1.83× | 0.99–1.05× |
| `parallel-chains-8` | 10.6–12.1× | 8.3–9.5 % | 1.34–1.56× | 2.22–2.65× | 0.96–1.10× |
| `mono-synths-16` | 15.7–18.9× | 5.3–6.4 % | 1.27–1.45× | 1.46–1.86× | 1.01–1.35× |
| `diffuser-16` | 4.8–7.3× | 14–21 % | 1.65–2.20× | 2.34–2.99× | 2.54–3.11× |
| `pitch-shift-16` | 9.6–12.9× | 7.7–10.4 % | 1.35–1.61× | 2.22–2.92× | 2.33–2.71× |
| `oscillators-1000` | 3.4–3.7× | 27–30 % | 1.40–1.52× | 2.07–2.34× | 1.95–1.98× |
| `fft-chains-16` | 27.6–29.9× | 3.3–3.6 % | 1.34–1.60× | 1.76–1.91× | 1.29–1.55× |
| `effect-chain-200` | 5.0–5.6× | 18–20 % | 0.92–1.11× | 0.88–1.00× | 0.99–1.06× |

- **A 16-voice subtractive synth under 30 % CPU at 128-frame buffers:**
  sixteen voices of the shipped Subtractive Synth Voice block, every voice
  sounding, take 7.2–8.6 % of one core on one thread, about a quarter of
  the budget, and run about twice as fast on four threads: each voice
  copy's body is one chain, and the sixteen run side by side.
- **The scheduler scaling on independent chains:** `parallel-chains-8`
  (eight chains 24 cheap nodes deep) runs 1.3–1.6× faster on two threads
  and 2.2–2.7× on four. Before chain fusion it got no faster (0.96–1.10×
  in these runs, 0.6–1.0× in earlier ones): each of its 27 waves held
  about 8 µs of work (a Biquad or a Delay takes 1–2 µs per block), and
  handing a wave to the workers and waiting for them cost about as much.
  Giving each thread the same steps every block didn't help in a trial
  (0.5–1.0× on four threads); fusing each linear run into one job, so the
  27 waves become three (the chains, the mix, the output), did. Chains of
  heavy nodes scaled before and still do: `diffuser-16` and
  `pitch-shift-16` up to about 3× on four threads, `oscillators-1000`
  2.1–2.3×. `effect-chain-200` is one chain from end to end, so it has
  nothing to run side by side and stays at 1×.
- **One thread costs the same as before:** callgrind counts 14.02 G
  instructions for the whole one-thread suite before chain fusion and
  14.14 G after (`valgrind --tool=callgrind ./target/release/audioc bench
  --seconds 0.5`), and the one-thread ratios above match the build before
  it within the run-to-run spread. Rendered samples are unchanged (the
  golden hashes didn't move).
- The threaded figures vary from run to run here because every vCPU is
  busy: when the system runs anything else, it takes a core from a worker
  that holds a job, and the wave waits for it. Four threads on four vCPUs
  vary the most; on a machine with spare cores, and with the real-time
  priority `audioc run --threads` asks for, they should be steadier.
