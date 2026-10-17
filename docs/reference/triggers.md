# Triggers & Gates

[← All nodes](README.md)

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

