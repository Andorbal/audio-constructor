# Sequencing & Notes

[← All nodes](README.md)

<a id="midi-note"></a>
## MIDI Note

Kind: `midi_note` · Also found by: note, pitch, monophonic, mono, keyboard, portamento, glide, cv

Turns MIDI notes into a pitch, a gate, and a velocity for one voice.

`freq` follows the sounding note (A4 = 440 Hz), `gate` is high while
any key is held, and `trigger` fires when a key press becomes the
sounding note. `velocity` (0–1) and `note` belong to the sounding
note. When several keys are held, `priority` picks which one sounds:
the last pressed, the lowest, or the highest. Releasing the sounding
key falls back to another held key by the same rule, keeping the gate
high and without a new trigger, so envelopes carry on (legato).
After the last key is released, `freq`, `note`, and `velocity` keep
their values so a release tail stays at the right pitch. Before the
first note, `freq` is 440 Hz and `note` is 69.

`glide` slides from one note to the next: the pitch moves in a
straight line in semitones and arrives after `glide` seconds,
whatever the interval. The first note starts at its own pitch. Pitch
bend shifts `freq` by up to `bend_range` semitones. `channel` 0
listens to every MIDI channel, 1–16 to one. All-notes-off and
all-sound-off messages release every key. Up to 16 held keys are
remembered; beyond that the oldest is forgotten.

**Try this:** `MIDI In` → MIDI Note, then `freq` into an `Oscillator`
and `gate` into an `ADSR` driving a `Gain` for a complete monosynth;
set `priority` to low and add a little `glide` for a sliding bass.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `midi` | Midi | — |  | Notes to follow. |
| `priority` | Int | `last` | `last`, `low`, `high` | Which held key sounds: last pressed, lowest, or highest. |
| `glide` | Float (s) | 0 | 0 … 2 | Time to slide from one note to the next. |
| `bend_range` | Float (st) | 2 | 0 … 24 | Pitch bend depth. |
| `channel` | Int | 0 | 0 … 16 | MIDI channel, 1–16; 0 listens to all. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `freq` | Float (Hz) | Pitch of the sounding note. |
| `gate` | Bool | High while a key is held. |
| `velocity` | Float | Velocity of the sounding note, 0–1. |
| `note` | Int | The sounding note number. |
| `trigger` | Trigger | Fires on each new note. |

