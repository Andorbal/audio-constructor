# Level & Dynamics

[← All nodes](README.md)

<a id="crossfade"></a>
## Crossfade

Kind: `crossfade` · Also found by: blend, xfade, fader, morph, dry wet, a/b, switch, mix two

Blends two signals: `mix` 0 is all `a`, 1 is all `b`, and values in
between mix them.

- `curve` **linear** (default) adds `a·(1 − mix) + b·mix`: right for
  two versions of the same signal (dry and wet) and for control
  values, where the midpoint is the average.
- `curve` **equal_power** uses `cos`/`sin` gains (−3 dB each at the
  midpoint), so two unrelated sounds keep the same loudness all the
  way across instead of dipping in the middle.

`mix` is held within 0..1 and can be modulated at audio rate. The
output has as many channels as the wider input; a mono input is
copied to every channel. `Float` signals (an LFO, an envelope) can be
wired into `a` and `b` too, through the automatic Float → Audio
conversion, and the output wired into a `Float` parameter reads
channel 0, so Crossfade blends control signals as well as audio.

**Try this:** wire a dry signal into `a` and the same signal through
a `Delay` or `Biquad` into `b` for a wet/dry control, or drive `mix`
from a slow `Oscillator` (`pure`, mapped to 0..1) with `equal_power`
to sweep between two loops.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `a` | Audio | — |  | Heard at `mix` 0. |
| `b` | Audio | — |  | Heard at `mix` 1. |
| `mix` | Float | 0.5 | 0 … 1 | 0 is all `a`, 1 is all `b`. |
| `curve` | Int | `linear` | `linear`, `equal_power` | Linear for related signals, equal power for unrelated ones. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The blend. |

<a id="envelope-follower"></a>
## Envelope Follower

Kind: `envelope_follower` · Also found by: detector, level, envelope, sidechain, peak, rms, amplitude, follower

Measures how loud a signal is, moment to moment: `level` (linear,
0 to 1 for full scale) and `level_db` (decibels, never below −120),
both at sample rate.

- `mode` **peak** follows the signal's peaks; **rms** follows its
  power (root mean square), which tracks perceived loudness more
  closely and reads 3 dB below the peak for a sine.
- `attack` is how quickly the level rises when the signal gets
  louder, `release` how quickly it falls when it gets quieter, both
  in seconds: the time to cover about 63 % of a jump. 0 follows
  instantly.

Multichannel input is linked: the level follows the loudest
channel, so a stereo compressor built from it turns both sides down
together and the image doesn't shift.

The detector is the smooth branching peak detector from Giannoulis,
Massberg & Reiss's compressor design tutorial (JAES 2012); in rms
mode it smooths the squared signal.

**Try this:** wire `level_db` into a `Gain Computer` and its
`gain_reduction` into a `Gain` on the same signal for a compressor,
or feed a drum loop in and wire `level` through a `Map` into a
`Biquad`'s `cutoff` for an envelope-following filter.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to measure. |
| `mode` | Int | `peak` | `peak`, `rms` | Follow peaks or power (RMS). |
| `attack` | Float (s) | 0.01 | 0 … 0.5 | How fast the level rises. |
| `release` | Float (s) | 0.1 | 0 … 2 | How fast the level falls. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `level` | Float | Level, linear (1 is full scale). |
| `level_db` | Float (dB) | Level in decibels (floor −120). |

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

<a id="gain-computer"></a>
## Gain Computer

Kind: `gain_computer` · Also found by: compressor, limiter, expander, gate, dynamics, ratio, threshold, knee, sidechain

Turns a level into a gain change: the decision-making half of a
compressor, limiter, expander, or gate.

