# UX

The editor is the product. Wiring hundreds of nodes gets tedious fast, so the
job of the UI is to make the common thing one gesture and the rare thing
discoverable. Blender's node editor is the reference point for interaction
patterns; the look is our own.

## Principles

1. **Everything on screen is a node or a wire, or a view of them.** No modal
   editors, no hidden state. The transport bar, panels, and the library are
   views.
2. **Minimal by default, rich on demand.** A node shows its name, sockets,
   and inline controls. Hover or expand for help, timings, and details.
3. **Keyboard first, mouse always.** Every action has a shortcut; every
   shortcut has a menu entry.
4. **Live everywhere.** Meters in sockets, values on hover, wires that flow,
   so the graph *is* the visualization.
5. **Forgiving.** Unlimited undo, nothing is destructive, errors are shown in
   place and never block the rest of the graph from running.
6. **Fun.** Motion with purpose (easing, not bouncing), satisfying snaps,
   tasteful optional UI sounds, and a canvas that feels like a workbench, not
   a form.

## Look

- Own look on every platform, like Blender. Dark theme by default; light
  theme later. All colors, radii, spacing, and type scale live in one theme
  module (`ac-app/src/theme.rs`) so the look can be tuned in one place.
- Typeface: Inter for UI, JetBrains Mono for values. Icons from a single
  consistent set (Phosphor).
- Nodes: rounded cards with a category-colored header strip, a subtle shadow,
  and a slightly stronger outline when selected. Collapsed nodes become a pill
  showing only name and sockets.
- Wires: cubic béziers colored by signal type, 2 px, with a soft glow on
  hover/selection. Audio wires show a subtle flowing texture when signal is
  present; the brightness follows level. Event wires (Trigger, Midi) pulse
  when an event passes.
- Canvas: dotted grid, zoom from 10 % to 300 % with smooth steps, minimap in a
  corner when the graph exceeds the viewport.

### Socket colors and shapes

Shape distinguishes stream from event (and is the colorblind-safe channel).

| Type | Color | Shape |
| --- | --- | --- |
| Audio | amber `#F0A030` | circle |
| Float | sky `#6FB1E6` | circle |
| Int | green `#5FC88F` | circle |
| Bool | pink `#E06C9F` | circle |
| Trigger | yellow `#F2D15C` | diamond |
| Midi | violet `#A98BF0` | diamond |
| Spectrum | cyan `#4FD1D9` | square |
| Buffer | sand `#C8A878` | square |

Category header colors: I/O red, Generators orange, Level/Dynamics gold,
Filters green, Time teal, Spectral cyan, Math blue, Modulation indigo,
Triggers yellow, Sequencing violet, Controls pink, Display grey, Utility
slate.

## Node anatomy

```
┌─ Oscillator ──────────── 0.12 ms ─┐   ← header: name (editable), timing (optional), menu
│ ○ freq   [ 440.0 Hz ]      out ○  │   ← unwired input shows inline widget; wired input shows the value
│ ○ shape  [ Saw      ▾ ]           │
│ ○ phase  [ 0.00     ]             │
│ ◇ start  [ On play  ▾ ]           │   ← trigger input with its "start on" dropdown
│ ◇ sync                            │
└───────────────────────────────────┘
```

- Unwired inputs show the right widget for their unit: drag-slider for
  floats (log scale for Hz, dB), dropdown for enums, checkbox for bools, a
  small button for triggers. Ctrl-drag for fine, Shift-drag for coarse, click
  to type.
- Wired inputs show the live value in muted text; hover a socket to see it
  large, with rate (const/block/sample) and channel count.
- Right-click a socket: disconnect, **Bind…** to a MIDI controller or note,
  key, gamepad control, or OSC address (learn; see Bindings), remove its
  bindings, expose as a group input, add a Reroute.
- Badges in the header: **disabled** (whole-input node fed by a live
  source), **baking…/baked**, **error** (type/channel mismatch, with the
  message on hover), **latency** (samples this node adds), **muted**.

## Canvas interactions

