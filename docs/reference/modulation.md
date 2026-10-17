# Modulation

[← All nodes](README.md)

<a id="adsr"></a>
## ADSR

Kind: `adsr` · Also found by: envelope, eg, adsr, contour, attack, release

An attack-decay-sustain-release envelope: when the gate opens it
rises to 1, falls to `sustain`, and holds; when the gate closes it
falls to 0. The output is a sample-rate `Float` from 0 to 1.

Drive it with `gate` (a `Bool`: attack on the rising edge, release on
the falling edge, detected per sample) or with the `on` and `off`
triggers, or both. Retriggering starts the attack from the current
level, so it never clicks.

- `attack` is the time from 0 to 1 (sooner when retriggered from
  higher up), `decay` the time from 1 down to `sustain`, `release`
  the time from wherever the level is down to 0. A time of 0 jumps on
  the next sample.
- `curve`: `linear` ramps, or `exponential` (default) analog-style
  curves: a bowed attack and decays that drop fast and then ease out.

`active` is true (per sample) whenever the envelope isn't idle, from
the attack until the release reaches 0; `end` fires on the sample the
release reaches 0. When a gate edge and a trigger land on the same
sample, releases (gate falling, `off`) apply before attacks (gate
rising, `on`), so a note-off and the next note-on retrigger cleanly.

**Try this:** wire `MidiNote`'s gate into `gate` and the output
through a `Map` (0..1 → -60..0 dB) into a `Gain` after an Oscillator
for a playable synth voice; wire `end` into a `Clip`'s `start` to
chain a sound after the note dies away.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `gate` | Bool | false |  | Attack while true, release when it falls. |
| `on` | Trigger | — |  | Start the attack from the current level. |
| `off` | Trigger | — |  | Start the release. |
| `attack` | Float (s) | 0.01 | 0 … 10 | Time from 0 to 1. |
| `decay` | Float (s) | 0.2 | 0 … 10 | Time from 1 down to the sustain level. |
| `sustain` | Float | 0.7 | 0 … 1 | Level held while the gate is open. |
| `release` | Float (s) | 0.3 | 0 … 20 | Time from the current level down to 0. |
| `curve` | Int | `exponential` | `linear`, `exponential` | Stage shape. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The envelope, 0 to 1. |
| `active` | Bool | True from the attack until the release ends. |
| `end` | Trigger | Fires when the release reaches 0. |

