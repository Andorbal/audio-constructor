# Generators

[← All nodes](README.md)

Sources of signal over time: oscillators, noise, audio files, samplers, drawn curves, and the song position.

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

**Placed in the song:** set `start` to *At beat…* and the clip
starts when the song reaches beat `at`, on that sample, and stops at
beat `until` (0 plays on). Start the
transport anywhere (play from bar 9, a seek, a loop) and the clip is
already where it would be had the song played from the top:
partway through, around again if it loops, or finished and silent
if it doesn't; before its beat it waits.

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
| `start` | Trigger | `play` | `play`, `load`, `never`, `at` | Starts or restarts. Unwired: on transport play, on load, never, or at a beat of the song. |
| `at` | Float (beats) | 0 | 0 … 256 | The song beat it plays from, with `start` set to At beat. |
| `until` | Float (beats) | 0 | 0 … 256 | The song beat it stops at, placed in the song; 0 plays on. |
| `stop` | Trigger | `stop` | `stop`, `never` | Stops. Also on transport stop, wired or not, unless set to never. |
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

<a id="granular"></a>
## Granular

Kind: `granular` · Also found by: grain, granular synthesis, cloud, texture, microsound, scatter, buffer, freeze, pad

A cloud of grains: tiny, overlapping slices of a recorded sound,
each a few milliseconds to a second long, played over and over from
around one spot. A plain recording turns into a texture, a pad, or
a shimmer, and it can stand still in time.

