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

<a id="voice"></a>
## Voice

Kind: `voice` · Also found by: polyphony, poly, note, pitch, gate, velocity, voice index

One voice's note, inside the body of a `Voices` node.

Each copy of a Voices body gets its own notes, and its Voice node
turns them into signals: `freq` (A4 = 440 Hz, shifted by pitch bend
up to `bend_range` semitones), `gate` (high while the note is held or
sustained), `velocity` (0–1), `note`, a `trigger` on each new note,
and `index`, which voice this is (0, 1, 2, ...), handy for spreading
voices across the stereo field or detuning them. `midi` passes the
voice's own notes on, for nodes that want MIDI (a `MIDI Note` for
glide, a `MIDI CC`).

When a voice is stolen for a new note, or the same note is played
again, `gate` drops for one sample before the new note so envelopes
start again. After a release, `freq` and `note` hold their values so
the tail stays in tune. Outside a Voices body the node does nothing
and the Problems tab says so.

**Try this:** wire `freq` into an Oscillator and `gate` into an ADSR
that drives a Gain, then multiply `index` by 0.3 and wire it into a
`Pan` for voices that fan out left to right.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `bend_range` | Float (st) | 2 | 0 … 24 | Pitch bend depth. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `freq` | Float (Hz) | Pitch of this voice's note. |
| `gate` | Bool | High while the note is held. |
| `velocity` | Float | Velocity of the note, 0–1. |
| `note` | Int | The note number. |
| `trigger` | Trigger | Fires on each new note. |
| `index` | Int | Which voice this is, from 0. |
| `midi` | Midi | This voice's MIDI, passed on. |

<a id="voices"></a>
## Voices

Kind: `voices` · Also found by: polyphony, polyphonic, poly, voice allocation, chords, synth

Makes any patch polyphonic: its body (a group) runs once per voice,
each voice plays one note at a time, and the voices' outputs are
added together.

Notes arriving at `midi` are shared out among `voices` copies of the
body (config, 1 to 64, default 8). Inside the body, a `Voice` node
gives each copy its own note: `freq`, `gate`, `velocity`, `note`,
`trigger`, and the voice's `index`. Build a mono voice with a `Voice`
node, an Oscillator, a filter, and an envelope, expose its output, and
it becomes a synth. The body's other inputs appear on the Voices node
and reach every voice; its Audio and Float outputs are summed over
the voices (other types come from the first voice).

A new note goes to a free voice, preferring the one released longest
ago so release tails can ring out. When every voice is busy, `steal`
(config) chooses which note gives way: `oldest` (default), `newest`,
`lowest`, `highest`, or `none` (the new note is ignored). A stolen or
repeated note retriggers its voice. The sustain pedal (CC 64) holds
released notes; pitch bend, pressure, and controllers reach every
voice. Double-click the node to edit its body.

**Try this:** select the Oscillator → Biquad → Gain voice of the
Subtractive Synth example, group it, swap its MIDI Note for a `Voice`
node, and change the group into a Voices node: play chords.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `midi` | Midi | — |  | Notes to share out among the voices. |

