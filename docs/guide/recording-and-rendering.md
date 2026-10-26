# Recording and rendering

There are two ways to get sound out of a patch and into a file:
**recording** what plays live, as you play it, and **rendering**, where
the computer works out the whole project as fast as it can, with no
sound card involved. This chapter covers both, and why a render comes
out exactly the same every time.

## Recording takes

A **Record** node writes whatever is wired into its `in` to a WAV file,
one file per take.

1. Save the project first. Takes go into the project's folder, so an
   untitled project can't record yet; its Record nodes say "save the
   project to record".
2. Add a Record node and wire what you want to keep into `in`: an Audio
   In for a guitar or a voice, or the end of your patch to capture a
   performance.
3. Press `R` (Transport ▸ Record) or the record button in the transport
   bar. The transport starts if it was stopped, and every Record node
   whose `arm` input is on (the default) records until you press `R`
   again or stop the transport (`Space`), which ends the take. The record
   button stays lit while recording is on.

Each take lands in the project's `assets/` folder as a 32-bit float WAV
named after the Record node's label, numbered `-001`, `-002`, and so on
(`assets/Lead-Vox-001.wav`). When a take is done, the status line says
"Recorded …" with a **Create Clip from take** button, which adds a Clip
playing it next to the Record node. The clip is placed in the song where
the take started (its `start` is *At beat…*), so it plays in time from
any bar, and it's wired into whatever the Record node's source also fed
(a Mix input, a Track), so the take plays where you heard yourself; the
source stays wired to the Record node for the next take.

**Punch in.** `Shift`-drag across the ruler under the transport bar to set
a punch range (a red band). While it's set, recording only happens inside
it: press `R` before the range and the Record nodes start on its first
sample and stop on its last. Right-click the ruler to clear it.

**Count-in.** Right-click the metronome button in the transport bar and
pick a number of bars: recording from a stop then clicks that many bars
first and starts on the next downbeat. The metronome itself (the button,
or `Shift+C`) clicks along while you play. Neither the click nor the
count-in is ever recorded or rendered.

**Monitor outputs.** Set an Audio Out's `role` to *monitor* (in the Node
tab) and it plays live only: a render leaves it out. Use it to hear your
input, or for a headphone cue mix, without either ending up in the
export.

More ways to drive it:

- `start` and `stop` triggers record a take whatever the transport does:
  wire a Button's `trigger` and `release` into them for a hold-to-record
  pedal.
- `arm` off keeps a Record node out of transport recording, so you can
  choose which inputs record.
- The `buffer` output holds the last finished take (up to ten minutes), so
  a Sampler can play it right away, with no file to load: record a phrase,
  then play it back at any pitch from a keyboard.
- `recording` is on while a take runs, and `length` is its duration.
- The `channels` setting is how many channels are written (two by
  default; a mono signal goes to both).

Takes start and end on buffer boundaries, a few milliseconds apart (on
the exact sample at a punch range's edges). If the disk can't keep up,
the node's display shows how many samples were dropped.

From the command line, `audioc run --record` plays a project live with
recording on, and the takes land in the same `assets/` folder.

> **Try this:** open the **Guitar Pedalboard**, save it under a new name,
> and wire the Level's `out` into a Record node too. Press `R`, play a
> phrase, press `R` again, and click **Create Clip from take** to hear it
> back.

## Rendering

### From the editor

**File ▸ Export WAV** (`Ctrl+E`) renders the whole project to a 24-bit
WAV file. It runs in the background while you keep working; the status
line says when it's done. The file is exactly what `audioc render` would
make with its default settings.

### From the command line

```
audioc render my-song.acproj -o my-song.wav
```

renders the project's whole length at its sample rate and prints the
peak and RMS level of each channel, so you can see at once if it clipped
or came out silent. Useful options:

| Option | Does |
| --- | --- |
| `--from 0:30 --to 1:00` | Render only part of the song, in seconds or `m:ss` |
| `--sample-rate 96000` | Render at another rate |
| `--bit-depth 16` | `16`, `24` (the default), or `32f` (32-bit float) |
| `--set 9.gain=-4` | Change an input's value for this render only (repeatable) |
| `--input take.wav` | Play this file into the Audio In nodes, as if from the sound card |
| `--midi-out notes.mid` | Also write what MIDI Out nodes send, as a standard MIDI file |
| `--block-size 64` | Render with another block size (see below) |
| `-q` | Print nothing but errors |

`--set` takes a node's id (shown in the Node tab under the node's name,
and by `audioc inspect`), a dot, the input's name, `=`, and the value:
`--set 3.cutoff=800`, `--set c.value=true`, `--set 4.shape=saw`. It works
on nodes at the top level of the patch, including a group's own inputs.

`--input` doesn't convert sample rates: give it a file at the render's
sample rate, or the render warns that it will play at the wrong speed.

The full list of commands is in [The command line](command-line.md).

### What plays in a render

A render has no sound card, keyboard, or network, so:

- Audio In nodes are silent, unless you pass `--input`.
- MIDI In, Keyboard In, Gamepad, and OSC In are quiet. For a patch you
  want to render, play its notes from a MIDI Pattern, a Pattern, or a
  Clock, the way the examples do.
- Nodes that need their whole input (Reverse, Normalize) always work,
  baked from the render's own input; see
  [Live and baked](live-and-baked.md).
- Record nodes don't record.
- Audio Outs set to the *monitor* role are left out, and so is the
  transport's click.
- `--from` starts the render partway through: every node starts fresh
  there, clips and patterns placed in the song (*At beat…*) start where
  they'd be, and the result is the same as seeking there live.

### Several files at once: File Out

A **File Out** node writes its input to a file of its own when the
project is rendered. Its `path` setting is relative to the folder of the
main output: `drums.wav`, or `stems/bass.wav` (`.wav` is added if there's
no extension). Each file covers the same span as the render, at the same
rate and bit depth. A path that would leave the output folder is refused,
so a project someone sent you can't write anywhere else.

> **Try this:** put a File Out after each instrument's last effect, named
> after the instrument, and render once: you get a set of stems that line
> up sample for sample with the full mix.

File Out does nothing while you play live; use Record for that.

## The same result every time

Render the same project at the same sample rate and block size, and you
get the same samples, bit for bit, every time. Playing it live in the
editor gives the same samples as rendering it too. That makes renders
safe to compare, automate, and redo.

- **Randomness comes from the seed.** Random, Probability, Noise,
  RandomWalk, and random LFO shapes are driven by the project's **Seed**
  (Project tab). The same seed gives the same choices on every play and
  every render. Change the seed for a new variation; set it back to get the
  old one.
- **Timing is exact.** The tempo map converts beats to samples exactly, so
  a note on beat 3 always starts on the same sample, at any tempo.
- **Block size.** Almost everything comes out the same at any block size.
  Two things depend on it: a **Feedback** node delays its loop by one
  block (unless its delay is set to **sample**), and a few values change
  once per block by nature (an LFO, a bar count). The Project tab's **Render block** sets the block size for
  renders (and for baking), so renders stay the same; `--block-size`
  overrides it.

> **Try this:** render the **Generative Patch** example twice and compare
> the files: they're identical. Change the seed in the Project tab, save,
> and render again for a different tune.

Next: [The command line](command-line.md).
