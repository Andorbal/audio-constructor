# Controls

[← All nodes](README.md)

<a id="button"></a>
## Button

Kind: `button` · Also found by: momentary, press, push, pad, bang

A momentary push button: a trigger on press, a gate while held, and
a trigger on release.

Holding the button down in the node (or on a panel) sets `pressed`.
Right-click `pressed` to bind it to a key or a MIDI note. `pressed`
can also be wired, which turns any gate into press and release
triggers. Both triggers land on the exact sample `pressed` changes.

**Try this:** wire `gate` into an `ADSR` to sound a note for as long
as the button is held, or wire `trigger` into a `Clip`'s `start` to
fire a sample.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `pressed` | Bool | false |  | True while the button is held. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `trigger` | Trigger | Fires when the button goes down. |
| `gate` | Bool | High while the button is held. |
| `release` | Trigger | Fires when the button comes up. |

<a id="dropdown"></a>
## Dropdown

Kind: `dropdown` · Also found by: menu, choice, select, list, options, mode, picker, combo

A menu of named choices; outputs the index of the one picked.

The `choices` config is the list of labels the menu shows (up to 64;
"A", "B", "C" by default). Picking one sets `value`, and `out` is
its index, from 0 for the first label. Labels that are numbers or
true/false are shown as text, other entries are skipped, extra
labels past 64 are dropped, and an empty or missing list falls back
to the default.

`changed` fires at the start of the block in which the choice
changes (not when the project loads). `value` can also be wired from
any `Int`; it is kept within the list, so `out` is always a valid
index.

**Try this:** list "sine", "triangle", "saw", "square" and wire `out`
into an `Oscillator`'s `shape` for a labeled waveform menu on a
panel, or wire `changed` into an `ADSR` to hear each switch.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `value` | Int | 0 | 0 … 2 | Index of the chosen label. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Int | Index of the chosen label, from 0. |
| `changed` | Trigger | Fires when the choice changes. |

<a id="keyboard"></a>
## Keyboard

Kind: `keyboard` · Also found by: piano, keys, notes, midi, play, virtual keyboard, synth

A piano keyboard you play with the mouse or the computer keyboard;
it sends the notes out as MIDI.

The `keys` config (1–88, default 25) sets how many keys it has and
`low` the MIDI note of the leftmost one (default 48, the C below
middle C). The keyboard is kept within MIDI's 128 notes, so a high
`low` gets fewer keys. Notes go out on MIDI channel `channel`
(1–16, default 1). Out-of-range config values are clamped.

Pressing a key sends a note-on with the `velocity` of that moment
(0–1, at least the softest MIDI velocity; NaN plays at 0.8), and
releasing it sends the matching note-off. `octave` (−3 to 3) shifts
the keys by octaves; a key keeps the note it started with, so
changing `octave` while holding a key releases the right note later.
Notes shifted outside MIDI's range don't sound. Two keys that land on
one note (after an octave change) sound it once, until both are up.
On one sample, releases go out before presses.

Each key is a hidden `key_N` input the editor holds down. Keys the
editor plays change at the start of a block; a key input wired to a
gate plays on the exact sample. Restarting the transport releases
every note; keys still held sound again from there.

**Try this:** wire `midi` into a `MIDI Note` and its `freq` and
`gate` into an `Oscillator` and an `ADSR`, pin the keyboard to a
panel, and play; or wire it into a `Voices` node for chords.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `velocity` | Float | 0.800000011920929 | 0 … 1 | How hard keys are played, 0–1. |
| `octave` | Int | 0 | -3 … 3 | Shifts the keys by octaves. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `midi` | Midi | The notes played. |

<a id="knob"></a>
## Knob

Kind: `knob` · Also found by: dial, pot, rotary, encoder, parameter, control

A round dial for a value you set by hand, with a range, a unit, and
a linear or log scale.

It works like `Slider` in a smaller space. The `min`, `max`, `unit`,
and `scale` config set the knob up: a frequency from 20 Hz to 20 kHz
on a log scale, a level in dB, a time in beats. `out` is `value`
kept within `min`..`max` (NaN reads as `min`). A log scale needs a
positive range; otherwise the knob is linear. A reversed range is
swapped and an unknown unit shows no unit. Right-click `value` to
bind it to a MIDI controller or a gamepad axis. `value` can also be
wired, which makes the knob a clamp with a readout.

