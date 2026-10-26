# Math & Logic

[← All nodes](README.md)

Arithmetic, comparisons, unit conversions, smoothing, randomness, and routing, for numbers and gates.

<a id="accumulate"></a>
## Accumulate

Kind: `accumulate` · Also found by: add, sum, total, increment, counter, running total

Adds `in` to a running total on every `trigger`.

`out` is `initial` plus every value of `in` added since the start or
the last `reset`. `in` is read on the trigger's exact sample, and the
total changes there (the output is a sample-rate signal). Since
`initial` is added live, changing it shifts the whole total.

`reset` clears the total, so `out` returns to `initial`. When `reset`
and `trigger` land on the same sample the reset happens first, and
the trigger's value starts the new total. A transport restart clears
the total too.

NaN or infinite inputs count as 0, and the total stays finite.

**Try this:** add 0.1 on every `Clock` tick and `reset` every bar
for a pitch that climbs in steps, or add a `Random` value (−1..1) on
each tick for a melody that wanders as a random walk.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Float | 1 | -1 … 1 | Added on each trigger. |
| `initial` | Float | 0 |  | Where the total starts. |
| `trigger` | Trigger | — |  | Add `in` to the total. |
| `reset` | Trigger | — |  | Back to `initial`. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | `initial` plus the total. |

<a id="clamp"></a>
## Clamp

Kind: `clamp` · Also found by: limit, bound, constrain, range, clip, saturate

Keeps `in` between `min` and `max`.

Values below `min` come out as `min`, values above `max` come out as
`max`, and anything in between passes unchanged. If `min` is above
`max` the two are swapped, so the range always works whichever way
round it is set.

The output runs at the fastest rate among the inputs: block rate when
all three are controls, sample rate when any carries audio-rate
modulation. Audio wired into an input arrives as its first channel.
A NaN or infinite input counts as 0, so the output is always finite.

**Try this:** add an `Oscillator` to a `Slider` with `Math` and clamp
the sum to 200..5000 before a `Biquad` cutoff, so deep modulation can
never push the filter out of its useful range. With `min` −0.5 and
`max` 0.5 on an audio-rate signal it is a hard clipper.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Float | 0 |  | The value to limit. |
| `min` | Float | 0 |  | Lowest value let through. |
| `max` | Float | 1 |  | Highest value let through. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | `in`, kept within the range. |

<a id="compare"></a>
## Compare

Kind: `compare` · Also found by: greater, less, threshold, if

Compares `a` with `b` and outputs a `Bool`: true while the comparison
holds.

`hysteresis` stops the output chattering when a noisy signal hovers
around the threshold: once the output is true, it stays true until the
comparison fails by more than `hysteresis`. With `hysteresis` at 0 it
is a plain comparison.

| `op` | turns true when | stays true while |
| --- | --- | --- |
| `greater` | a > b | a > b − hysteresis |
| `greater_equal` | a ≥ b | a ≥ b − hysteresis |
| `less` | a < b | a < b + hysteresis |
| `less_equal` | a ≤ b | a ≤ b + hysteresis |
| `equal` | a = b | a is within hysteresis of b |
| `not_equal` | a ≠ b | a ≠ b (hysteresis has no effect) |

The output runs at the fastest rate of `a`, `b`, and `hysteresis`, and
the held state carries from one sample (or block) to the next. NaN or
infinite inputs count as 0, and a negative `hysteresis` counts as 0.
Restarting the transport clears the held state.

**Try this:** compare a slow `Oscillator` (or any level signal) with a
`Slider` threshold, `hysteresis` 0.05, for a clean gate to drive an
`ADSR`. Or compare `Transport.time` with a number to switch something
on partway through a song.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `a` | Float | 0 |  | The value to test. |
| `b` | Float | 0 |  | What to compare it with. |
| `op` | Int | `greater` | `greater`, `greater_equal`, `less`, `less_equal`, `equal`, `not_equal` | The comparison. |
| `hysteresis` | Float | 0 | 0 … 1 | How far past the threshold a true result must fall to turn false. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Bool | True while the comparison holds. |