| Action | Gesture |
| --- | --- |
| Pan | middle-drag, space+drag, two-finger scroll |
| Zoom | wheel, pinch, `Home` to fit all, `.` to fit selection |
| Select | click, shift-click, box-drag, `A` all, `Alt+A` none, `L` linked |
| Move | drag selected; `G` grab (Blender style); snaps to grid with Ctrl |
| Add node | `Shift+A` or double-click empty canvas: search palette (fuzzy, synonyms: "volume" finds Gain, "lpf" finds Biquad) |
| Wire | drag from socket to socket; drop on empty canvas opens the palette **filtered to compatible nodes** and auto-wires the result |
| Insert on wire | drop a node onto a wire to splice it in (if types match) |
| Disconnect | drag a wire end away and release, or `Ctrl+drag` across wires to cut |
| Connect selected to new node | select two or more nodes, then add a node (`Shift+A` or the Add menu): they're wired into it in selection order and it goes to their right (see below) |
| Lazy connect | `F` with two selected nodes wires the left one into the right one (see below); the status line says what was wired, or why not |
| Reroute | `Shift+right-drag` across a wire |
| Delete | `X`/`Delete`; `Ctrl+X` deletes and reconnects through (keeps the chain) |
| Duplicate | `Shift+D` (with internal wires), `Ctrl+Shift+D` (also keeps incoming wires) |
| Copy/paste | `Ctrl+C`/`Ctrl+V` as JSON text; works across projects and into chat |
| Mute | `M` bypasses the node (passes `in` → `out` where types match) |
| Collapse | `H`; `Ctrl+H` collapses all unwired sockets |
| Group | `Ctrl+G` makes a group of the selection, exposing dangling wires as sockets; `Tab` enters/leaves a group; `Ctrl+Alt+G` ungroups |
| Frame | `Ctrl+J` puts a labeled frame around the selection; frames move their contents |
| Align / distribute | `Shift+Alt+A` menu at the pointer (also Edit ▸ Align / Distribute, and a node's right-click menu with several selected); **auto-layout** `Ctrl+L` lays out the selection, or the whole graph with nothing selected (layered, left to right) |
| Undo/redo | `Ctrl+Z`/`Ctrl+Shift+Z`, unlimited, including parameter nudges (coalesced) |
| Find | `Ctrl+F` finds nodes by name, kind, or id, inside groups too, and jumps to them: `↑`/`↓` choose, `Enter` jumps, `Esc` closes |
| Help | `F1` on a node opens its reference in the side panel |
| Pin to a panel | `P` pins the selected controls and displays to the current panel (`P` again unpins) |
| Panels | `Ctrl+Tab` switches between the canvas and the panels; `F11` performance mode (`Esc` or `F11` leaves) |

Wire routing: béziers by default, with a "straight with bends" option. Wires
behind nodes are dimmed. Long wires can be turned into named **portals**
(a pair of Reroutes that are drawn as labels instead of a line) to tame
spaghetti.

### Arranging and quick wiring

Each of these is one undo step; moving cards never interrupts the sound.

- **Auto-layout** (`Ctrl+L`) arranges the selection, or the whole graph
  when nothing is selected, in columns that follow the wires from left to
  right, keeping its top-left corner where it was. A loop is drawn from
  its start to its `Feedback` node, with the wire back from there.
  Sources and modulators sit just before what they drive. Separate
  pieces of the graph stack in the order they were in, and unwired cards
  (notes, mostly) line up in a row above or below, on the side they were
  on. A frame whose nodes are all being laid out moves and resizes with
  them; a frame only partly selected stays put. Cards keep their sizes:
  collapsed pills, reroutes, and notes take the room they're drawn with.
- **Align / distribute** (`Shift+Alt+A`, since `Alt+A` is select none):
  align left, center, right, top, middle, or bottom, or distribute
  horizontally or vertically with equal gaps (the two outer cards stay).
  Only nodes move; frames stay where they are.
- **Lazy connect** (`F`, two nodes selected) wires the left node's first
  output into the first free input of the right node that takes it, in
  socket order (main sockets come first), unless a pair's names match
  (`out` → `in`, `out_l` → `in_l`, `midi` → `midi`); among one output's
  choices, an input of the same type beats one that converts. If nothing
  fits left to right, it tries right to left. It never makes a loop
  without a `Feedback` node. Press it again for the next free input.
- **Connect selected to new node**: with two or more nodes selected,
  the node added from the palette (`Shift+A`, which says so) or the Add
  menu is wired from them, each one's first output in the order they
  were selected (nodes selected together, by a box or `A`, count top to
  bottom), into the new node's inputs of one type: the first type the
  first selected node fits, so two oscillators go into a `Mix`'s `in_0`
  and `in_1`, never its gains or a filter's cutoff. A `Mix` (or `Select`)
  grows inputs to fit. The new node goes to their right. If any selected
  node doesn't fit, nothing is wired, the node goes where it was added,
  and the status line says why.
- **Find** (`Ctrl+F`) searches labels, kind names, and ids (from their
  start) in every graph of the project, and notes by their text. A node
  inside a group shows the group it's in, once per instance of it.
  `Enter` enters the group, selects the node, and centers it; the bar
  stays open for the next result until `Esc` or a click elsewhere.

## Side panel

A resizable panel (`N` toggles) with tabs:

- **Node**: everything about the selected node: full parameter list, docs,
  bindings, timing, latency.
- **Library**: nodes by category and user/shared blocks; drag to the canvas.
  Search here is the same engine as the palette.
- **Project**: sample rate, length, tempo map, seed, bindings, assets,
  missing files.
- **Devices**: audio/MIDI/gamepad devices, buffer size, measured latency,
  xrun counter, per-core load, OSC (a checkbox and port to listen on, 9000
  unless changed and off until turned on; the address to send to; what has
  arrived and what OSC Out nodes sent), and transport buttons.
- **Problems**: compile errors and disabled nodes, click to jump.

## Transport bar

Play/stop/record, position in bars:beats and time, tempo and time signature
(click to edit the tempo map), loop toggle and range, metronome toggle (a
built-in block), master meter, CPU and xrun indicator. `Space` plays/stops,
`R` records, `Shift+Space` plays from the start. At its right end, a
**Canvas | Panels** switch and the performance-mode button.

The keyboard button (also Transport ▸ Play Keys, `Ctrl+K`) turns on **Play
keys**: computer keys go to the patch (`KeyboardIn` nodes, key bindings)
instead of running canvas shortcuts. It's off by default because the
canvas's shortcuts are single keys. While it's on, `Escape` and shortcuts
with `Ctrl`/`Cmd`/`Alt` still work, text fields still get typing, holding a
key doesn't repeat it, and every held key is released when Play keys turns
off, a text field takes focus, or the window loses focus. The status bar
says it's on. Performance mode turns Play keys on by itself (below); the
panel view in the editor doesn't, since you still arrange cards and use
`Space` for the transport there.

## Panels (the "performance view")

Any control or display node can be **pinned** to a panel (`P`). A panel is a
grid of the pinned nodes' widgets without the graph: buttons, sliders, a
keyboard, meters, scopes. Panels are views: they contain nothing that isn't
a node. Several panels can exist (Live, Mixer, Debug); `Ctrl+Tab` switches
between the canvas and the panels; a panel can be shown full-screen for a
performance. This is how a user builds a mixer window, a drum pad, or a
pedalboard without the app providing one.

- **Pinning.** `P` pins the selected nodes that have a widget to the
  current panel, making "Panel 1" if there is none; on nodes already
  pinned there, `P` unpins them. Widgets are the controls (Button, Toggle,
  Slider, Knob, XY Pad, Keyboard, Number, Dropdown), the displays (Meter,
  Scope, Analyzer, Readout), and Clip, Record, and Audio Out. A node's
  right-click menu pins it to any panel or a new one, and unpins it. Pinned
  nodes carry a pin badge in their header. Only top-level nodes can be
  pinned (the menu says so inside a group): a group's insides are shared by
  all its instances, so a pin there couldn't say which one it plays.
  Deleting a node, or grouping it, takes it off every panel in the same
  undo step.
- **Showing panels.** `Ctrl+Tab`, View ▸ Canvas / Panels, or the switch in
  the transport bar shows the panels in place of the canvas. Tabs across the
  top show one panel each: click to show it, double-click to rename it,
  right-click to rename, move left or right, or delete it; `+` adds a panel
  and names it. While the panels show, shortcuts that act on the graph are
  off; transport, undo, and file shortcuts still work.
- **The grid.** Each card has the node's label and its live widget, played
  exactly as on the canvas: dragging a slider is one undo step, a held
  button or key is momentary and never touches the project, and meters,
  scopes, analyzers, and readouts move with the audio. Cards flow left to
  right in pin order; the Keyboard, Scope, Analyzer, and XY Pad get bigger
  cells. Drag a card by its edge (anywhere but its widget) to move it;
  `−` and `+` in its corner make it smaller or bigger, and `×` unpins it.
  Each is one undo step. A panel taller than the window scrolls.
- **Performance mode.** `F11`, View ▸ Performance Mode, or the Perform
  button shows the current panel full screen with bigger cells. The menu
  bar, side panel, status bar, and transport bar go; play/stop, the
  position, the panel tabs, a Play keys button, and Exit stay. `Esc` or
  `F11` leaves, and so does taking the window out of full screen.
  Performance mode turns **Play keys** on, so the computer keys play the
  patch (`Space` too: use the play button, or `Ctrl+K` or the keyboard
  button to turn Play keys off); leaving puts Play keys back the way it
  was. `Esc` and `F11` always leave, even with Play keys on.

## Bindings

Any unwired `Float`, `Int`, `Bool`, or `Trigger` input can be bound to a
MIDI controller or note, a computer key, a gamepad button or axis, or an
OSC address, and transport actions can be bound the same way. Bindings are
stored in the project, so the editor, `audioc run`, and offline renders
all honor them (DECISIONS D92).

- **Learn**: right-click an input → **Bind…**, then move the control. The
  status line says what's listening ("move a control: MIDI, key, gamepad,
  or OSC… Esc cancels"). Notes, keys, and buttons bind on press; a
  controller, stick, or OSC value binds once it really moves (8 % of its
  span), so jitter or a resting stick doesn't bind. A key pressed on the
  canvas while learning is a candidate too, and runs no shortcut. The new
  binding is one undo step. Control nodes (Slider, Knob, Toggle, Button,
  Number, Dropdown) offer **Bind…** in their title's menu for their main
  value; an XY Pad offers **Bind X…** and **Bind Y…**.
- **Mapping**: controllers and sticks sweep the input's range (its slider's
  range, on a log scale where the slider is log); notes press with their
  velocity, keys and buttons press fully; triggers fire once per press; a
  `Bool` follows the control (momentary) or flips on each press (toggle).
  Pressed controls learned for a `Bool` start as toggles, except inputs
  meant to be held (a Button's `pressed`, a keyboard key, `touched`,
  `gate`). Each binding can set its own range and be inverted (D93).
