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

- Own look on every platform, like Blender. Dark theme by default, a
  light theme, and a high-contrast one: View ▸ Theme picks **Dark**,
  **Light**, **High Contrast**, or **Follow System** (the operating
  system's light or dark mode, followed while the editor runs), then the
  person's own themes, switching at once and remembered in the editor's
  settings by name (D107, D109, D180). All colors, radii, spacing, and type
  scale live in one theme module (`ac-app/src/theme.rs`) so the look can be
  tuned in one place: each theme is a `Palette` of every color the editor
  draws with.
- **High Contrast** is white text on black, at least 7:1 on every surface
  (WCAG AAA), with white outlines on cards, buttons, and fields, outlines
  drawn twice as thick, a yellow focus ring unlike any other color on
  screen, and socket colors brightened so types that share a shape stay
  far apart in color.
- **Theme files** (D181): JSON in `themes/` beside `settings.json`, each
  with a `name`, a `base` (`dark`, `light`, or `high_contrast`), an
  optional `header_mix`, and `colors` overriding any `Palette` color by
  its field name (`#RRGGBB` or `#RRGGBBAA`); anything left out is the
  base's. View ▸ Theme lists them after the built-ins, with **Open Themes
  Folder** and **Reload Themes**. A file that can't be used is listed in
  the menu with why (and said at startup); a misspelled name or a bad
  color is skipped and noted on hover; a theme whose body text falls
  under 4.5:1 on any surface is marked in the menu and says so when
  chosen. A saved theme whose file is gone shows Dark. The guide's
  [Themes](guide/themes.md) chapter lists every field with the built-in
  values (a test keeps it complete and current).
- Readable in both themes: body text has at least 4.5:1 contrast on every
  panel and card (the WCAG AA level), and signal colors at least 3:1 on the
  canvas and on cards; tests check both. The light theme keeps the dark
  theme's hues but deepens them (amber audio becomes a burnt orange, the
  yellow trigger an olive), and node headers mix the category color into
  white with dark titles instead of into the card with light ones.
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
- Minimap: while any of the graph is out of view, a small map sits in the
  canvas's bottom-right corner, above the zoom readout: cards as dots in
  their category colors (reroutes and portals in their wire's color),
  frames, the selection outlined, and the visible part of the canvas as an
  accent rectangle. Pressing in it moves the view there and dragging
  follows the pointer; it shows the graph of the group you're in. It hides
  itself once everything fits, and `Shift+M` (View ▸ Minimap) turns it off
  and on.

### Socket colors and shapes

Shape distinguishes stream from event (and is the colorblind-safe channel).

| Type | Color (dark) | Light theme | Shape |
| --- | --- | --- | --- |
| Audio | amber `#F0A030` | `#C46F00` | circle |
| Float | sky `#6FB1E6` | `#2B7BC0` | circle |
| Int | green `#5FC88F` | `#1F8A55` | circle |
| Bool | pink `#E06C9F` | `#C2407D` | circle |
| Trigger | yellow `#F2D15C` | `#977C00` | diamond |
| Midi | violet `#A98BF0` | `#7A55D6` | diamond |
| Spectrum | cyan `#4FD1D9` | `#138A94` | square |
| Buffer | sand `#C8A878` | `#9A7444` | square |
| Text | ivory `#E8DFC0` | `#8E7E4A` | square |

Category header colors: I/O red, Generators orange, Level/Dynamics gold,
Filters green, Time teal, Spectral cyan, Math blue, Modulation indigo,
Triggers yellow, Sequencing violet, Controls pink, Display grey, Utility
slate.

### Language

View ▸ Language picks **System** (the operating system's language when
the editor has a catalog for it, else English), **English**, or
**Pseudo**, remembered in the settings (D182). Everything the editor says
itself goes through the catalog: menus and command names, panel and tab
titles, the transport bar's labels and tooltips, dialogs, status and
error messages, undo names, hints, badges, and the settings and help
chrome. Node names, node docs, socket names, node settings labels, units,
and project content aren't translated yet. Pseudo accents every letter,
pads each message to about 135 % of its English length, and brackets it,
so untranslated text (no brackets) and layouts that can't take longer
words show up at a glance; a test opens every menu in Pseudo and fails on
a label without brackets. [TRANSLATING.md](TRANSLATING.md) has the rest.

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
  small button for triggers, a text field for text (an OSC address; click
  to type, Enter to keep). Ctrl-drag for fine, Shift-drag for coarse, click
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
| Zoom | wheel, pinch, `+`/`-`, `Home` to fit all, `.` to fit selection (`Ctrl+=`/`Ctrl+-` scale the whole interface instead; see "Accessibility and scaling") |
| Select | click, shift-click, box-drag, `A` all, `Alt+A` none, `L` linked |
| Move | drag selected; `G` grab (Blender style); snaps to grid with Ctrl |
| Add node | `Shift+A` or double-click empty canvas: search palette (fuzzy, synonyms: "volume" finds Gain, "lpf" finds Biquad); it also lists groups, blocks, and the plugins installed on this computer by name |
| Wire | drag from socket to socket; drop on empty canvas opens the palette **filtered to compatible nodes** and auto-wires the result |
| Insert on wire | drop a node onto a wire to splice it in (if types match) |
| Disconnect | drag a wire end away and release, or `Ctrl+drag` across wires to cut |
| Connect selected to new node | select two or more nodes, then add a node (`Shift+A` or the Add menu): they're wired into it in selection order and it goes to their right (see below) |
| Lazy connect | `F` with two selected nodes wires the left one into the right one (see below); the status line says what was wired, or why not |
| Reroute | `Shift+right-drag` across a wire |
| Portal | right-click a wire or a reroute ▸ **Make Portal**; `Shift+P` makes portals of the selected reroutes, or of the wires between the selected nodes, and turns selected portal tags back into wires |
| Delete | `X`/`Delete`; `Ctrl+X` deletes and reconnects through (keeps the chain) |
| Duplicate | `Shift+D` (with internal wires), `Ctrl+Shift+D` (also keeps incoming wires) |
| Copy/paste | `Ctrl+C`/`Ctrl+V` as JSON text; works across projects and into chat |
| Mute | `M` bypasses the node (passes `in` → `out` where types match) |
| Collapse | `H`; `Ctrl+H` collapses all unwired sockets |
| Group | `Ctrl+G` makes a group of the selection, exposing dangling wires as sockets; double-click or `Ctrl+Enter` enters a group, `Ctrl+Enter` leaves it (D183); `Ctrl+Alt+G` ungroups |
| Frame | `Ctrl+J` puts a labeled frame around the selection; frames move their contents |
| Minimap | `Shift+M` shows or hides it; click or drag in it to move the view |
| Align / distribute | `Shift+Alt+A` menu at the pointer (also Edit ▸ Align / Distribute, and a node's right-click menu with several selected); **auto-layout** `Ctrl+L` lays out the selection, or the whole graph with nothing selected (layered, left to right) |
| Undo/redo | `Ctrl+Z`/`Ctrl+Shift+Z`, unlimited, including parameter nudges (coalesced) |
| Find | `Ctrl+F` finds nodes by name, kind, or id, inside groups too, and jumps to them: `↑`/`↓` choose, `Enter` jumps, `Esc` closes |
| Plugin window | A Plugin node's right-click menu ▸ **Show Plugin Window** (also the Node tab's button) opens the plugin's own interface beside the editor (D134) |
| Help | `F1` on a node opens its reference in the side panel's Help tab (also Node Help in its right-click menu, and the Help button in the Node tab); with nothing selected, the list of every node |
| Pin to a panel | `P` pins the selected controls and displays to the current panel (`P` again unpins) |
| Panels | `Ctrl+Tab` switches between the canvas and the panels; `F11` performance mode (`Esc` or `F11` leaves) |
| Keyboard focus | `Tab`/`Shift+Tab` move a focus ring through the cards in signal order (columns left to right, top to bottom in each), and the selection follows; `Shift+Enter` keeps the focused card selected (adds or removes it); `Alt+↓`/`Alt+↑` step through the focused card's inputs, then outputs; `Enter` selects the focused card or opens a selected group; `F6` moves the keyboard between the canvas and the side panel; `Esc` with nothing to cancel hides the ring (D183) |
| Wire from the keyboard | `Enter` on a socket picks up a wire (the hint says from where); move the focus and `Enter` again wires it to the focused socket, or to the focused card's first socket that fits; `Esc` drops it |
| Keyboard moves and values | arrows nudge the selection a grid step (`Shift` five, `Ctrl` one unit; one undo step per run), or pan with nothing selected; after `G` they move the grab and `Enter` places it (splicing onto a wire under the moved card); `F2` on an input types its value; `Shift+F10` opens the focused card's or socket's menu with the keyboard in it |

Wire routing: béziers (a "straight with bends" option is planned, not built
yet). Wires behind nodes are dimmed. Long wires can be turned into named **portals**
(a pair of Reroutes that are drawn as labels instead of a line) to tame
spaghetti:

- **Making one.** Right-click a wire ▸ **Make Portal**: a *sender* tag
  appears by the source and a *receiver* tag by the destination, and the
  line between them goes. A plain reroute's menu does the same, the reroute
  becoming the sender with a receiver by each node it fed. `Shift+P` works
  on the selection: selected reroutes become portals, or else the wires
  between the selected nodes do. Each is one undo step. The portal is named
  after its source (`Oscillator out`), and names stay unique in a graph.
- **One sender, many receivers.** A source has at most one sender: making a
  portal of another wire from the same output adds a receiver to it, and so
  does pasting a copy of a receiver (or `Shift+D` on one) where its sender
  is.
- **Tags.** A tag is a small flag pointing the way the signal goes, in the
  wire's color, with the portal's name and one socket: a sender's input on
  its left, a receiver's output at its tip, so wires connect to tags like
  to any socket. A tag with no other end (its sender was deleted, or a
  sender with no receivers left) is outlined in the warning color.
  Hovering a tag lights up its other end; selecting one does too.
  Double-clicking jumps to the other end (a sender with several receivers
  goes to the next one each time); the tag's menu has the same jump.
- **Renaming.** `F2`, the tag's menu, or the Node tab's name field renames
  the whole portal (its sender and every receiver).
- **Back to a wire.** `Shift+P` on selected tags, or **Back to Wire** in a
  tag's menu, removes them and wires each destination straight from the
  source again. A sender takes all its receivers; a receiver goes alone,
  and its sender with it once none is left.
- Portals work inside groups, and copy and paste keep them sane: a pasted
  portal gets a name of its own, and a receiver pasted without its sender
  joins the sender of its name. In the project a portal is ordinary
  Reroutes and wires with a name on each tag (DECISIONS D104), so it plays
  exactly like the wire it replaced.

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
  collapsed pills, reroutes, portal tags, and notes take the room they're
  drawn with. A portal's sender goes just after its source and each
  receiver just before its destination, however far apart they end up.
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
  without a `Feedback` node, and a portal tag offers only the socket it
  shows. Press it again for the next free input.
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
  start) in every graph of the project, portals by name, and notes by
  their text. A node
  inside a group shows the group it's in, once per instance of it.
  `Enter` enters the group, selects the node, and centers it; the bar
  stays open for the next result until `Esc` or a click elsewhere.