Wire a level in decibels into `level` (usually an `Envelope
Follower`'s `level_db`). Out come `gain_reduction`, the change in
decibels (0, or negative when turning down), and `gain`, the same
as a linear factor (1 for no change) to multiply a signal by.

- `mode` **compress**: levels above `threshold` are turned down so
  that every `ratio` dB over the threshold comes out as 1 dB over
  (4 means 4:1). Very high ratios (20 and up) make a limiter.
- `mode` **expand**: levels below `threshold` are turned down
  further, every 1 dB under becoming `ratio` dB under: a downward
  expander, and a gate at high ratios.
- `knee` (dB) rounds the corner at the threshold over that width so
  the change sets in gradually; 0 is a hard knee.

`ratio` is held between 1 (no change) and 1000. `gain_reduction`
stops at −120 dB, where `gain` is exactly 0. The outputs run at the
rate of the fastest input, so a sample-rate level gives a
sample-rate gain. The curve is the soft-knee gain computer from
Giannoulis, Massberg & Reiss's compressor design tutorial (JAES
2012), mirrored below the threshold for expansion.

The computer has no attack or release of its own: that's the
follower's job. Detecting a different signal from the one being
turned down makes a sidechain compressor (a kick drum ducking a
bass line).

**Try this:** chain Envelope Follower `level_db` → Gain Computer
`level`, and `gain_reduction` → a `Gain`'s `gain` on the same audio
for a compressor; add a `Lookahead` in front (follower on `early`,
Gain on `out`) with ratio 100 for a limiter.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `level` | Float (dB) | 0 | -60 … 6 | Input level in decibels (from an Envelope Follower). |
| `threshold` | Float (dB) | -20 | -60 … 0 | Where compression (or expansion) begins. |
| `ratio` | Float (×) | 4 | 1 … 20 | How strongly levels past the threshold are changed. |
| `knee` | Float (dB) | 6 | 0 … 24 | Width of the soft corner at the threshold. |
| `mode` | Int | `compress` | `compress`, `expand` | Compress above the threshold or expand below it. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `gain_reduction` | Float (dB) | Gain change in decibels (0 or negative). |
| `gain` | Float | The same change as a linear factor. |

<a id="lookahead"></a>
## Lookahead

Kind: `lookahead` · Also found by: look ahead, limiter, delay, latency, transient, brickwall, dynamics

Lets a level detector see the signal before it is heard: `out` is
the input delayed by `time`, and `early` is the input as it comes
in, `time` ahead of `out`.

Feed the detector (`Envelope Follower` → `Gain Computer`) from
`early` and the `Gain` that does the turning down from `out`. The
gain change then starts `time` before the transient that caused it
reaches the Gain, so a limiter can catch a peak completely instead
of letting its first few milliseconds through.

Lookahead reports `time` as its latency, so the engine delays other
paths to stay in step with `out`: a dry signal mixed with `out`
lines up. Both outputs come from the same node, so the engine treats
them as equally late and puts no delay between them; that is what
keeps `early` early. (It also means `early` mixed with other audio
arrives `time` ahead of it.)

`time` is a config (seconds, default 0.005, at most 0.1) because the
engine needs the latency before playing. Both outputs have as many
channels as the input; up to 64 channels are delayed, and `out` is
silent on any beyond that.

**Try this:** build a limiter: `in` → Lookahead; `early` →
Envelope Follower (`attack` about the lookahead `time`, `release`
0.1) → Gain Computer (`ratio` 100, `knee` 0, `threshold` −1) →
`gain_reduction` into a `Gain` whose `in` is Lookahead's `out`.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to look ahead into. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The input, `time` later: feed the Gain. |
| `early` | Audio | The input, `time` ahead of `out`: feed the detector. |

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

<a id="normalize"></a>
## Normalize

Kind: `normalize` · whole input (baked) · Also found by: loudness, lufs, peak, maximize, volume, level, master, whole input

Turns a whole signal up or down so its peak, or its loudness,
lands on `target`.

The `mode` config picks what is measured:
- `"peak"` (default): the highest sample over the whole signal,
  across all channels; `target` is in dBFS (default −1).
