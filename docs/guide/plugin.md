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
- Macro bindings are saved in the project like any other. In a project
  file, a macro binding's source is `{"type": "macro", "number": 3}`;
  `min`, `max`, and `invert` work as for a MIDI controller. In the
  [editor](#the-editor), you learn one like any control: right-click an
  input, choose **Bind…**, and move the macro (in the host, or on its
  slider in the Devices tab).
- One macro can drive several inputs, each with its own range: bind it
  more than once.

> **Try this:** on a new Effect, automate Macro 1 with a slow ramp from
> 0.5 down to 0 and back: the track fades out by 24 dB and comes back.

## The editor

Open the plugin's window (the host's editor button) and you get the whole
editor, the same as the app's: the canvas, the panels, the palette, the
side panel. Edits play at once, and they're saved with your session.
Drag the window's corner to make it bigger; it opens at the size you
left it, and closing it loses nothing: undo still remembers what you did
before.

The editor is in the CLAP plugins. The VST3 plugins don't have it yet
(see [What doesn't work yet](#what-doesnt-work-yet)): in a host that can
load both, use CLAP.

A few things work differently inside a plugin:

- **File ▸ Open** loads a project into the plugin, with the audio files
  it plays, which then travel in the session. **File ▸ New** starts over
  with the plugin's default patch, and the examples open the same way.
  **Save As** writes a project file, with the audio files it plays beside
  it, for opening in the app.
- **A file you pick** for a Clip goes into the session too, so the
  session plays it wherever it's opened.
- **The transport bar** shows whether the host is playing, and has no
  play or record buttons: the host's transport plays the patch, and
  Space goes to the host.
- **The Devices tab** says what the host runs the patch at (sample rate,
  buffer, load) and lists the sixteen **macros**: what each drives, its
  value as the host shows it, and a slider. Moving a slider moves the
  host's parameter, so the host records it as automation, like moving it
  in the host.
- There's no **Quit**; close the window from the host. **Performance
  mode** (`F11` in the app, View ▸ Performance Mode) fills the plugin's
  window instead of the screen.

> **Try this:** on a new Instrument, open the window, go to the Devices
> tab, and drag Macro 1 while a note plays: the cutoff moves, and the
> host draws it into the macro's automation lane if it's recording.

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

## What the host looks after

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

## Performance

The plugin runs the patch on the host's audio thread, at the host's sample
rate and buffer size, and reports any latency the patch adds (such as a
lookahead limiter's) so the host can line tracks up. A heavy patch costs
what it costs in the app; the host's own meter shows it.

## What doesn't work yet

- **The editor in the VST3 plugins.** They have no window yet: hosts show
  their own controls for the macros, and a VST3 plugin plays its default
  patch or the patch its session saved. The CLAP plugins have the
  [editor](#the-editor).
- **Some shortcuts in the plugin's window.** The function keys (`F1`,
  `F2`, `F11`) and shortcuts that hold Ctrl (or Cmd) with Shift or a
  symbol (Redo, Save As, Duplicate with Inputs, UI scale) don't reach
  the editor; their menu entries do. Keys the editor doesn't use while
  you aren't typing go on to the host, so its own shortcuts may answer
  them too.
- **Following the host's tempo.** The patch keeps its project tempo (see
  above).
- **Recording.** Record nodes don't record in a plugin, and File Out
  writes only in `audioc render`; record the track in your host.
- **A sidechain input.** The Effect has one stereo input; a second input
  for sidechaining isn't there yet.
- **Macro names.** Hosts list the macros as Macro 1 to Macro 16 whatever
  they drive; the readout shows the value.
- **Downloads.** The release archives don't include the plugin yet, and
  the bundles aren't signed: build them as in [Install](#install).
