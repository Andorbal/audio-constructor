# Triggers & Gates

[← All nodes](README.md)

Clocks and rhythms, and tools that make, delay, count, filter, and reshape triggers and gates.

<a id="burst"></a>
## Burst

Kind: `burst` · Also found by: ratchet, repeat, roll, flam, stutter, retrigger, train

Fires a quick run of triggers after each trigger.

A trigger on `trigger` starts a burst of `count` (1–256) triggers on
`out`, `interval` seconds apart, the first on the incoming trigger's
own sample. Every one lands on its exact sample, even several blocks
later. `done` fires on the sample of the burst's last trigger, so
chained bursts follow on without a gap.

`count` and `interval` are taken when a burst starts and hold for
that burst. The spacing is at least one sample, so an `interval` of
0 (or a negative or NaN one) fires the burst on consecutive samples.
A new trigger during a burst starts a new burst from that sample and
the old one ends without `done`. Several triggers on one sample count
once. Restarting the transport stops the burst.

**Try this:** feed a `Clock` through a `Probability` into `trigger`
with `count` at 3 and `interval` at 0.04 for drum rolls that crop up
now and then, or wire `done` into an `ADSR` to mark the end of each
roll.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `trigger` | Trigger | — |  | Starts a burst. |
| `count` | Int | 4 | 1 … 16 | Triggers in each burst (1–256). |
| `interval` | Float (s) | 0.05 | 0 … 0.5 | Time between the burst's triggers. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Trigger | The burst's triggers. |
| `done` | Trigger | Fires with the burst's last trigger. |

<a id="clock"></a>
## Clock

Kind: `clock` · Also found by: metronome, tick, pulse, lfo

Fires a trigger every `interval`, locked to the transport.

Ticks sit on a grid counted from the start of the song: tick *k*
falls on `offset + k × interval`, measured in beats (following tempo
changes) or in seconds, as chosen by the `unit` config. Every tick
lands on its exact sample. Because the grid is worked out from the
song position rather than counted up, the clock never drifts and is
already in step after a seek, a loop, or a restart. A new `interval`
or `offset` takes effect from the next tick.

`gate` goes high at each tick for `duty` × `interval` (at 1 it stays
high). `count` is the number of ticks since the start of the song on
the current grid, so it jumps when `interval` changes. The clock is
quiet while the transport is stopped.

**Try this:** set `interval` to 0.25 for sixteenth notes and wire
`trigger` into an `ADSR` for a hi-hat pulse, or wire `gate` into an
envelope and use `duty` as the note length.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `interval` | Float (beats) | 1 | 0.0625 … 16 | Time between ticks. |
| `duty` | Float | 0.5 | 0 … 1 | Gate length as a fraction of the interval. |
| `offset` | Float (beats) | 0 | 0 … 16 | Shifts the grid later. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `trigger` | Trigger | Fires on each tick. |
| `gate` | Bool | High for `duty` of each interval. |
| `count` | Int | Ticks since the start of the song. |

<a id="clock-divide"></a>
## Clock Divide

Kind: `clock_divide` · Also found by: divider, every nth, skip, slower, half time, subdivide, count

Passes every Nth trigger.

Counting from the last `reset` (or the start), the first trigger on
`in` passes, then every `divide`th after it: with `divide` at 4,
triggers 1, 5, 9, … come out on `out`, on the same sample they came
in. `offset` shifts which ones pass by that many triggers (at 1 they
are 2, 6, 10, …); negative offsets count back. `divide` below 1
counts as 1, which passes everything.

`divide` and `offset` can change at any time: the count since the
last reset decides, so the pattern stays lined up with the reset.
`reset` sets the count back to the start; a `reset` and a trigger on
the same sample apply in that order, so the trigger counts as the
first and passes (with `offset` 0). Several triggers on one sample
count once. Restarting the transport resets the count too.

**Try this:** wire a `Clock` at sixteenth notes into `in` and set
`divide` to 4 for quarter notes; use several dividers with different
`offset`s to spread one clock across a kick, a snare, and a hi-hat.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Trigger | — |  | The triggers to divide. |
| `divide` | Int | 2 | 1 … 16 | Pass one trigger in this many. |
| `offset` | Int | 0 | 0 … 15 | Shifts which triggers pass. |
| `reset` | Trigger | — |  | Starts counting again. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Trigger | Every Nth trigger. |

