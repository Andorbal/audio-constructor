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