**Source.** A wired `buffer` (a `Record` node's last take, say) is
played; with `buffer` unwired, the `file` config (an audio file in
the project) is used, converted to the engine's rate when the
project loads. Nothing to play gives silence; a missing file is
flagged in the editor. Each grain keeps the sound it started with,
so a new buffer comes in grain by grain.

**Grains.** About `density` grains start each second, at random
moments around that rate. Each lasts `size` seconds under a smooth
(Hann) fade in and out, and reads the sound from `position` (0 to
1, a fraction of the whole sound), moved by a random amount of up
to `spray` seconds either way so the grains don't all repeat the
same slice. `pitch` (semitones) sets how fast each grain plays, so
the cloud can be transposed while `position` holds still. Each grain
sits somewhere in the stereo field, randomly within `spread` (0: all
in the middle, 1: anywhere from left to right). Every input is read
when a grain starts, so the cloud follows modulation grain by grain.
As grains overlap more, each is turned down (by the square root of
how many overlap), so the cloud stays about as loud as one grain;
`gain` is in decibels. At most 128 grains sound at once.

**Generator contract.** `start` begins the cloud, `stop` lets the
grains that are playing finish and starts no more (`end` fires),
and `reset` starts the random choices over, so a cloud repeats
exactly. The randomness comes from the project's seed, this node's
place, and the `seed` config, so renders repeat; output doesn't
depend on the block size.

**Try this:** record a phrase into a `Record` node and wire its
`buffer` here with `size` at 0.08, `density` 30, and `spray` 0.05;
sweep `position` slowly with an `LFO` to drift through the phrase,
or hold it still and play `pitch` from a `Keyboard` for a frozen,
tuned texture.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `buffer` | Buffer | — |  | Sound to granulate; unwired, the `file` config plays. |
| `position` | Float | 0 | 0 … 1 | Where grains read from (fraction of the sound). |
| `size` | Float (s) | 0.1 | 0.005 … 1 | Length of each grain. |
| `density` | Float (Hz) | 20 | 0.5 … 200 | Grains started per second. |
| `spray` | Float (s) | 0.02 | 0 … 1 | Random offset of each grain's start, either way. |
| `pitch` | Float (st) | 0 | -24 … 24 | Transposes every grain. |
| `spread` | Float | 0.5 | 0 … 1 | How far grains scatter across the stereo field. |
| `gain` | Float (dB) | 0 | -60 … 12 | Level in decibels. |
| `start` | Trigger | `play` | `play`, `load`, `never` | Starts or restarts. Unwired: on transport play, on load, or never. |
| `stop` | Trigger | `stop` | `stop`, `never` | Stops. Also on transport stop, wired or not, unless set to never. |
| `reset` | Trigger | — |  | Returns to the beginning. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The cloud, in stereo. |
| `playing` | Bool | True while running. |
| `end` | Trigger | Fires on reaching the end or stopping. |

<a id="graph"></a>
## Graph

Kind: `graph` · Also found by: curve, envelope, automation, breakpoint, draw, line, ramp, shape, lfo

A hand-drawn curve played over time: an envelope, an automation
lane, or (looped) a custom LFO, as a sample-rate `Float`.

The curve lives in the `points` config as `[x, y]` breakpoints:
`x` is the fraction of the way through (0 at the start, 1 at the
end) and `y` is the output value, any range. A third entry bends the
segment that starts at that point: `"step"` holds `y` until the next
point, `"smooth"` eases in and out, and a number from −1 to 1 bows
the segment (positive rises late, negative rises early); without it
the segment is a straight line. Before the first point and after
the last the curve holds their values. The default rises from 0 to 1
and falls back: `[[0, 0], [0.5, 1], [1, 0]]`.

`duration` is how long one pass takes: seconds, or quarter-note
beats that follow the tempo when the `unit` config is `"beats"`.
Durations shorter than a sample take one sample.

`out` is the curve at the playhead. Before the first `start` it
holds the value at `x` = 0; after a pass finishes it holds the end
value (the value at 1, or at 0 when reversed). `position` reports
the playhead (0 to 1, once per block).

**Placed in the song:** set `start` to *At beat…* and the curve
starts at song beat `at` (and stops at `until`, unless that's 0). Start the transport anywhere and the curve
is already where it would be: an automation lane holds its place
through a seek or a loop. Before its beat it holds its first value,
and past the end of an unlooped pass, its last.

Generator contract: `start` plays from the beginning (from the end
when reversed). A pass lasts `duration`; on its last frame `out`
reaches the end value, `end` fires, and the Graph stops, or wraps
around and keeps going if `loop` is on (a looped pass repeats
exactly every `duration`). `stop` freezes the playhead, holding the
current value, and fires `end`; `reset` jumps back to the beginning
without changing whether it plays. `reverse` flips the direction
and `rate` scales the speed (negative plays backwards). Events at
the same frame apply in the order reverse, stop, start, reset.

**Try this:** wire a `Clock` into `start` and `out` into a `Gain`
(through a `Map` into decibels) for a hand-drawn envelope on every
beat, or set `loop` with `unit` beats and `duration` 4 for a
one-bar modulation shape that stays in time.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `duration` | Float (s) | 1 | 0 … 10 | How long one pass takes. |
| `start` | Trigger | `play` | `play`, `load`, `never`, `at` | Starts or restarts. Unwired: on transport play, on load, never, or at a beat of the song. |
| `at` | Float (beats) | 0 | 0 … 256 | The song beat it plays from, with `start` set to At beat. |
| `until` | Float (beats) | 0 | 0 … 256 | The song beat it stops at, placed in the song; 0 plays on. |
| `stop` | Trigger | `stop` | `stop`, `never` | Stops. Also on transport stop, wired or not, unless set to never. |
| `reset` | Trigger | — |  | Returns to the beginning. |
| `loop` | Bool | false |  | Wrap around at the end. |
| `reverse` | Trigger | — |  | Toggles playback direction. |
| `rate` | Float (×) | 1 | 0 … 4 | Playback speed. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The curve at the playhead. |
| `position` | Float | Playhead, 0 to 1. |
| `playing` | Bool | True while running. |
| `end` | Trigger | Fires on reaching the end or stopping. |

<a id="impulse"></a>
## Impulse

Kind: `impulse` · Also found by: click, dirac, spike, ping, test signal, impulse response

A single-sample click on each `trigger`, as a sample-rate `Float`.

The output is `gain` (default 1) on the exact frame of each trigger
and 0 everywhere else; several triggers on one frame make one click.
Negative `gain` clicks downwards. Unwired, `trigger` fires when the
transport starts (or on load, or never: pick in the dropdown), so
pressing play sends one click; its button fires one by hand.

A click contains every frequency at once, which makes it the
standard way to hear (or measure) what a filter, delay, or reverb
does: what comes out is the patch's impulse response.

**Try this:** wire it into a `Biquad` with high `q` for a pitched
"ping", or into a `Delay` with high `feedback` to hear the echo
pattern; drive `trigger` from a `Clock` for a metronome click.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `trigger` | Trigger | `play` | `play`, `load`, `never` | Each trigger makes one click. Unwired: on transport play, on load, or never. |
| `gain` | Float | 1 | -1 … 1 | Height of the click. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The clicks; 0 between them. |

<a id="noise"></a>
## Noise

Kind: `noise` · Also found by: white noise, pink noise, brown noise, hiss, random, static, rumble

Random noise in three colors: white, pink, or brown, as a
sample-rate `Float` that wires into audio or into any parameter.

- `color`: **white** has equal energy at every frequency (bright
  hiss); **pink** loses 3 dB per octave, so every octave carries the
  same energy (closer to rain or a waterfall, and the usual test
  signal for mixes); **brown** loses 6 dB per octave (a deep rumble).
  All three come out at about the same level, roughly −5 dBFS RMS
  before `gain`; pink and brown can peak above 1 at 0 dB `gain`.
- `gain`: output level in decibels (default −12). At −60 dB and below
  the output is silent.

The noise is seeded, so a project renders the same every time. Each
node draws its own sequence (from the project seed and the node's
place in the patch); the `seed` config (a whole number, default 0)
picks a different one, so two Noise nodes can be made to differ or
match exactly. Changing `color` while playing switches smoothly: all
three colors are computed from the same random values.

Generator contract: silent until `start`. Each start continues the
random sequence, so repeated bursts are all different; `reset`
re-seeds without changing whether it plays, so the sequence starts
over, as it does when the transport returns to zero. `stop` silences
it and fires `end`. Events at the same frame apply in the order
stop, start, reset.

**Try this:** wire a `Clock` into an `ADSR` (short decay) and
multiply it with white Noise through a `Biquad` bandpass for a
snare or hi-hat, or feed pink Noise into a `Meter` and a `Biquad` to
hear what a filter does across the whole spectrum.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `color` | Int | `white` | `white`, `pink`, `brown` | White, pink, or brown. |
| `gain` | Float (dB) | -12 | -60 … 12 | Output level in decibels. |
| `start` | Trigger | `play` | `play`, `load`, `never` | Starts or restarts. Unwired: on transport play, on load, or never. |
| `stop` | Trigger | `stop` | `stop`, `never` | Stops. Also on transport stop, wired or not, unless set to never. |
| `reset` | Trigger | — |  | Returns to the beginning. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The noise. |
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
| `stop` | Trigger | `stop` | `stop`, `never` | Stops. Also on transport stop, wired or not, unless set to never. |
| `reset` | Trigger | — |  | Returns to the beginning. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The waveform, -1 to 1. |
| `playing` | Bool | True while running. |
| `end` | Trigger | Fires on reaching the end or stopping. |

<a id="sample"></a>
## Sample

Kind: `sample` · Also found by: file, audio file, wav, buffer, sound, load, drum, one shot

An audio file as a sound to wire: its `buffer` output holds the
whole file, for a `Sampler`, a `Slicer`, or anything else that
plays a Buffer.

Assign the file in the side panel (the `file` config, a path
relative to the project). It's converted to the engine's sample rate
once, when the project loads, and keeps all its channels. `length`
is how long it is, in seconds. A missing file gives no sound (the
Sampler it feeds plays silence) and is flagged in the editor.

A Sample is how a sound gets into a block from outside: a pad or
keyboard block can take its sound as a Buffer input, so one block
plays any file.

**Try this:** wire Sample into a `Sampler`'s `buffer` and a pad's
trigger into the Sampler's `start`; then wire it into a `Slicer`
too, set to eight slices, and play the slices from a `Pads` grid.

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `buffer` | Buffer | The whole file. |
| `length` | Float (s) | How long the file is. |

<a id="sampler"></a>
## Sampler

Kind: `sampler` · Also found by: sample, one shot, drum, pitch, keyboard, instrument, loop, buffer

Plays a recorded sound at any pitch, with start and end points,
a sustain loop, and the generator contract: the building block for
drum machines and sampled instruments.

**Source.** A wired `buffer` (for example a `Record` node's last
take) is played; with `buffer` unwired, the `file` config (an audio
file in the project, assigned in the side panel) is played instead,
converted to the engine's rate once when the project loads. A wired
`buffer` that holds nothing yet, or a missing file, plays silence
(a missing file is flagged in the editor). A new buffer takes effect
at the next `start`, so a sound never changes under a playing note.
The output has `channels` channels (config, 1 to 64, default 2): a
mono sound goes to every channel and extra channels are dropped.

**Pitch.** `note` is a MIDI note number (fractions bend it); at the
`root` config note (default 60, middle C) the sound plays at its own
speed, and each semitone above multiplies the speed by 2^(1/12).
`rate` multiplies on top. Fractional speeds are interpolated (cubic
Hermite); a buffer recorded at another sample rate still plays at
its own pitch. `gain` is in decibels (silent at −60 and below), and
`velocity` (0 to 1, taken at each `start`) scales the note's level,
so a `Note Split`'s `velocity` wires straight in.

**Region and loop.** `start_point` and `end_point` (0 to 1, fractions
of the whole sound) mark the part that plays; if `end_point` is
before `start_point` the two swap. With `loop` on, playback runs
from `start_point` into the loop (`loop_start` to `loop_end`, also
fractions of the whole sound, kept inside the region) and repeats
it until stopped, or until `loop` turns off, when it plays on to
`end_point`. A loop shorter than one sample loops the whole region.
`crossfade` (seconds of the sound) blends the end of each pass into
the audio just before `loop_start` (equal-power), hiding the seam;
it is shortened to fit the audio available there and the loop's
length. Points are followed sample by sample, so they can be
modulated.

**Generator contract.** `start` (re)triggers from `start_point`
(from `end_point` when reversed). Reaching the end stops it and
fires `end`; `stop` silences it and fires `end`; `reset` jumps back
to the beginning without changing whether it plays. `reverse` flips
the direction (backward loops cross-fade into the audio just after
`loop_end`). Events at the same frame apply in the order reverse,
stop, start, reset. Nothing to play (an empty sound or region)
fires `end` as soon as it starts. `position` reports where playback
is, in seconds from the beginning of the sound.

**Try this:** inside a `Voices` body, wire the `Voice` node's
`trigger` into `start` and its `note` into `note` for a playable
sampled instrument; or wire a `Record` node's `buffer` here and a
`Clock` into `start` to loop a phrase you just played.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `buffer` | Buffer | — |  | Sound to play; unwired, the `file` config plays. |
| `note` | Float | 60 | 0 … 127 | MIDI note; the `root` note plays at the sound's own pitch. |
| `gain` | Float (dB) | 0 | -60 … 12 | Level in decibels. |
| `velocity` | Float | 1 | 0 … 1 | How hard the note was played (0–1), taken at each start. |
| `start_point` | Float | 0 | 0 … 1 | Where playback starts (fraction of the sound). |
| `end_point` | Float | 1 | 0 … 1 | Where playback ends (fraction of the sound). |
| `loop_start` | Float | 0 | 0 … 1 | Start of the loop (fraction of the sound). |
| `loop_end` | Float | 1 | 0 … 1 | End of the loop (fraction of the sound). |
| `crossfade` | Float (s) | 0 | 0 … 0.5 | Blend across the loop seam. |
| `start` | Trigger | `play` | `play`, `load`, `never` | Starts or restarts. Unwired: on transport play, on load, or never. |
| `stop` | Trigger | `stop` | `stop`, `never` | Stops. Also on transport stop, wired or not, unless set to never. |
| `reset` | Trigger | — |  | Returns to the beginning. |
| `loop` | Bool | false |  | Wrap around at the end. |
| `reverse` | Trigger | — |  | Toggles playback direction. |
| `rate` | Float (×) | 1 | 0 … 4 | Playback speed. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The sound. |
| `position` | Float (s) | Playback position in the sound. |
| `playing` | Bool | True while running. |
| `end` | Trigger | Fires on reaching the end or stopping. |

<a id="slicer"></a>
## Slicer

Kind: `slicer` · Also found by: slice, chop, loop, break, beat, transient, pads, drum loop, cut, sampler

Cuts a sound into slices and plays them one at a time: a drum loop
becomes a kit on a pad grid.

Wire a sound into `buffer` (a `Sample`, or a `Record` node's take).
The `mode` config says where the slices are:

- `equal` (the default): `count` slices of the same length;
- `transients`: a slice at every hit the sound has, as many as
  `sensitivity` (0 to 1) finds, up to `count`;
- `markers`: at the `markers` config, fractions of the sound from 0
  to 1, set by dragging in the Node tab's waveform.

A `trigger` plays slice `slice` (from 0) at `velocity` (0 to 1).
MIDI notes play slices too: note `low` (config, default 36) is
slice 0, the next note slice 1, and so on, at the note's velocity,
so a `Pads` grid plays the slices directly. One slice sounds at a
time: a new one cuts the last. `rate` sets the speed (and pitch) and
`gain` the level in dB. `playing` is high while a slice plays,
`end` fires when one finishes, `slice` says which played last, and
`slices` how many there are.

The slices are found again when the sound, `count`, or
`sensitivity` change; finding transients reads the whole sound, so
it costs a moment the first time.

**Try this:** wire a `Sample` of a drum loop into `buffer`, set
`count` to 8, and wire a 4 × 2 `Pads` grid's `midi` into `midi`:
eight pads, each a hit from the loop, played as hard as you hit
them. Switch `mode` to `transients` for slices that start on each
hit.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `buffer` | Buffer | — |  | The sound to slice. |
| `midi` | Midi | — |  | Notes from `low` up play the slices. |
| `trigger` | Trigger | — |  | Plays slice `slice`. |
| `slice` | Int | 0 | 0 … 63 | Which slice a trigger plays, from 0. |
| `velocity` | Float | 1 | 0 … 1 | How hard a trigger plays, 0 to 1. |
| `count` | Int | 8 | 1 … 64 | Slices (equal), or the most to find (transients). |
| `sensitivity` | Float | 0.5 | 0 … 1 | How faint a hit still starts a slice (transients). |
| `rate` | Float (×) | 1 | 0.25 … 4 | Playback speed. |
| `gain` | Float (dB) | 0 | -60 … 12 | Level in decibels. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The slice playing. |
| `slice` | Int | The slice played last, from 0. |
| `slices` | Int | How many slices there are. |
| `playing` | Bool | True while a slice plays. |
| `end` | Trigger | Fires when a slice finishes. |

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

<a id="wavetable"></a>
## Wavetable

Kind: `wavetable` · Also found by: wave table, morph, scan, oscillator, single cycle, frames, synth

An oscillator that plays single-cycle waveforms from a table and
morphs between them with `position`, as a sample-rate `Float`.

The `table` config chooses the waveforms:
- `"basic"` (default): sine, triangle, saw, and square, in that
  order, with the shapes and phase of the `Oscillator` node.
- `"buffer"`: the waveforms come from a `table` `Buffer` input
  (adds the socket), such as a `Record` node's take: channel 0 is
  cut into consecutive frames of `frame_size` samples (config,
  default 2048), each one cycle. A trailing partial frame is
  ignored, and so are frames past `max_frames` (config, default 64).

`position` (0 to 1) scans the frames, crossfading between
neighbours: with the basic table, 0 is the sine, 1/3 the triangle,
2/3 the saw, and 1 the square. `freq` + `fm` sets the pitch in hertz
(linear FM; negative totals run the waveform backwards), `phase`
offsets it by a fraction of a cycle, and `sync` resets the phase to
0 (hard sync, not band-limited).

Every frame is stored once per octave with the harmonics that would
alias at that pitch removed, and playback fades between octaves, so
sweeps stay clean and high notes lose their top harmonics smoothly.
Preparing those copies takes time, and the audio thread can't stop
to do it all at once: when a new buffer arrives, its frames are
prepared one piece at a time while playback goes on (a 2048-sample
frame every 4 ms or so at 48 kHz, a full 64-frame table in about a
quarter of a second). Meanwhile `position` scans the frames that are
ready, and the output is silent until the first one is. Memory for
`max_frames` frames is reserved up front (about 5.8 MB at the
defaults), which is why the frame count has a limit.

Generator contract: silent until `start`, which starts from phase
0. `stop` silences it and fires `end`; `reset` returns to phase 0
without changing whether it plays. Events at the same frame apply in
the order stop, start, reset, sync.

**Try this:** sweep `position` with a slow `Oscillator` (`pure`,
mapped to 0..1) for an evolving pad, or record a few seconds of
sound into a `Record` node and wire its `buffer` into a `"buffer"`
Wavetable to play it back as a scanned waveform.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `freq` | Float (Hz) | 440 | 20 … 20000 | Frequency in hertz. |
| `position` | Float | 0 | 0 … 1 | Scans the frames (0 is the first, 1 the last). |
| `phase` | Float | 0 | 0 … 1 | Phase offset in cycles. |
| `sync` | Trigger | — |  | Hard sync: resets the phase to 0. |
| `fm` | Float (Hz) | 0 | -1000 … 1000 | Linear FM: hertz added to `freq`. |
| `start` | Trigger | `play` | `play`, `load`, `never` | Starts or restarts. Unwired: on transport play, on load, or never. |
| `stop` | Trigger | `stop` | `stop`, `never` | Stops. Also on transport stop, wired or not, unless set to never. |
| `reset` | Trigger | — |  | Returns to the beginning. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The waveform. |
| `playing` | Bool | True while running. |
| `end` | Trigger | Fires on reaching the end or stopping. |