<a id="clock-multiply"></a>
## Clock Multiply

Kind: `clock_multiply` · Also found by: multiplier, subdivide, ratchet, faster, double time, interpolate

Fills the gap between incoming triggers with evenly spaced ones.

Each trigger on `in` passes straight to `out`, and the time since the
one before it is split into `multiply` equal parts: with `multiply`
at 4 and triggers a beat apart, `out` fires on every sixteenth note.
The extra triggers land on exact samples (rounded to the nearest),
across block boundaries, and follow the input when it speeds up or
slows down from one trigger to the next.

The node has to hear two triggers before it knows the spacing, so
the first trigger after loading (or a transport restart) passes alone
and the extra triggers start with the second. Each new trigger starts
a new set of extra triggers spaced by the time since the last one; if
it comes early, the old set's remaining triggers are dropped. When
the input stops, `out` stops after one more interval. A new
`multiply` (1–64) takes effect at the next trigger on `in`. Triggers
that would fall on the same sample (an interval shorter than
`multiply` samples) come out once.

**Try this:** wire a `Clock` at one beat into `in` and set `multiply`
to 3 for triplets, or tap a `Button` into `in` and let the multiplier
follow your tempo.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Trigger | — |  | The triggers to multiply. |
| `multiply` | Int | 2 | 1 … 16 | Triggers out per trigger in (1–64). |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Trigger | The input and the triggers between. |

<a id="euclidean"></a>
## Euclidean

Kind: `euclidean` · Also found by: rhythm, pattern, bjorklund, sequencer, polyrhythm, drum, steps

Steps through a Euclidean rhythm: `pulses` hits spread as evenly as
possible over `steps`.

Each trigger on `clock` moves one step (the first clock plays step
0) and fires `trigger` if that step is a hit. `gate` is high from a
hit's clock until the next clock, so hits on neighbouring steps make
one long gate; use `trigger` to sound each hit. `step` is the step
played last (0 before the first clock), as of the end of the block.

The patterns come from Bjorklund's algorithm, as in Toussaint's "The
Euclidean Algorithm Generates Traditional Musical Rhythms": 3 pulses
in 8 steps give `x..x..x.`, 5 in 8 give `x.xx.xx.`, 7 in 16 give
`x..x.x.x..x.x.x.`. `rotation` moves the pattern later by that many
steps (rotation 1 turns `x..x..x.` into `.x..x..x`); negative values
move it earlier. `steps` (1–64), `pulses` (0 to `steps`), and
`rotation` are read once per block, and out-of-range values are
clamped or wrapped.

`reset` makes the next clock play step 0 again, without touching
`gate` or `step`. A `reset` and a `clock` on the same sample apply in
that order, so that clock plays step 0. Restarting the transport
resets the pattern and drops the gate.

**Try this:** drive three Euclideans from one sixteenth-note `Clock`
with 4, 3, and 7 pulses over 16 steps for a kick, a snare, and a
hi-hat, and turn `rotation` until the groove sits right.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `clock` | Trigger | — |  | Moves one step. |
| `steps` | Int | 8 | 1 … 32 | Steps in the pattern (1–64). |
| `pulses` | Int | 3 | 0 … 32 | Hits spread over the steps. |
| `rotation` | Int | 0 | 0 … 31 | Moves the pattern later by this many steps. |
| `reset` | Trigger | — |  | The next clock plays step 0. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `trigger` | Trigger | Fires on each hit. |
| `gate` | Bool | High from a hit until the next clock. |
| `step` | Int | The step played last. |

<a id="frequency-trigger"></a>
## Frequency Trigger

Kind: `frequency_trigger` · Also found by: band, detector, pitch, range, kick, sidechain, threshold

Fires when there is enough energy in a frequency band, or when a
pitch enters a range.

Config `mode` picks what it listens to:

- `"band"` (default): the first channel of `in` is filtered to the
  band from `low` to `high` Hz (4th-order Butterworth highpass and
  lowpass, 24 dB per octave outside the band) and its level is
  followed with a mean-square detector averaging over 10 ms, or one
  period of `low` if that is longer. The level reads in dB where a
  full-scale sine is 0 dB. The condition is met when the level rises
  to `threshold` and stays met until it falls below `threshold −
  hysteresis`, so a level hovering at the threshold doesn't chatter.
  `low` and `high` are read once per block, kept between 10 Hz and
  just under half the sample rate, and swapped if reversed.