## Side panel

A resizable panel (`N` toggles) with tabs:

- **Node**: everything about the selected node: full parameter list, docs
  (with its "Try this" tip highlighted), bindings, timing, latency.
- **Library**: nodes by category and user/shared blocks; drag to the canvas.
  Search here is the same engine as the palette. Every block that makes
  sound has a play button at the right of its row: it plays the block's
  preview over whatever the patch is playing (its preview file, or a
  render made then, with a spinner-like **…** while it renders) and
  becomes a stop button while it plays; one preview plays at a time
  (D190, D193). A block's right-click menu has **Export block file…**,
  which writes it with its preview beside it. Below the blocks, **Import
  block file…** and **Import from Link…**, then **Shared libraries**:
  each library added by link (a collapsible section named after it, with
  its description), its blocks filtered by the same search, each with its
  version and author, a play button when it has a preview (downloaded on
  first play), and **Install** (a spinner while it downloads,
  **Installed** once that version is in the library), plus **Refresh**
  and **Remove** for the library. A library still loading shows a
  spinner; one that couldn't be read says why (D194).
- **Library**: nodes by category, the plugins installed on this computer
  (**Plugins**), and user/shared blocks; drag to the canvas. Search here is
  the same engine as the palette.
- **Project**: sample rate, length, tempo map, seed, bindings, assets,
  missing files.
