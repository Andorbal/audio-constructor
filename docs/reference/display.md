# Display

[← All nodes](README.md)

Meters, scopes, readouts, and analyzers that show what a signal is doing.

<a id="analyzer"></a>
## Analyzer

Kind: `analyzer` · Also found by: spectrum, analyser, fft, frequency, eq, rta

A spectrum analyzer: shows how loud each part of the frequency range
is, and passes the signal through unchanged so it can sit inline.

The display has 64 bands spaced evenly in pitch from 20 Hz up to half
the sample rate (about a sixth of an octave each at 48 kHz). Each
band shows its level in dB, where 0 dB is a full-scale sine inside
the band, down to a floor of −120 dB. Bands too narrow to hold a
frequency bin read the spectrum interpolated at their center, so the
curve has no gaps at the low end. Stereo signals show the average
power of the first two channels; all channels pass through.

Levels jump up at once and fall back by 20 dB every `decay` seconds,
so short peaks stay visible long enough to read; 0 follows the
signal frame by frame.

Config:
- `size`: FFT size, a power of two from 64 to 16384 (default 4096).
  Bigger sizes resolve low frequencies better and respond slower.
  Frames overlap by four with a Hann window.

For the editor: telemetry value `k` (0..64) is band `k`'s level in
dB, lowest band first, refreshed every block; `band_edges` gives the
65 band edges in Hz for a sample rate. NaN or infinite samples count
as silence in the display; `out` is still an exact copy of `in`.

**Try this:** put an Analyzer after a `Biquad` and sweep the
`cutoff` to watch the slope move, or before `AudioOut` to see where
the energy in a mix sits.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to analyze. |
| `decay` | Float (s) | 0.5 | 0 … 5 | Time for a band to fall by 20 dB. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The same signal, unchanged. |

<a id="arrangement"></a>
## Arrangement

Kind: `arrangement` · Also found by: timeline, song, lanes, playlist, arranger, tracks, clips, sequence

The song laid out in time: a lane for every Clip, MIDI Pattern,
Pattern, and Graph placed in the song.

The Arrangement holds nothing of its own. Each generator whose
`start` is *At beat…* is a block on its lane, from its `at` beat for
as long as it plays (to its `until` beat, or its own length; one
that loops with no `until` runs to the end of the song). Drag a
block to move it (its `at`, and its `until` with it), drag its right
edge to trim it (its `until`), and double-click it to play its whole
length again; moves snap to the beat (hold `Shift` for sixteenths).
The markers, the loop, and the playhead are drawn across the top.
Pin it to a panel to see the song; its Node tab shows it too.

**Try this:** set two Clips' `start` to *At beat…*, pin an
Arrangement, and drag the second clip's block to bar 5: press play
from bar 1 and hear it come in there.

<a id="light"></a>
## Light

Kind: `light` · Also found by: lamp, led, indicator, status, lit, on, playing, state

A lamp that shows a state: on when its input is, dimmed by a level.

Wire a `Bool` into `in` (a clip's `playing`, a toggle, a gate) and
the light is on while it's true; a level from 0 to 1 sets its
brightness, so a slow envelope glows. A gate shorter than the screen
refreshes still flashes: the light shows the brightest moment of each
block. The `color` config picks the lamp's color: `amber` (the
default), `green`, `red`, `blue`, `white`, or a `#RRGGBB` code.

**Try this:** wire each clip's `playing` into a Light, pin them all to
a panel, and the panel shows which clips are running; or wire a
`Pattern`'s `gate_0` into one to see the beat.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Float | 0 | 0 … 1 | On while true; a level 0–1 sets the brightness. |

<a id="midi-monitor"></a>
## MIDI Monitor

Kind: `midi_monitor` · Also found by: midi, monitor, debug, log, events, notes, inspect, sniffer

Shows the last MIDI messages on a wire, newest first: notes with
their names and velocities, controllers, pitch bend, pressure, and
program changes, each with its channel.

It passes `midi` through to `out` unchanged, so it can sit inline.
The card keeps the last eight messages and how many have passed.

**Try this:** put a MIDI Monitor between a `MIDI In` and whatever it
plays to see which notes and controllers your keyboard really sends,
or after a `Note Split`'s source to check a pad's note number.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `midi` | Midi | — |  | Messages to show. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Midi | The same messages, passed on. |

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

<a id="spectrogram"></a>
## Spectrogram

