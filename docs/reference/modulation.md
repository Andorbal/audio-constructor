# Modulation

[← All nodes](README.md)

Envelopes, LFOs, step sequencers, and random movement, for moving other nodes' inputs over time.

<a id="adsr"></a>
## ADSR

Kind: `adsr` · Also found by: envelope, eg, adsr, contour, attack, release

An attack-decay-sustain-release envelope: when the gate opens it
rises to 1, falls to `sustain`, and holds; when the gate closes it
falls to 0. The output is a sample-rate `Float` from 0 to 1.

Drive it with `gate` (a `Bool`: attack on the rising edge, release on
the falling edge, detected per sample) or with the `on` and `off`
triggers, or both. Retriggering starts the attack from the current
level, so it never clicks.

- `attack` is the time from 0 to 1 (sooner when retriggered from
  higher up), `decay` the time from 1 down to `sustain`, `release`
  the time from wherever the level is down to 0. A time of 0 jumps on
  the next sample.
- `curve`: `linear` ramps, or `exponential` (default) analog-style
  curves: a bowed attack and decays that drop fast and then ease out.

`active` is true (per sample) whenever the envelope isn't idle, from
the attack until the release reaches 0; `end` fires on the sample the
release reaches 0. When a gate edge and a trigger land on the same
sample, releases (gate falling, `off`) apply before attacks (gate
rising, `on`), so a note-off and the next note-on retrigger cleanly.

**Try this:** wire `MidiNote`'s gate into `gate` and the output
through a `Map` (0..1 → -60..0 dB) into a `Gain` after an Oscillator
for a playable synth voice; wire `end` into a `Clip`'s `start` to
chain a sound after the note dies away.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `gate` | Bool | false |  | Attack while true, release when it falls. |
| `on` | Trigger | — |  | Start the attack from the current level. |
| `off` | Trigger | — |  | Start the release. |
| `attack` | Float (s) | 0.01 | 0 … 10 | Time from 0 to 1. |
| `decay` | Float (s) | 0.2 | 0 … 10 | Time from 1 down to the sustain level. |
| `sustain` | Float | 0.7 | 0 … 1 | Level held while the gate is open. |
| `release` | Float (s) | 0.3 | 0 … 20 | Time from the current level down to 0. |
| `curve` | Int | `exponential` | `linear`, `exponential` | Stage shape. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The envelope, 0 to 1. |
| `active` | Bool | True from the attack until the release ends. |
| `end` | Trigger | Fires when the release reaches 0. |

<a id="envelope"></a>
## Envelope

Kind: `envelope` · Also found by: eg, ahdsr, msegs, multi-stage, breakpoint, contour, shape, loop

A multi-stage envelope drawn as breakpoints: it plays a curve of
levels over time each time it starts, with an optional sustain point
and loop.

The `points` config is the curve: a list of `[time, level]` pairs
(time in seconds from the start, level 0..1), each optionally with a
third entry shaping the segment that starts there: `"step"`,
`"smooth"`, or a bend from −1 to 1. The default is an AHDSR shape:
up to 1 in 10 ms, hold 50 ms, down to 0.6 over 200 ms, sustain, and
a 300 ms release to 0. Levels outside 0..1 are clamped, negative
times count as 0, and if the first point comes after time 0 the
envelope waits at its level until then.

Three more config entries pick points by their position in the list
(counting from 0; −1 means none, and a position past the end means
the last point):

- `sustain` (default 3): while `gate` is high, the envelope stops at
  this point and holds its level.
- `loop_start` and `loop_end` (default −1): while `gate` is high,
  reaching `loop_end` jumps back to `loop_start` and plays that
  stretch again. The loop needs `loop_start` before `loop_end`.
  Whichever of the sustain point and the loop end comes first wins.