<a id="constant"></a>
## Constant

Kind: `constant` · Also found by: value, number

A fixed `Float`, `Int`, `Bool`, or `Text` value, set with its inline
widget.

The `type` setting chooses the kind of value: `float` (the default),
`int`, `bool`, or `text`. The `value` input and the `out` output both
take that type. Leave `value` unwired and set it with its widget; that
widget is the control.

`out` is a block-rate signal. If you wire something into `value`, it is
passed through once per block (the value at the start of the block), so
a `Constant` also turns a fast signal into a block-rate one. A `float`
value that is NaN or infinite comes out as 0.

**Try this:** wire one `Constant` into the `freq` of several
`Oscillator`s and retune them all from one widget. A `bool` Constant
makes a project-wide switch: wire it into every `loop` input you want
to flip at once. A `text` Constant holds an OSC address that several
`OSC In` and `OSC Out` nodes share.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `value` | Float | 0 |  | The value to output. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The value, once per block. |

<a id="convert"></a>
## Convert

Kind: `convert` · Also found by: db, gain, midi, note, pitch, hz, semitones, ratio, beats, seconds, samples, units

Converts `in` from one unit to another.

| `op` | `out` |
| --- | --- |
| `db_to_gain` | decibels → linear gain, 10^(dB/20): 0 dB is 1, −6 dB about 0.5 |
| `gain_to_db` | linear gain → decibels, 20·log10(\|gain\|), never below −120 dB |
| `hz_to_note` | frequency → MIDI note number (A4 = 440 Hz = 69), with fractions |
| `note_to_hz` | MIDI note number → frequency, equal temperament |
| `semitones_to_ratio` | semitones → frequency ratio: 12 is 2, −12 is 0.5 |
| `ratio_to_semitones` | frequency ratio → semitones |
| `seconds_to_beats` | seconds → beats at the current tempo |
| `beats_to_seconds` | beats → seconds at the current tempo |
| `samples_to_seconds` | samples → seconds at the engine's sample rate |
| `seconds_to_samples` | seconds → samples at the engine's sample rate |

The beat conversions use the tempo at the song position where the
block starts, so they follow tempo changes. `db_to_gain` has no
floor: −60 dB is 0.001, not silence (the `Gain` node is the one that
treats −60 dB as off). The output socket is labeled as a gain (×),
the unit of the default op; whatever op is chosen, the number is in
that op's output unit.

The output runs at the rate of `in` and is always finite: a NaN or
infinite input counts as 0, `hz_to_note` and `ratio_to_semitones` of
0 or less give 0, and a result too large to represent is 0.

**Try this:** wire a `MidiNote` note (or a `Slider` from 36 to 84)
through `note_to_hz` into an `Oscillator` freq to play it in tune,
or put `semitones_to_ratio` between an `LFO` (amount 0.2) and a
`Math` multiply on the frequency for a vibrato measured in
semitones.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Float | 0 |  | The value to convert. |
| `op` | Int | `db_to_gain` | `db_to_gain`, `gain_to_db`, `hz_to_note`, `note_to_hz`, `semitones_to_ratio`, `ratio_to_semitones`, `seconds_to_beats`, `beats_to_seconds`, `samples_to_seconds`, `seconds_to_samples` | From which unit to which. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float (×) | The converted value, in the op's output unit. |

<a id="counter"></a>
## Counter

Kind: `counter` · Also found by: count, increment, step, index, wrap, modulo, tally

Counts triggers: each `trigger` adds `step` to `count`, wrapping
around at `max`.

With `max` above 0 the count runs from 0 to `max` − 1 and wraps (8
becomes 0 when `max` is 8), and `wrapped` fires on the sample where
it wraps. A negative `step` counts down and wraps from 0 to
`max` − 1, which also fires `wrapped`. With `max` at 0 or below the
count never wraps (it stops at the largest `Int`). Lowering `max`
below the current count takes effect at the next trigger.

`reset` sets the count back to 0. When `reset` and `trigger` land on
the same sample, the reset happens first, so the count comes out as
`step`: a `Clock` wired into both starts each bar from the top.

