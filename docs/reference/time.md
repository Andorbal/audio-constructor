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

<a id="diffuser"></a>
## Diffuser

Kind: `diffuser` · Also found by: reverb, diffusion, allpass, smear, wash, space, early reflections, stereo, decorrelate, ambience

Smears a signal into a dense wash: eight modulated allpass filters
in series per channel, the building block of a reverb.

An impulse comes out as a cloud of echoes spread over tens of
milliseconds, with nothing lost: the allpasses pass every frequency
at full level, so the long-term level matches the input and the
Diffuser never rings on or builds up by itself.

- `size` (0 to 1) scales every delay: at 1 the chain spans about
  60 ms per channel, at 0 it collapses to a few samples (almost no
  effect). Changes are smoothed over about 20 ms, so moving it bends
  the pitch of the tail slightly instead of crackling.
- `diffusion` (0 to 0.9, clamped) is the allpass gain: higher values
  give a smoother, longer smear; 0 just delays.
- `mod_depth` (0 to 1) wobbles each delay by up to ±1 ms, each at
  its own phase, which breaks up metallic ringing; a little goes a
  long way. `mod_rate` (hertz) sets the wobble speed.
- `mix` blends the dry input (0) with the diffused signal (1).

The output is stereo: the left and right chains use different,
mutually prime delays, so a mono input comes out wide and the two
sides are decorrelated. A stereo input keeps its sides. Delays that
fall between samples (any `size` but 1, and all modulation) are read
through allpass interpolators, so even then nothing is lost.

**Try this:** feed a Diffuser into a feedback `Comb` (`delay` 0.03 s,
`feedback` 0.8, `damping` 0.4, `mix` 1) and blend that with the dry
signal for a quick reverb, or put one after a snare with `size` 0.3
and `mix` 0.4 to thicken it.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Mono or stereo signal to diffuse. |
| `size` | Float | 0.5 | 0 … 1 | Scales every delay; 1 spans about 60 ms. |
| `diffusion` | Float | 0.7 | 0 … 0.9 | Allpass gain: how smooth and long the smear is. |
| `mod_depth` | Float | 0.3 | 0 … 1 | Delay wobble, up to ±1 ms. |
| `mod_rate` | Float (Hz) | 0.5 | 0.05 … 5 | Wobble speed. |
| `mix` | Float | 1 | 0 … 1 | 0 is dry, 1 is fully diffused. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The diffused signal, in stereo. |

<a id="freeze"></a>
## Freeze

Kind: `freeze` · Also found by: hold, loop, stutter, sustain, infinite, capture, buffer, glitch, repeat, looper

Holds a short loop of the input while `hold` is on: catch a moment
and let it ring forever.

Freeze keeps recording its input. When `hold` goes high it takes the
last `size` seconds, ending `position` seconds before that moment,
and loops them until `hold` goes low; otherwise the input passes
straight through, untouched.

- `hold` (gate): loop while high.
- `size` (seconds, 0.01 to 2, default 0.25): the loop length. Short
  loops buzz at 1 / `size` Hz; long ones repeat a phrase.
- `position` (seconds back): how long before the hold the loop ends;
  0 loops the most recent audio, larger values reach back (up to
  `max_time` − `size`).

`size` and `position` are read at the moment `hold` rises and kept
for that loop, so the loop never shifts under you. Nothing clicks:
the loop's seam is crossfaded (equal-power, 10 ms or half the loop if
shorter, blending the loop's end into the audio just before its
start), and the switch between input and loop fades over 5 ms each
way. While holding, recording pauses so the loop stays intact; it
resumes on release. Raising `hold` again before the 5 ms fade-out
ends picks the same loop back up instead of taking a new one.

Config: `max_time`, how far back Freeze can reach in seconds
(default 4, 0.02 to 60), which sizes its buffer. The first two
channels are frozen; further channels pass through.

**Try this:** wire a `Clock`'s `gate` (interval one beat, `duty` 0.5)
into `hold` on a drum loop for rhythmic stutters, or hold a sustained
chord with `size` 0.3 and feed it into a `Diffuser` for an endless pad.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to freeze. |
| `hold` | Bool | false |  | Loop while high. |
| `size` | Float (s) | 0.25 | 0.01 … 2 | Loop length, read when hold rises. |
| `position` | Float (s) | 0 | 0 … 4 | How long before the hold the loop ends. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The input, or the loop while holding. |

