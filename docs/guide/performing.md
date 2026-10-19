# Performing

A patch becomes an instrument when you can play it with your hands. This
chapter covers the on-screen controls, panels for playing without the
wiring in view, and every way to bring outside controls in: MIDI, the
computer keyboard, gamepads, and phones and tablets over the network.
Finally, **bindings** connect any of those to any knob in the patch,
without a wire.

## Controls

Control nodes are widgets you play with the mouse, right on the canvas.
They're in the **Controls** category:

| Node | Gives you |
| --- | --- |
| **Button** | A `trigger` when pressed, a `gate` while held, and a `release` trigger |
| **Toggle** | An on/off switch |
| **Slider**, **Knob** | A value with a range, a unit, and a linear or log scale (set in the Node tab) |
| **XY Pad** | Two values at once from one point you drag, plus a gate while you touch it |
| **Keyboard** | A piano you click; it sends MIDI |
| **Number** | A number you type, optionally kept within limits |
| **Dropdown** | A menu of named choices; outputs the number of the one picked |

Playing a control is part of the patch, like turning a knob on a
hardware synth: a drag is one undo step, and holding a Button or a
Keyboard key never changes the saved project. Double-click a Knob to put
it back to its default, or a Slider to type a value.

> **Try this:** wire a Knob into a Delay's `feedback` and a Button's
> `trigger` into a Clip's `start`, then play both while the patch runs.

## Panels

A **panel** is a page of controls and displays picked from your patch,
without the wiring: a mixer, a drum pad, a pedalboard. A project can have
several.

### Pinning

Select control or display nodes and press `P` (Edit ▸ Pin to Panel). They
go onto the current panel, which is created ("Panel 1") if there isn't one
yet. `P` on nodes already there takes them off again. A node's right-click
menu can also pin it to any panel, or to a new one, and unpin it. Pinned
nodes carry a pin badge.

What can be pinned: the controls (Button, Toggle, Slider, Knob, XY Pad,
Keyboard, Number, Dropdown), the displays (Meter, Scope, Analyzer,
Readout), and Clip, Record, and Audio Out. Only nodes on the top level of
the patch can be pinned, not nodes inside a group.

### Showing panels

`Ctrl+Tab`, View ▸ Canvas / Panels, or the **Canvas | Panels** switch at
the right of the transport bar shows the panels instead of the canvas.

- Tabs across the top show one panel each. Click one to show it,
  double-click to rename it, right-click to rename, move, or delete it,
  and click `+` to add a panel.
- Each card has the node's label and its live widget, played exactly as
  on the canvas. Meters and scopes move with the sound.
- Drag a card by its edge (anywhere but the widget) to move it. The `−`
  and `+` in its corner make it smaller or bigger, and `×` unpins it.
- A panel taller than the window scrolls.

While the panels show, shortcuts that change the patch are off;
transport, undo, and file shortcuts still work.

### Performance mode

`F11`, View ▸ Performance Mode, or the performance button at the far
right of the transport bar shows the current panel full screen, with
bigger cards. Only play/stop, the song position, the panel tabs, a Play
keys button, and **Exit** remain. `Esc` or `F11` leaves.

Performance mode turns on **Play keys** (below), so the computer keyboard
plays the patch, `Space` included: use the play button, or turn Play keys
off with `Ctrl+K` or its button. Leaving performance mode puts Play keys
back the way it was.

## The computer keyboard

Most single keys are editor shortcuts, so the computer keyboard only
reaches the patch while **Play keys** is on: the keyboard button in the
transport bar, Transport ▸ Play Keys, or `Ctrl+K`. The status line says
when it's on. While it is:

- keys go to the patch instead of running shortcuts;
- `Esc` and shortcuts with `Ctrl`, `Alt`, or `Cmd` still work, and text
  fields still take typing;
- holding a key down doesn't repeat it;
- every held key is let go when Play keys turns off, when you start
  typing in a field, or when the window loses focus, so no note hangs.

A **Keyboard In** node brings the keys into the patch:

- Its `midi` output plays the middle row of letters like a piano: `A S D
  F G H J K L ; '` are the white keys from C up, and `W E T Y U O P` the
  black keys between them. `Z` and `X` shift down and up an octave (up to
  three each way). The `A` key plays the note set in the Node tab's
  **Settings** (**Note of the A key**: middle C, 60, by default), at the
  `velocity` input.
- **Keys** in its Settings lists other keys to bring in by name,
  separated by commas (default `1, 2, 3, 4`; names such as `Space`,
  `Enter`, `Up`, and `F1` work too). Each gets a gate output (`key_1`,
  high while held) and a press
  trigger (`key_1_press`): fire samples, toggle effects, or step through a
  pattern.