`count` is an `Int` that changes once per block (it shows the count
after the block's triggers); `wrapped` is sample-accurate. A
transport restart sets the count back to 0.

**Try this:** count `Clock` ticks with `max` 4 and wire `count` into
a `Select`'s `index` to step through four values; wire `wrapped`
into a second Counter to count bars.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `trigger` | Trigger | — |  | Add `step` to the count. |
| `reset` | Trigger | — |  | Back to 0 (before a trigger on the same sample). |
| `step` | Int | 1 | -16 … 16 | Added on each trigger; negative counts down. |
| `max` | Int | 0 | 0 … 64 | Wraps at this count (0 = never). |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `count` | Int | The count, once per block. |
| `wrapped` | Trigger | Fires when the count wraps. |

<a id="differentiate"></a>
## Differentiate

Kind: `differentiate` · Also found by: derivative, slope, rate of change, velocity, difference, delta

How fast `in` is changing, in units per second.

A ramp rising by 2 every second gives 2; a steady value gives 0; a
falling value gives a negative rate. At sample rate it is the
difference between this sample and the last, times the sample rate.

The output runs at the rate of `in`. When `in` changes once per
block, the rate is the change since the previous block divided by
that block's length, so it reads correctly at any block size (but
stays one block behind the change). The first value after loading or
a transport restart has nothing to compare with and gives 0.

A NaN or infinite input counts as 0, and a rate too large to
represent is 0. Steps in the input give spikes in the output: a jump
of 1 in one sample reads as 48 000 per second at 48 kHz.

**Try this:** differentiate a `Slider` and take `Unary` (`abs`) of
the result to get how fast it is being moved, then `Compare` it with
a threshold to fire on quick gestures. Differentiating an `ADSR`
gives a positive bump on each attack and a negative one on each
release.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Float | 0 |  | The changing value. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | Rate of change per second. |

<a id="integrate"></a>
## Integrate

Kind: `integrate` · Also found by: integral, accumulate, sum, ramp, leaky, area

Adds up `in` over time: the running integral, per second.

A constant `in` of 1 rises by 1 every second, so Integrate turns a
rate into a position: a speed into a distance, a constant into a
ramp. `out` is the integral up to (not including) the current
sample: it starts at 0, and integrating 1 gives exactly the seconds
since the start or the last `reset`.

- `leak` is the fraction of the stored value that drains away each
  second: 0 keeps it forever, 0.5 halves it every second, 1 forgets
  immediately. A leaky integrator settles instead of drifting.
- `reset` sets the integral back to 0 on its exact sample (the
  output reads 0 there).
- With `clamp` on, the integral stays between `min` and `max`
  (swapped if reversed), and it stops at the bound rather than
  winding up beyond it, so it leaves the bound as soon as `in` turns
  around.

The output is a sample-rate signal. NaN or infinite inputs count as
0, `leak` is limited to 0..1, and the integral stays finite. A
transport restart clears it.

**Try this:** integrate a `Slider` (−1..1) with `clamp` on and `min`
0, `max` 1 for a fader that moves at the slider's speed; wire a
`Clock` trigger into `reset` and a constant `in` of 2 for a ramp that
restarts on every beat.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Float | 0 | -1 … 1 | The rate to add up, per second. |
| `leak` | Float | 0 | 0 … 1 | Fraction of the value lost each second. |
| `reset` | Trigger | — |  | Back to 0. |
| `clamp` | Bool | false |  | Keep the value between min and max. |
| `min` | Float | 0 |  | Lower bound when clamped. |
| `max` | Float | 1 |  | Upper bound when clamped. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The integral. |

<a id="logic"></a>
## Logic

Kind: `logic` · Also found by: and, or, xor, not, nand, nor, boolean, gate

Combines two gates, `a` and `b`, with a logic operation.

| `op` | `out` is true when |
| --- | --- |
| `and` | both are true |
| `or` | either is true |
| `xor` | exactly one is true |
| `not` | `a` is false (`b` is ignored) |
| `nand` | not both are true |
| `nor` | neither is true |

