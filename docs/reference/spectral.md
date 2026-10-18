# Spectral

[← All nodes](README.md)

<a id="band-split"></a>
## Band Split

Kind: `band_split` · Also found by: crossover, multiband, bands, linkwitz, split, frequency

Splits a signal into frequency bands, one output per band, that add
back up to the original.

Config:
- `edges`: the band edges in Hz, a list such as `[200, 2000]`
  (the default: three bands, below 200 Hz, 200 Hz to 2 kHz, and
  above). Edges are sorted and kept within 20 Hz to 20 kHz; at most
  15 are used (16 bands), and entries that aren't numbers are
  ignored. An empty list gives one band. Outputs are `band_0` (the
  lowest) up to `band_N`. At low sample rates, edges near or above
  half the sample rate leave the bands above them (almost) empty.
- `mode`: how to split.
  - `"crossover"` (default): Linkwitz-Riley 4th-order crossovers
    (24 dB per octave, −6 dB at each edge) arranged in a tree, with
    allpass filters keeping every band in phase with the others. The
    bands sum to the input with a flat level and a smoothly shifted
    phase, with no latency. The way to build multiband dynamics.
  - `"fft"`: brick-wall bands from a short-time Fourier transform
    (Hann window, overlap 4): each frequency bin goes to exactly one
    band, so the bands sum back to the input exactly, delayed by
    `size − 1` samples, which the node reports as latency. Sharper
    than crossovers, but sharp edges ring around transients.
- `size`: FFT size for `"fft"` mode, a power of two from 64 to 16384
  (default 2048).

Every band has the input's channel count. The first two channels are
split; any further channels go to `band_0` unchanged (not delayed in
`"fft"` mode) and are silent in the other bands.

**Try this:** split a drum loop at `[150, 2500]`, run each band
through its own `Gain` (and later a compressor block), and `Mix`
them back together: a multiband processor. Mute the middle band to
hear just the kick and the cymbals.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to split. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `band_0` | Audio | One frequency band of the input (band_0 is the lowest). |
| `band_1` | Audio | One frequency band of the input (band_0 is the lowest). |
| `band_2` | Audio | One frequency band of the input (band_0 is the lowest). |

<a id="fft"></a>
## FFT

Kind: `fft` · Also found by: spectrum, fourier, stft, frequency, analysis, bins

Turns audio into a stream of spectra: a short-time Fourier transform
of `in`, one frame every hop.

Every hop (`size / overlap` samples) the newest `size` samples of the
first channel of `in` are multiplied by the window and transformed.
Each frame is stamped with the sample at which its last sample
arrived, so spectral processing stays sample-accurate at any block
size. A frame holds `size / 2 + 1` complex bins from 0 Hz up to half
the sample rate, `sample_rate / size` Hz apart. Values are unscaled:
a full-scale sine centered on a bin reads `size / 4` with the Hann
window.

Config:
- `size`: samples per frame, a power of two from 64 to 16384
  (default 2048). Other values go to the nearest power of two in
  that range. Larger sizes resolve frequency more finely but blur
  timing.
- `overlap`: frames per `size` samples, 1, 2, 4, or 8 (default 4);
  other values go to the nearest of those. More overlap is smoother
  after processing and costs more.
- `window`: `"hann"` (default), `"hamming"`, `"blackman"`, or
  `"rectangular"`; unknown names use Hann.

The node reports `size − 1` samples of latency: `IFFT` turns each
frame back into audio starting at the frame's stamp, so FFT → IFFT
gives back the input exactly `size − 1` samples later, and the engine
delays parallel dry paths by the same amount so they line up. Before
the first `size` samples the missing history counts as silence; the
first frame arrives after one hop. Only the first channel is
analyzed (use `SplitChannels` and one FFT per channel for more), and
NaN or infinite samples count as silence.

**Try this:** wire FFT → `SpectralGate` → `IFFT` to strip hiss from a
recording, or FFT → `SpectralShift` (`scale` 2) → `IFFT` and mix it
quietly under the dry signal for an octave shimmer.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to analyze (first channel). |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `spectrum` | Spectrum | One frame of complex bins every hop. |

