# Sequencing & Notes

[← All nodes](README.md)

Notes and MIDI: turning MIDI into signals and back, patterns, piano rolls, arpeggios, chords, scales, and polyphony.

<a id="arpeggiator"></a>
## Arpeggiator

Kind: `arpeggiator` · Also found by: arp, arpeggio, broken chord, sequence, latch, pattern, notes

Plays the notes you hold one at a time, stepping on each clock tick.

Hold a chord on `midi` and each `clock` trigger plays the next of
its notes, by `mode`: `up` (low to high), `down`, `up_down` (up and
back without repeating the ends), `random`, or `order` (the order
the keys went down). `octaves` (1–4) repeats the notes that many
octaves up before starting over. A chord pressed after every key
was let go starts from the beginning. Up to 16 held keys count;
beyond that the oldest is forgotten. Notes that `octaves` pushes
past 127 are rests. Each note keeps the velocity and channel of its
key.

Each note lasts `gate` (0–1) of the time between the last two
clocks, measured as they arrive (until two have arrived, a beat at
the current tempo stands in), and at least one sample. A clock that
comes while a note still sounds ends it first, on the same sample.
At 1 the notes run into each other (legato): each ends at the next
clock, or after two intervals if the clock stops. Letting go of
every key ends the sounding note at once, unless `latch` is on: then
the notes keep playing after you let go, until you press a new chord
(which replaces them) or turn `latch` off.

Nothing plays without a `clock`: wire a `Clock`. Messages other
than notes (controllers, pitch bend, pressure, program changes) pass
straight through. On the same sample, incoming notes apply first,
then a note due to end, then the clock. All-notes-off clears the
held keys and ends the sounding note, and a transport reset ends it
at the start of the next block.

**Try this:** `MIDI In` → Arpeggiator → a synth, with a `Clock`
(`interval` 0.25) into `clock`, `mode` up_down, `octaves` 2, and
`gate` 0.5. Hold a chord, turn on `latch`, and change chords on the
beat while you shape the sound.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `midi` | Midi | — |  | The notes to play. |
| `clock` | Trigger | — |  | Plays the next note. |
| `mode` | Int | `up` | `up`, `down`, `up_down`, `random`, `order` | The order notes play in. |
| `octaves` | Int | 1 | 1 … 4 | Octaves the notes repeat over. |
| `gate` | Float | 0.5 | 0 … 1 | Note length as a fraction of the time between clocks. |
| `latch` | Bool | false |  | Keep playing after the keys are let go, until a new chord. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `midi` | Midi | One note at a time, and everything else. |

<a id="chord"></a>
## Chord

Kind: `chord` · Also found by: harmony, triad, seventh, interval, harmonizer, voicing, power chord, inversion, notes

Plays a whole chord for every incoming note.

`chord` picks the shape built on each note: major, minor, sus2,
sus4, dim, aug, maj7, min7, dom7, power (root and fifth), octave
(root and the octave above), or custom, which uses the `intervals`
config: up to 8 semitone offsets from the played note, −48 to 48,
such as `[0, 3, 7, 10]` (an empty list plays just the note).
`inversion` (0–3) raises the lowest note by an octave that many
times, so 1 is the first inversion. `spread` (0–3 octaves) opens the
voicing by raising every second note, counted from the bottom, by
that many octaves.

Every chord note gets the played note's velocity and channel. Each
note-off releases exactly the notes its note-on started, even if the
settings changed in between. Notes that would fall outside 0–127 are
left out. When two held chords share a note, it sounds once and ends
with the last chord that holds it. Polyphonic pressure goes to every
note of its chord; other messages pass through. All-notes-off and
all-sound-off release the chords on their channel, and a transport
reset releases every sounding note at the start of the next block.