A `Float` wired into `a` or `b` counts as true above 0.5. The output
runs at the fastest rate of the inputs, so two sample-rate gates
combine sample by sample.

**Try this:** `and` a `Clock` gate with a `Toggle` to mute a rhythm
without stopping the clock, or `xor` two clocks at different
intervals for a syncopated gate to drive an `ADSR`.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `a` | Bool | false |  | First gate. |
| `b` | Bool | false |  | Second gate (ignored by `not`). |
| `op` | Int | `and` | `and`, `or`, `xor`, `not`, `nand`, `nor` | The operation. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Bool | The result. |

<a id="map"></a>
## Map

Kind: `map` · Also found by: range, scale, remap, lerp

Remaps `in` from one range (`in_min`..`in_max`) to another
(`out_min`..`out_max`), along a straight line or a curve.

- `linear`: equal steps in give equal steps out.
- `exponential`: equal steps in multiply the output by equal ratios,
  out_min·(out_max/out_min)^t where t runs 0..1 across the input
  range. This is the natural curve for frequencies and times. It needs
  `out_min` and `out_max` both above 0; otherwise it acts as `linear`.
- `logarithmic`: the inverse, for an input that is spread
  exponentially: t = log(in/in_min) / log(in_max/in_min), then a
  straight line to the output. It needs `in_min` and `in_max` both
  above 0; otherwise it acts as `linear`. A `logarithmic` Map with its
  ranges swapped undoes an `exponential` one.

With `clamp` on (the default) the output stays within the output
range; with it off, inputs outside the input range extend the curve.
Either range may run backwards (`out_min` above `out_max` flips the
direction).

The output runs at the fastest rate among the inputs and is always
finite: NaN or infinite inputs count as 0, `in_min` equal to `in_max`
gives `out_min`, an unclamped `logarithmic` input at or below 0 gives
`out_min`, and a result too large to represent is 0.

**Try this:** map a 0..1 `Slider` to 20..20000 `exponential` and wire
it into a `Biquad` cutoff: the sweep feels even across the whole
range. Or map an `Oscillator` (−1..1) to 200..2000 for a filter LFO
with exactly the range you want.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Float | 0 |  | The value to remap. |
| `in_min` | Float | 0 |  | Start of the input range. |
| `in_max` | Float | 1 |  | End of the input range. |
| `out_min` | Float | 0 |  | Start of the output range. |
| `out_max` | Float | 1 |  | End of the output range. |
| `curve` | Int | `linear` | `linear`, `exponential`, `logarithmic` | The shape of the mapping. |
| `clamp` | Bool | true |  | Keep the output inside the output range. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The remapped value. |

<a id="math"></a>
## Math

Kind: `math` · Also found by: add, multiply, subtract, divide, arithmetic, plus, times

Combines two values, `a` and `b`, with an arithmetic operation.

| `op` | `out` |
| --- | --- |
| `add` | a + b |
| `subtract` | a − b |
| `multiply` | a × b |
| `divide` | a ÷ b |
| `min`, `max` | the smaller or larger of a and b |
| `power` | a raised to the power b |
| `modulo` | the remainder of a ÷ b, always from 0 up to \|b\| (Euclidean: −0.25 modulo 1 is 0.75) |
| `atan2` | the angle in radians (−π..π) of the point with x = b and y = a |

The output runs at the fastest rate of `a` and `b`: block rate when
both are controls, sample rate when either carries audio-rate
modulation. Audio wired into `a` or `b` arrives as its first channel.

The result is always finite. A NaN or infinite input counts as 0, and
anything undefined or too large is 0: dividing by 0, modulo 0, and a
negative number to a fractional power all give 0.

**Try this:** `multiply` an `Oscillator` by a `Slider` for a depth
control, then `add` a second `Slider` to set the centre, and wire the
result into a `Biquad` cutoff. `modulo` with `b` = 1 wraps a rising
ramp (such as `Transport.time`) into a repeating 0..1 sawtooth.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `a` | Float | 0 |  | First operand. |
| `b` | Float | 0 |  | Second operand. |
| `op` | Int | `add` | `add`, `subtract`, `multiply`, `divide`, `min`, `max`, `power`, `modulo`, `atan2` | The operation. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The result. |