- **Devices**: audio/MIDI/gamepad devices, buffer size, measured latency,
  xrun counter, per-core load, OSC (a checkbox and port to listen on, 9000
  unless changed and off until turned on; the address to send to; what has
  arrived and what OSC Out nodes sent), and transport buttons. Under MIDI
  inputs and under OSC, **Last messages** lists the newest messages the
  ports received ("CC 74 = 127 · ch 1", `/fader/1/3 0.5`), newest first
  with their age, so a control's number or address is read off the list
  rather than guessed. Click an OSC address to copy it; with one OSC In
  selected, **Use** beside each address sets that node's address. Clear
  empties a list (D115). In a plugin's window the host owns the devices:
  the tab says so, shows the rate, buffer, channels, and load the host
  runs the patch at, and lists the **macros** (name, what each drives,
  its value as the host shows it, and a slider that moves the host's
  parameter as one gesture the host records) (D127). In the desktop
  editor, **Plugins** says how many CLAP and VST3 plugins were found,
  rescans (**Rescan** for new and changed ones, **Rescan all**), lists the
  folders searched, and lists the plugins that couldn't be opened and why
  (D135). In a browser the tab shows what the page runs at, the
  microphone (**Use the microphone**, or the one in use), the MIDI inputs
  (all heard) and outputs (checked ones get MIDI Out nodes' messages) with
  their last messages, the gamepads, and, plainly, what a browser can't
  do: OSC, recording takes to disk, plugins, choosing the output device
  (D154).
- **Problems**: compile errors and disabled nodes, click to jump.
- **Help**: a node's reference (`F1`, D110): its name and category, what
  it does, its "Try this" tip in a highlighted box, every socket with its
  type, unit, default, and range or choices (as the node has them with
  its default settings), and **Used in**: the blocks in the library
  (shipped, the person's own, imported) and the shipped example projects
  that hold the node, inside nested groups too. Clicking a block adds it
  at the center of the view, like the Library tab; clicking an example
  opens it as a new, untitled project (after asking about unsaved
  changes), the way the Start project opens, so saving never overwrites
  the shipped file. "All nodes" goes back to the list of every node by
  category; the search field finds nodes by name or synonym, like the
  palette.

When the panel is too narrow for every tab's name, the tabs not shown keep
only their icons, named on hover.

## Sharing blocks

- **File ▸ Import from Link…** asks for a link (an `.acblock`, an
  `.acpack`, or a shared library's `index.json`) in a small dialog and
  downloads it in the background, the status bar showing what it's
  downloading: a block joins the library under Imported (the Library tab
  opens on it), a pack is saved to the Downloads folder and opens as a
  pack file does (asking about unsaved changes first), and a library is
  listed under Shared libraries. Failures (an `http` link elsewhere than
  this computer, a 404, a download that doesn't match its index) are
  reported in the status bar (D192).
- **Export block file…** in the Node tab (under **Group definition**,
  next to **Save to library**) and on library rows asks where to save
  and writes the block and its preview in the background; the status bar
  says when it's done, or why there's no preview.

## Transport bar

Play/stop/record, position in bars:beats and time, tempo and time signature
(click to edit the tempo map), loop toggle and range, master meter, CPU and
xrun indicator. (A metronome toggle is planned, not built yet; meanwhile the
**Metronome** block in the library clicks along with the transport.) `Space` plays/stops,
`R` records, `Shift+Space` plays from the start. At its right end, a
**Canvas | Panels** switch and the performance-mode button. In a plugin's
window the host's transport plays the patch: the bar shows whether the
host plays instead of the play, record, rewind, and loop buttons, and
`Space`, `Shift+Space`, and `R` are the host's (D127).

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
- Hover a socket to see its live value large. (Hovering a wire for its value
  or a mini-scope is planned, not built yet.)
- Optional per-node timing in headers (`Ctrl+T`, View ▸ Node Timing),
  heat-colored, so a slow patch shows where its time goes: each header shows
  the node's processing time per block (`37 µs`), steadied over about half
  a second, on a chip colored from cool through the warning color to the
  error color by its share of the block's real-time budget (log scale: a
  node at 0.2 % or less is cool, a quarter of the budget or more is as hot
  as it gets). Hover the chip for the share. A group's header shows the sum
  of everything inside it, and a Voices node the sum of every voice; inside
  a Voices body each node shows the sum of its copies. The Node tab shows
  the selected node's time and share with a small bar, or a **Measure**
  button while timing is off. It is off by default: off, the engine reads
  no clock and the editor reads nothing (D105).
- Disabled, baking, and error states are visible from across the canvas.

## Onboarding and examples

- First launch opens a **Start** project with a guided patch: an oscillator
  wired to the output with comments explaining what to try.
- **File ▸ Examples** lists the shipped projects (see ROADMAP and
  `docs/guide/examples.md`) and opens one as an untitled copy, so saving
  asks where to put it instead of overwriting the example. Each is a
  working patch and a lesson, from "Start" and "Guitar Pedalboard" to
  "Build a Keyboard" and "Gamepad Jam". In a browser the examples come
  from the site, the first page opens Start (or the example its
  `?example=` names), and File ▸ Open Recent isn't offered: files are
  opened and saved through the browser (D153).
- Every node's help has "try this" tips and links to blocks that use it:
  the Help tab (`F1`) shows the tip highlighted and lists, under **Used
  in**, the blocks and examples built with the node. Every node's doc
  comment has its tip as a paragraph starting with `**Try this:**` (a test
  fails for a node without one).

## Accessibility and scaling

- UI scale from 75 % to 200 %; everything is vector. View ▸ UI Scale has
  the steps 75, 90, 100, 110, 125, 150, 175, and 200 %; `Ctrl+=` and
  `Ctrl+-` go a step up or down and `Ctrl+0` back to 100 %, even while
  typing or with the panels shown. It is egui's zoom factor, so everything
  scales, canvas text included; the canvas's own zoom is separate. The
  scale is remembered in the editor's settings (D108, D109).
- A light theme and a High Contrast theme (View ▸ Theme) with the same
  contrast rules as the dark one, and High Contrast's stricter ones (see
  "Look"). The keyboard focus ring is the theme's `focus_ring` color, at
  least 3:1 on the canvas, cards, and panels in every theme, and unlike
  the selection outline.
- Socket shapes carry type information in addition to color.
- **Keyboard.** Every canvas action has a keyboard path ("Canvas
  interactions": focus, sockets, wiring, values, moves, menus); the
  palette and find bar are fully keyboard driven. On the panels, `F6` puts
  the keyboard on the first card and `Tab` reaches each one: arrows turn a knob, slider, or number (a hundredth of
  the range, a tenth with `Shift`/`Page Up`/`Page Down`; whole numbers by
  at least one), `Home`/`End` go to the ends, `Enter` (or `F2`) types a
  value, opens a dropdown, flips a toggle, or picks a clip's file, and
  `Space` holds a Button down while it's held; in the panel view (not
  performance mode) `Delete` unpins the card and `+`/`-` resize it. The
  update and crash windows give their first button the keyboard when they
  appear.
- **Screen readers** (AccessKit, D185). Each card is a button named for
  its title, kind (when it has a title of its own), and socket counts,
  with its selected, muted, and collapsed states ("Level, Gain, 2 inputs,
  1 output, selected"); each socket says its card, name, type, side, and
  what it's wired to or set to ("Level gain: Float input, -6 dB"). The
  canvas is named for its node count and the group it shows. On a panel,
  knobs and sliders are sliders (numbers spin buttons) with value, range,
  and increment, decrement, and set value; toggles are switches; meters
  say their peak; readouts their number. Icon-only buttons are named for
  what they do (a library block's play button says "Play preview of
  Plate Reverb"), icons are left out of every name, decorative icons are
  hidden, and check-marked menu entries are checked menu items. A screen
  reader's activate selects a card, and moving its focus moves the
  canvas's.
- **View ▸ Reduce Motion** (D184): no fades or ghosts, no wire snap ease,
  no flowing-wire texture or event pulses, no smooth zoom or fit (the view
  jumps), and egui's own animations off. Meters and scopes still move.
  Remembered in the settings.
- **The audit** (`crates/ac-app/tests/accessibility.rs`, run in CI): it
  walks the AccessKit tree of the main window on the Start example with
  the side panel on each of its tabs and a node selected, the panel view
  and performance mode on an example with a panel, and every menu and
  submenu, and fails on any interactive node (one that takes focus or a
  click, a text field, a slider) without a name, or whose name carries an
  icon's code point. egui's own scroll bars are left out (their role is
  all they say). Other tests there find cards and sockets by their names,
  build, wire, set, and play a patch from the keyboard alone, play panel
  controls from the keys and from AccessKit actions, and check Reduce
  Motion. At 100 % every pointer target in the window is at least 14
  points on its short side (egui's compact one-line buttons; most buttons
  are 22) and every socket's grab area 20 × 20; scroll bars can be
  grabbed 14 points wide. That is under WCAG 2.5.8's 24 px for the compact
  buttons; UI Scale 175 % brings them there.
- **Not covered yet**: the canvas's inline widgets (an input's slider,
  dropdown, or checkbox on a card) are reached through their socket (`F2`)
  rather than as widgets of their own; a panel's XY Pad and Keyboard play
  from the pointer only (Play keys plays notes from the computer
  keyboard); a frame's rename and remove are pointer-only; panel and side
  panel widths change by dragging their edges only; System language finds
  English only (no other catalogs yet), and in the browser it can't read
  the system's language. Manual checks with NVDA, VoiceOver, and Orca are
  listed in the M3.5 merge request.

## Fun, deliberately

- Wires snap into sockets with a short ease and a soft click (sound optional,
  off by default).
- A fresh node fades in where the cursor is; a deleted node shrinks away.
- The Start project ends with a "now break it" section.
- Nothing bounces, nothing takes longer than 150 ms, nothing animates while
  the CPU is busy with audio.
- View ▸ Reduce Motion turns every one of these off (see "Accessibility
  and scaling").
