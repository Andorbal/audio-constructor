# The plugin

Audio Constructor also runs inside other music software, as a plugin: put
a patch on a track, play it from the track's notes, automate it, and save
it with your session. There are two plugins, in the CLAP and VST3
formats:

- **Audio Constructor Effect** goes on an audio track or bus. The track's
  audio comes into the patch at **Audio In** and goes back out of **Audio
  Out**. It works in stereo, in mono, and mono in with stereo out. It also
  takes the track's notes, for **MIDI In** nodes and MIDI bindings.
- **Audio Constructor Instrument** goes on an instrument track. The
  track's notes come in at **MIDI In**, and **Audio Out** is the
  instrument's stereo output. Notes that **MIDI Out** nodes send go back
  to the host, so an instrument can also be a note generator.

## Install

Build the bundles from the source (see the [README](../../README.md)):

```
python3 scripts/bundle_plugin.py --install
```

This makes `Audio Constructor.clap` and `Audio Constructor.vst3` and
copies them into your own plugin folders: `~/.clap` and `~/.vst3` on
Linux, `~/Library/Audio/Plug-Ins/CLAP` and `~/Library/Audio/Plug-Ins/VST3`
on macOS, and `%LOCALAPPDATA%\Programs\Common\CLAP` and `...\VST3` on
Windows. Each bundle holds both plugins. Then let your host rescan its
plugins; they're listed under the vendor **Audio Constructor**. If your
host can load either format, prefer CLAP.

On macOS, a bundle you copied from somewhere else may be quarantined:
clear it with `xattr -dr com.apple.quarantine "Audio Constructor.clap"`
(and the same for the `.vst3`).

## What a new plugin plays

A new Effect passes its audio through a Gain called **Level**, with
**Macro 1** on its level: 0 dB in the middle, from −24 dB to +24 dB.

A new Instrument plays an eight-voice synth (the Subtractive Synth Voice
block) from the track's notes. **Macro 1** is its filter cutoff,
**Macro 2** the resonance, **Macro 3** how far the envelope opens the
filter, and **Macro 4** the level.

## Macros: what the host automates

Every plugin has sixteen parameters, **Macro 1** to **Macro 16**. They're
what your host shows in its automation lanes and its generic plugin
controls, and on control surfaces that show a plugin's pages (two pages of
eight). Each is a position from 0 to 1, in the middle when the plugin is
new.

What a macro does is up to the patch: a macro is a control source like a
MIDI knob, and you connect it to any unwired input with a
[binding](performing.md#bindings), with the same range, invert, and
on/off options. So the host's list never changes, whatever patch you
load, and automation you've drawn keeps working when you change the
patch.

- A macro that drives something shows the value it sets in the host's
  readout ("-12 dB", "1.2 kHz", a choice's name), and you can type a value
  that way too. One that drives nothing shows a percentage.
- Automation lands on its sample: a macro that moves in the middle of a
  block changes the patch on that sample, smoothed like any input over a
  few milliseconds.
- To bind a macro, learn it like any control: start learning on an input
  and move the macro in the host. In a project file, a macro binding's
  source is `{"type": "macro", "number": 3}`.
- One macro can drive several inputs, each with its own range: bind it
  more than once.

> **Try this:** on a new Effect, automate Macro 1 with a slow ramp from
> 0.5 down to 0 and back: the track fades out by 24 dB and comes back.

## Saving and sessions

Your host saves the plugin's state with the session: the whole project
(its nodes, wires, groups, and bindings), the audio files its Clips and
Samplers play, and the macros' positions. Reopening the session, or
sending it to someone else, brings the patch back exactly, sounds
included, even where the original files aren't. A session with long
samples in its patches is bigger for it.

If a saved state can't be used (it's damaged, or it was saved by a newer
version of Audio Constructor), the plugin starts with its default project
instead and says why; it never takes the host down with it.

## Editing the patch

The plugin's window holds the same editor as the app, on the patch the
plugin is playing. Edits are heard as you make them, as in the app,
without a gap. Open a project (`.acproj`) there to load it into the
plugin, with the audio files it uses; New goes back to the default patch.

A few things are the host's job inside a plugin:

- **The transport.** The patch plays, stops, and jumps with the host's
  transport, so Patterns, Clips, and anything set to start on play follow
  the session. If the host has no transport, the patch just runs.
- **Tempo.** The patch keeps its own project tempo. Set it to your
  session's tempo for tempo-synced parts (Clocks, LFOs in sync, delays in
  beats) to line up.
- **Sound devices.** The host owns the audio and MIDI ports: Audio In and
  Audio Out are the track's, MIDI In gets the track's notes, and the
  computer keys, gamepads, and OSC don't reach a plugin.

A few nodes behave differently in a plugin: **Record** doesn't record
(record the track in your host instead), and **File Out** writes files
only when `audioc render` renders a project.

## Performance

The plugin runs the patch on the host's audio thread, at the host's sample
rate and buffer size, and reports any latency the patch adds (such as a
lookahead limiter's) so the host can line tracks up. A heavy patch costs
what it costs in the app; the host's own meter shows it.
