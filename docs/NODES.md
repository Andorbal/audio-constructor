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
  (Oscillator, Clip, Graph, Sampler, Pattern, LFO, envelopes) has:
  - `start: Trigger` — starts or restarts. Unwired, it shows a dropdown:
    *On transport play* (default) / *Never* (only via wire).
  - `stop: Trigger`, `reset: Trigger`.
  - `loop: Bool` for anything with a finite length.
  - `reverse: Trigger` toggles direction where that makes sense (Clip, Graph,
    Sampler, Pattern), and `rate: Float(Ratio)` scales speed.
  - `playing: Bool` and `end: Trigger` outputs so generators can chain.
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
| `Record` | P1 | Writes `in: Audio` to a file while `arm: Bool` is true (or between `start`/`stop` triggers). Outputs `recording: Bool`, `length: Float(Seconds)`. Phase 2 adds `buffer: Buffer` output so a recording can feed a Sampler immediately. |
| `FileOut` | P2 | Offline-only sink: render `in` to a file path. Lets a project produce several files in one `audioc render`. |
| `Gamepad` | P2, Live | Buttons as `Bool` + `Trigger` (press/release), sticks and triggers as `Float` with dead zone and curve. Config: controller index. |
| `KeyboardIn` | P2, Live | Computer keys as `Bool`/`Trigger`, plus a `midi` output mapping a QWERTY row to notes. |
| `OscIn` / `OscOut` | P2, Live | Open Sound Control messages ↔ `Float`/`Trigger`, for phone and tablet control surfaces. |

## Generators

