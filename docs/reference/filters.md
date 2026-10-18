# Filters

[← All nodes](README.md)

Filters for shaping tone, for synth sounds, and for building reverbs and resonators.

<a id="allpass"></a>
## Allpass

Kind: `allpass` · Also found by: schroeder, diffusion, reverb, phase, smear, dispersion

A Schroeder allpass filter: smears a signal into a train of echoes
without changing its level at any frequency.

A click comes out as a dense, decaying train of echoes, which makes
it a reverb ingredient.

- `delay` (seconds) is the spacing of the echoes. Reads between
  samples go through an allpass interpolator, which keeps the response
  flat, so it can be modulated (slow wobble breaks up metallic
  ringing); a block-rate `delay` glides across each block. It is held
  between one sample and `max_delay` (delays under 1.5 samples are
  held at 1).
- `gain` (−0.99 to 0.99, clamped) sets how much echoes feed back and
  forward: higher magnitudes give longer, denser smears. 0 is a plain
  delay.

The magnitude response is flat at any delay: what changes is only
the phase, so on its own it sounds subtle on steady tones and obvious
on clicks and drums. Several in series with unrelated delays
(1 to 20 ms, gain around 0.6) make a diffuser; see also `Diffuser`,
which does that in one node.

Config: `max_delay`, the longest delay in seconds (default 0.1, 0.001
to 2), which sizes the buffer. The first two channels are filtered;
further channels pass through dry.

**Try this:** chain four Allpasses with delays 4.7, 3.6, 12.7, and
9.3 ms and gains 0.75, 0.75, 0.625, 0.625 between a drum Clip and a
feedback `Comb` for a small, home-made reverb.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to smear. |
| `delay` | Float (s) | 0.01 | 0 … 0.1 | Echo spacing. |
| `gain` | Float | 0.5 | -0.99 … 0.99 | Echo feedback and feedforward amount. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The smeared signal, at full level. |

<a id="biquad"></a>
## Biquad

Kind: `biquad` · Also found by: lpf, hpf, bpf, filter, eq, lowpass, highpass, bandpass, notch, peak, shelf, tone

A two-pole filter: lowpass, highpass, bandpass, notch, peak, low
shelf, or high shelf. Works on any number of channels, each with its
own state.

- `cutoff` is the corner (or center) frequency in hertz.
- `q` sets the resonance (lowpass/highpass), the width
  (bandpass/notch/peak), or the shelf slope; 0.707 is flat.
- `gain` is the boost or cut in dB for peak and shelf modes; other
  modes ignore it.

Coefficients come from the Audio EQ Cookbook. They are recomputed once
per block for steady settings and every sample when `cutoff`, `q`, or
`gain` is wired to a sample-rate signal, so a filter swept by an
envelope stays smooth. The cutoff is kept below 0.49 × the sample rate
and `q` above zero, so the filter stays stable whatever is wired in.
Up to 64 channels are filtered; any beyond that pass through.

**Try this:** sweep `cutoff` with an `ADSR` through a `Map`
(0..1 → 200..5000 Hz) for a classic synth filter envelope, or chain
three Biquads (lowshelf, peak, highshelf) for a tone-control EQ.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to filter. |
| `mode` | Int | `lowpass` | `lowpass`, `highpass`, `bandpass`, `notch`, `peak`, `lowshelf`, `highshelf` | Filter response. |
| `cutoff` | Float (Hz) | 1000 | 20 … 20000 | Corner or center frequency. |
| `q` | Float | 0.707 | 0.1 … 20 | Resonance or bandwidth; 0.707 is flat. |
| `gain` | Float (dB) | 0 | -24 … 24 | Boost or cut for peak and shelf modes. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The filtered signal. |

<a id="comb"></a>
## Comb

Kind: `comb` · Also found by: comb filter, karplus strong, pluck, string, resonator, flanger, reverb, metallic, physical model

A comb filter: a copy of the input delayed by `delay`, fed back (a
resonator ringing at 1 / `delay`) or added once (evenly spaced
notches).

