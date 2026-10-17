# Time

[← All nodes](README.md)

<a id="delay"></a>
## Delay

Kind: `delay` · Also found by: echo, repeat, slapback, feedback, tape, tempo

Echoes: the input comes back after `time`, repeating with
`feedback`, blended with the dry signal by `mix` (0 dry, 1 wet).

Config:
- `unit`: `"seconds"` (default) or `"beats"`. In beats mode `time` is
  in quarter notes and follows the project tempo.
- `max_time`: the longest delay in seconds (default 4, 0.01 to 60).
  This sizes the buffer; longer times are clamped to it, in beats
  mode too.

`time` can be modulated: reads are interpolated, so sweeping it bends
the pitch of the repeats like tape instead of clicking. A block-rate
`time` (a slider, a tempo change) glides across each block. Delays
shorter than one sample are held at one sample. `feedback` is kept
within ±0.99 so the echoes always die away; negative values flip the
polarity of every other repeat.

The first two channels are delayed; further channels pass through dry.

**Try this:** set `unit` to beats with `time` 0.75 for a dotted-eighth
echo, or wire a slow `pure` Oscillator through a `Map` into `time`
(around 0.005..0.01 s) with `mix` 0.5 for a chorus.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to delay. |
| `time` | Float (s) | 0.25 | 0 … 4 | Delay time. |
| `feedback` | Float | 0.3 | 0 … 0.99 | How much of each echo feeds back into the next. |
| `mix` | Float | 0.5 | 0 … 1 | 0 is dry, 1 is only the echoes. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | Dry and delayed signal. |