| Node | Phase | What it does |
| --- | --- | --- |
| `Oscillator` | P1 | `freq: Float(Hz)`, `shape` (sine, tri, saw, square, pulse with `width`), `phase: Float`, `sync: Trigger` (hard sync), `fm: Float` (linear FM). Out: `out: Float` at sample rate (wires to Audio or to any parameter). Band-limited for audio use; a `pure` config flag disables anti-aliasing for LFO-style use. |
| `Clip` | P1 | Plays an audio file. Generator contract (start/stop/loop/reverse/rate). `position: Float(Seconds)` input for seeking; outputs `out: Audio`, `position: Float`, `end: Trigger`. Resamples to engine rate on load. |
| `Transport` | P1 | Song time as signals: `time: Float(Seconds)`, `beat: Float(Beats)`, `bar: Int`, `beat_in_bar: Int`, `bpm: Float`, `playing: Bool`, `play: Trigger`, `stop: Trigger`. (The user's "SongTime".) |
| `Noise` | P2 | White, pink, brown; seeded. `gain`, `seed` config. |
| `Graph` | P2 | A hand-drawn curve (lines or béziers, editable in the node) with `duration: Float(Seconds or Beats)`; generator contract. Out: `out: Float`. Doubles as an envelope and, via `Shaper`, as a transfer function. |
| `Sampler` | P2 | Plays a `Buffer` with pitch (`note`/`freq`), start/end points, loop points, crossfade, and the generator contract. The building block for drum machines and keyboards. |
| `Wavetable` | P2 | Scans a table (from a `Buffer` or built-in set) with `position: Float` morphing. |
| `Impulse` | P2 | A single-sample click on `trigger`; for testing filters and building percussion. |
| `Granular` | P3 | Grain cloud over a `Buffer`: grain size, density, spray, pitch, each a socket. |

## Level, dynamics, shaping

| Node | Phase | What it does |
| --- | --- | --- |
| `Gain` | P1 | `in: Audio`, `gain: Float(dB)`. Channel-polymorphic. (Also "Volume".) |
| `Pan` | P1 | Mono or stereo in, stereo out, `pan: Float(-1..1)`: constant-power law for mono, balance (far side fades, near side stays at unity) for stereo. |
| `Mix` | P1 | N inputs (config), each with `gain: Float(dB)`; sums to `out`. Select four nodes and add a Mix to auto-wire them. |
| `Crossfade` | P2 | `a`, `b`, `mix: Float(0..1)`. Works for `Audio` and `Float`. |
| `EnvelopeFollower` | P2 | `in: Audio` → `level: Float` (peak or RMS, attack/release). Half of a compressor, all of a sidechain detector. |
| `GainComputer` | P2 | `level: Float(dB)` → `gain_reduction: Float(dB)` from `threshold`, `ratio`, `knee`; `expand` mode for gates/expanders. The other half of a compressor; also a sidechain compressor when the detector is wired to a different source. |
| `Lookahead` | P2 | Delays `in` by `time` and reports it as latency so `GainComputer` can see transients early. Lookahead + follower + computer + gain = limiter block. |
| `Shaper` | P2 | Waveshaping: built-in curves (tanh, hard clip, fold, tube-ish) or a `Graph` as transfer function; `drive`, `bias`. |
| `Normalize` | P2, WI | Scales a whole signal to a target peak or LUFS. |

## Filters

| Node | Phase | What it does |
| --- | --- | --- |
| `Biquad` | P1 | LP/HP/BP/notch/peak/low-shelf/high-shelf; `cutoff: Float(Hz)`, `q`, `gain`. Coefficients recomputed per block (or per sample when `cutoff` is sample-rate). |
| `SVF` | P2 | State-variable filter with simultaneous LP/BP/HP outputs and stable audio-rate modulation. The synth filter. |
| `Ladder` | P2 | 4-pole ladder with drive; the other synth filter. |
| `Comb` | P2 | Feed-forward/feed-back comb with `delay`, `feedback`, `damping`. Reverb and Karplus-Strong ingredient. |
| `Allpass` | P2 | Schroeder allpass; `delay`, `gain`. Reverb ingredient. |
| `DcBlock` | P2 | Removes DC offset. |
| `Formant` | P3 | Vowel filter bank with morphing. |

## Time

| Node | Phase | What it does |
| --- | --- | --- |
| `Delay` | P1 | `time: Float(Seconds or Beats)` (modulatable, interpolated), `feedback`, `mix`, optional tempo sync. |
| `Reverse` | P2, WI | Plays its input backwards. The canonical whole-input node. |
| `Freeze` | P2 | Holds a short loop of the input while `hold: Bool` is true; `size`, `position`. |
| `PitchShift` | P2 | Real-time pitch shift (granular/PSOLA), `semitones`, `formant`. Reports its latency. |
| `Diffuser` | P2 | A bank of modulated allpasses; one node because doing it with eight Allpass nodes is tedious and slower. |
| `TimeStretch` | P3, WI | Stretch without pitch change (phase-vocoder). |
| `Convolution` | P3 | Partitioned convolution against a `Buffer` (impulse responses). Reports latency. |

## Spectral

| Node | Phase | What it does |
| --- | --- | --- |
| `FFT` | P2 | `in: Audio` → `spectrum: Spectrum`; config: size, window, overlap. Reports latency. |
| `IFFT` | P2 | `Spectrum` → `Audio` with overlap-add. |
| `BandSplit` | P2 | N outputs (config: band edges, editable in the node), each the input filtered to one band. Implemented as crossover filters by default (phase-coherent, low latency) with an FFT mode option. Multiband compression is `BandSplit` + N compressor blocks + `Mix`. |
| `SpectralGate` | P2 | Zero bins below a threshold; `threshold`, `smoothing`. |
| `SpectralShift` | P2 | Shift/scale bin frequencies. |
| `PitchDetect` | P2 | `in: Audio` → `freq: Float(Hz)`, `confidence: Float`, `note: Int`, `onset: Trigger`. Guitar-to-synth. |
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
| `Logic` | P2 | and, or, xor, not, nand. |
| `Convert` | P2 | Unit conversions: dB↔gain, Hz↔MIDI note, semitones↔ratio, seconds↔beats, samples↔seconds. |
| `Smooth` | P2 | Slew limiter / one-pole smoother; `rise`, `fall` times. |
| `Integrate` / `Differentiate` | P2 | With leak/reset. |
| `Random` | P2 | Seeded random on `trigger`; uniform or normal; `min`, `max`. |
| `SampleHold` | P2 | Sample `in` on `trigger`. |
| `Quantize` | P2 | Round `in` to `step`. |
| `Counter` | P2 | Increments on `trigger`, wraps at `max`, `reset`. Out: `Int`. |
| `Accumulate` | P2 | Adds `in` on `trigger`. |
| `Select` | P2 | N inputs (config), `index: Int` picks one. Works for any type. |
| `Switch` | P2 | `a`/`b` by `Bool`. |

## Modulation and envelopes

| Node | Phase | What it does |
| --- | --- | --- |
| `ADSR` | P1 | `gate: Bool` (or `on`/`off` triggers), attack/decay/sustain/release, curve shape (linear or exponential). Out: `out: Float` (sample rate), `active: Bool` (sample rate), `end: Trigger`. |
| `LFO` | P2 | Block-rate oscillator with shapes, `rate` in Hz or beats, `phase`, `retrigger`, `unipolar/bipolar`, `smooth` for random shapes. Lighter than `Oscillator` for hundreds of modulators. |
| `Envelope` | P2 | Multi-stage (AHDSR, arbitrary breakpoints) with loop points. |
| `StepSequencer` | P2 | N steps of `Float` values; advances on `clock`; `direction`, `reset`. Out: `value`, `step: Int`, `step_trigger`. |
| `RandomWalk` | P2 | Smooth random modulation; `rate`, `range`, `seed`. |
| `Chaos` | P3 | Lorenz/Rössler attractor outputs. |

## Triggers and gates

| Node | Phase | What it does |
| --- | --- | --- |
| `Clock` | P1 | Emits `trigger` every `interval` (beats, with division/multiplier, or seconds), sample-accurate to the transport. Also outputs `gate` (high for `duty`). |
| `Button` (see Controls) | P1 | Manual trigger. |
| `ThresholdTrigger` | P2 | Fires when `in` (a level, usually from `EnvelopeFollower`) rises above `threshold` (with `hysteresis`), or stays above for `hold: Float(Seconds)`. Outputs `trigger`, `gate`, `release: Trigger`. |
| `FrequencyTrigger` | P2 | Fires when energy in a band (`low`, `high`) exceeds `threshold`, or when `PitchDetect`'s frequency is above/below/within a range. Outputs `trigger`, `gate`. |
| `Onset` | P2 | Transient detector (spectral flux); `sensitivity`. Out: `trigger`, `strength: Float`. |
| `ClockDivide` | P2 | Pass every Nth trigger; `offset`. |
| `ClockMultiply` | P2 | Interpolates N triggers between incoming ones. |
| `TriggerDelay` | P2 | Delay a trigger by time or beats. |
| `GateToTrigger` | P2 | Rising/falling/both edges. |
| `TriggerToGate` | P2 | Gate high for `length` after a trigger. |
| `Euclidean` | P2 | `steps`, `pulses`, `rotation` → trigger pattern on `clock`. |
| `Probability` | P2 | Pass a trigger with `chance`; seeded. |
| `Burst` | P2 | N triggers at `rate` after one trigger. |
| `Latch` | P2 | `set`/`reset` triggers → `Bool`; or toggle mode. |
| `Timer` | P2 | `start` → `elapsed: Float`, `done: Trigger` after `duration`. |

## Sequencing and notes

| Node | Phase | What it does |
| --- | --- | --- |
| `MidiNote` | P1 | Monophonic `Midi` → `freq: Float(Hz)`, `gate: Bool`, `velocity: Float`, `note: Int`, `trigger`; last/low/high note priority, `glide`. |
| `MidiCC` | P2 | One controller number → `Float` (0..1 or custom range), `channel` filter. |
| `MidiFilter` | P2 | Channel, note range, event type. |
| `MidiTranspose` | P2 | Semitones; `scale` snap option. |
| `Pattern` | P2 | A grid of N step buttons (config: steps, rows). Advances on `clock`; per row outputs `trigger` and `gate`; also `step: Int`, `reset`, `reverse`, generator contract. The user's "pattern of 4, 8, 16 buttons". |
| `MidiPattern` | P2 | A piano-roll clip (notes, lengths, velocities) in beats; generator contract; `midi` output. From a riff to a whole song. |
| `NoteToMidi` | P2 | `trigger`/`gate` + `note: Int` + `velocity` → `Midi` events. How a hand-built keyboard of `Button`s becomes a MIDI source. |
| `Arpeggiator` | P2 | Held notes → sequenced notes on `clock`; modes, octaves. |
| `Chord` | P2 | One note → several (intervals config). |
| `ScaleQuantize` | P2 | Snap `Float`/`Int` notes or `Midi` to a scale; `root`, `scale` with custom masks. |
| `Voices` | P2 | **Polyphony.** A group instance run once per voice (config: voice count, steal policy). `midi` in; inside the body a `Voice` node exposes `freq`, `gate`, `velocity`, `index`, `trigger`; `Audio` outputs from all voices are summed. The synth-building primitive. |

## Controls (UI nodes)

These are nodes whose body is a widget. They are also what gets pinned to
Panels (UX.md) to make a performance surface.

| Node | Phase | What it does |
| --- | --- | --- |
| `Button` | P1 | Momentary: `trigger` on press, `gate: Bool` while held, `release: Trigger`. Bindable to a key or MIDI note. |
| `Toggle` | P1 | `Bool` on/off. Bindable. |
| `Slider` | P1 | `Float` with range, unit, and log/linear scale. Bindable to a MIDI CC or a gamepad axis via the node's menu. |
| `Knob` | P2 | Same as Slider, round. |
| `XYPad` | P2 | Two `Float` outputs plus `gate` on touch. |
| `Keyboard` | P2 | Virtual piano → `midi`; also plays from the computer keyboard when focused. |
| `Number` | P2 | A typed `Float`/`Int` field. |
| `Dropdown` | P2 | Labeled choices → `Int`. |

## Display nodes

| Node | Phase | What it does |
| --- | --- | --- |
| `Meter` | P1 | Peak/RMS with hold; also passes `in` through to `out` so it can sit inline. |
| `Scope` | P1 | Oscilloscope on `Audio`/`Float`, with `trigger` input for stable display; passes through. |
| `Readout` | P1 | Shows the current value of any `Float`/`Int`/`Bool` input. |
| `Analyzer` | P2 | Spectrum analyzer display. |
| `Spectrogram` | P3 | Scrolling spectrogram. |

## Utility

| Node | Phase | What it does |
| --- | --- | --- |
| `Feedback` | P1 | The only legal way to make a cycle. One block of delay; shows its delay in the header. |
| `Reroute` | P1 | A dot for tidy wiring. |
| `Group` | P1 | An instance of a `GroupDef`. Double-click to enter. |
| `SplitChannels` | P1 | `Audio(N)` → N `Audio(1)` outputs. |
| `MergeChannels` | P1 | N `Audio(1)` → `Audio(N)`. |
| `MidSide` | P2 | Encode/decode. |
| `Latency` | P2 | Delay by N samples and report it; also reads the compensation applied at this point (for debugging PDC). |
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
in-app help for the nodes it uses.