- `mode`: `feedback` (default) recirculates the delayed signal, so an
  impulse comes back every `delay`, scaled by `feedback` each time.
  `feedforward` adds a single delayed copy, scaled by `feedback`, and
  nothing recirculates.
- `delay` (seconds) sets the spacing: peaks (feedback) or notches
  (feedforward) every 1 / `delay` Hz. Reads are interpolated, so it
  can be swept or modulated smoothly; a block-rate `delay` glides
  across each block. It is held between one sample and `max_delay`.
- `feedback` (−0.999 to 0.999, clamped): positive values put peaks at
  multiples of 1 / `delay`; negative values put them halfway between
  and drop the pitch an octave. In feedback mode, peaks reach
  1 / (1 − |`feedback`|), so a steady tone on a peak gets loud at
  high settings.
- `damping` (0 to 1) is a one-pole lowpass inside the loop (on the
  delayed copy in feedforward mode): 0 is off, and it sweeps the
  corner from 20 kHz down to 200 Hz, so highs die away faster than
  lows, like a real string or room.
- `mix` blends the dry input (0) with what comes out of the delay
  line (1). At 1 in feedback mode you hear only the resonance; in
  feedforward mode, 0.5 with `feedback` at ±1 gives the deepest
  notches.

Config: `max_delay`, the longest delay in seconds (default 0.1, 0.001
to 2), which sizes the buffer. The first two channels are filtered;
further channels pass through dry.

**Try this:** for a plucked string (Karplus-Strong), gate a short
burst of noise or a bright Oscillator with a `Clock` into a feedback
Comb with `delay` = 1 / pitch (0.00454 s for A 220), `feedback` 0.99,
`damping` 0.3, and `mix` 1.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to filter. |
| `mode` | Int | `feedback` | `feedback`, `feedforward` | Recirculate, or add one copy. |
| `delay` | Float (s) | 0.005 | 0 … 0.1 | Delay time; the comb's spacing is 1 / delay. |
| `feedback` | Float | 0.7 | -0.999 … 0.999 | Gain of the delayed copy. |
| `damping` | Float | 0 | 0 … 1 | Lowpass in the loop: highs die away faster. |
| `mix` | Float | 0.5 | 0 … 1 | 0 is dry, 1 is only the delay line's output. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The filtered signal. |

<a id="dc-block"></a>
## DC Block

Kind: `dc_block` · Also found by: dc offset, highpass, hpf, rumble, subsonic, offset, center

Removes DC offset: a gentle one-pole highpass that pulls a signal
back to center without touching anything you can hear.

`cutoff` (hertz, default 10, kept between 1 Hz and 0.49 × the sample
rate) is the -3 dB corner; the slope below it is 6 dB/oct and DC is
removed completely, so a constant input decays to exactly 0 (with a
time constant of 1 / (2π·`cutoff`) seconds: 16 ms at 10 Hz). Raise it
towards 30..80 Hz to clear rumble as well.

Use it after anything that can push a signal off center: asymmetric
waveshaping, a `Math` add, `Unary` abs, rectified envelopes, or a long
feedback loop that slowly creeps. The filter is the trapezoidal
one-pole, prewarped so the corner is exact at any sample rate, and
safe to modulate. Any number of channels is filtered (up to 64 with
their own state; more pass through).

**Try this:** put a DC Block inside a `Feedback` loop that runs
through a `Shaper` or a drive stage, so the loop can't build up an
offset and choke.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to center. |
| `cutoff` | Float (Hz) | 10 | 1 … 200 | Corner frequency; below it is removed. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The signal without DC. |

<a id="ladder"></a>
## Ladder

Kind: `ladder` · Also found by: transistor ladder, filter, lowpass, lpf, 24db, 4 pole, resonance, self-oscillating, drive, saturation, synth filter, warm

A four-pole transistor-ladder filter with drive: warm, squelchy, and
able to self-oscillate.

- `mode` picks the response, mixed from the ladder's four stages:
  `lp24` (24 dB/oct lowpass, the classic), `lp12` (12 dB/oct
  lowpass), `bp` (bandpass, 12 dB/oct each side, 0 dB at the cutoff
  before resonance), or `hp` (24 dB/oct highpass).