<a id="merge"></a>
## Merge

Kind: `merge` · Also found by: combine, join, midi merge, trigger merge, together, union, funnel

Combines several MIDI or trigger streams into one, so two sources can
play one instrument at once.

The `inputs` config sets how many inputs there are (2 to 16, default
2), named `in_0`, `in_1`, …, and the `type` config what they carry:
`midi` (the default) or `trigger`. `out` has the same type.

- `trigger`: `out` fires on every sample where any input fires; two
  triggers on one sample are one.
- `midi`: every input's events in time order; on one sample, a lower
  input's events come first. A note held from several inputs at once
  (same channel and note) sounds once: the first note-on passes, and
  the note-off passes when the last input holding the note lets go.
  A note-off for a note its input isn't holding is dropped. A note-on
  for a note that only its own input holds retriggers it: a note-off,
  then the note-on.

The sustain pedal (CC 64) is merged the same way: it stays down while
any input holds it down, and a pedal message passes only when no
other input holds the pedal on its channel. An all-notes-off or
all-sound-off ends the notes its input holds on that channel, and
passes on itself only when no other input holds a note or the pedal
there, so another input's notes keep sounding. Pitch bend, pressure,
program changes, and other controllers pass as they come.

When a block brings more MIDI than an output can carry, note-ons and
other messages give way so note-offs and pedal releases always get
through. Restarting the transport releases every note and pedal still
held, at the start of the next block.

**Try this:** wire `MIDI In` into `in_0` and a `MIDI Pattern` into
`in_1`, and `out` into a `Voices` synth: play along with the pattern
on your MIDI keyboard. With `type` `trigger`, merge two `Euclidean`
rhythms into one `Sampler`'s `start` for a busier beat.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in_0` | Midi | — |  | MIDI to merge. |
| `in_1` | Midi | — |  | MIDI to merge. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Midi | Every input's MIDI in time order, each held note once. |

<a id="number-to-text"></a>
## Number to Text

Kind: `number_to_text` · Also found by: format, string, template, label, address, print, join

Writes numbers into a text template: `/fader/{}` with `value` 3
gives `/fader/3`.

`template` is the text to produce, with `{}` where each number goes.
Placeholders take the values in order (`value`, then `value_2` to
`value_4`, as many as the `args` setting gives, 1–4). A placeholder
can name a value, counting from 0 (`{1}`), and ask for a fixed
number of decimals (`{:.2}`, `{0:.1}`); plain `{}` writes whole
numbers without a decimal point and others with as many digits as
they need. `{{` and `}}` give literal braces. A template with a
stray brace, an unknown format, or a placeholder past the last
value writes nothing for that part; a value that isn't a number
writes 0. The text is cut at 128 bytes.

`out` changes once per block, from the values at the block's start,
so a moving signal doesn't produce every intermediate number.

**Try this:** wire a `Counter` into `value` with the template
`/fader/{}` and feed `out` into an `OSC In`'s `address` to step
through a surface's faders, or expose `value` as a group input so
each instance of a block listens to its own control.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `template` | Text | `{}` |  | The text to write, with {} where numbers go. |
| `value` | Float | 0 |  | The number for the first placeholder. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Text | The filled-in text, once per block. |

<a id="quantize"></a>
## Quantize

Kind: `quantize` · Also found by: round, snap, step, grid, bitcrush, staircase

Rounds `in` to the nearest step of a grid: multiples of `step`,
shifted by `offset`.

The grid is `offset + k × step` for every whole number k, and `out`
is the grid point nearest to `in`; a value exactly halfway between two
points rounds up. A negative `step` counts as positive, and a `step`
of 0 passes `in` through unchanged.

The output runs at the fastest rate among the inputs and is always
finite: a NaN or infinite input counts as 0.

