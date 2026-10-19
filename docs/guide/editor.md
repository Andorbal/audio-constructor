# The editor

The editor is a workbench: a canvas where you place nodes and wire them
together, with the transport above and the side panel beside it. This
chapter covers every way of working on the canvas. Everything you do is
undoable, and the sound keeps playing while you edit: changing a patch
never stops the music.

Every command is in a menu, with its shortcut beside it, and
**Help ▸ Keyboard Shortcuts** (`Ctrl+/`) lists them all. A full table is
at the [end of this chapter](#all-shortcuts).

## Moving around the canvas

| To | Do |
| --- | --- |
| Pan | Drag with the middle mouse button, hold `Space` and drag, or scroll with two fingers on a trackpad |
| Zoom | Mouse wheel, pinch on a trackpad, or `+` and `-` |
| See everything | `Home` (View ▸ Frame All) |
| See the selection | `.` (View ▸ Frame Selected) |

The canvas zooms from 10 % to 300 %; the current zoom is shown in its
bottom-right corner. When the patch is bigger than the window, a
**minimap** appears in that corner: dots for the nodes in their category
colors, an outline for what you can see, and the selection highlighted.
Click or drag in it to move there. `Shift+M` (View ▸ Minimap) hides it or
shows it again.

## Adding nodes

Press `Shift+A`, or double-click an empty spot on the canvas. The **add
palette** opens where the pointer is:

1. Type part of a name. The search is forgiving and knows other words for
   things: "volume" finds Gain, "lpf" finds Biquad, "echo" finds Delay.
2. Use `↑` and `↓` to choose; the line at the bottom describes the
   highlighted node.
3. Press `Enter` (or click) to add it. `Esc` closes the palette.

The palette lists nodes, the groups already in your project, and the
blocks in your library. Two other ways to add things:

- **The Add menu** lists every node by category.
- **The Library tab** in the side panel lists nodes by category and every
  block. Drag one onto the canvas, or click it to add it in the middle of
  the view.

## Wiring

Drag from a socket to another socket to connect them. Outputs are on the
right of a node, inputs on the left. While you drag, the wire snaps to the
nearest socket it can connect to, and you can also drop it on a node's
card to let the editor pick the right socket.

- **An input takes one wire.** Wiring into an input that already has one
  replaces it. An output can feed as many inputs as you like.
- **Drop on empty canvas** to open the palette showing only nodes that
  can take (or feed) that wire. The node you pick is added there and wired
  up.
- **Drop a node on a wire** to splice it in: drag an unwired node onto a
  wire, and if the types fit it is inserted between the two ends.
- **Disconnect** by dragging a wire's end off its input and letting go, by
  right-clicking the socket or the wire and choosing **Disconnect**, or by
  holding `Ctrl` and dragging across one or more wires to cut them.

Wires show what is flowing: audio wires shimmer and brighten with the
level, and trigger and MIDI wires pulse when an event passes.

### Sockets and types

A socket's color and shape tell you what it carries. Round sockets carry
a continuous stream, diamonds carry events, and squares carry larger
things.

| Type | Color | Shape | Carries |
| --- | --- | --- | --- |
| Audio | amber | circle | Sound, one or more channels |
| Float | sky blue | circle | A number, such as a frequency, a level, or a position |
| Int | green | circle | A whole number, such as a step or a note number |
| Bool | pink | circle | On or off: a gate, a switch |
| Trigger | yellow | diamond | A moment: a beat, a button press, a note start |
| Midi | violet | diamond | MIDI notes and controllers |
| Spectrum | cyan | square | Frequency content from an FFT |
| Buffer | sand | square | A whole recording, for a Sampler |

Some connections convert on their own:

- a number into an audio input becomes a signal at that value, so an
  Oscillator (whose output is a Float) plays through any audio input;
- audio into a number input uses the audio as a moving value (its first
  channel), so a sound can drive a parameter;
- Int and Bool go into Float inputs as numbers (on is 1, off is 0);
- mono audio into a stereo input is copied to both sides.

Anything else needs a node in between, because there's a choice to make:
**Gate to Trigger** and **Trigger to Gate** turn gates and moments into
each other, and **MIDI Note** and **MIDI CC** turn MIDI into numbers.
While you drag, a wire only snaps to sockets that can take it; the editor
won't connect types that don't fit, and the status line says why.

**Hover any socket** to see its type, unit, range, what it's for, and its
value right now (a level in dBFS for audio, a count for events).

### Inline controls

An input that isn't wired shows a control for its value:

- **Numbers**: drag left and right to change them, hold `Ctrl` for fine
  steps or `Shift` for coarse ones, or click and type. Frequencies move
  on a log scale, so the low end doesn't rush by.
- **Choices** (a filter's mode, an oscillator's shape): click and pick.
- **On/off**: a checkbox.
- **Triggers**: a small button that fires it once.

A wired input shows the value arriving on the wire instead.

Generators such as Oscillator, Clip, and Sampler have a `start` input.
Unwired, it says when they start by themselves: **On play** (when the
transport starts, the usual choice), **On load** (as soon as the project
opens, and keep running), or **Never** (only when something wires a
trigger into it). `stop` works the same way.

### The Node tab

Select a node to see everything about it in the side panel's **Node** tab:

- its name, category, and id; a **Label** field (the name shown on the
  card); and **Muted**;
- its processing **Time** (see [Node timing](#node-timing));
- an **Editor** for nodes you draw rather than type: the curve of a Graph
  or a Shaper, the grid of a Pattern, the piano roll of a MIDI Pattern,
  the bands of a Band Split, the steps of a StepSequencer. **Larger
  editor** opens it in its own window;
- every **Input**, with its control or the wire feeding it, the node's
  **Bindings** (see [Performing](performing.md#bindings)), and every
  **Output** with its value right now;
- **Settings**: choices that change the node's sockets or what it loads or
  sends to, such as a Clip's audio file (**Choose…**), a Mix's number of
  inputs, a Delay's unit (seconds or beats), or an OSC node's address;
- for a group, its definition (see [Groups](#groups));
- **Help**: the node's description and its "Try this" tip, with a link to
  its full page in the Help tab.

## Selecting, moving, and changing nodes

| To | Do |
| --- | --- |
| Select | Click a node; `Shift`-click adds or removes; drag a box on empty canvas (`Shift` adds to the selection) |
| Select all / none | `A` / `Alt+A` |
| Select what's connected | `L` selects every node connected to the selection, directly or through other nodes |
| Move | Drag, or press `G` and move the pointer, then click to drop (right-click puts them back). Hold `Ctrl` to snap to the grid |
| Rename | `F2`, or double-click the title, or type in the Node tab's Label field |
| Copy and paste | `Ctrl+C`, `Ctrl+V`. Copies are plain text, so they paste into another project, or into a chat message to share |
| Duplicate | `Shift+D` copies the selection with the wires between its nodes; `Ctrl+Shift+D` also keeps the wires coming in |
| Delete | `X`, `Delete`, or `Backspace` |
| Delete and keep the chain | `Ctrl+X` removes the node and wires what fed it into what it fed |
| Mute | `M`: the node is bypassed, passing its input through where the types match |
| Collapse | `H` turns the node into a small pill; the arrow at the left of its title does the same |
| Hide unwired sockets | `Ctrl+H` |

Right-click a node, a socket, a wire, or the empty canvas for a menu of
what you can do with it.

## Frames and notes

- **Frames** label a part of the patch. Select some nodes and press
  `Ctrl+J` (Add ▸ Frame Selection). Dragging a frame's title moves
  everything inside; clicking it selects what's inside; dragging its
  bottom-right corner resizes it. Double-click the title (or right-click
  ▸ Rename Frame) to name it. Right-click ▸ Remove Frame removes the
  frame and keeps the nodes.
- **Notes** are Comment nodes: add one from the palette ("note" or
  "comment"), double-click it to type, and drag its bottom-right corner to
  resize it. The example projects explain themselves this way.

## Groups

A group packs part of a patch into one node with its own sockets: a whole
synth voice, an effect chain, a channel strip.

1. Select the nodes and press `Ctrl+G` (Add ▸ Make Group). Wires that
   crossed the edge of the selection become the group's sockets.
2. Press `Tab` to go inside (or double-click the group, or right-click ▸
   Enter Group). Press `Tab` again to come back out.
3. Inside, the group's inputs and outputs are two cards at the edges. Drag
   a wire to the empty socket on one of them to add a socket. Right-click
   an inner socket ▸ **Expose as Group Input** (or Output) does the same;
   right-click a socket on the cards to rename or remove it.
4. `Ctrl+Alt+G` (Add ▸ Ungroup) puts the nodes back on the canvas.

Copies of a group (`Shift+D`) share one inside: change the inside of one
and every copy follows. The Node tab of a group shows its name, how many
copies there are, a **Polyphonic** checkbox that turns it into a
[Voices](building-sounds.md#polyphony-with-voices) node, and **Save to
library**, which makes it a [block](blocks.md).

## Undo

`Ctrl+Z` undoes and `Ctrl+Shift+Z` redoes, without limit. Everything is
undoable: adding, wiring, moving, settings, and value changes (one drag
of a knob is one step). The Edit menu says what the next undo will undo
("Undo Add Biquad").

## Find

`Ctrl+F` (Edit ▸ Find…) opens a search bar over the canvas. It finds nodes
by label, by kind ("delay"), or by id, inside groups too, as well as
portals by name and notes by their text. Use `↑` and `↓` to choose and
`Enter` to jump there (entering groups as needed, selecting and centering
the node). The bar stays open for the next result until `Esc`.

## Tidying up

- **Auto-layout** (`Ctrl+L`, Edit ▸ Auto-Layout) arranges the selection,
  or the whole patch when nothing is selected, in columns that follow the
  signal from left to right. Frames move with their nodes. It's one undo
  step, so `Ctrl+Z` puts everything back.
- **Align and distribute** (`Shift+Alt+A` opens a menu at the pointer;
  also Edit ▸ Align / Distribute, and the right-click menu with several
  nodes selected): line up left edges, centers, right edges, tops,
  middles, or bottoms, or space the nodes evenly across or down.

## Quick wiring

- **Connect selected** (`F`): with two nodes selected, wires the left
  one's first output into the first input of the right one that takes it
  (matching names win: `out` goes to `in`, `midi` to `midi`). Press `F`
  again for the next free input. The status line says what was wired.
- **Wire into a new node**: select two or more nodes, then add a node with
  `Shift+A` or the Add menu. The selected nodes are wired into it in the
  order you selected them, and it is placed to their right. Select four
  oscillators, add a Mix, and the Mix grows to four inputs, each wired.

## Reroutes and portals

- **Reroutes** are dots that bend a wire where you want it. Hold `Shift`
  and right-drag across one or more wires to add them, or right-click a
  wire ▸ **Add Reroute Here**.
- **Portals** replace a long wire with two small tags: a *sender* by the
  source and a *receiver* by the destination, with no line between them.
  Right-click a wire ▸ **Make Portal**, or select nodes and press
  `Shift+P` to turn the wires between them into portals. One sender can
  feed many receivers. Hover a tag to light up its other end, double-click
  it to jump there, and press `F2` to rename the portal. `Shift+P` on
  selected tags (or right-click ▸ **Back to Wire**) turns them back into a
  wire. A tag whose other end is gone is outlined in the warning color.

Reroutes and portals change nothing about the sound.

## Node timing

`Ctrl+T` (View ▸ Node Timing) shows in each node's title how long it takes
to compute each buffer, for example `37 µs`, on a chip colored from cool to
hot by how much of the time budget it uses. Hover the chip for its share.
A group shows the total of everything inside it, and a Voices node the
total of all its voices. It's off by default because measuring costs a
little; the Node tab has a **Measure** button that turns it on.

## Badges

Small icons in a node's title tell you about its state. Hover one for the
details.

| Badge | Meaning |
| --- | --- |
| Red warning circle | An error: the node can't work as wired (the message says why) |
| Yellow warning triangle | A warning, such as a binding that can't act |
| Grey crossed circle | **Disabled**: the node needs its whole input and a live source feeds it (see [Live and baked](live-and-baked.md)) |
| Clock | **Baking**: its input is being rendered in the background |
| Green check | **Baked**: it plays a render of its whole input |
| Timer | It adds latency, in samples |
| Stack | A Voices node, with its number of voices |
| Crossed-out file | A Clip whose audio file is missing |
| Question mark | A node kind this version doesn't know; it's kept, with its wires |
| Pin | Pinned to a [panel](performing.md#panels) |
| Crossed-out speaker | Muted |

## The side panel

`N` (View ▸ Side Panel) shows or hides it; drag its edge to resize it.
When it's narrow, tabs show only their icons.

| Tab | What's in it |
| --- | --- |
| **Node** | The selected node: label, settings, every input and output, bindings, timing, and help |
| **Library** | Every node by category, and the blocks in your library, with search, origin filters (Shipped, Mine, Imported), and tags; see [Blocks and sharing](blocks.md) |
| **Project** | Name, sample rate, length, render block size, and random seed; the tempo map and loop; the project's bindings; library blocks with newer versions; the latency the patch adds; and its audio files, with **Locate…** for missing ones |
| **Devices** | Audio devices, buffer, load, xruns, and latency; MIDI inputs and outputs; gamepads; OSC; and transport buttons |
| **Problems** | Everything the patch can't do as wired, and nodes that are disabled while live; click one to jump to the node. The tab shows how many there are |
| **Help** | A node's reference (`F1`) or the list of every node, with search |

## Theme, language, and size

- **View ▸ Theme** picks **Dark** (the default), **Light**, **High
  Contrast** (white on black, strong outlines, and a yellow focus ring),
  or **Follow System**, which follows your system's light or dark setting
  while the editor runs. Your own themes, small files in a themes folder,
  are listed below them; [Themes](themes.md) says how to make one.
- **View ▸ Language** picks the language of the editor's menus, panels,
  and messages: **System** (your computer's language if the editor has
  it, else English), **English**, or **Pseudo**, a made-up language for
  checking translations (accented letters in brackets). Node names and
  their help stay in English for now.
- **View ▸ UI Scale** sizes the whole interface from 75 % to 200 %.
  `Ctrl+=` and `Ctrl+-` step up and down and `Ctrl+0` goes back to 100 %,
  even while typing. The canvas zoom (`+`, `-`, the wheel) is separate.

All of them are remembered the next time you start the editor.

## All shortcuts

The same list is in Help ▸ Keyboard Shortcuts. On macOS, shortcuts with
`Ctrl` use `Cmd`.

| Command | Shortcut |
| --- | --- |
| New, Open, Save, Save As | `Ctrl+N`, `Ctrl+O`, `Ctrl+S`, `Ctrl+Shift+S` |
| Export WAV | `Ctrl+E` |
| Quit | `Ctrl+Q` |
| Undo, Redo | `Ctrl+Z`, `Ctrl+Shift+Z` |
| Copy, Paste | `Ctrl+C`, `Ctrl+V` |
| Duplicate, Duplicate with Inputs | `Shift+D`, `Ctrl+Shift+D` |
| Delete, Delete and Reconnect | `X` (or `Delete`, `Backspace`), `Ctrl+X` |
| Select All, Select None, Select Linked | `A`, `Alt+A`, `L` |
| Find | `Ctrl+F` |
| Move (grab) | `G` |
| Rename | `F2` |
| Mute, Collapse, Hide Unwired Sockets | `M`, `H`, `Ctrl+H` |
| Make Portal / Back to Wire | `Shift+P` |
| Connect Selected | `F` |
| Auto-Layout | `Ctrl+L` |
| Align / Distribute menu | `Shift+Alt+A` |
| Pin to Panel | `P` |
| Add Node | `Shift+A` |
| Frame Selection | `Ctrl+J` |
| Make Group, Ungroup | `Ctrl+G`, `Ctrl+Alt+G` |
| Enter / Leave Group | `Tab` |
| Node Help | `F1` |
| Frame All, Frame Selected | `Home`, `.` |
| Zoom In, Zoom Out | `+`, `-` |
| Bigger, Smaller Interface, Interface at 100 % | `Ctrl+=`, `Ctrl+-`, `Ctrl+0` |
| Side Panel | `N` |
| Node Timing | `Ctrl+T` |
| Minimap | `Shift+M` |
| Canvas / Panels | `Ctrl+Tab` |
| Performance Mode | `F11` |
| Play / Stop | `Space` |
| Play from Start | `Shift+Space` |
| Record | `R` |
| Play Keys | `Ctrl+K` |
| Keyboard Shortcuts | `Ctrl+/` |

Transport ▸ Loop and Transport ▸ Go to Start have no shortcut; use the
transport bar's buttons.

Next: [Building sounds](building-sounds.md).