A rising `gate` or a `trigger` starts the curve from its first point
(jumping to its level). When `gate` falls before the envelope has
passed the sustain point, it skips to the sustain point and plays the
rest of the curve from there, with the first segment after the
sustain point starting from the level it had reached, so a short
note releases smoothly from mid-attack. Without a sustain point the
gate only ends the loop, and the curve plays to its end. A `trigger`
while `gate` is low plays the whole curve straight through.

`time_scale` stretches every time: 2 plays the curve twice as
slowly, 0.5 twice as fast (limited to 0.001..1000). `out` is a
sample-rate signal; `active` is true from the start until the last
point, and `end` fires on the sample the last point is reached.
After the end, `out` holds the last point's level; before the first
start it is 0. When a gate edge and a trigger land on the same
sample, the release applies before the restart.

**Try this:** wire a `Clock` gate into `gate` and the output into a
`Gain` (through `Map` to dB) after an `Oscillator`, then set
`loop_start` 1 and `loop_end` 2 with `sustain` −1 to make the
attack-hold stretch flutter while the gate is held.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `gate` | Bool | false |  | Start on the rising edge; release when it falls. |
| `trigger` | Trigger | — |  | Start from the first point. |
| `time_scale` | Float (×) | 1 | 0.1 … 10 | Stretches every time. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The envelope level. |
| `active` | Bool | True from the start until the last point. |
| `end` | Trigger | Fires when the last point is reached. |

<a id="lfo"></a>
## LFO

Kind: `lfo` · Also found by: low frequency oscillator, modulation, wobble, vibrato, tremolo, sweep, sync

A low-frequency oscillator for modulation: a cheap, block-rate wave
to move other parameters.

`shape` picks the wave: `sine`, `triangle`, `saw` (rising), `ramp_down`
(falling), `square`, `random` (a new random value each cycle, held),
or `smooth_random` (glides from one random value to the next across
each cycle). Every shape starts its cycle at phase 0: the sine and
triangle at 0 heading up, the saw at its bottom.

The `unit` config sets what `rate` means:

- `hz` (default): cycles per second. The LFO runs freely, even with
  the transport stopped, and `retrigger` restarts the cycle at phase
  0 on its exact sample.
- `beats`: beats per cycle (1 = once per beat, 4 = once per 4/4
  bar). The phase is worked out from the song position, so the LFO
  stays locked to the beat through tempo changes, seeks, and loops,
  and holds still while the transport is stopped. Changing `rate`
  jumps to where the new rate falls on that grid. `retrigger` does
  nothing in this mode.

`phase` (0..1) shifts the wave later in its cycle. `polarity` picks
`bipolar` (−1..1) or `unipolar` (0..1, the bipolar wave moved up and
halved), and the result is multiplied by `amount`. `smooth` (0..1)
softens the random shapes with a low-pass whose time constant is
that fraction of a cycle: 0 leaves the steps sharp, 1 rounds them
into curves (and narrows their swing); other shapes ignore it.

`out` changes once per block (the value at the block's first
sample), which keeps hundreds of LFOs cheap; use an `Oscillator`
with `pure` on when you need audio-rate modulation. Random values
come from the project's seed and this node's place in the patch, so
renders repeat. NaN or infinite inputs are repaired: a rate is
limited to half the sample rate (Hz) or 1/256..100 000 beats.

**Try this:** set `unit` to `beats`, `rate` 4, and `shape`
`ramp_down`, then `Map` the output to 200..4000 into a `Biquad`
cutoff for a filter that sweeps down once every bar, in time with
the song. A `random` LFO at 8 Hz with `amount` 0.3 into an
`Oscillator`'s pitch (through `Convert` `semitones_to_ratio`) makes
a sample-and-hold warble.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `shape` | Int | `sine` | `sine`, `triangle`, `saw`, `ramp_down`, `square`, `random`, `smooth_random` | The wave. |
| `rate` | Float (Hz) | 1 | 0.01 … 20 | Cycles per second. |
| `phase` | Float | 0 | 0 … 1 | Shifts the wave within its cycle. |
| `retrigger` | Trigger | — |  | Restart the cycle (Hz mode). |
| `polarity` | Int | `bipolar` | `bipolar`, `unipolar` | −1..1 or 0..1. |
| `smooth` | Float | 0 | 0 … 1 | Softens the random shapes. |
| `amount` | Float | 1 | 0 … 1 | Scales the output. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The wave, once per block. |

