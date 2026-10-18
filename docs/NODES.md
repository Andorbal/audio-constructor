# Node Catalog

This is the starting library: what ships in Phase 1 (the minimum to play,
record, and build a synth and an effect chain) and what fills it out in
Phase 2. It is a floor, not a ceiling. New nodes are welcome whenever they
are **primitive**: if a node could be built from existing nodes without
losing performance or clarity, it should be a *block* (a saved group) in
`blocks/`, not a node.

## Conventions every node follows

- **One file per node** in `crates/ac-nodes/src/<category>/<name>.rs`, with a
  doc comment that becomes its entry in the generated reference and the
  in-app help panel. Include one or two "try this" wiring tips.
- **Sockets by name**, snake_case, stable forever. Prefer `in`/`out` for the
  main signal, descriptive names for the rest (`cutoff`, `resonance`).
- **Every numeric input is a `Float` socket** with a unit hint and a soft
  range, so it shows an inline widget when unwired and accepts a wire
  otherwise. No hidden parameters. Config (things that change the socket
  list, like a mixer's input count) is the only exception.
- **Generators share a contract.** Anything that produces a signal over time
  (Oscillator, Clip, Graph, Sampler, Pattern, MidiPattern) has:
  - `start: Trigger` — starts or restarts. Unwired, it shows a dropdown:
    *On transport play* (default) / *Never* (only via wire).
  - `stop: Trigger`, `reset: Trigger`.
  - `loop: Bool` for anything with a finite length.
  - `reverse: Trigger` toggles direction where that makes sense (Clip, Graph,
    Sampler, Pattern), and `rate: Float(Ratio)` scales speed.
  - `playing: Bool` and `end: Trigger` outputs so generators can chain.
  Modulators that follow a gate or a clock instead (ADSR, Envelope, LFO,
  StepSequencer) restart from their own inputs (`gate`/`trigger`,
  `retrigger`, `reset`) and don't take the full contract (DECISIONS D64).
- **Stateful nodes implement `reset()`** and are reset when the transport
  restarts from zero.
- **Sample-rate independent.** Never hard-code 44.1/48 kHz.
- **Tests**: every node has unit tests next to it, and every category has at
  least one golden project under `tests/golden/`.

Socket colors and shapes are in [UX.md](UX.md). Categories below map to
header colors.

Legend: **P1** Phase 1, **P2** Phase 2, **P3** Phase 3. **WI** = `WholeInput`
(see ARCHITECTURE §9). **Live** = a live source for causality analysis.

---

## Input / Output

| Node | Phase | What it does |
| --- | --- | --- |
| `AudioIn` | P1, Live | A device input channel (or pair). Config: channel selection. Out: `out: Audio`. |
| `AudioOut` | P1 | A device output. In: `in: Audio`. Multiple instances allowed (main, cue). |
| `MidiIn` | P1, Live | A MIDI device (or all). Out: `midi: Midi`. |
| `MidiOut` | P2 | Send `Midi` to a device. |
| `Record` | P1 | Writes `in: Audio` to a file while `arm: Bool` is true (or between `start`/`stop` triggers). Outputs `recording: Bool`, `length: Float(Seconds)`, and (Phase 2) `buffer: Buffer`, the last finished take, so a recording can feed a Sampler immediately. |
| `FileOut` | P2 | Offline-only sink: render `in` to a file path. Lets a project produce several files in one `audioc render`. |
| `Gamepad` | P2, Live | A game controller (config `pad`, 1–16). Every button as a `Bool` gate (`south`, `east`, …, `dpad_right`) plus a `<button>_press` `Trigger` (`<button>_release` too with the `releases` config); sticks (`left_x` … `right_y`, −1..1) and analog triggers (`left_trigger_axis`, `right_trigger_axis`, 0..1) as `Float`, shaped by `dead_zone` (radial for sticks, rescaled to full travel) and `curve` (an exponent). Sample-accurate; state survives a transport restart. |
| `KeyboardIn` | P2, Live | Computer keys (config `keys`, a list of names, default `1, 2, 3, 4`) as `key_<name>: Bool` plus `key_<name>_press: Trigger`; and `midi`, playing the middle letter row like a piano (`A S D F G H J K L ; '` white keys from config `low`, default 60; `W E T Y U O P` black keys; `Z`/`X` octave down/up, ±3) on config `channel` at the `velocity` input. Note-offs follow the note each key started; a reset releases notes and strikes held keys again. |
| `OscIn` / `OscOut` | P2, Live | Open Sound Control messages ↔ `Float`/`Trigger`, for phone and tablet control surfaces. |

## Generators

| Node | Phase | What it does |
| --- | --- | --- |
| `Oscillator` | P1 | `freq: Float(Hz)`, `shape` (sine, tri, saw, square, pulse with `width`), `phase: Float`, `sync: Trigger` (hard sync), `fm: Float` (linear FM). Out: `out: Float` at sample rate (wires to Audio or to any parameter). Band-limited for audio use; a `pure` config flag disables anti-aliasing for LFO-style use. |
| `Clip` | P1 | Plays an audio file. Generator contract (start/stop/loop/reverse/rate). `position: Float(Seconds)` input for seeking; outputs `out: Audio`, `position: Float`, `end: Trigger`. Resamples to engine rate on load. |
| `Transport` | P1 | Song time as signals: `time: Float(Seconds)`, `beat: Float(Beats)`, `bar: Int`, `beat_in_bar: Int`, `bpm: Float`, `playing: Bool`, `play: Trigger`, `stop: Trigger`. (The user's "SongTime".) |
| `Noise` | P2 | White, pink, brown at the same RMS; seeded. `gain`, `seed` config; start/stop/reset. |
| `Graph` | P2 | A hand-drawn curve (straight, held, smooth, or bent segments, drawn in the node editor) with `duration: Float(Seconds or Beats)`; generator contract. Out: `out: Float`, `position`. Doubles as an envelope. |
| `Sampler` | P2 | Plays a `Buffer` (or, unwired, an audio file) with pitch (`note` against a `root` config), start/end points, loop points, crossfade, and the generator contract. The building block for drum machines and keyboards. |
| `Wavetable` | P2 | Scans a table (from a `Buffer` or the built-in sine → triangle → saw → square set) with `position: Float` morphing; band-limited per octave; `freq`, `phase`, `sync`, `fm`. |
| `Impulse` | P2 | A single-sample click on `trigger`; for testing filters and building percussion. |
| `Granular` | P3 | Grain cloud over a `Buffer`: grain size, density, spray, pitch, each a socket. |

## Level, dynamics, shaping

| Node | Phase | What it does |
| --- | --- | --- |
| `Gain` | P1 | `in: Audio`, `gain: Float(dB)`. Channel-polymorphic. (Also "Volume".) |
| `Pan` | P1 | Mono or stereo in, stereo out, `pan: Float(-1..1)`: constant-power law for mono, balance (far side fades, near side stays at unity) for stereo. |
| `Mix` | P1 | N inputs (config), each with `gain: Float(dB)`; sums to `out`. Select four nodes and add a Mix to auto-wire them. |
| `Crossfade` | P2 | `a`, `b`, `mix: Float(0..1)`, linear or equal-power `curve`. Works for `Audio` and `Float`. |
| `EnvelopeFollower` | P2 | `in: Audio` → `level: Float` and `level_db` (peak or RMS, attack/release, channels linked). Half of a compressor, all of a sidechain detector. |
| `GainComputer` | P2 | `level: Float(dB)` → `gain_reduction: Float(dB)` and linear `gain` from `threshold`, `ratio`, `knee`; `expand` mode for gates/expanders. The other half of a compressor; also a sidechain compressor when the detector is wired to a different source. |
| `Lookahead` | P2 | Two outputs from one input: `out`, delayed by `time` (reported as latency), and `early`, undelayed, for the detector, so it sees transients before they arrive. Lookahead + follower + computer + gain = limiter block. |
| `Shaper` | P2 | Waveshaping: built-in curves (tanh, hard clip, soft clip, fold, tube) or a custom transfer curve drawn in the node editor; `drive`, `bias`, `mix`; 2× oversampled. |
| `Normalize` | P2, WI | Scales a whole signal to a target sample peak or integrated loudness (LUFS, ITU-R BS.1770); `mode` config. |

## Filters

| Node | Phase | What it does |
| --- | --- | --- |
| `Biquad` | P1 | LP/HP/BP/notch/peak/low-shelf/high-shelf; `cutoff: Float(Hz)`, `q`, `gain`. Coefficients recomputed per block (or per sample when `cutoff` is sample-rate). |
| `SVF` | P2 | State-variable filter with simultaneous lowpass/bandpass/highpass/notch outputs and stable audio-rate modulation of `cutoff` and `resonance`. The synth filter. |
| `Ladder` | P2 | 4-pole ladder with `drive` and lp24/lp12/bp/hp modes; self-oscillates in tune near full `resonance`. The other synth filter. |
| `Comb` | P2 | Feed-forward/feed-back comb with `delay`, `feedback`, `damping`, `mix`; `max_delay` config. Reverb and Karplus-Strong ingredient. |
| `Allpass` | P2 | Schroeder allpass; `delay` (modulatable, allpass-interpolated so it stays lossless), `gain`. Reverb ingredient. |
| `DcBlock` | P2 | Removes DC offset (a one-pole highpass at `cutoff`, 10 Hz by default). |
| `Formant` | P3 | Vowel filter bank with morphing. |

## Time

| Node | Phase | What it does |
| --- | --- | --- |
| `Delay` | P1 | `time: Float(Seconds or Beats)` (modulatable, interpolated), `feedback`, `mix`, optional tempo sync. |
| `Reverse` | P2, WI | Plays its input backwards. The canonical whole-input node. |
| `Freeze` | P2 | Holds a short loop of the input while `hold: Bool` is true, with crossfaded seams; `size`, `position` (read when `hold` rises), `max_time` config. |
| `PitchShift` | P2 | Real-time pitch shift (two crossfading read heads), `semitones`, `formant` (cepstral envelope correction), `grain`, `mix`. Reports its latency, set by the `max_grain` config. |
| `Diffuser` | P2 | Eight modulated allpasses per channel with different prime delays left and right, mono or stereo in, stereo out; `size`, `diffusion`, `mod_depth`, `mod_rate`, `mix`. One node because doing it with eight Allpass nodes is tedious and slower. |
| `TimeStretch` | P3, WI | Stretch without pitch change (phase-vocoder). |
| `Convolution` | P3 | Partitioned convolution against a `Buffer` (impulse responses). Reports latency. |

## Spectral

| Node | Phase | What it does |
| --- | --- | --- |
| `FFT` | P2 | `in: Audio` (first channel) → `spectrum: Spectrum`; config: size, window, overlap. Reports `size − 1` samples of latency, which the engine carries through Spectrum wires so dry paths line up. |
| `IFFT` | P2 | `Spectrum` → `Audio` with least-squares overlap-add, exact at overlap 2 and up; `window` config must match the FFT's. |
| `BandSplit` | P2 | N outputs (config: band edges, editable in the node), each the input filtered to one band. Implemented as crossover filters by default (phase-coherent, low latency) with an FFT mode option. Multiband compression is `BandSplit` + N compressor blocks + `Mix`. |
| `SpectralGate` | P2 | Turn down bins below `threshold` (absolute dB per bin) to `floor`, gliding over `smoothing`. |
| `SpectralShift` | P2 | Shift (`shift`, Hz) and scale (`scale`, ratio) bin frequencies with a peak-locked phase vocoder. |
| `PitchDetect` | P2 | `in: Audio` → `freq: Float(Hz)`, `confidence: Float`, `note: Int`, `onset: Trigger` (YIN; `min_freq`, `max_freq`, `threshold`). Guitar-to-synth. |
| `SpectralFreeze` | P3 | Hold a spectral frame. |

## Math and logic

All math nodes are rate-polymorphic (output rate = highest input rate) and
work on `Float`; `Audio` can be wired in via the implicit conversion.

| Node | Phase | What it does |
| --- | --- | --- |
| `Constant` | P1 | A `Float`/`Int`/`Bool` value with an inline widget. |
| `Math` | P1 | Binary: add, subtract, multiply, divide, min, max, power, modulo, atan2. |
| `Unary` | P1 | sin, cos, tan, exp, log, abs, sqrt, floor, ceil, round, sign, negate, 1/x. `Transport.time → Unary(sin)` is an oscillator locked to song time. |
| `Compare` | P1 | `a`, `b`, op → `Bool`. `hysteresis` input to avoid chatter. |
| `Map` | P1 | Linear/exponential/log remap from one range to another, with clamp option. |
| `Clamp` | P2 | `min`, `max`. |
| `Logic` | P2 | and, or, xor, not, nand, nor. |
| `Convert` | P2 | Unit conversions: dB↔gain, Hz↔MIDI note, semitones↔ratio, seconds↔beats, samples↔seconds. |
| `Smooth` | P2 | Slew limiter / one-pole smoother; `rise`, `fall` times. |
| `Integrate` / `Differentiate` | P2 | Integrate over seconds with `leak`, `reset`, and optional bounds that don't wind up; differentiate per second. |
| `Random` | P2 | Seeded random on `trigger`; uniform or normal; `min`, `max`. |
| `SampleHold` | P2 | Sample `in` on `trigger`. |
| `Quantize` | P2 | Round `in` to `offset + k·step`. |
| `Counter` | P2 | Increments on `trigger`, wraps at `max`, `reset`. Out: `Int`. |
| `Accumulate` | P2 | Adds `in` on `trigger`, from `initial`; `reset`. |
| `Select` | P2 | N inputs (config), `index: Int` picks one (wrapping). Works for any type; audio crossfades over 5 ms, MIDI releases held notes when the route changes. |
| `Switch` | P2 | `a`/`b` by `Bool`, any type, with the same crossfade and note release as `Select`. |

## Modulation and envelopes

| Node | Phase | What it does |
| --- | --- | --- |
| `ADSR` | P1 | `gate: Bool` (or `on`/`off` triggers), attack/decay/sustain/release, curve shape (linear or exponential). Out: `out: Float` (sample rate), `active: Bool` (sample rate), `end: Trigger`. |
| `LFO` | P2 | Block-rate oscillator with shapes, `rate` in Hz or beats, `phase`, `retrigger`, `unipolar/bipolar`, `smooth` for random shapes. Lighter than `Oscillator` for hundreds of modulators. |
| `Envelope` | P2 | Multi-stage (AHDSR, arbitrary breakpoints drawn in the editor) with a sustain point and loop points; `gate`, `trigger`, `time_scale`. |
| `StepSequencer` | P2 | N steps of `Float` values (drawn in the editor); advances on `clock`; `direction`, `length`, `glide`, `reset`. Out: `value`, `step: Int`, `step_trigger`. |
| `RandomWalk` | P2 | Smooth random modulation; `rate`, `range`, `seed`. |
| `Chaos` | P3 | Lorenz/Rössler attractor outputs. |

## Triggers and gates

| Node | Phase | What it does |
| --- | --- | --- |
| `Clock` | P1 | Emits `trigger` every `interval` (beats, with division/multiplier, or seconds), sample-accurate to the transport. Also outputs `gate` (high for `duty`). |
| `Button` (see Controls) | P1 | Manual trigger. |
| `ThresholdTrigger` | P2 | Fires when `in` (a level, usually from `EnvelopeFollower`) rises above `threshold` (with `hysteresis`), or stays above for `hold: Float(Seconds)`. Outputs `trigger`, `gate`, `release: Trigger`. |
| `FrequencyTrigger` | P2 | Fires when energy in a band (`low`, `high`) exceeds `threshold`, or when `PitchDetect`'s frequency is above/below/within a range. Outputs `trigger`, `gate`. |
| `Onset` | P2 | Transient detector (spectral flux); `sensitivity`, `min_interval`. Out: `trigger` (placed at the attack, with fixed latency), `strength: Float`. |
| `ClockDivide` | P2 | Pass every Nth trigger; `offset`. |
| `ClockMultiply` | P2 | Passes each trigger and fills in `multiply − 1` more, evenly spaced by the last interval between incoming ones. |
| `TriggerDelay` | P2 | Delay each trigger by `time` (seconds, or beats with config `unit`). |
| `GateToTrigger` | P2 | Rising/falling/both edges. |
| `TriggerToGate` | P2 | Gate high for `length` after a trigger (seconds or beats); a retrigger restarts the length. |
| `Euclidean` | P2 | `steps`, `pulses`, `rotation` → trigger pattern on `clock`; also `gate` and `step: Int`. |
| `Probability` | P2 | Pass a trigger with `chance`, otherwise send it to `else`; seeded. |
| `Burst` | P2 | `count` triggers, `interval` apart, after one trigger; `done` with the last. |
| `Latch` | P2 | `set`/`reset`/`toggle` triggers → `Bool`, starting at `initial`. |
| `Timer` | P2 | `start`/`stop` → `elapsed: Float`, `running: Bool`, `progress`, and `done: Trigger` after `duration` (where it stops). |

## Sequencing and notes

| Node | Phase | What it does |
| --- | --- | --- |
| `MidiNote` | P1 | Monophonic `Midi` → `freq: Float(Hz)`, `gate: Bool`, `velocity: Float`, `note: Int`, `trigger`; last/low/high note priority, `glide`. |
| `MidiCC` | P2 | One controller number → `Float` (0..1 or custom range, with `smooth`), `channel` filter, `changed: Trigger`. Controller 128 reads pitch bend, 129 channel pressure. |
| `MidiFilter` | P2 | Channel, note range, event types. A note-off whose note-on passed always passes. |
| `MidiTranspose` | P2 | Semitones; `snap` to a `scale` over `root` (or a custom `mask`). Note-offs follow the note each note-on became. |
| `Pattern` | P2 | A grid of N step buttons (config: steps, rows; painted in the editor). Advances on `clock`; per row `k` outputs `trigger_k` and `gate_k`; also `step: Int`, `reset`, `reverse`, generator contract. The user's "pattern of 4, 8, 16 buttons". |
| `MidiPattern` | P2 | A piano-roll clip (notes, lengths, velocities) in beats, drawn in the editor; generator contract with `rate`; `midi` and `position` outputs. From a riff to a whole song. |
| `NoteToMidi` | P2 | `trigger`/`gate` + `note: Int` + `velocity` → `Midi` events. How a hand-built keyboard of `Button`s becomes a MIDI source. |
| `Arpeggiator` | P2 | Held notes → sequenced notes on `clock`; modes up/down/up-down/random/as played, octaves, `gate` length, `latch`. |
| `Chord` | P2 | One note → several: named chords or custom `intervals`, `inversion`, `spread`. |
| `ScaleQuantize` | P2 | Snap `Float`/`Int` notes or `Midi` to a scale; `root`, `scale` with custom masks. |
| `Voices` | P2 | **Polyphony.** A group instance run once per voice (config: voice count 1–64, steal policy oldest/newest/lowest/highest/none). `midi` in; inside the body a `Voice` node exposes `freq`, `gate`, `velocity`, `index`, `trigger`; `Audio` and `Float` outputs from all voices are summed. The synth-building primitive. |
| `Voice` | P2 | Inside a `Voices` body: this voice's `freq`, `gate`, `velocity`, `note`, `trigger`, `index`, and its own `midi` stream. Does nothing elsewhere. |

## Controls (UI nodes)

These are nodes whose body is a widget. They are also what gets pinned to
Panels (UX.md) to make a performance surface.

| Node | Phase | What it does |
| --- | --- | --- |
| `Button` | P1 | Momentary: `trigger` on press, `gate: Bool` while held, `release: Trigger`. Bindable to a key or MIDI note. |
| `Toggle` | P1 | `Bool` on/off. Bindable. |
| `Slider` | P1 | `Float` with range, unit, and log/linear scale. Bindable to a MIDI CC or a gamepad axis via the node's menu. |
| `Knob` | P2 | Same as Slider, round. |
| `XYPad` | P2 | Two `Float` outputs plus `gate` while touched and `touch: Trigger` on press. |
| `Keyboard` | P2 | Virtual piano → `midi` (config: lowest note, keys, channel; `velocity`, `octave`). Computer-keyboard play comes with `KeyboardIn` (M2.4). |
| `Number` | P2 | A typed `Float`/`Int` field with optional limits. |
| `Dropdown` | P2 | Labeled choices (config) → `Int`, and `changed: Trigger`. |

## Display nodes

| Node | Phase | What it does |
| --- | --- | --- |
| `Meter` | P1 | Peak/RMS with hold; also passes `in` through to `out` so it can sit inline. |
| `Scope` | P1 | Oscilloscope on `Audio`/`Float`, with `trigger` input for stable display; passes through. |
| `Readout` | P1 | Shows the current value of any `Float`/`Int`/`Bool` input. |
| `Analyzer` | P2 | Spectrum analyzer display: 64 log-spaced bands from 20 Hz to Nyquist in dB, falling by 20 dB every `decay`; passes `in` through unchanged. |
| `Spectrogram` | P3 | Scrolling spectrogram. |

## Utility

| Node | Phase | What it does |
| --- | --- | --- |
| `Feedback` | P1 | The only legal way to make a cycle. One block of delay; shows its delay in the header. |
| `Reroute` | P1 | A dot for tidy wiring. |
| `Group` | P1 | An instance of a `GroupDef`. Double-click to enter. |
| `SplitChannels` | P1 | `Audio(N)` → N `Audio(1)` outputs. |
| `MergeChannels` | P1 | N `Audio(1)` → `Audio(N)`. |
| `MidSide` | P2 | Encode stereo to `mid`/`side`, or decode back with `width` (`mode` config); the round trip is exact. |
| `Latency` | P2 | Delay by N samples (config) and report it; also outputs the path latency at this point (for debugging PDC). |
| `Info` | P2 | `sample_rate`, `block_size`, `cpu_load`, `xruns` as signals. |
| `Comment` | P1 | Sticky-note text. (Frames around nodes are a canvas feature, not a node.) |

## Blocks that ship in `blocks/` (built from the above)

Compressor, Limiter, Noise Gate, Sidechain Ducker, Multiband Compressor,
Plate Reverb, Hall Reverb, Chorus, Flanger, Phaser, Tremolo, Vibrato,
Auto-Wah, Karplus-Strong String, Subtractive Synth Voice, FM Operator Pair,
Drum Machine (Pattern + Samplers), Metronome, Track (Clip → Gain → Pan with
mute/solo), Vocoder (BandSplit × 2 + EnvelopeFollowers), Tape Stop,
Stutter, Euclidean Drummer.

Every block is also an example of good patching and is linked from the
in-app help for the nodes it uses. Each is a file in `blocks/` with a golden
project (`tests/golden/block-<id>.acproj`) that plays it.

Notes on the blocks as built:

- A block's insides are shared by all its instances (DECISIONS D82), so
  **Track** takes audio in (wire a `Clip` into it) rather than holding a
  Clip, and **Drum Machine** and **Euclidean Drummer** synthesize their
  drums; both also output a trigger per drum to play your own `Sampler`s.
  Track's `soloed`/`any_solo` sockets make solo work across a mix.
- The three instruments (**Karplus-Strong String**, **Subtractive Synth
  Voice**, **FM Operator Pair**) are bodies for `Voices`: add one, tick
  Polyphonic in the Node tab, and play MIDI into it.
- **Phaser** has no feedback control: a loop around its allpasses needs a
  `Feedback` node, whose one-block delay would turn it into a static,
  block-size-dependent comb. **Flanger** feedback is 0–0.95.
- **Multiband Compressor** uses the Compressor block inside and has fixed
  crossover frequencies (BandSplit's `edges` are config).