Keys can also be [bound](#bindings) to any input, without a node.

> **Try this:** wire a Keyboard In's `midi` into a Voices synth, turn on
> Play keys, and play chords on the letter keys.

## MIDI

Every connected MIDI input reaches the patch through **MIDI In** nodes
(see [Sound devices](getting-started.md#midi)). From there:

- **MIDI Note** turns notes into a pitch, a gate, and a velocity for one
  voice; **Voices** shares notes out among copies of a voice for chords.
- **MIDI CC** turns one controller (a knob or fader on your keyboard, the
  mod wheel, the sustain pedal) into a value in a range you choose.
  Controller 128 reads pitch bend and 129 channel pressure.
- **MIDI Filter** keeps one channel, a range of notes, or chosen kinds of
  message; **MIDI Transpose** moves notes up or down and can snap them to
  a scale; **Chord** and **Arpeggiator** turn single notes into more.
- **Merge** joins several MIDI streams into one, since an input takes
  one wire: a MIDI keyboard, the computer keys, and a MIDI Pattern can
  play one synth together. A note held from two of them sounds once and
  ends when the last lets go. Set to triggers, it joins rhythms.
- **MIDI Out** plays external hardware. Choose its port in the Devices
  tab under **MIDI outputs**. It ends every note it started when the
  transport stops, and its `panic` input silences everything.

## Gamepads

Plug a game controller in at any time. The Devices tab's **Gamepads**
section lists the connected ones by number: each takes the lowest free
number (from 1) when it connects, and keeps it while it stays connected.

A **Gamepad** node brings one in. Its **Gamepad** setting (Node tab,
Settings) picks which: 1 is the first. Every button is a gate (`south`, `east`, `west`, `north` for the
four face buttons, then the bumpers, triggers, `select`, `start`, the
stick presses, and the D-pad) plus a `_press` trigger. The sticks
(`left_x`, `left_y`, `right_x`, `right_y`) run from −1 to 1, and the analog
triggers (`left_trigger_axis`, `right_trigger_axis`) from 0 to 1.
`dead_zone` ignores small movements around the center, and `curve` makes
the sticks finer near the center (above 1) or touchier (below 1).

> **Try this:** wire `left_x` through a Map into a filter's `cutoff`, and
> `south` into an ADSR's `gate`, for a synth you play from the couch.

On Linux, see [Gamepad not seen](troubleshooting.md#gamepad-not-seen-on-linux)
if yours doesn't show up.

## Phones and tablets: OSC

Many phone and tablet apps send **OSC** (Open Sound Control) messages over
the network from faders, buttons, and pads you lay out yourself.

1. In the Devices tab's **OSC** section, tick **Listen on UDP port**. The
   port is 9000 unless you change it. It says where to send: "Send to
   192.168.1.20:9000", this computer's address on the network.
2. In the app on your phone, set that address and port as the place to
   send to. The phone and the computer must be on the same network.
3. Move a control on the phone. The Devices tab counts the messages that
   arrive.
4. Add an **OSC In** node and, in its Settings, set **Address** to the
   address the control sends, exactly as the app writes it (for example
   `/fader/1`). `value` holds the last number received and `received`
   fires on each message. A control that sends several numbers (an XY
   pad) needs **Numbers per message** set to 2 or more, for `value_2` and
   so on.

**OSC Out** sends values the other way, to move faders on the phone or
drive other software. In its Settings, set **Send to** (the phone's IP
address), **Port**, and **Address**.

OSC listening is off each time the editor starts, until you turn it on;
it isn't saved with the project. From the command line,
`audioc run --osc-in 9000` listens.

## Bindings

A **binding** connects a hardware control (a MIDI knob or note, a key, a
gamepad button or stick, or an OSC address) to an input on a node,
without a wire. Any unwired number, on/off, or trigger input can be bound,
and so can the transport.

### Learning a binding

1. Right-click an unwired input ▸ **Bind…**. For a Slider, Knob, Toggle,
   Button, Number, or Dropdown, right-click the node itself ▸ **Bind…**
   binds its main value (an XY Pad offers **Bind X…** and **Bind Y…**).
2. The status line says it's listening. Move the control: press a key or
   button, play a note, or turn a knob or push a stick (it has to really
   move, so a resting stick or a jittery knob doesn't bind by accident).
   A key works even with Play keys off. `Esc` cancels.
3. The status line says what was bound. The binding is one undo step.

A bound input shows a small link mark left of its socket; hover it to see
which control. While the patch plays, the input's control on the canvas,
and on panels, follows the hardware. That isn't an edit: it adds nothing
to undo, and the value saved in the project is the one you set by hand.
Moving the control by hand takes over until the hardware moves again.

If you wire an input that has a binding, the wire wins; the link mark
turns to the warning color.

### How controls map

- A knob, fader, stick, or OSC value sweeps the input's whole range (on a
  log scale where the input's control is one, like a cutoff in Hz).
- A note presses with its velocity; keys and buttons press fully.
- A trigger input fires once per press.
- An on/off input either follows the control (**Momentary**: on while
  held) or flips on each press (**Toggle**). Learned presses start as
  toggles, except for inputs meant to be held, such as a Button's press or
  a gate.

### Changing and removing bindings

The **Project** tab lists every binding: the control, what it drives
(click to jump to the node), its range (**from** and **to**, for numbers),
**invert**, Momentary or Toggle (for on/off inputs), and the value it last
set. `×` removes one. The Node tab lists the selected node's bindings the
same way. Right-click a bound input ▸ **Remove Bindings** removes its
bindings.

Bindings are saved in the project, so `audioc run` and offline renders use
them too. Inside a group, a binding belongs to the one copy of the group
you went into, not to every copy.

To learn an OSC address, the project must already mention it, in an OSC
In node's **Address** or another OSC binding. Add the OSC In node first,
then learn.

### Transport buttons

The Devices tab's **Transport buttons** section binds the transport:

- **Add gamepad transport buttons** sets up any gamepad's A to play and
  stop, B to stop, and X to record.
- Or pick an action (Play, Stop, Play / stop, Record, Play from start) and
  click **Bind…** to learn any control for it: a MIDI pedal, a key, an OSC
  button.

They're listed there and in the Project tab, and saved with the project.

Next: [Recording and rendering](recording-and-rendering.md).
