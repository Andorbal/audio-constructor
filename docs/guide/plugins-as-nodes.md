# Plugins as nodes

Synths and effects you've installed as CLAP plugins run inside a patch as
**Plugin** nodes: play a synth from a MIDI keyboard or a pattern, send a
Clip through an effect, and wire any of their knobs to an LFO, an
envelope, or a gamepad stick like any other input.

## Finding your plugins

The editor looks for plugins each time it starts, in the standard folders
for each format:

| System | CLAP folders | VST3 folders |
| --- | --- | --- |
| Linux | `~/.clap`, `/usr/lib/clap`, `/usr/local/lib/clap` | `~/.vst3`, `/usr/lib/vst3`, `/usr/local/lib/vst3` |
| macOS | `~/Library/Audio/Plug-Ins/CLAP`, `/Library/Audio/Plug-Ins/CLAP` | `~/Library/Audio/Plug-Ins/VST3`, `/Library/Audio/Plug-Ins/VST3` |
| Windows | `%LOCALAPPDATA%\Programs\Common\CLAP`, `C:\Program Files\Common Files\CLAP` | `%LOCALAPPDATA%\Programs\Common\VST3`, `C:\Program Files\Common Files\VST3` |

Folders listed in the `CLAP_PATH` and `VST3_PATH` environment variables
(separated like `PATH`) are searched first.

The first time takes a moment: each plugin is opened once to read its
inputs, outputs, and knobs. That happens in a separate process, so a
plugin that crashes while it's looked at can't close the editor; it's
listed as one that couldn't be opened instead. After that only new or
changed plugins are opened again.

The **Devices** tab's **Plugins** section says how many were found,
which folders were searched, and which plugins couldn't be opened, and
has **Rescan** (for plugins installed since) and **Rescan all**.

> VST3 plugins are found and listed, but only CLAP plugins play as nodes
> in this version.

## Adding one

Every plugin found is in the add-node palette (`Shift+A`) under its own
name, and under **Plugins** in the Library tab. Search by name, by maker,
or by "instrument" or "effect". Adding one makes a **Plugin** node with
the plugin's sockets:

- **`in`** and **`out`**: its audio, with as many channels as the plugin
  has. A plugin with more inputs or outputs (a sidechain, separate
  outputs per voice) gets `in_2`, `out_2`, and so on.
- **`midi`**: notes and MIDI for a synth, or for an effect that follows
  notes. **`midi_out`**: notes the plugin sends, for an arpeggiator or a
  note generator.
- **An input for each knob** (up to 128), named after it: `cutoff`,
  `filter_resonance`, `osc_2_level`. Set it right on the node, or wire
  anything into it. A wired knob follows sample-accurately; one that
  moves with the signal (an LFO at audio rate) updates every 32 samples.

> **Try this:** add a synth plugin, wire a **Keyboard** node into its
> `midi` and its `out` into **Audio Out**, and play. Then wire an
> **LFO** into one of its knobs and watch the knob follow.

## The plugin's own window

Right-click the node ▸ **Show Plugin Window**, or select it and press
**Show Plugin Window** in the Node tab: the plugin's own interface opens
in a window beside the editor. Turning a knob there turns the node's
input too, so what you hear is what the project saves. Close the window
whenever you like; the plugin keeps playing.

## Saving

Saving the project saves everything the plugin keeps (its preset, its
settings), along with its knobs. Copy and paste, packs, and the command
line carry it too. One exception: a plugin inside a group saves its knobs
but not the rest of its state, since every instance of the group shares
the node.

## When a plugin is missing

Open a project on a computer without one of its plugins and the project
still opens: the Plugin node keeps its sockets and wires, plays silence,
and the Problems tab (and the Node tab) says which plugin is missing.
Install it, press **Rescan** in the Devices tab, and it plays again.

## Latency

A plugin that delays its sound (a look-ahead limiter, a linear-phase EQ)
says by how much, and the engine delays the other paths where they meet
so everything stays in time, as it does for its own nodes (see
[Live and baked](live-and-baked.md)).

## From the command line

`audioc plugins` lists the plugins it finds (`--json` for scripts), and
`audioc render`, `run`, and `validate` play the plugins a project uses.
See [The command line](command-line.md#plugins).
