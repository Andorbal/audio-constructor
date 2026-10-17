# Filters

[← All nodes](README.md)

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

