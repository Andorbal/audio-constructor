# Generators

[← All nodes](README.md)

<a id="clip"></a>
## Clip

Kind: `clip` · Also found by: sample, audio file, wav, player, playback, loop, take, file

Plays an audio file, with looping, reverse, and variable speed.

Assign a file in the side panel (the `file` config, a path relative
to the project). The output has `channels` channels (set from the
file when you assign it): a mono file is copied to every channel, and
extra file channels are dropped. Files at another sample rate are
converted once when the project loads. A missing file plays silence
and reports itself as missing so the editor can flag it.

Generator contract: silent until `start`, which plays from the
beginning (the last frame when reversed). At the end it stops and
fires `end`, or wraps around if `loop` is on. `stop` silences it and
fires `end`; `reset` jumps back to the beginning without changing
whether it plays. `reverse` flips the direction, and `rate` sets the
speed (2 is double speed and an octave up; negative values play
backwards); fractional rates are interpolated. Events at the same
frame apply in the order reverse, stop, start, reset. A missing or
empty file fires `end` as soon as it starts, so chains keep moving.

Changing `position` (seconds) seeks there, at the start of the next
block. After loading or a transport reset the clip counts as being at
0, so a fixed nonzero `position` acts as the point transport playback
starts from. The `position` output reports where playback is.

**Try this:** wire a `Clock` into `start` to retrigger a drum loop
on every bar, or chain two Clips by wiring the first one's `end` into
the second one's `start`.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `position` | Float (s) | 0 | 0 … 600 | Seeks here when it changes. |
| `start` | Trigger | `play` | `play`, `load`, `never` | Starts or restarts. Unwired: on transport play, on load, or never. |
| `stop` | Trigger | `stop` | `stop`, `never` | Stops. Unwired: on transport stop, or never. |
| `reset` | Trigger | — |  | Returns to the beginning. |
| `loop` | Bool | false |  | Wrap around at the end. |
| `reverse` | Trigger | — |  | Toggles playback direction. |
| `rate` | Float (×) | 1 | 0 … 4 | Playback speed. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The clip's audio. |
| `position` | Float (s) | Playback position. |
| `playing` | Bool | True while running. |
| `end` | Trigger | Fires on reaching the end or stopping. |

<a id="oscillator"></a>
## Oscillator

Kind: `oscillator` · Also found by: osc, vco, lfo, tone, sine, saw, square, triangle, pulse, wave

A periodic waveform (sine, triangle, saw, square, or pulse) at
`freq` + `fm` Hz, as a sample-rate `Float` that wires into audio or
into any parameter.

The output swings between -1 and 1. Saw, square, and pulse are
band-limited with PolyBLEP and the triangle with PolyBLAMP, so high
notes stay clean; set the `pure` config flag for the naive shapes
(sharper corners, no smoothing) when using the oscillator as an LFO.

- `width` sets the pulse's duty cycle (0.5 is a square).
- `phase` offsets the waveform by a fraction of a cycle; wiring a
  signal into it gives phase modulation.
- `sync` resets the phase to 0 at each trigger (hard sync). The jump
  at the sync point is not band-limited.
- `fm` adds Hz to `freq` (linear FM); negative totals run the
  waveform backwards (through-zero FM).

Generator contract: silent until `start`, which starts (or restarts)
from phase 0. `stop` silences it and fires `end`; `reset` returns to
phase 0 without changing whether it plays. Events at the same frame
apply in the order stop, start, reset, sync.

**Try this:** wire a second Oscillator (`pure`, 5 Hz, sine) through a
`Map` (-1..1 → -20..20) into `fm` for vibrato, or run a saw through a
`Biquad` lowpass for a classic subtractive synth voice.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `freq` | Float (Hz) | 440 | 20 … 20000 | Frequency in hertz. |
| `shape` | Int | `sine` | `sine`, `triangle`, `saw`, `square`, `pulse` | Waveform. |
| `width` | Float | 0.5 | 0.01 … 0.99 | Pulse width (fraction of the cycle spent high). |
| `phase` | Float | 0 | 0 … 1 | Phase offset in cycles. |
| `sync` | Trigger | — |  | Hard sync: resets the phase to 0. |
| `fm` | Float (Hz) | 0 | -1000 … 1000 | Linear FM: hertz added to `freq`. |
| `start` | Trigger | `play` | `play`, `load`, `never` | Starts or restarts. Unwired: on transport play, on load, or never. |
| `stop` | Trigger | `stop` | `stop`, `never` | Stops. Unwired: on transport stop, or never. |
| `reset` | Trigger | — |  | Returns to the beginning. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The waveform, -1 to 1. |
| `playing` | Bool | True while running. |
| `end` | Trigger | Fires on reaching the end or stopping. |

<a id="transport"></a>
## Transport

Kind: `transport` · Also found by: song time, songtime, position, tempo, bpm

Song time as signals: seconds, beats, bars, tempo, and play state.

`time` (seconds) and `beat` (quarter notes) count from the start of
the song and advance every sample while the transport plays; they
hold still while it is stopped. `bar` and `beat_in_bar` count from 0,
update once per block, and follow time-signature changes. `bpm`
follows tempo changes. `play` fires when the transport starts (or
jumps back at a loop point) and `stop` when it stops.

**Try this:** multiply `beat` by 6.283 with a `Math` node and feed it
to a `Unary` (sin) for a wobble locked to the beat, or wire `play`
into an envelope's trigger to restart it whenever the song starts.

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `time` | Float (s) | Song position in seconds. |
| `beat` | Float (beats) | Song position in beats (quarter notes). |
| `bar` | Int | Bar number, from 0. |
| `beat_in_bar` | Int | Beat within the bar, from 0. |
| `bpm` | Float (BPM) | Current tempo. |
| `playing` | Bool | True while the transport plays. |
| `play` | Trigger | Fires when the transport starts. |
| `stop` | Trigger | Fires when the transport stops. |