**Try this:** `MIDI In` → Chord (min7) → a `Voices` synth, and every
key plays a minor seventh; set `chord` to power with `spread` 1 for
wide fifths under a bass line.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `midi` | Midi | — |  | Notes to build chords on. |
| `chord` | Int | `major` | `major`, `minor`, `sus2`, `sus4`, `dim`, `aug`, `maj7`, `min7`, `dom7`, `power`, `octave`, `custom` | The chord shape. |
| `inversion` | Int | 0 | 0 … 3 | Raise the lowest note an octave this many times. |
| `spread` | Int | 0 | 0 … 3 | Octaves to raise every second note by. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `midi` | Midi | The chords and everything else. |

<a id="humanize"></a>
## Humanize

Kind: `humanize` · Also found by: groove, feel, jitter, random, loose, timing, velocity, drift, human

Loosens a part the way a player would: each trigger or note comes a
little late and a little louder or softer, differently every time.

Set the `type` config to *trigger* (the default) or *midi*. Each
event on `in` is delayed by a random time from 0 to `timing`
(seconds, default 10 ms) and its level moved by up to `dynamics` of
itself, up or down (0.1 is ±10%). MIDI note-ons have their velocity
changed (never below 1), and each note-off is delayed as much as its
note-on, so notes keep their length; other messages pass on time.
For triggers, `level` (0 to 1, default 1) is read at each trigger,
changed the same way, and held on `velocity` from where the trigger
comes out, so a Pattern's `velocity_k` wired into `level` keeps its
accents, loosened.

The randomness comes from the project's seed, so a render sounds
the same every time, and restarting the transport replays it. Every
event uses the same two draws whatever `timing` and `dynamics` are,
so changing them doesn't reshuffle the part. Up to 1024 events can
wait at once; more are dropped.

**Try this:** put Humanize (`timing` 0.008, `dynamics` 0.2) between
a Pattern's hat row and its envelope, with `velocity_2` into `level`
and Humanize's `velocity` into the hat's level: the same grid, but
it breathes.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Trigger | — |  | The triggers to loosen. |
| `timing` | Float (s) | 0.01 | 0 … 0.05 | Each event comes up to this much late. |
| `dynamics` | Float | 0.1 | 0 … 1 | How far each level moves, up or down, as a share of itself. |
| `level` | Float | 1 | 0 … 1 | Each trigger's level, read as it arrives. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Trigger | The triggers, a little late. |
| `velocity` | Float | The last trigger's level, loosened, from where it comes out. |

<a id="midi-cc"></a>
## MIDI CC

Kind: `midi_cc` · Also found by: controller, cc, control change, knob, fader, mod wheel, pitch bend, aftertouch, pressure, expression

Turns one MIDI controller into a `Float` in a range you choose.

`controller` picks the controller number, 0–127 (1 is the mod wheel,
7 volume, 11 expression, 64 sustain, 74 brightness). Two numbers past
the controllers follow other continuous messages: 128 is pitch bend
(centered halfway between `min` and `max`) and 129 is channel
pressure (aftertouch). `channel` 0 listens to every MIDI channel,
1–16 to one.

`out` maps the controller's value from 0–127 onto `min`–`max` (set
`min` above `max` to turn it upside down) and holds it until the
next message. A new value takes effect on the exact sample its
message arrives. With `smooth` above 0 the output glides there in a
straight line instead, arriving `smooth` seconds later, which hides
the steps of a 7-bit controller; each new message starts a new
glide from wherever the output is. `changed` fires on each message
that changes the value.