**Try this:** quantize a `Slider` (or an `LFO` scaled to 48..72) with
`step` 1 and feed it through `Convert` (`note_to_hz`) into an
`Oscillator`: it now plays only semitones. On audio, a `step` of 0.1
is a crude bit-crusher.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Float | 0 |  | The value to round. |
| `step` | Float | 1 | 0 … 12 | Grid spacing (0 passes through). |
| `offset` | Float | 0 | -1 … 1 | Shifts the grid. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The nearest grid point. |

<a id="random"></a>
## Random

Kind: `random` · Also found by: noise, dice, chance, sample and hold, s&h, randomize, gaussian

A new random value on every `trigger`, held until the next.

Each trigger picks a random position within the range and holds it;
`out` is that position placed between `min` and `max`. Because the
position is what is held, moving `min` or `max` moves the output
with them (wire them from a `Slider` to open up the range live).
Before the first trigger the position is the middle of the range.

- `uniform`: every value in the range is equally likely.
- `normal`: values cluster around the middle of the range in a bell
  curve, with the range spanning ±3 standard deviations; the rare
  values beyond are clamped to the range.

The value changes on the trigger's exact sample (the output is a
sample-rate signal). The sequence of values comes from the project's
seed and this node's place in the patch, so a render repeats exactly,
and a transport restart starts the sequence over. NaN or infinite
`min`/`max` count as 0.

**Try this:** trigger it from a `Clock`, set the range to 48..72, and
wire it through `Quantize` (`step` 1) and `Convert` (`note_to_hz`)
into an `Oscillator` for a random melody that repeats on every
render.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `trigger` | Trigger | — |  | Pick a new value. |
| `distribution` | Int | `uniform` | `uniform`, `normal` | Flat, or clustered around the middle. |
| `min` | Float | 0 |  | Bottom of the range. |
| `max` | Float | 1 |  | Top of the range. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The held random value. |

<a id="route"></a>
## Route

Kind: `route` · Also found by: demux, demultiplexer, send, distribute, gate, round robin, layer, split, router, pass

Sends its input to one of several outputs, picked by `index`: the
mirror of Select. Or, in gate mode, passes it while `open` is on.

The `type` config sets what it carries: `trigger` (the default),
`midi`, `audio`, `buffer`, `float`, `bool`, or `int`. The `outputs`
config sets how many outputs there are (1 to 64, default 4), named
`out_0`, `out_1`, ….

`index` picks the output, counting from 0, and wraps around like
Select's: with four outputs, 4 is `out_0` again and −1 is `out_3`,
so a `Counter` steps through them for round robin. It's an `Int`, so
the choice changes at block boundaries. The other outputs carry
nothing: no triggers or events, silence, no sound, or 0.

- `audio`: a new `index` fades the old output out and the new one in
  over 5 ms, so it doesn't click.
- `midi`: a new `index` releases the notes (and sustain pedals) still
  held on the old output, so nothing hangs there.

**Gate mode** (`mode` config `gate`): one output, `out`, which passes
`in` while `open` is on, on its exact sample: triggers and MIDI
events that arrive while it's open, audio faded in and out over
5 ms, the sound while open at the start of a block, and values
(else 0). Closing releases held notes.