Kind: `spectrogram` · Also found by: sonogram, waterfall, spectrum, fft, frequency, time, analyser, voiceprint

A scrolling spectrogram: a picture of a sound over time, with
frequency going up, time running left to right (the newest moment
at the right edge), and loudness as brightness. It passes the signal through unchanged, so it can sit
inline.

Each column of the picture is one slice of time, read in the same
64 bands as the `Analyzer` (spaced evenly in pitch from 20 Hz to
half the sample rate, each in dB where 0 dB is a full-scale sine in
the band). The display holds 128 columns across `window` seconds;
each column shows the loudest moment of its slice, so short sounds
aren't missed when the window is long. Stereo signals show the
average power of the first two channels; all channels pass through.

Config:
- `size`: FFT size, a power of two from 64 to 16384 (default 2048).
  Bigger sizes resolve low notes better and blur fast changes more.
  Frames overlap by four with a Hann window.

For the editor: the scope ring holds 128 columns of 64 levels (dB,
lowest band first, oldest column first from the ring position), and
telemetry value 0 is the time each column covers, in seconds. NaN
or infinite samples count as silence in the picture; `out` is still
an exact copy of `in`.

**Try this:** put a Spectrogram after a `Formant` with an `LFO` on
its `vowel` to watch the resonances move, or after a `Chaos`-swept
filter on noise to see weather in the spectrum.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to picture. |
| `window` | Float (s) | 5 | 0.5 … 30 | Time the picture spans. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The same signal, unchanged. |

<a id="tuner"></a>
## Tuner

Kind: `tuner` · Also found by: tune, pitch, guitar, note, cents, intonation, strobe, frequency

A tuner: the note a sound is playing and how many cents sharp or
flat, as a needle on the card.

The first channel of `in` is analyzed with the YIN method about
twenty times a second, from 47 Hz up (the low E of a bass and every
string of a guitar). `freq` is the pitch in Hz, `note` the nearest
MIDI note, and `cents` how far from it (−50 to 50); they hold while
the sound is unclear, so the needle rests on the last note. `in`
passes through to `out` unchanged.

The `reference` input sets the A above middle C (default 440 Hz).

**Try this:** wire an `Audio In` with your guitar into a Tuner, pin
it to a panel, and tune each string until the needle stands in the
middle.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | The sound to tune (first channel). |
| `reference` | Float (Hz) | 440 | 415 … 466 | The pitch of A above middle C. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | `in`, unchanged. |
| `freq` | Float (Hz) | The pitch; holds while unclear. |
| `note` | Int | The nearest MIDI note. |
| `cents` | Float | How far from the note, −50 to 50. |

<a id="waveform"></a>
## Waveform

Kind: `waveform` · Also found by: wave, sample, buffer, display, start, end, loop points, editor

Draws a sound, with where it's playing and handles for the points
that pick which part plays.

Wire a sound into `buffer` (a `Sample`, a `Record` node's take) and
the card shows its waveform. Wire a Sampler's or a Clip's `position`
into `position` (seconds) for a playhead.

The four handles, `start_point`, `end_point`, `loop_start`, and
`loop_end` (fractions of the sound, 0 to 1), are dragged on the card
(or on a panel) and come out of the outputs of the same names: wire
them into a `Sampler`'s points and set its start and end by eye. The
end handles can't cross the start ones.

**Try this:** wire a `Sample` into Waveform and into a `Sampler`'s
`buffer`, Waveform's four outputs into the Sampler's four points,
and the Sampler's `position` back into Waveform's `position`; drag
the handles while a pad plays it.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `buffer` | Buffer | — |  | The sound to draw. |
| `position` | Float (s) | -1 | 0 … 60 | Where it's playing, for the playhead. |
| `start_point` | Float | 0 | 0 … 1 | A handle, dragged on the card (fraction of the sound). |
| `end_point` | Float | 1 | 0 … 1 | A handle, dragged on the card (fraction of the sound). |
| `loop_start` | Float | 0 | 0 … 1 | A handle, dragged on the card (fraction of the sound). |
| `loop_end` | Float | 1 | 0 … 1 | A handle, dragged on the card (fraction of the sound). |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `start_point` | Float | The handle's place, for a Sampler's point. |
| `end_point` | Float | The handle's place, for a Sampler's point. |
| `loop_start` | Float | The handle's place, for a Sampler's point. |
| `loop_end` | Float | The handle's place, for a Sampler's point. |