Before its first message a controller sits at the `initial` config,
a fraction from 0 (`min`, the default) to 1 (`max`); pitch bend
always starts centered. The node remembers the last value of every
controller, so switching `controller` moves to that controller's
value (or `initial` if it hasn't moved yet), gliding if `smooth` is
set. Controller values are 7-bit and pitch bend 14-bit; 14-bit
controller pairs aren't combined.

**Try this:** `MIDI In` → MIDI CC with `controller` 1, `min` 200 and
`max` 8000, into a `Biquad`'s `cutoff`: the mod wheel sweeps the
filter. Add `smooth` 0.02 to take the zipper out of slow sweeps, or
wire `changed` into a `Clip`'s `start` to fire a sample from a pad.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `midi` | Midi | — |  | Messages to read the controller from. |
| `controller` | Int | 1 | 0 … 129 | Controller number 0–127; 128 pitch bend, 129 channel pressure. |
| `channel` | Int | 0 | 0 … 16 | MIDI channel, 1–16; 0 listens to all. |
| `min` | Float | 0 | 0 … 1 | Output at controller value 0. |
| `max` | Float | 1 | 0 … 1 | Output at controller value 127. |
| `smooth` | Float (s) | 0 | 0 … 1 | Glide time to each new value. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The controller's value mapped onto min–max. |
| `changed` | Trigger | Fires when the value changes. |

<a id="midi-filter"></a>
## MIDI Filter

Kind: `midi_filter` · Also found by: split, keyboard split, zone, range, channel, block, thru, layer

Passes only the MIDI you want: one channel, a range of notes, and chosen kinds of message.

`channel` 0 passes every channel, 1–16 just one. Notes pass when
their number is from `low` to `high` (inclusive; the two swap if
`low` is above `high`), so two MIDI Filters with ranges 0–59 and
60–127 split a keyboard between two sounds. The toggles pass or
block whole kinds of message: `notes` (note-ons and note-offs),
`controllers` (control changes), `pitch_bend`, `pressure` (channel
and polyphonic aftertouch), and `program` (program changes).
Polyphonic pressure belongs to a note, so it also has to be in the
range. The toggles are read on the sample each message arrives.

A note-off always passes when its note-on did, so changing the
range, the channel, or a toggle while keys are held never leaves a
note hanging. All-notes-off and all-sound-off (controllers 123 and
120) release the notes that passed on their channel even when they
are filtered out themselves: those notes get note-offs instead. A
transport reset releases the notes that passed at the start of the
next block.

**Try this:** wire `MIDI In` into two MIDI Filters, one with `high`
59 feeding a bass synth and one with `low` 60 feeding a lead, for a
split keyboard; or turn `notes` off to pass only the knobs on to a
`MIDI CC`.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `midi` | Midi | — |  | Messages to filter. |
| `channel` | Int | 0 | 0 … 16 | MIDI channel to pass, 1–16; 0 passes all. |
| `low` | Int | 0 | 0 … 127 | Lowest note that passes. |
| `high` | Int | 127 | 0 … 127 | Highest note that passes. |
| `notes` | Bool | true |  | Pass note-ons and note-offs. |
| `controllers` | Bool | true |  | Pass control changes. |
| `pitch_bend` | Bool | true |  | Pass pitch bend. |
| `pressure` | Bool | true |  | Pass channel and key pressure. |
| `program` | Bool | true |  | Pass program changes. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `midi` | Midi | The messages that passed. |

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

<a id="midi-pattern"></a>
## MIDI Pattern

Kind: `midi_pattern` · Also found by: piano roll, clip, sequence, riff, melody, notes, song, loop, sequencer, midi clip

A piano-roll clip of notes in beats that plays as MIDI, from a one-bar riff to a whole song.

The clip is `length` beats long (config, 0.25–1024, default 4; a
beat is a quarter note). Its notes are the `notes` config, edited in
the piano roll: each is `[start, length, note, velocity]`, with
`start` and `length` in beats from the top of the clip, `note`
0–127, and `velocity` 1–127 (100 if left out), such as
`[[0, 1, 60, 100], [1, 0.5, 64, 90]]`. Notes are cut off at the end
of the clip (and at its start); notes that start past the end, have
no length, or hold something other than numbers are skipped. Up to
4096 notes, played on MIDI channel `channel` (config, 1–16,
default 1).

**Banks:** a clip can hold several sets of notes of the same length
(up to 16), added as tabs in the piano roll; `notes` is then a list
of note lists. The `bank` input picks which plays, counting from 0.
A new bank starts with the next pass, so the riff changes on its
downbeat; while stopped it applies at once.

**Swing:** `swing` (0 to 1) moves the notes on every other
sixteenth (or eighth, with the `swing_grid` config) later, by up to
half a sixteenth, stretching the time between so nothing changes
order: at about 0.67 a straight riff shuffles in triplets.

Timing follows the song's tempo map from wherever the clip started:
a note `n` beats into the clip plays `n / rate` song beats after the
start, on the exact sample the tempo map gives for that beat, so
tempo changes are followed and nothing drifts, however long it
loops. The clip keeps its own count of samples since it started, so
it also plays when started while the transport is stopped. `rate`
scales the speed (2 is double time; 0 pauses, holding any sounding
notes).

**Placed in the song:** set `start` to *At beat…* and the clip
starts at song beat `at` and stops at `until` (unless that's 0). Start the transport anywhere and it
carries on from where the song is: around again if it loops,
finished if it doesn't, waiting before its beat. Notes already
under way where the transport starts stay silent; the next ones
play.

Generator contract: `start` (on transport play by default) plays
from the top: note-offs for anything sounding, then the notes at
beat 0 on the same sample. `stop` ends sounding notes and fires
`end`; `reset` jumps back to the top without stopping. At the end of
the clip it wraps around while `loop` is on (the default), ending
every sounding note before the first notes of the next pass; with
`loop` off it ends them, stops, and fires `end`. A note overlapping
an earlier one of the same pitch ends it where it starts. Events on
the same sample apply in the order stop, start, reset. `position`
is how many beats into the clip it is (held while stopped). A
transport reset releases sounding notes at the start of the next
block.

**Try this:** wire MIDI Pattern into a `MIDI Note` driving an
`Oscillator`, with the gate into an `ADSR` on a `Gain`, for a
looping riff; feed a `Voices` synth instead for chords, and chain
clips by turning `loop` off and wiring one's `end` into the next
one's `start`.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `start` | Trigger | `play` | `play`, `load`, `never`, `at` | Starts or restarts. Unwired: on transport play, on load, never, or at a beat of the song. |
| `at` | Float (beats) | 0 | 0 … 256 | The song beat it plays from, with `start` set to At beat. |
| `until` | Float (beats) | 0 | 0 … 256 | The song beat it stops at, placed in the song; 0 plays on. |
| `stop` | Trigger | `stop` | `stop`, `never` | Stops. Also on transport stop, wired or not, unless set to never. |
| `reset` | Trigger | — |  | Returns to the beginning. |
| `loop` | Bool | true |  | Wrap around at the end. |
| `rate` | Float (×) | 1 | 0 … 4 | Playback speed: 2 is double time, 0 pauses. |
| `swing` | Float | 0 | 0 … 1 | Moves every other sixteenth later, up to half of one. |
| `bank` | Float | 0 | 0 … 15 | Which notes play, from 0; a new bank starts with the next pass. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `midi` | Midi | The clip's notes. |
| `position` | Float (beats) | Beats into the clip. |
| `playing` | Bool | True while running. |
| `end` | Trigger | Fires on reaching the end or stopping. |

<a id="midi-transpose"></a>
## MIDI Transpose

Kind: `midi_transpose` · Also found by: transpose, shift, octave, key, pitch, scale, snap, harmonize

Shifts MIDI notes up or down by semitones, optionally snapping them into a scale.

Every note moves by `semitones` (−48 to 48; 12 is an octave up).
With `snap` on, the shifted note then moves to the nearest note of
`scale` over `root` (exactly halfway rounds down), which pushes a
part into a key. `custom` uses `mask`, the scale as 12 bits: bit 0 is
the root, bit 1 a semitone above, and so on (2741 is the major
scale). Notes that would land outside 0–127 are dropped, and so are
their note-offs.

Each held note remembers the note it started, so its note-off ends
that note even if `semitones`, `snap`, or the scale change while it's
held. When two held keys land on the same note (common with `snap`),
it sounds once and ends when the last of them is released.
Polyphonic pressure follows its note; controllers, pitch bend,
channel pressure, and program changes pass through unchanged.
All-notes-off and all-sound-off release the notes on their channel,
and a transport reset releases every sounding note at the start of
the next block.

**Try this:** put MIDI Transpose with `semitones` 12 between `MIDI In`
and a second synth to double a part an octave up; or turn on `snap`
with `scale` minor_pentatonic and `root` A, and every key you play
lands in A minor pentatonic.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `midi` | Midi | — |  | Notes to shift. |
| `semitones` | Int | 0 | -48 … 48 | How far to shift; 12 is an octave. |
| `snap` | Bool | false |  | Snap shifted notes into the scale. |
| `scale` | Int | `major` | `chromatic`, `major`, `minor`, `harmonic_minor`, `melodic_minor`, `dorian`, `phrygian`, `lydian`, `mixolydian`, `locrian`, `major_pentatonic`, `minor_pentatonic`, `blues`, `whole_tone`, `custom` | Scale to snap to. |
| `root` | Int | `C` | `C`, `C#`, `D`, `D#`, `E`, `F`, `F#`, `G`, `G#`, `A`, `A#`, `B` | The scale's root note. |
| `mask` | Int | 2741 | 0 … 4095 | Custom scale: bit 0 the root, bit 1 a semitone up, and so on. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `midi` | Midi | The shifted notes and everything else. |

<a id="note-split"></a>
## Note Split

Kind: `note_split` · Also found by: drum map, drum, pad, note, split, trigger, gate, velocity, choke, midi to trigger

Splits MIDI into a trigger, a gate, and a velocity for each of a
list of notes: one node turns a drum pad's or a pattern's notes into
what each sound needs.

The `notes` config lists the notes, such as `[36, 38, 42, 46]`
(kick, snare, closed and open hat); without it, `count` notes from
`low` (default one note, 36). Up to 64. For the note at place *k* in
the list:

- `trigger_k` fires on its note-on, on that sample;
- `gate_k` is high from the note-on until its note-off;
- `velocity_k` is how hard it was played, 0 to 1, from the note-on
  (wire it into a `Sampler`'s `velocity`);
- `off_k` fires when the gate ends.

**Choke groups:** the `chokes` config gives each note a group, such
as `[0, 0, 1, 1]`. A note in a group ends every other note of its
group (their gates close and their `off` fires), so wiring `off_k`
into a Sampler's `stop` lets the closed hat cut the open one. Group 0
chokes nothing. `channel` (config, 1–16; 0, the default, for all)
picks the MIDI channel to listen to.

**Try this:** wire a `Pads` grid's `midi` into Note Split with
`notes` `[36, 37, 38, 39]`, and each `trigger_k` and `velocity_k`
into the `start` and `velocity` of a Sampler: four pads that play
soft and hard, in two nodes per pad.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `midi` | Midi | — |  | Notes to split. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `trigger_0` | Trigger | Fires on this note's note-on. |
| `gate_0` | Bool | High while this note is held. |
| `velocity_0` | Float | How hard this note was played, 0 to 1. |
| `off_0` | Trigger | Fires when this note ends or is choked. |

<a id="note-to-midi"></a>
## Note To MIDI

Kind: `note_to_midi` · Also found by: gate to midi, trigger to midi, keyboard, button, note, cv to midi, notes

Builds MIDI notes from signals: a gate or a trigger, a note number, and a velocity.

With `gate` wired, a note starts when the gate rises and ends when
it falls. Otherwise each `trigger` starts a note that ends `length`
seconds later (at least one sample); the unwired gate's own toggle
still plays a note while it's on. `note` (a MIDI note number, read
when the note starts, rounded and kept within 0–127; `Int` sources
wire straight in), `velocity` (0–1, at least the softest MIDI
velocity), and `channel` (1–16) are taken when each note starts, and
the note-off always matches the note that started.

One note sounds at a time: a new note ends the previous one first,
on the same sample (note-off before note-on). On the same sample a
falling gate or an ending note comes first, then a rising gate, then
a trigger. A transport reset ends the sounding note at the start of
the next block, and a gate still high afterwards starts a new note.

**Try this:** wire several `Button`s through `Select` into `note`
and their gates through `Logic` (or) into `gate` to build a
keyboard, then feed `midi` into a `Voices` synth; or drive `trigger`
from a `Clock` and `note` from a `Random` for a generative line.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `gate` | Bool | false |  | A note plays while this is high. |
| `trigger` | Trigger | — |  | Plays a note `length` long (when `gate` isn't wired). |
| `note` | Float | 60 | 0 … 127 | MIDI note number, read when a note starts. |
| `velocity` | Float | 0.8 | 0 … 1 | Velocity, 0–1, read when a note starts. |
| `channel` | Int | 1 | 1 … 16 | MIDI channel, 1–16. |
| `length` | Float (s) | 0.1 | 0.01 … 2 | How long a triggered note lasts. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `midi` | Midi | The notes. |

<a id="pattern"></a>
## Pattern

Kind: `pattern` · Also found by: step sequencer, drum machine, grid, beat, rhythm, steps, sequencer, drums, trigger, gate

A grid of step buttons that plays one step per clock tick, like a drum machine's pattern.

The grid has `steps` columns (config, 1–64, default 16) and `rows`
rows (config, 1–16, default 4); click cells in the node to switch
them on, and drag up or down on one to set its level. The `cells`
config holds the grid as one string per row, `x` for an on step at
full level, `1` to `9` for an on step at that many tenths, and `.`
for an off one, such as `["x...x...x...x...", "....x.......x..."]`.
Missing rows and steps are off, and any other character counts as
off.

**Banks:** a Pattern can hold several grids of the same size (up to
16), added as tabs in the editor; `cells` is then a list of grids,
one list of strings each. The `bank` input picks which plays,
counting from 0. A new bank starts at the beginning of the next
pass, so a scene change lands on the downbeat; while stopped it
applies at once.

Each `clock` trigger moves to the next step. Where that step is on
in row *n*, `trigger_n` fires on the clock's sample, `gate_n` goes
high until the next clock, and `velocity_n` holds the step's level
(0 to 1) until the row's next on step (rows count from 0, the top
row). On steps next to each other keep the gate high from one to the
next, so use the trigger to restart an envelope on every step. `step` is
the last step played, counting from 0. Nothing moves without a
clock: wire a `Clock` into `clock` (`interval` 0.25 for sixteenth
notes).

**Placed in the song:** set `start` to *At beat…* and the pattern
starts at song beat `at` (and stops at `until`, unless that's 0), with `interval` beats per step (0.25,
sixteenths, by default: set it to the clock's interval). Each clock
then plays the step the song is on, counted from `at`, so after a
seek, a loop, or play from the middle the pattern is on the right
step at once; clocks before its beat play nothing.

Generator contract: `start` (on transport play by default) goes
back to the beginning and plays, so the next clock, or one on the
same sample, plays the first step (the gates close until then).
`stop` closes the gates, ignores the clock, and fires `end`; `reset`
goes back to the beginning without stopping. After the last step the
pattern wraps around while `loop` is on (the default). With `loop`
off, the clock after the last step stops it and fires `end` instead,
so the last step gets its full length and a second Pattern started
by `end` plays its first step on that same clock. `reverse` flips
the direction: the pattern steps backwards, and its beginning
becomes the last step. Events on the same sample apply in the order
reverse, stop, start, reset, clock.

**Try this:** wire a `Clock` (`interval` 0.25) into `clock`, then
`trigger_0` into the `on` of an `ADSR` (sustain 0) on a low
`Oscillator` for a kick, and `trigger_1` into another on a `Noise`
for a snare. Fill row 0 with `x...x...x...x...` and row 1 with
`....x.......x...`. Paint the hats at level 5 with an `x` on every
fourth step, and wire `velocity_2` into their amplifier, for accents.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `clock` | Trigger | — |  | Moves to the next step. |
| `start` | Trigger | `play` | `play`, `load`, `never`, `at` | Starts or restarts. Unwired: on transport play, on load, never, or at a beat of the song. |
| `at` | Float (beats) | 0 | 0 … 256 | The song beat it plays from, with `start` set to At beat. |
| `until` | Float (beats) | 0 | 0 … 256 | The song beat it stops at, placed in the song; 0 plays on. |
| `interval` | Float (beats) | 0.25 | 0 … 4 | Beats per step, placed in the song: each clock plays the step the song is on. |
| `stop` | Trigger | `stop` | `stop`, `never` | Stops. Also on transport stop, wired or not, unless set to never. |
| `reset` | Trigger | — |  | Returns to the beginning. |
| `loop` | Bool | true |  | Wrap around at the end. |
| `reverse` | Trigger | — |  | Toggles the direction. |
| `bank` | Float | 0 | 0 … 15 | Which grid plays, from 0; a new one starts with the next pass. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `trigger_0` | Trigger | Fires when a clock lands on an on step in this row. |
| `gate_0` | Bool | High from an on step until the next clock. |
| `velocity_0` | Float | The level of this row's last on step, 0 to 1. |
| `trigger_1` | Trigger | Fires when a clock lands on an on step in this row. |
| `gate_1` | Bool | High from an on step until the next clock. |
| `velocity_1` | Float | The level of this row's last on step, 0 to 1. |
| `trigger_2` | Trigger | Fires when a clock lands on an on step in this row. |
| `gate_2` | Bool | High from an on step until the next clock. |
| `velocity_2` | Float | The level of this row's last on step, 0 to 1. |
| `trigger_3` | Trigger | Fires when a clock lands on an on step in this row. |
| `gate_3` | Bool | High from an on step until the next clock. |
| `velocity_3` | Float | The level of this row's last on step, 0 to 1. |
| `step` | Int | The last step played, from 0. |
| `playing` | Bool | True while running. |
| `end` | Trigger | Fires on reaching the end or stopping. |

<a id="scale-quantize"></a>
## Scale Quantize

Kind: `scale_quantize` · Also found by: quantize, quantizer, scale, key, snap, tune, pentatonic, generative, pitch, melody

Snaps notes to a scale: a `Float` note number, or the notes of a MIDI stream.

The `type` config picks the form. `float` (the default) quantizes a
number: `in` is a MIDI note number (60 is middle C; fractions are
fine, and `Int` sources wire straight in), `out` is the nearest note
of the scale as a whole number, at the input's rate, and `note` is
the same as an `Int`, taken at the start of each block. A non-finite
`in` gives 0, and inputs beyond ±10 000 are clamped first.

`midi` quantizes the notes in `midi` instead. Each held note
remembers the note it started, so its note-off ends that note even
if the scale changes while it's held; two keys landing on the same
note sound it once, until the last is released. Polyphonic pressure
follows its note and other messages pass through. All-notes-off and
all-sound-off release the notes on their channel, and a transport
reset releases every sounding note at the start of the next block.

Either way, `scale` and `root` choose the notes allowed (C major by
default) and `custom` uses `mask`, the scale as 12 bits: bit 0 is
the root, bit 1 a semitone above, and so on (2741 is the major
scale; 0 counts as every note). Exactly halfway between two scale
notes rounds down.

**Try this:** wire a slow `Oscillator` (`pure`, 0.2 Hz) through a
`Map` (−1..1 → 48..72) into `in`, and `out` through a `Convert` (MIDI
note → Hz) into a second Oscillator's `freq`: with `scale`
minor_pentatonic the sweep turns into a melody that never plays a
wrong note. Set `type` to midi after `MIDI In` to keep any keyboard
part in key.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Float | 60 | 0 … 127 | A MIDI note number (fractions allowed). |
| `scale` | Int | `major` | `chromatic`, `major`, `minor`, `harmonic_minor`, `melodic_minor`, `dorian`, `phrygian`, `lydian`, `mixolydian`, `locrian`, `major_pentatonic`, `minor_pentatonic`, `blues`, `whole_tone`, `custom` | Scale to snap to. |
| `root` | Int | `C` | `C`, `C#`, `D`, `D#`, `E`, `F`, `F#`, `G`, `G#`, `A`, `A#`, `B` | The scale's root note. |
| `mask` | Int | 2741 | 0 … 4095 | Custom scale: bit 0 the root, bit 1 a semitone up, and so on. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Float | The nearest scale note. |
| `note` | Int | The nearest scale note, per block. |

<a id="velocity"></a>
## Velocity

Kind: `velocity` · Also found by: dynamics, velocity curve, sensitivity, accent, midi, touch, fixed

Reshapes how hard MIDI notes are played: scale, curve, range, or one
fixed velocity for every note.

Each note-on's velocity, as 0 to 1, is bent by `curve` (positive
makes soft playing louder, negative makes it quieter; 0 leaves it),
multiplied by `scale`, and placed between `min` and `max`. A `fixed`
above 0 replaces it all with that one velocity. Notes stay at least
velocity 1, so a soft hit is never taken for a note-off. Note-offs
and every other message pass unchanged.

**Try this:** put Velocity between a drum pad controller and a
`Note Split`, with `curve` at 0.5 for pads that feel too stiff, or
`min` at 0.4 so the softest hit still speaks; set `fixed` to 1 to
hear every hit at full strength.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `midi` | Midi | — |  | Notes to reshape. |
| `scale` | Float (×) | 1 | 0 … 2 | Multiplies every velocity. |
| `curve` | Float | 0 | -1 … 1 | Positive: soft playing louder; negative: quieter. |
| `min` | Float | 0 | 0 … 1 | The softest velocity out, 0 to 1. |
| `max` | Float | 1 | 0 … 1 | The hardest velocity out, 0 to 1. |
| `fixed` | Float | 0 | 0 … 1 | Above 0: every note at this velocity. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `midi` | Midi | The notes, reshaped. |

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

<a id="zones"></a>
## Zones

Kind: `zones` · Also found by: keymap, key zone, velocity layer, split, layer, multisample, map, range, sampler

Finds which zone of a key and velocity map a note falls in: the
heart of a keymapped or velocity-layered instrument.

The `zones` config lists the zones, each `[low, high, soft, hard]`:
the notes from `low` to `high` (MIDI numbers, both included) played
at a velocity from `soft` to `hard` (0 to 1). Draw them in the Node
tab's map: drag a zone to move it, its edges to resize it. The
default is one zone over every note and velocity. Up to 32.

On each `trigger`, the `note` and `velocity` at that sample pick
the zones they fall in: every one fires its `trigger_k`, and `zone`
becomes the first of them (−1 when none). With `trigger` unwired,
`zone` follows `note` and `velocity` all the time.

**Try this:** in a `Voices` body, wire the Voice node's `note`,
`velocity`, and `trigger` into Zones with two zones, `[0, 59, 0, 1]`
and `[60, 127, 0, 1]`, and `trigger_0` and `trigger_1` into the
`start` of two Samplers: a low sound and a high one. Split a zone by
velocity instead for a soft and a hard layer.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `note` | Float | 60 | 0 … 127 | The note number. |
| `velocity` | Float | 1 | 0 … 1 | How hard it was played, 0 to 1. |
| `trigger` | Trigger | — |  | When to look the note up. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `zone` | Int | The first zone the note falls in, from 0; −1 for none. |
| `trigger_0` | Trigger | Fires when a triggered note falls in this zone. |