<a id="ifft"></a>
## IFFT

Kind: `ifft` · Also found by: inverse, resynthesis, overlap-add, spectrum, fourier, istft

Turns a stream of spectra back into audio by overlap-adding inverse
transforms.

The frame size and hop come from the incoming frames, so IFFT works
behind any `FFT`. Each frame is transformed back, multiplied by the
window, and added into the output starting at the sample the frame
is stamped with. The windows' overlap is normalized position by
position (least-squares resynthesis), so an unmodified FFT → IFFT
round trip is exact, within rounding, for every window at overlap 2
or more. At overlap 1 only the rectangular window reconstructs;
tapered windows dip toward their frame edges.

Config:
- `window`: the window the frames were analyzed with, `"hann"`
  (default), `"hamming"`, `"blackman"`, or `"rectangular"`. Set it to
  match the `FFT`, or the level will ripple at the hop rate.

IFFT adds no latency of its own: the `FFT` reports the round trip's
`size − 1` samples. With nothing wired the output is silent. When the
incoming frame size or hop changes, pending output is dropped and
resynthesis starts over. Frame sizes other than powers of two from
16 to 16384 give silence. NaN or infinite bins count as zero, and
the output is mono.

**Try this:** wire `FFT` → IFFT with nothing in between and mix the
result against the dry signal with one side inverted: the engine
lines the paths up, so they cancel to silence. Then put a
`SpectralGate` in the middle and listen to what it takes away.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `spectrum` | Spectrum | — |  | Frames to turn back into audio. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The resynthesized signal. |

<a id="pitch-detect"></a>
## Pitch Detect

Kind: `pitch_detect` · Also found by: pitch, tracker, tuner, frequency, yin, f0, guitar

Follows the pitch of a monophonic sound: frequency, nearest note,
how sure it is, and a trigger when a new note starts.

Every `hop` samples the newest `window` samples of the first channel
of `in` are analyzed with the YIN method, which compares the newest
half of the window with delayed copies of itself to find its period.

- `freq` (Hz) is the detected fundamental. It holds its last value
  while the sound is unclear (confidence below `threshold`) and is 0
  until the first pitch is found.
- `confidence` (0..1) is how periodic the latest analysis was: near
  1 for a clean tone, near 0 for noise or silence.
- `note` is the MIDI note nearest to `freq` (0 before any pitch).
- `onset` fires when a new stable pitch starts: two analyses in a row
  agree within half a semitone, after a pause or more than three
  quarters of a semitone away from the last note it fired for.

`min_freq` and `max_freq` limit the search (a narrow range avoids
octave mistakes); the lowest pitch a window can measure is about
`2 × sample_rate / window` (47 Hz with the defaults at 48 kHz), and
`max_freq` is kept below a quarter of the sample rate. If they are
reversed they are swapped. `threshold` is the confidence a reading
needs to count; lower it for breathy or distorted sources.

Config:
- `window`: samples per analysis, a power of two from 256 to 16384
  (default 2048). Bigger windows reach lower notes and react slower.
- `hop`: samples between analyses, 16 up to `window` (default 256).

The outputs update once per block from the latest analysis. A note
after a pause is recognized once a few of its periods have arrived
(sooner for higher notes); a change straight from one note to
another takes about half a window, until the old note has left the
analysis; `onset` then needs one more hop to confirm. Readings
describe the newest half of the window, centered a quarter window
back, so the node reports `window / 4 + hop` samples of latency (768
with the defaults) and the engine delays a dry path by that much to
keep `onset` close to the sound. NaN or infinite samples count as
silence.

**Try this:** play a guitar or a voice into PitchDetect, wire `freq`
to an `Oscillator` and `onset` to an `ADSR` on its gain: a synth that
follows you. Use `confidence` through a `Compare` to mute it when
nothing clear is playing.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Monophonic sound to follow (first channel). |
| `min_freq` | Float (Hz) | 60 | 20 … 1000 | Lowest pitch to look for. |
| `max_freq` | Float (Hz) | 1500 | 100 … 5000 | Highest pitch to look for. |
| `threshold` | Float | 0.8 | 0 … 1 | Confidence a reading needs to count. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `freq` | Float (Hz) | Detected pitch; holds while unclear. |
| `confidence` | Float | How clear the latest reading was, 0..1. |
| `note` | Int | Nearest MIDI note. |
| `onset` | Trigger | Fires when a new stable pitch starts. |