- `cutoff` in hertz, kept between 1 Hz and 0.49 × the sample rate.
- `resonance` (0 to 1, clamped) feeds the output back to the input.
  The peak grows towards the top of the range and, from about 0.95,
  the filter self-oscillates: a sine at `cutoff` that keeps going with
  no input, so it can be played as an oscillator.
- `drive` (dB) boosts the input into the ladder's `tanh` input stage.
  At 0 dB a full-scale signal already rounds off a little; more drive
  adds harmonics and compresses, less (negative values) keeps the
  filter clean and quieter. The saturation keeps the output bounded
  (around ±1) however hard it is driven.

Like the circuit it models, resonance thins out the bass of the
lowpass modes (feedback divides their passband by up to 5). Half of
that loss is made up by raising the input level with resonance, so
full resonance costs about 4.5 dB of passband instead of 14 (and
drives the input stage a little harder); the bandpass and highpass
need no make-up.

The ladder is solved with zero-delay feedback, so cutoff stays in
tune and resonance stays stable when either is swept at audio rate.
Coefficients are worked out once per block for steady settings and
every sample when `cutoff` or `resonance` is a sample-rate signal.
Any number of channels is filtered (up to 64 with their own state;
more pass through).

**Try this:** set `resonance` to 1 with no input and play `cutoff`
from a `MIDI Note`'s `freq` for a pure sine lead, or push `drive` to
+12 dB on a sawtooth bass and sweep `cutoff` with an `ADSR`.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to filter. |
| `mode` | Int | `lp24` | `lp24`, `lp12`, `bp`, `hp` | Filter response. |
| `cutoff` | Float (Hz) | 1000 | 20 … 20000 | Corner frequency. |
| `resonance` | Float | 0.3 | 0 … 1 | Feedback; self-oscillates near 1. |
| `drive` | Float (dB) | 0 | -12 … 24 | Input gain into the saturating ladder. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The filtered signal. |

<a id="svf"></a>
## SVF

Kind: `svf` · Also found by: state variable, filter, lowpass, highpass, bandpass, notch, lpf, hpf, bpf, resonance, synth filter, multimode

A resonant state-variable filter with lowpass, bandpass, highpass,
and notch outputs at once: the synth filter.

- `cutoff` is the corner (or center) frequency in hertz, kept between
  1 Hz and 0.49 × the sample rate.
- `resonance` (0 to 1) raises the peak at the cutoff. It maps to a Q
  from 0.5 (no peak) to 100 evenly in decibels, so each step of the
  knob raises the peak by the same amount; about 0.07 is a flat
  (Butterworth) response, 0.5 a Q of 7, and near 1 the filter rings
  like a struck bell but never runs away. Values outside 0..1 are
  clamped.

At the cutoff the lowpass and highpass are lifted by Q (up to +40 dB
at full resonance), so a tone sitting on the cutoff gets loud.
`bandpass` is normalized to 0 dB at its center (resonance narrows it
without making it louder) and `notch` is the input minus `bandpass`.

The filter is the trapezoidal (zero-delay feedback) state-variable
design, whose state lives in its integrators, so `cutoff` and
`resonance` can be swept at audio rate, by an envelope or an
oscillator, without zipper noise or blowing up. Coefficients are
worked out once per block for steady settings and every sample when
either input is a sample-rate signal. Any number of channels is
filtered (up to 64 with their own state; more pass through).

**Try this:** wire an `ADSR` through a `Map` (0..1 → 200..4000 Hz)
into `cutoff` with `resonance` at 0.6 for a plucky synth bass, or
feed an Oscillator at 2 kHz into `cutoff` for growling audio-rate FM
of the filter.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to filter. |
| `cutoff` | Float (Hz) | 1000 | 20 … 20000 | Corner or center frequency. |
| `resonance` | Float | 0.2 | 0 … 1 | Peak at the cutoff: 0 none, 1 ringing. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `lowpass` | Audio | Frequencies below the cutoff. |
| `bandpass` | Audio | A band around the cutoff, 0 dB at its center. |
| `highpass` | Audio | Frequencies above the cutoff. |
| `notch` | Audio | Everything but a band around the cutoff. |