- `"pitch"`: `freq` is a frequency in Hz, usually wired from
  `PitchDetect`. `condition` chooses `within` (`low` ≤ `freq` ≤
  `high`), `above` (`freq` > `high`), or `below` (`freq` < `low`).
  A `freq` of 0 or less (no pitch yet), or NaN, never meets the
  condition. Inputs are read every sample, so an audio-rate `freq`
  is followed exactly.

`trigger` fires on the sample the condition becomes true; `gate` is
high while it holds.

**Try this:** in band mode, set `low` 40 and `high` 120 Hz on a drum
loop and wire `trigger` into an `ADSR` that ducks a pad: a kick
sidechain. In pitch mode, wire `PitchDetect`'s `freq` in with
`condition` above 400 Hz to switch effects on only for high notes.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to listen to (first channel). |
| `low` | Float (Hz) | 200 | 20 … 20000 | Bottom of the band. |
| `high` | Float (Hz) | 2000 | 20 … 20000 | Top of the band. |
| `threshold` | Float (dB) | -30 | -80 … 0 | Band level that fires the trigger. |
| `hysteresis` | Float (dB) | 3 | 0 … 24 | How far the level must fall before it can fire again. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `trigger` | Trigger | Fires when the condition becomes true. |
| `gate` | Bool | High while the condition holds. |

<a id="gate-to-trigger"></a>
## Gate to Trigger

Kind: `gate_to_trigger` · Also found by: edge, rising, falling, detect, change, bool to trigger, onset

Fires a trigger where a gate turns on, turns off, or both.

`edge` chooses which changes of `in` fire: `rising` (off to on, the
default), `falling` (on to off), or `both`. The trigger lands on the
exact sample the gate changes when `in` is wired at audio rate, and
at the start of the block for slower signals. The gate counts as off
before the first block and after a transport restart, so a gate that
is already on fires a rising edge straight away.

**Try this:** wire a `Compare` into `in` to fire a trigger whenever a
signal crosses a level, or set `edge` to falling and wire a `Button`'s
`gate` in to fire on release.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Bool | false |  | The gate to watch. |
| `edge` | Int | `rising` | `rising`, `falling`, `both` | Which changes fire. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Trigger | Fires on each chosen edge. |

<a id="latch"></a>
## Latch

Kind: `latch` · Also found by: flip flop, set reset, toggle, memory, hold, sr latch, switch

Remembers an on/off state that triggers set, reset, and flip.

A trigger on `set` turns `out` on, `reset` turns it off, and
`toggle` flips it. Each change lands on the trigger's exact sample.
Triggers on the same sample apply in the order reset, set, toggle:
`set` and `reset` together leave it on, and all three together leave
it off. Several triggers on one input on the same sample count once.

`initial` is the state after loading and after a transport restart;
changing it later doesn't move the latch.

**Try this:** wire one `Button` into `set` and another into `reset`
for start and stop pads, or wire a `Threshold Trigger` into `toggle`
so each loud hit switches an effect in or out.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `set` | Trigger | — |  | Turns the latch on. |
| `reset` | Trigger | — |  | Turns the latch off. |
| `toggle` | Trigger | — |  | Flips the latch. |
| `initial` | Bool | false |  | The state after loading and after a transport restart. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Bool | The latch's state. |

<a id="onset"></a>
## Onset

Kind: `onset` · Also found by: transient, attack, beat, hit, drum, detector, spectral flux

Fires a trigger at the start of each new sound: drum hits, plucked
notes, consonants. A transient detector based on spectral flux.

The first channel of `in` is analyzed in short overlapping frames
(about 21 ms, a quarter of that apart). For each frame the node adds
up how much louder every frequency got since the previous frame, on
a logarithmic scale, so a new sound counts wherever it lands in the
spectrum while fading sounds don't. A frame whose rise is a local
peak and stands well out from the average of the last 40 ms or so is
an onset, so steady noise and held notes don't fire.
The trigger is then placed at the sample where the sound's energy
jumps, not just at the frame.