**Try this:** pin a row of knobs to a panel for a synth's cutoff,
resonance, and envelope times, or set `unit` to semitones with a
range of −12 to 12 for a detune control.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `value` | Float | 0 | 0 … 1 | The knob's position. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | `value`, kept within the range. |

<a id="number"></a>
## Number

Kind: `number` · Also found by: value, field, type, entry, integer, float, parameter, constant

A number you type in: a `Float` or an `Int`, optionally kept within
a range.

The `type` config chooses `float` (the default) or `int`; `value`
and `out` both take that type. `min` and `max` (optional) bound the
number: `out` is `value` kept within them, and a missing bound
leaves that side open. A reversed range is swapped, bounds that
aren't numbers are ignored, and an `int` number's bounds round to
whole numbers. The number starts at 0, or at the nearest bound if 0
is outside the range.

`value` can be wired, which makes the node a clamp with a readout.
A float `out` follows the rate of `value`; NaN reads as the starting
value and infinities as the bounds (or the largest float), so `out`
is always finite. An int `out` changes once per block.

**Try this:** make an `int` Number from 1 to 16 and wire it into a
`Euclidean`'s `pulses`, then type new counts while it plays; or wire a
float Number into an `Oscillator`'s `freq` to tune it to the exact
hertz you type.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `value` | Float | 0 |  | The number. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | `value`, kept within the range. |

<a id="slider"></a>
## Slider

Kind: `slider` · Also found by: fader, knob, parameter, control

A value you set by hand, with a range, a unit, and a linear or log
scale.

The `min`, `max`, `unit`, and `scale` config set the slider up: a
frequency from 20 Hz to 20 kHz on a log scale, a level in dB, a time
in beats. `out` is `value` kept within `min`..`max`. A log scale
needs a positive range; otherwise the slider is linear. Right-click
`value` to bind it to a MIDI controller or a gamepad axis. `value`
can also be wired, which makes the slider a clamp with a readout.

**Try this:** make a log slider from 20 to 20 000 Hz, wire `out` into
a `Biquad`'s `cutoff`, and pin it to a panel; or wire one dB slider
into several `Gain` nodes as a group fader.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `value` | Float | 0 | 0 … 1 | The slider's position. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | `value`, kept within the range. |

<a id="toggle"></a>
## Toggle

Kind: `toggle` · Also found by: switch, on off, checkbox, latch, flip flop

An on/off switch.

Clicking the switch in the node (or on a panel) sets `value`, and
`out` follows it. `flip` turns the switch over from a wire, so a
`Button` or a `Clock` can toggle it. Whenever `value` changes, the
switch follows it again, whatever `flip` did before. Right-click
`value` to bind it to a key or a MIDI control.

**Try this:** wire a `Button`'s `trigger` into `flip` for a latching
footswitch, or multiply a signal by `out` with a `Math` node to
switch it on and off.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `value` | Bool | false |  | Sets the switch. |
| `flip` | Trigger | — |  | Turns the switch over. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Bool | The switch's state. |

<a id="xy-pad"></a>
## XY Pad

Kind: `xy_pad` · Also found by: xy, pad, touch, joystick, 2d, vector, surface, trackpad

A square pad you drag a point around on: two values at once, plus a
gate while you touch it.

Dragging the point in the node (or on a panel) sets `x` and `y`, and
holding it down sets `touched`. The `x` and `y` outputs are the
inputs kept within the ranges from the `x_min`, `x_max`, `y_min`, and
`y_max` config (0 to 1 by default; a reversed range is swapped, and
NaN reads as the low end). The point starts in the middle. `gate`
follows `touched`, and `touch` fires when the pad is pressed, on the
exact sample when `touched` is wired at audio rate and at the start
of the block otherwise. The inputs can be wired too, which turns the
pad into a two-way clamp with a picture of where the signals are.

**Try this:** wire `x` into a `Biquad`'s `cutoff` (set `x_min` 200
and `x_max` 8000) and `y` into its `q`, and `gate` into an `ADSR` on
the output level, so touching the pad plays and shapes the sound at
once.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `x` | Float | 0.5 | 0 … 1 | The point's horizontal position. |
| `y` | Float | 0.5 | 0 … 1 | The point's vertical position. |
| `touched` | Bool | false |  | True while the pad is held. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `x` | Float | `x`, kept within its range. |
| `y` | Float | `y`, kept within its range. |
| `gate` | Bool | High while the pad is held. |
| `touch` | Trigger | Fires when the pad is pressed. |