**Try this:** wire a pad's trigger into `in` and a `Counter` of the
same trigger into `index`, then `out_0` and `out_1` into two
Samplers of the same snare: round robin, so a roll never sounds
like a machine gun. Or give `index` a `Zones` node's `zone` to send
each hit to the sample for its key or velocity.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Trigger | — |  | What to send on. |
| `index` | Int | 0 | 0 … 3 | Which output gets `in` (wraps around). |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out_0` | Trigger | `in`, while `index` picks it. |
| `out_1` | Trigger | `in`, while `index` picks it. |
| `out_2` | Trigger | `in`, while `index` picks it. |
| `out_3` | Trigger | `in`, while `index` picks it. |

<a id="sample-hold"></a>
## Sample & Hold

Kind: `sample_hold` · Also found by: sample and hold, s&h, latch, freeze, snapshot, track

Samples `in` at each `trigger` and holds that value until the next.

The value is read on the trigger's exact sample, so an audio-rate
`in` is caught precisely; the output is a sample-rate signal that
only changes at triggers. Before the first trigger (and after a
transport restart) it holds 0. A NaN or infinite sample is held
as 0.

**Try this:** sample a slow `Oscillator` (or `RandomWalk`) on every
`Clock` tick and wire the result into a `Biquad` cutoff for a
stepped filter pattern, or sample `Transport.time` on a `Button`
press to remember when it was pressed.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Float | 0 |  | The signal to sample. |
| `trigger` | Trigger | — |  | Take a sample. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The last sample taken. |

<a id="select"></a>
## Select

Kind: `select` · Also found by: switch, choose, pick, router, multiplexer, mux, index

Passes on one of several inputs, picked by `index`.

The `inputs` config sets how many inputs there are (1 to 64, default
4), named `in_0`, `in_1`, …, and the `type` config sets what they
carry: `float` (the default), `audio`, `int`, `bool`, `trigger`,
`midi`, or `buffer`. `out` has the same type.

`index` picks the input, counting from 0. It wraps around: with four
inputs, 4 picks `in_0` again and −1 picks `in_3`, so a `Counter`
without a `max` steps through the inputs over and over. `index` is
an `Int`, so the choice changes at block boundaries.

- `float` and `bool`: `out` runs at the fastest rate among the
  inputs; NaN or infinite values come out as 0.
- `audio`: changing `index` crossfades from the old input to the new
  one over 5 ms so the switch doesn't click. `out` has as many
  channels as the widest input.
- `trigger`: passes on the picked input's triggers.
- `midi`: passes on the picked input's events. Changing `index`
  releases any notes (and sustain pedals) still held from the old
  input, so nothing hangs.
- `buffer`: passes on the picked sound (from a `Sample` or a
  `Record`), so one `Sampler` can play any of several.

**Try this:** set four `Constant`s to note numbers, wire them into
`in_0`..`in_3`, and drive `index` from a `Counter` counting `Clock`
ticks: a four-step arpeggio for `Convert` (`note_to_hz`) and an
`Oscillator`. With `type` `audio`, Select switches between sound
sources.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in_0` | Float | 0 |  | An input to pick from. |
| `in_1` | Float | 0 |  | An input to pick from. |
| `in_2` | Float | 0 |  | An input to pick from. |
| `in_3` | Float | 0 |  | An input to pick from. |
| `index` | Int | 0 | 0 … 3 | Which input to pass on (wraps around). |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The picked input. |

<a id="smooth"></a>
## Smooth

Kind: `smooth` · Also found by: slew, lag, glide, portamento, filter, ease, dezipper

Smooths a changing value: a slew limiter or a one-pole smoother, with
separate times for rising and falling.

- `linear`: a slew limiter. The output moves toward `in` at a fixed
  speed: at most 1 unit per `rise` seconds going up and 1 unit per
  `fall` seconds going down, so a jump of 0.5 takes half the time.
- `exponential` (default): a one-pole smoother. `rise` and `fall` are
  time constants: the output covers about 63% of the remaining
  distance in that time (99% in about 4.6 times as long), quickly at
  first and then easing in.

A time of 0 (or less) follows `in` immediately in that direction.
The first value after loading or a transport restart is taken as-is,
so the output never glides up from 0 at the start. The output is a
sample-rate signal even when `in` changes once per block, which is
what turns a stepped control into a smooth one. A NaN or infinite
input counts as 0.

**Try this:** put Smooth (`exponential`, rise and fall 0.05) between a
`MidiNote` note and `Convert` (`note_to_hz`) for portamento, or set
`rise` short and `fall` long on a level signal for a quick-attack,
slow-release envelope.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Float | 0 |  | The value to smooth. |
| `rise` | Float (s) | 0.1 | 0 … 10 | Time for rising changes. |
| `fall` | Float (s) | 0.1 | 0 … 10 | Time for falling changes. |
| `mode` | Int | `exponential` | `linear`, `exponential` | Slew or one-pole. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The smoothed value. |

<a id="switch"></a>
## Switch