- **On the canvas** a bound input shows a small link left of its socket
  (in the warning color if the input is wired, since a wired input ignores
  its binding); hovering it names the control. While the patch plays, the
  input's widget follows the control, on the canvas and on performance
  panels alike. That isn't an edit: nothing is added
  to undo, the project isn't marked changed, and the stored value is what's
  saved. Moving the widget by hand takes over until the control moves
  again. Right-click a bound input → **Remove Bindings**.
- **Project tab** lists every binding: the control, what it drives (click
  to jump to the node), its range, invert, momentary or toggle, and the
  value it last set; × removes it. The **Node tab** lists the selected
  node's bindings.
- **Inside groups** a binding targets the instance the group was entered
  through, not every instance (D94). Deleting a node removes its bindings
  in the same undo step; grouping or ungrouping keeps them with their
  nodes.
- **Transport buttons** (Devices tab): one click adds the gamepad preset,
  A = play/stop, B = stop, X = record, on any gamepad; any transport action
  (play, stop, play/stop, record, play from start) can also be learned from
  any control. Transport bindings are listed there and in the Project tab.
- OSC events reach the engine as a hash of their address, so learning can
  name an OSC address only if the project already mentions it (an OSC
  node's settings, or another OSC binding) (D95).

## Live feedback

- Socket meters: audio sockets show a small level bar; float sockets show a
  tick on their range.
- Hover a wire to see its live value or a mini-scope.
- Optional per-node timing in headers (`Ctrl+T`), heat-colored, so a slow
  patch shows where its time goes.
- Disabled, baking, and error states are visible from across the canvas.

## Onboarding and examples

- First launch opens a **Start** project with a guided patch: an oscillator
  wired to the output with comments explaining what to try.
- **Examples** menu with the shipped projects (see ROADMAP). Each is a
  working patch and a lesson: "Guitar Pedalboard", "Subtractive Synth",
  "Four Tracks", "Sidechain Pump", "Drum Machine", "Build a Keyboard",
  "Reverse Delay", "Gamepad Jam".
- Every node's help has "try this" tips and links to blocks that use it.

## Accessibility and scaling

- UI scale from 75 % to 200 %; everything is vector.
- Socket shapes carry type information in addition to color.
- All canvas actions have keyboard equivalents; the palette is fully
  keyboard driven.

## Fun, deliberately

- Wires snap into sockets with a short ease and a soft click (sound optional,
  off by default).
- A fresh node fades in where the cursor is; a deleted node shrinks away.
- The Start project ends with a "now break it" section.
- Nothing bounces, nothing takes longer than 150 ms, nothing animates while
  the CPU is busy with audio.