- `sensitivity` (0..1) sets how far a rise must stand out: higher
  finds softer onsets (and more false ones). 0.5 suits drums and
  plucked instruments; near 1, even busy noise fires.
- `min_interval` (seconds) is the shortest gap between triggers;
  onsets closer than that to the previous one are ignored.
- `strength` is how strong the latest onset was, 0..1, held until
  the next one: handy as a velocity.

Finding an onset needs a little of what follows it, so triggers come
out a fixed `frame / 2 + hop` samples after the onset (768 samples,
16 ms, at 44.1 or 48 kHz). The node reports that as its latency, so
the engine delays a parallel dry path by the same amount and the
triggers line up with the audio. `sensitivity` and `min_interval`
are read at each frame. NaN or infinite samples count as silence.

**Try this:** wire a drum loop into Onset and `trigger` into an
`ADSR` gating a noise burst or a sampled clap to layer a sound on
every hit; use `strength` to scale its level.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to listen to (first channel). |
| `sensitivity` | Float | 0.5 | 0 … 1 | Higher finds softer onsets. |
| `min_interval` | Float (s) | 0.05 | 0 … 1 | Shortest time between triggers. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `trigger` | Trigger | Fires at each onset. |
| `strength` | Float | How strong the latest onset was, 0..1. |

<a id="probability"></a>
## Probability

Kind: `probability` · Also found by: chance, random, maybe, coin, bernoulli, skip, humanize, gate

Lets each trigger through with a chance.

Each trigger on `in` passes to `out` with probability `chance`
(0–1): at 0.5 about half get through, at 1 all of them, at 0 none.
The ones that don't pass come out of `else` instead, on the same
sample, so the two outputs split one stream between two sounds.
`chance` is read at each trigger's sample; values below 0, or NaN,
count as 0 and values above 1 count as 1. Several triggers on one
sample count once.

The coin flips come from the project's seed, so a render sounds the
same every time, and restarting the transport replays the same
sequence. Every trigger uses up one flip whatever `chance` is, so
changing `chance` keeps the same triggers in the lead.

**Try this:** put a Probability at 0.7 between a sixteenth-note
`Clock` and a hi-hat for a part that never quite repeats, and wire
`else` into a quieter sound to fill the gaps.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Trigger | — |  | The triggers to filter. |
| `chance` | Float | 0.5 | 0 … 1 | Probability that a trigger passes, 0–1. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Trigger | The triggers that passed. |
| `else` | Trigger | The triggers that didn't. |

<a id="threshold-trigger"></a>
## Threshold Trigger

Kind: `threshold_trigger` · Also found by: level, detect, envelope, gate, schmitt, hit, transient, sidechain

Fires a trigger when a level rises above a threshold.

The level counts as *above* from the sample `in` rises past
`threshold` until it falls to `threshold − hysteresis` or below, so a
noisy level hovering around the threshold doesn't fire over and over.
With `hysteresis` at 0 it is a plain `in > threshold`.

`hold` (seconds) is how long the level must stay above before the
trigger fires: at 0 it fires on the sample the level crosses; at
0.01 it fires 10 ms later, and only if the level is still above by
then, which ignores short spikes. `gate` is high from the trigger
until the level falls back, and `release` fires on that sample. A
level that falls back before `hold` is up fires nothing at all.

Every input is read sample by sample when wired at audio rate, and
the triggers land on the exact sample. NaN or infinite inputs count
as 0, and a negative `hysteresis` or `hold` counts as 0. Restarting
the transport forgets the level, so a level that is already above
fires again.

**Try this:** wire an `Envelope Follower` on a drum loop into `in`,
set `threshold` to 0.3, and use `trigger` to start a `Clip` or an
`ADSR`, so every hit fires a sample of your own.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Float | 0 | 0 … 1 | The level to watch. |
| `threshold` | Float | 0.5 | 0 … 1 | The level that fires the trigger. |
| `hysteresis` | Float | 0.05 | 0 … 0.5 | How far below the threshold the level must fall to re-arm. |
| `hold` | Float (s) | 0 | 0 … 1 | How long the level must stay above before firing. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `trigger` | Trigger | Fires when the level has risen above. |
| `gate` | Bool | High from the trigger until the level falls back. |
| `release` | Trigger | Fires when the level falls back. |

