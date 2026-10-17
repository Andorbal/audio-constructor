# Math & Logic

[← All nodes](README.md)

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

A fixed `Float`, `Int`, or `Bool` value, set with its inline widget.

The `type` setting chooses the kind of value: `float` (the default),
`int`, or `bool`. The `value` input and the `out` output both take that
type. Leave `value` unwired and set it with its widget; that widget is
the control.

`out` is a block-rate signal. If you wire something into `value`, it is
passed through once per block (the value at the start of the block), so
a `Constant` also turns a fast signal into a block-rate one. A `float`
value that is NaN or infinite comes out as 0.

**Try this:** wire one `Constant` into the `freq` of several
`Oscillator`s and retune them all from one widget. A `bool` Constant
makes a project-wide switch: wire it into every `loop` input you want
to flip at once.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `value` | Float | 0 |  | The value to output. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The value, once per block. |

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

