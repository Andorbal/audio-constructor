# Display

[← All nodes](README.md)

<a id="meter"></a>
## Meter

Kind: `meter` · Also found by: level, vu, peak, rms

Shows the peak and RMS level of a signal, and passes it through
unchanged so it can sit inline anywhere in a chain.

The node measures each block it processes and the editor draws the
readings, adding peak hold and meter ballistics from successive
blocks. Stereo signals get two bars; a mono signal shows the same
reading on both. With more than two channels only the first two are
metered, but all of them pass through.

Non-finite samples (NaN or infinity) count as silence in the readings;
`out` is still an exact copy of `in`.

**Try this:** drop a `Meter` on the wire just before `AudioOut` to
watch your headroom, or put one either side of a `Gain` to see
exactly how much it changes the level.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to measure. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The same signal, unchanged. |

<a id="readout"></a>
## Readout

Kind: `readout` · Also found by: display, value, monitor, debug

Shows the current value of whatever is wired into it, as a number.

`in` accepts a `Float`, `Int`, or `Bool` (a `Bool` reads as 0 or 1);
audio shows its first channel. The node publishes the value at the
last frame of each block, and the editor shows the latest one.

The value is shown exactly as it arrives, including NaN or infinity,
so a `Readout` is a quick way to find where a bad value starts.

**Try this:** hang a `Readout` off `Transport.beat` to watch song
position, or off a `MidiNote` `freq` to check what pitch a key
produces.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Float | 0 |  | Value to show. |

<a id="scope"></a>
## Scope

Kind: `scope` · Also found by: oscilloscope, waveform

An oscilloscope: draws the waveform of a signal over a time `window`,
and passes the signal through unchanged.

The scope shows the first channel of `in`. Wire a `Float` signal
(an envelope, an LFO) into `in` to watch it too; it arrives as mono
audio. `window` sets how much time the display spans, from 1 ms for a
single cycle up to a couple of seconds for envelopes; longer windows
keep every n-th sample to fit the display.

A waveform drawn from free-running samples drifts across the screen.
Wire `trigger` (for example from a `Clock`, or the `MidiNote` trigger
that starts each note) and the display lines up on each trigger, so a
repeating shape stands still.

**Try this:** watch an `ADSR` with `window` at 1 s and `trigger` wired
from the `MidiNote` trigger that starts it, to see the envelope's shape
on every note. Or set `window` to 5 ms and scope a saw `Oscillator`
after a `Biquad` to see the filter round off its corners.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to display (first channel). |
| `trigger` | Trigger | — |  | Lines the display up for a stable picture. |
| `window` | Float (s) | 0.02 | 0.001 … 2 | Time span shown. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The same signal, unchanged. |