<a id="timer"></a>
## Timer

Kind: `timer` · Also found by: stopwatch, elapsed, countdown, duration, alarm, time since, one shot

Counts the time since `start`, and fires `done` once `duration` has
passed.

A trigger on `start` sets `elapsed` (seconds) to 0 and starts it
counting, sample by sample; a `start` while running starts again
from 0. `stop` halts it, and `elapsed` holds where it stopped.
`running` is high while it counts.

When `elapsed` reaches `duration`, `done` fires on that exact sample
and the timer stops by itself, with `elapsed` holding at `duration`,
so `running` is a gate exactly `duration` long. `progress` goes from
0 to 1 over the duration. With `duration` at 0 (or negative or NaN)
the timer is a plain stopwatch: it runs until stopped, `done` never
fires, and `progress` stays 0. `duration` is read every sample, so
shortening it below the time already counted ends the run at once.

On one sample, reaching `duration` comes first (so `done` still
fires when a `start` lands on the same sample), then `stop`, then
`start`, so a `start` always wins. The timer counts whether or not
the transport is playing; restarting the transport stops it and sets
`elapsed` back to 0.

**Try this:** wire a `Button` into `start` and `running` into an
`ADSR`'s `gate` for a note that lasts exactly `duration` however you
press, or wire `progress` through a `Map` into a `Biquad`'s `cutoff`
for a sweep that takes as long as you like.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `start` | Trigger | — |  | Starts counting from 0. |
| `stop` | Trigger | — |  | Stops counting. |
| `duration` | Float (s) | 1 | 0 … 10 | When `done` fires; 0 runs until stopped. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `elapsed` | Float (s) | Time since the start. |
| `running` | Bool | High while counting. |
| `done` | Trigger | Fires when `duration` has passed. |
| `progress` | Float | `elapsed` over `duration`, 0–1. |

<a id="trigger-delay"></a>
## Trigger Delay

Kind: `trigger_delay` · Also found by: delay, late, offset, shift, echo, swing, nudge

Delays each trigger by a time in seconds or beats.

Every trigger on `in` comes out of `out` `time` later, on its exact
sample, even when that falls several blocks later. The `unit` config
chooses seconds (the default) or beats, which follow the tempo at
the moment the trigger arrives. Each trigger keeps the delay it
arrived with: changing `time` moves only triggers still to come, so
a shorter time can let a later trigger overtake an earlier one. A
`time` of 0 passes triggers through on the same sample; negative or
NaN times count as 0.

Up to 4096 triggers can be waiting at once; triggers arriving while
the queue is full are dropped. Triggers that come due on the same
sample come out once. Restarting the transport drops every waiting
trigger.

**Try this:** wire a `Clock` into a `Trigger Delay` with `unit` set
to beats and `time` at 0.75, and send both the clock and the delayed
trigger into envelopes for a dotted-eighth echo rhythm.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Trigger | — |  | The triggers to delay. |
| `time` | Float (s) | 0.25 | 0 … 2 | How long each trigger waits. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Trigger | The delayed triggers. |

<a id="trigger-to-gate"></a>
## Trigger to Gate

Kind: `trigger_to_gate` · Also found by: gate length, pulse, hold, one shot, monostable, note length

Turns each trigger into a gate of a set length.

`gate` goes high on the sample a trigger arrives on `in` and stays
high for `length`, in seconds or, with the `unit` config set to
beats, in beats at the tempo of the moment the trigger arrives. The
gate is always at least one sample long, so a `length` of 0 (or a
negative or NaN one) still shows every trigger.

A trigger that arrives while the gate is high starts the length
again from that sample, so the gate stays high without dropping in
between: retriggering stretches the gate, it doesn't make a new one.
Wire the trigger itself to anything that should restart on every
hit. Restarting the transport drops the gate.

**Try this:** wire a `Clock` into `in` and `gate` into an `ADSR` to
turn clock ticks into notes of a fixed length, or set `unit` to beats
and `length` to 0.5 for eighth-note gates at any tempo.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Trigger | — |  | Starts the gate. |
| `length` | Float (s) | 0.1 | 0 … 2 | How long the gate stays high. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `gate` | Bool | High for `length` after each trigger. |