<a id="random-walk"></a>
## Random Walk

Kind: `random_walk` · Also found by: drift, wander, brownian, smooth random, jitter, organic, noise

Smooth random modulation that wanders around `center`, never further
than `range` from it.

About `rate` times a second it picks a new point to head for, a
random step away from the last one (bouncing back off the edges), and
a smooth curve passes through the points, so the output drifts like
something alive rather than jumping. Over time it roams the whole
span, spending most of its time within four fifths of `range` of
`center` (the curve only reaches the edges when it swings past a
point near them). A negative `rate` counts as positive; 0 freezes
it.

The `seed` config (an integer, default 0) is mixed with the
project's seed and this node's place in the patch: two RandomWalks
with different seeds wander differently, and every render repeats
exactly. It runs freely, even with the transport stopped, and a
transport restart starts the same walk over. The output is a
sample-rate signal; NaN or infinite inputs count as 0.

**Try this:** add a RandomWalk (`rate` 0.5, `range` 3) to an
`Oscillator`'s frequency for slow analog-style drift, or wire one
(`center` 0, `range` 0.8) into a `Pan` for a sound that wanders
around the stereo field.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `rate` | Float (Hz) | 1 | 0.01 … 20 | How often it picks a new direction. |
| `range` | Float | 1 | 0 … 1 | Furthest it strays from the center. |
| `center` | Float | 0 | -1 … 1 | What it wanders around. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The wandering value. |

<a id="step-sequencer"></a>
## Step Sequencer

Kind: `step_sequencer` · Also found by: sequencer, steps, pattern, melody, arp, cv, analog sequencer

A row of step values played one after another, moving on with each
`clock` trigger.

The `steps` config sets how many steps there are (1 to 64, default
8) and `values` holds one number per step (missing values are 0,
extra ones are ignored). `length` plays only the first `length`
steps (1 up to `steps`).

`direction` sets the order: `forward`, `backward`, `pingpong` (up
and back down, without repeating the end steps), or `random` (any
active step, picked with the project's seeded generator so renders
repeat).

Before the first clock, and after a `reset`, the sequencer waits on
step 0 with `value` showing step 0's value. The next clock then plays
the first step of the pattern (step 0 forward or pingpong, the last
active step backward, a random one in `random`) rather than moving
past it; every later clock advances. When `reset` and `clock` land on
the same sample the reset comes first, so a `Clock` wired into
`reset` once a bar and into `clock` every beat restarts the pattern
cleanly on the downbeat.

`value` is a sample-rate signal that changes on the clock's exact
sample; with `glide` above 0 it slides from where it was to the new
step's value over that many seconds instead. `step` is the current
step (an `Int`, once per block, after the block's clocks), and
`step_trigger` fires on every clock. A transport restart goes back
to waiting on step 0.

**Try this:** set `values` to note numbers like 48, 55, 60, 63, wire
a `Clock` (interval 0.25) into `clock`, and `value` through `Convert`
(`note_to_hz`) into an `Oscillator`; use `step_trigger` to fire an
`ADSR` and a small `glide` for a classic sequenced bass line.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `clock` | Trigger | — |  | Advance to the next step. |
| `reset` | Trigger | — |  | Wait on step 0 again. |
| `direction` | Int | `forward` | `forward`, `backward`, `pingpong`, `random` | Step order. |
| `length` | Int | 8 | 1 … 8 | How many steps play. |
| `glide` | Float (s) | 0 | 0 … 1 | Slide time between values. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `value` | Float | The current step's value. |
| `step` | Int | The current step, from 0. |
| `step_trigger` | Trigger | Fires on each clock. |