Kind: `switch` · Also found by: toggle, choose, either, if, router, bypass, ab

Passes on `a` while `select` is false and `b` while it is true.

The `type` config sets what `a`, `b`, and `out` carry: `float` (the
default), `audio`, `int`, `bool`, `trigger`, `midi`, or `buffer`. `select` is
a gate read per sample, so a sample-rate gate switches on its exact
sample.

- `float` and `bool`: `out` runs at the fastest rate among the
  inputs; NaN or infinite values come out as 0.
- `audio`: flipping `select` crossfades over 5 ms so the switch
  doesn't click. `out` has as many channels as the wider input.
- `int`: reads `select` once per block.
- `trigger`: each trigger passes if it arrives from the input
  `select` picks at that sample.
- `midi`: events pass the same way, and flipping `select` releases
  any notes (and sustain pedals) still held from the other input.
- `buffer`: passes on one sound, read from `select` once per block.

**Try this:** wire a `Toggle` into `select` with `type` `audio` for a
click-free A/B comparison of a dry and an effected signal, or switch
an `Oscillator` frequency between two `Constant`s with a `Clock` gate
for a two-note trill.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `a` | Float | 0 |  | Passed on while `select` is false. |
| `b` | Float | 0 |  | Passed on while `select` is true. |
| `select` | Bool | false |  | False picks `a`, true picks `b`. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The picked input. |

<a id="text-to-number"></a>
## Text to Number

Kind: `text_to_number` · Also found by: parse, string, extract, read, pattern, address

Reads a number out of text: with the pattern `/fader/{}`, the text
`/fader/3` gives 3.

`pattern` is the text to expect, with one `{}` where the number is;
the rest must match exactly, and whatever sits in the placeholder's
place (spaces around it ignored) is read as a number. An empty
pattern reads the whole text. `value` holds the number and `ok` is
on while the text matches; when it doesn't (wrong text, nothing
numeric in the placeholder, no `{}` in a non-empty pattern), `value`
is 0 and `ok` is off, and nothing stops.

The `type` setting picks the output: `float` (the default) or
`int`, which rounds. Both change once per block.

**Try this:** wire a group's text input through this node to turn
a channel name such as `ch3` into the `index` of a `Select`, with
the pattern `ch{}` and type `int`.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `text` | Text | `` |  | The text to read. |
| `pattern` | Text | `{}` |  | The expected text, with {} at the number. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `value` | Float | The number read, or 0. |
| `ok` | Bool | On while the text matches the pattern. |

<a id="unary"></a>
## Unary

Kind: `unary` · Also found by: sin, abs, function

Applies a one-input function to `in`: trigonometry, exponentials,
rounding, and the like.

| `op` | `out` |
| --- | --- |
| `sin`, `cos`, `tan` | trigonometry, with `in` in **radians** |
| `exp` | e raised to `in` |
| `log` | natural logarithm |
| `abs` | absolute value |
| `sqrt` | square root |
| `floor`, `ceil` | round down, round up |
| `round` | nearest whole number (halves round away from zero) |
| `sign` | −1, 0, or 1 |
| `negate` | −`in` |
| `reciprocal` | 1 ÷ `in` |

The output runs at the rate of `in`: block rate for controls, sample
rate for audio-rate signals.

The result is always finite. A NaN or infinite input counts as 0.
`log` of 0 or less, `sqrt` of a negative number, and `reciprocal` of 0
give 0; `exp` limits its input to ±87 so it can't overflow. Anything
else too large to represent is 0.

**Try this:** wire `Transport.time` through a `Math` (`multiply`) by
2π·f into `Unary` (`sin`): a sine at f Hz locked to song time, so it
restarts in the same place on every play. It suits slow modulation
best, because a long song's time loses precision at audio rates. Or
put `abs` after an `Oscillator` for a full-wave rectifier.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Float | 0 |  | The input value. |
| `op` | Int | `sin` | `sin`, `cos`, `tan`, `exp`, `log`, `abs`, `sqrt`, `floor`, `ceil`, `round`, `sign`, `negate`, `reciprocal` | The function to apply. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The result. |

