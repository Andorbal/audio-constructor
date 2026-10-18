# User guide

This guide is for musicians and sound designers who want to build
instruments, effects, and other sound machines with Audio Constructor. You
don't need to know how to program. If you have ever plugged a guitar into
a pedal, a pedal into an amp, and turned the knobs until it sounded right,
you already know the idea: here the pedals are small parts called
**nodes**, the cables are **wires**, and you can take any pedal apart to
see how it works.

The guide describes the editor and the `audioc` command line as they are
in this version. For every node's sockets and settings, see the
[node reference](../reference/README.md).

## How to read it

Start with the first three chapters in order; dip into the rest when you
need them.

1. [Getting started](getting-started.md): install, first launch, your
   sound card, and saving.
2. [The editor](editor.md): the canvas, adding and wiring nodes, and every
   way to tidy, find, and arrange them.
3. [Building sounds](building-sounds.md): a first synth step by step, then
   MIDI, polyphony, modulation, effects, and spectral tools.
4. [Live and baked](live-and-baked.md): why a few nodes switch themselves
   off when you play live, what "baking" means, and latency.
5. [Blocks and sharing](blocks.md): groups, saving your own blocks, the
   library, updates, and packs.
6. [Performing](performing.md): controls, panels and performance mode,
   playing from the computer keyboard, MIDI, gamepads, and phones, and
   binding any of them to any knob.
7. [Recording and rendering](recording-and-rendering.md): recording takes,
   turning a project into audio files, and getting the same result every
   time.
8. [The command line](command-line.md): every `audioc` command, with
   examples.
9. [Examples tour](examples.md): what each example project teaches and
   what to try in it.
10. [Troubleshooting](troubleshooting.md): no sound, crackles, latency,
    devices that don't show up, and other problems.

"Try this" boxes like the one below suggest something to do with what a
section just explained:

> **Try this:** open the editor, press `Space`, and drag the Oscillator's
> `freq` value left and right while it plays.

Keys are written as they are on a PC keyboard. On macOS, menu shortcuts
written with `Ctrl` use `Cmd` instead.

## Help inside the app

- **`F1`** on a selected node opens its page in the side panel's **Help**
  tab: what it does, a highlighted "Try this" tip, every socket with its
  type and range, and the blocks and example projects that use it (click
  one to add or open it). With nothing selected, `F1` lists every node by
  category, with a search field. Help ▸ Node Help, a node's right-click
  menu, and the **Help** button in the Node tab do the same.
- **Help ▸ Keyboard Shortcuts** (`Ctrl+/`) lists every shortcut and
  mouse gesture.
- **Hover a socket** to see its type, unit, range, a description, and its
  value right now.
- **The status line** along the bottom of the window gives hints for
  what you're doing ("Drop on a socket to connect · on empty canvas to add
  a node") and says what a command did, or why it couldn't.
- **The Problems tab** lists anything the patch can't do as wired, such
  as a loop without a Feedback node. Click a problem to jump to the node.
- **The example projects** have notes on the canvas explaining every
  part. The [Examples tour](examples.md) says where to start.

## Words used in this guide

| Word | Meaning |
| --- | --- |
| Node | One small part that does one job: an oscillator, a filter, a meter. |
| Socket | A connection point on a node: inputs on the left, outputs on the right. |
| Wire | A connection from an output socket to an input socket. |
| Patch, project | The nodes and wires you build, saved as an `.acproj` file. |
| Canvas | The area where you place and wire nodes. |
| Group | Several nodes packed into one node with its own sockets. |
| Block | A group saved to the library so any project can use it. |
| Transport | Play, stop, record, and the song position and tempo. |
| Live source | Something that comes from outside as you play: a microphone, MIDI, a gamepad. |
| Bake | Render part of a patch ahead of time, in the background, so a node that needs to hear all of its input first can play it. |
| Panel | A page of knobs, buttons, and meters picked from the patch, for playing it. |
| Binding | A link from a hardware control (a MIDI knob, a key, a gamepad button) to a node's input. |

## More

- [Node reference](../reference/README.md): every node, generated from the
  same text as the Help tab.
- [Project README](../../README.md): what Audio Constructor is, and how to
  download or build it.
- [What changed in each version](../../CHANGELOG.md).
