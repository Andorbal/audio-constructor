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
- Right-click a socket: disconnect, bind to MIDI CC / key / gamepad control
  (MIDI learn), expose as a group input, add a Reroute.
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
| Connect selected to new node | select several nodes, add a `Mix`: the inputs are wired in selection order |
| Lazy connect | `F` with two selected nodes wires the first compatible sockets |
| Reroute | `Shift+right-drag` across a wire |
| Delete | `X`/`Delete`; `Ctrl+X` deletes and reconnects through (keeps the chain) |
| Duplicate | `Shift+D` (with internal wires), `Ctrl+Shift+D` (also keeps incoming wires) |
| Copy/paste | `Ctrl+C`/`Ctrl+V` as JSON text; works across projects and into chat |
| Mute | `M` bypasses the node (passes `in` → `out` where types match) |
| Collapse | `H`; `Ctrl+H` collapses all unwired sockets |
| Group | `Ctrl+G` makes a group of the selection, exposing dangling wires as sockets; `Tab` enters/leaves a group; `Ctrl+Alt+G` ungroups |
| Frame | `Ctrl+J` puts a labeled frame around the selection; frames move their contents |
| Align / distribute | `Alt+A` menu; **auto-layout selection** `Ctrl+L` (layered, left-to-right) |
| Undo/redo | `Ctrl+Z`/`Ctrl+Shift+Z`, unlimited, including parameter nudges (coalesced) |
| Find | `Ctrl+F` finds nodes by name/type and jumps to them |
| Help | `F1` on a node opens its reference in the side panel |
| Pin to a panel | `P` pins the selected controls and displays to the current panel (`P` again unpins) |
| Panels | `Ctrl+Tab` switches between the canvas and the panels; `F11` performance mode (`Esc` or `F11` leaves) |

Wire routing: béziers by default, with a "straight with bends" option. Wires
behind nodes are dimmed. Long wires can be turned into named **portals**
(a pair of Reroutes that are drawn as labels instead of a line) to tame
spaghetti.

## Side panel

A resizable panel (`N` toggles) with tabs:

- **Node**: everything about the selected node: full parameter list, docs,
  bindings, timing, latency.
- **Library**: nodes by category and user/shared blocks; drag to the canvas.
  Search here is the same engine as the palette.
- **Project**: sample rate, length, tempo map, seed, assets, missing files.
- **Devices**: audio/MIDI/gamepad devices, buffer size, measured latency,
  xrun counter, per-core load.
- **Problems**: compile errors and disabled nodes, click to jump.

## Transport bar

Play/stop/record, position in bars:beats and time, tempo and time signature
(click to edit the tempo map), loop toggle and range, metronome toggle (a
built-in block), master meter, CPU and xrun indicator. `Space` plays/stops,
`R` records, `Shift+Space` plays from the start. At its right end, a
**Canvas | Panels** switch and the performance-mode button.

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
  position, the panel tabs, and Exit stay. `Esc` or `F11` leaves, and so
  does taking the window out of full screen.

## Bindings

Any `Bool`/`Trigger`/`Float` input or control node can be bound to a MIDI
CC/note, a computer key, or a gamepad control. **Learn** mode: right-click →
Bind → move the control. Bindings are stored in the project and listed in the
Project tab. Gamepad buttons can be bound to transport actions (A = play,
B = stop, X = record) in the Devices tab.

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