<a id="spectral-gate"></a>
## Spectral Gate

Kind: `spectral_gate` · Also found by: denoise, noise reduction, hiss, gate, spectrum, cleanup

Silences the bins of a spectrum that fall below a threshold, leaving
the louder ones: a noise gate per frequency.

- `threshold` is an absolute level in dB for each bin, where 0 dB is
  a full-scale sine analyzed with the Hann window (with Hamming it
  reads 0.7 dB higher, Blackman 1.5 dB lower, rectangular 6 dB
  higher). Bins at or above it pass; quieter bins are turned down.
- `floor` is the gain, in dB, for bins below the threshold: −120 or
  less removes them entirely; −20 just tucks the noise under.
- `smoothing` is how long, in seconds, each bin's gain takes to move
  between open and closed (a one-pole glide, the same both ways).
  Without it, bins hovering near the threshold flicker on and off
  from frame to frame and the leftover noise turns into watery
  "musical noise"; 0 switches at once.

Inputs are read at each frame's own sample, so a wired `threshold`
changes the gate at the right moment. The output has the input's
frame size and hop, and phases pass through untouched. The gate opens
or closes fully on the first frame after a reset or a change of frame
size. NaN or infinite bins come out as zero.

**Try this:** put it between `FFT` and `IFFT` on a hissy recording,
start with `threshold` at −60 dB and raise it until the hiss goes,
then raise `smoothing` until what remains stops warbling.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `spectrum` | Spectrum | — |  | Frames to gate. |
| `threshold` | Float (dB) | -50 | -120 … 0 | Bins quieter than this are turned down. |
| `smoothing` | Float (s) | 0.05 | 0 … 1 | Time each bin's gain takes to open or close. |
| `floor` | Float (dB) | -80 | -120 … 0 | Gain for gated bins; -120 removes them. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `spectrum` | Spectrum | The gated frames. |

<a id="spectral-shift"></a>
## Spectral Shift

Kind: `spectral_shift` · Also found by: frequency shift, pitch shift, transpose, inharmonic, phase vocoder

Moves every partial of a spectrum to a new frequency:
`frequency × scale + shift`.

- `scale` multiplies frequencies, keeping harmonics in tune with each
  other: 2 is an octave up, 0.5 an octave down, 1.5 a fifth up.
  Values are kept between 1/16 and 16.
- `shift` then adds a fixed number of hertz to every partial, which
  pulls harmonics out of tune with each other: small shifts sound
  metallic and bell-like, larger ones robotic.

It works as a peak-locked phase vocoder: each spectral peak's exact
frequency is worked out from how far its phase moved since the
previous frame, the peak and the bins around it move together to the
new frequency (keeping the partial's shape and level, to within
about 0.3 dB), and the output phase advances at the new frequency,
so moved partials stay smooth across overlapping frames. Use an
`FFT` overlap of 4 or more; overlap 1 or 2 can't follow phases and
sounds rough. Partials that land on top of each other add up;
partials moved below 0 Hz or past half the sample rate are dropped.
With `scale` 1 and `shift` 0 frames pass through exactly.

Inputs are read at each frame's own sample. The output has the
input's frame size and hop. Phase tracking starts over after a reset
or a change of frame size. NaN or infinite bins count as zero.

**Try this:** between `FFT` and `IFFT`, set `shift` to 5 Hz and mix
the result with the dry signal for a slow, swirling beat; or set
`scale` to 2 on a melody for an octave-up double.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `spectrum` | Spectrum | — |  | Frames to shift. |
| `shift` | Float (Hz) | 0 | -2000 … 2000 | Hertz added to every partial. |
| `scale` | Float (×) | 1 | 0.25 … 4 | Every frequency is multiplied by this. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `spectrum` | Spectrum | The shifted frames. |