<a id="pitch-shift"></a>
## Pitch Shift

Kind: `pitch_shift` · Also found by: pitch, transpose, harmonizer, octave, formant, chipmunk, detune, shifter, voice

Shifts pitch up or down without changing speed, with the formants
(the vowel color of a voice) kept in place or moved on their own.

- `semitones` (−24 to 24, clamped): the shift; 12 is an octave up.
- `formant` (semitones, −12 to 12, clamped): where the spectral
  envelope goes. 0 keeps it where the input had it, so a shifted voice
  still sounds like the same person; positive values move it up
  (smaller, brighter), negative down (bigger, darker), whatever the
  pitch does. Set it equal to `semitones` to let the formants move
  with the pitch (the classic sped-up-tape sound), which skips the
  correction entirely.
- `grain` (seconds, 0.01 up to `max_grain`): the length of the
  overlapping grains. Short grains suit speech and drums but buzz on
  low notes; long grains are smoother on sustained sounds but smear
  attacks into slight echoes.
- `mix` blends the input (0) with the shifted signal (1). The dry
  signal is delayed to line up with the shifted one, so 0.5 makes a
  clean two-voice harmony.

Pitch is shifted in the time domain: two read heads sweep through a
short delay line faster (up) or slower (down) than it fills, each
faded in and out with a raised-cosine window, and they take turns so
one is always at full level while the other jumps back. The formant
correction then works on overlapping FFT frames (about 11 ms, hop of a
quarter frame): it estimates the input's spectral envelope by
cepstral smoothing and reshapes the shifted signal's spectrum towards
the envelope moved by `formant`, by at most ±24 dB per frequency.

Limits: shifting up passes whatever the input has near the top
unfiltered, so bright material can alias slightly. The envelope
estimate assumes a pitched, harmonic-rich sound (voice, strings,
synths); a very sparse sound such as a pure sine has no real envelope,
so with `formant` at 0 it loses level when shifted far. Large shifts
of complex mixes warble, as with any real-time shifter.

Latency: the output runs half of `max_grain` plus one FFT frame
behind the input (2913 samples, about 61 ms, at 48 kHz with the
default `max_grain` of 0.1 s). The node reports it, so the engine
delays parallel paths to match; for live playing, lower `max_grain`
(0.02 gives about 21 ms).

Config: `max_grain`, the longest grain in seconds (default 0.1, 0.01
to 0.2), which sets the latency. Mono or stereo; both channels share
one set of read heads, so the stereo image holds.

**Try this:** mix a voice with itself at `semitones` 4 and 7 through
two Pitch Shifts and a `Mix` for an instant major chord, or set
`semitones` 0 and sweep `formant` from −6 to 6 to morph one voice
from giant to elf.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Mono or stereo signal to shift. |
| `semitones` | Float (st) | 12 | -24 … 24 | Pitch shift; 12 is an octave up. |
| `formant` | Float (st) | 0 | -12 … 12 | Formant shift; 0 keeps the input's. |
| `grain` | Float (s) | 0.05 | 0.01 … 0.1 | Grain length. |
| `mix` | Float | 1 | 0 … 1 | 0 is the (delayed) input, 1 only the shifted signal. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The shifted signal. |

<a id="reverse"></a>
## Reverse

Kind: `reverse` · whole input (baked) · Also found by: backwards, reversed, flip, palindrome, whole input

Plays its input backwards.

Reverse needs to hear all of its input before it can play the end
first, so it doesn't run live like other nodes. When everything
feeding it is known ahead of time (a Clip, an Oscillator, a Pattern,
a Slider), the engine renders that part of the patch over the project
length in the background (**baking**, shown on the node) and Reverse
plays the result in time with the transport. Change anything upstream
and it bakes again; the old bake plays until the new one is ready.
When a live source such as Audio In feeds it, Reverse is **disabled**
and outputs silence, since it can't know what you'll play next. An
offline render (`audioc render`, Export WAV) always works, because the
whole input is known.

The `segment` config reverses the input in slices of that many
seconds instead of all at once (0, the default, reverses the whole
project): each slice plays backwards in its own place, which keeps
the music where it was.

**Try this:** feed it a drum Clip with `segment` set to one beat's
length (0.5 s at 120 BPM) for a reversed groove that stays in time,
or put a `Delay` after it for the classic reverse-echo swell.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to reverse. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The input, backwards. |

