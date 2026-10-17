# Level & Dynamics

[← All nodes](README.md)

<a id="gain"></a>
## Gain

Kind: `gain` · Also found by: volume, amp, level, attenuate, boost, vca

Scales a signal by a gain in decibels. Works on any number of channels.

At -60 dB and below the output is silent.

**Try this:** wire an `ADSR` through a `Map` (0..1 → -60..0 dB) into
`gain` to shape a note, or drive `gain` from a slow `Oscillator` for
tremolo.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to scale. |
| `gain` | Float (dB) | 0 | -60 … 12 | Gain in decibels. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The scaled signal. |

<a id="mix"></a>
## Mix

Kind: `mix` · Also found by: mixer, sum, bus, add, combine, merge

Sums several audio signals into one, each with its own gain in
decibels.

The number of inputs is set by the `inputs` config (1 to 64, default
4); each `in_N` has a matching `gain_N`. The output has as many
channels as the widest input, and mono inputs are copied to every
channel, so a mono and a stereo track mix to stereo. At -60 dB and
below an input is silent. Nothing limits the sum, so lower the gains
when many loud inputs add up.

**Try this:** select four nodes and add a Mix to auto-wire them, or
put a `Pan` before each input to place tracks in a stereo mix.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in_0` | Audio | — |  | A signal to add. |
| `in_1` | Audio | — |  | A signal to add. |
| `in_2` | Audio | — |  | A signal to add. |
| `in_3` | Audio | — |  | A signal to add. |
| `gain_0` | Float (dB) | 0 | -60 … 12 | Gain for the matching input. |
| `gain_1` | Float (dB) | 0 | -60 … 12 | Gain for the matching input. |
| `gain_2` | Float (dB) | 0 | -60 … 12 | Gain for the matching input. |
| `gain_3` | Float (dB) | 0 | -60 … 12 | Gain for the matching input. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The sum of all inputs. |

<a id="pan"></a>
## Pan

Kind: `pan` · Also found by: balance, stereo, panner, position, left, right

Places a mono signal in the stereo field, or shifts the balance of a
stereo one. The output is always stereo.

`pan` runs from -1 (left) through 0 (center) to 1 (right).

- **Mono in:** constant-power (sine/cosine) panning, so the sound
  keeps the same loudness as it moves. At the center each side is
  -3 dB (×0.707).
- **Stereo in:** balance. The side being panned toward stays at full
  level and the other side fades out along a quarter cosine, so at
  the center a stereo signal passes unchanged.

**Try this:** wire a slow `pure` sine Oscillator (0.2 Hz) into `pan`
for auto-pan, or put one Pan per track before a `Mix` to place a
band across the stereo field.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Mono or stereo signal. |
| `pan` | Float | 0 | -1 … 1 | -1 left, 0 center, 1 right. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | Stereo output. |