- `"lufs"`: integrated loudness as broadcast and streaming services
  measure it (ITU-R BS.1770-4: K-weighted, with the standard's −70
  LUFS and relative gates, so silence and quiet passages don't drag
  the figure down); `target` is in LUFS (default −14). Material
  quieter than −70 LUFS throughout is measured without the gates.
  This mode doesn't limit, so a loud target can push peaks past 0
  dBFS; follow it with a clipping `Shaper` or lower the target.

One gain applies to the whole signal and every channel, so the
dynamics and the stereo image are untouched. Silence stays silent,
and the gain never rises above +60 dB, so a nearly silent signal
isn't turned into a wall of noise. If `target` is wired, its value
at the start is used.

Normalize needs to hear all of its input before it knows the gain,
so it works like `Reverse`: when everything feeding it is known
ahead of time, the engine renders that part of the patch over the
project length in the background (**baking**) and Normalize plays
the result in time with the transport, baking again after any
change upstream. Fed by a live source such as Audio In it is
**disabled** and outputs silence. Offline renders always work.

**Try this:** put Normalize (`lufs`, −14) just before Audio Out so
a render comes out at streaming loudness whatever you changed
upstream, or normalize a quiet `Clip` to −1 dBFS peak before a
`Shaper` so the drive behaves the same for any file.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to normalize. |
| `target` | Float (dB) | -1 | -24 … 0 | Peak level to reach, in dBFS. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The input at the target level. |

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

<a id="shaper"></a>
## Shaper

Kind: `shaper` · Also found by: distortion, saturation, overdrive, waveshaper, clipper, fold, fuzz, tube, transfer function

Waveshaping: bends the signal through a curve for saturation,
clipping, folding, or any transfer function you draw.

`shape` picks the curve. All of them pass quiet signals unchanged
and shape only what gets near or past ±1:
- **tanh**: smooth, warm saturation.
- **hard_clip**: flat tops at ±1.
- **soft_clip**: a cubic curve that rounds into ±1 by 1.5.
- **fold**: peaks past ±1 fold back down instead of clipping, again
  and again as the drive rises (a West-coast style folder).
- **tube**: soft on positive swings, clipping at half level on
  negative ones; the asymmetry adds even harmonics.
- **custom**: the `points` config as a transfer function: `x` is the
  input (−1 to 1), `y` the output, in the breakpoint format the
  `Graph` node uses (`[[x, y], [x, y, "smooth"], ...]`). Past the
  first and last points the curve holds their values. The default
  `[[-1, -1], [1, 1]]` passes −1..1 through and clips beyond.

`drive` (dB) amplifies the input before the curve, so more drive
means more shaping; `bias` (added after the drive) shifts the signal
along the curve, making symmetric curves asymmetric. The constant
offset the bias would add to the output is subtracted again, so
silence stays silent. `mix` blends the dry signal (0) with the
shaped one (1, the default).

The curve runs at twice the sample rate, between 51-tap halfband
filters, so the new harmonics it creates fold back as aliasing far
less. That costs 25 samples of delay (about half a millisecond),
which the node reports so the engine keeps parallel paths aligned;
the dry part of `mix` is delayed to match. Up to 64 channels are
processed; the output has as many channels as the input. A `Float`
wired through the automatic Float → Audio conversion is shaped too,
which makes the custom curve a mapping for control signals.

**Try this:** put a `tanh` Shaper with 12 dB of `drive` after an
`Oscillator` sine for a warm overdrive, or `fold` a sine with `drive`
swept by an `ADSR` (through a `Map` into dB) for a metallic pluck.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to shape. |
| `shape` | Int | `tanh` | `tanh`, `hard_clip`, `soft_clip`, `fold`, `tube`, `custom` | The curve. |
| `drive` | Float (dB) | 0 | 0 … 48 | Gain into the curve. |
| `bias` | Float | 0 | -1 … 1 | Offset added before the curve. |
| `mix` | Float | 1 | 0 … 1 | 0 is dry, 1 is fully shaped. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The shaped signal. |

