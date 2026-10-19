# Changelog

Every notable change to Audio Constructor, newest first. Each merged merge
request is a release: it raises the version and adds its section here, and
the section becomes the release notes (see
[CLAUDE.md](CLAUDE.md#versions-changelog-and-releases)).

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Before 1.0, a minor version marks a milestone, new features, or a change to
project files, the command line, or rendered output; a patch version marks
fixes and smaller changes.

## [0.10.0] - 2026-10-19

### Added

- **Audio Constructor as a plugin**: one library holds two plugins, in
  CLAP and VST3 formats. **Audio Constructor Effect** takes the host's
  audio (stereo, mono, or mono in and stereo out) and its notes; **Audio
  Constructor Instrument** plays the host's notes, and notes from MIDI Out
  nodes go back to the host. A project's Audio In, Audio Out, and MIDI In
  nodes are the plugin's ports. The host sets the sample rate and block
  size, and the patch plays, stops, and jumps with the host's transport
  (it keeps its own tempo).
- A new Effect starts as a level control on Macro 1; a new Instrument as
  an eight-voice synth with its cutoff, resonance, envelope amount, and
  level on Macros 1 to 4.
- Sixteen **macro** parameters (`Macro 1` to `Macro 16`) are what the host
  automates. Bind any input to one, the same way as a MIDI controller, and
  the host's automation moves it on the right sample. The host's readout
  shows the value a macro sets ("-12 dB", "1.2 kHz") rather than a bare
  number. In project files, a macro binding's source is
  `{"type": "macro", "number": 3}`.
- The host saves the project, its audio files, and the macro values with
  its session, and reopens them as they were, on any machine. A saved
  state that can't be read leaves a working default patch, never a broken
  plugin.
- `python3 scripts/bundle_plugin.py` builds the `.clap` and `.vst3`
  bundles (`--install` copies them into your plugin folders). The guide
  has a new chapter on the plugin.

## [0.9.0] - 2026-10-18

OSC addresses are now inputs rather than settings, which means a block can
expose one as a group input and every instance of it can listen to, or send
to, a different control. Behind that is a new kind of signal: text.

### Added

- **Text signals.** A new socket type carries short text (up to 128 bytes)
  once per block, shown as an ivory square. An unwired text input is a
  text field on the node and in the Node tab (click to type, Enter to
  keep); a wired one follows whatever produces it. Text only connects to
  text. Inside a group, "Expose as group input" works on text inputs like
  any other, so a block gets a text parameter on every instance.
- **Constant** can hold text (`type` setting `text`), for an address that
  several OSC nodes share.
- **Number to Text** writes numbers into a template: `/fader/{}` with 3
  gives `/fader/3`; `{1:.2}` picks the second number with two decimals. A
  template with a stray brace or an unknown format writes nothing, and a
  value that isn't a number writes 0.
- **Text to Number** reads the number where a pattern has its `{}`
  (`/fader/{}` on `/fader/3` gives 3), with an `ok` output that's on while
  the text matches; when it doesn't, the value is 0 and nothing stops.
  Its `type` setting gives a whole number instead of a float.

### Changed

- **OSC In's `address` is an input**, and so are **OSC Out's `address`,
  `host`, and `port`**: type them on the node, wire a `Number to Text` in
  to build an address from a counter or a group parameter, or expose them
  as group inputs. A wired address is followed block by block. The Node
  tab's Settings section keeps `args`, and still warns about an address,
  host, or port that can't work; the Devices tab's **Use** button sets
  the input.
- **Project files are version 2.** Older files open as before: the OSC
  nodes' address, host, and port move from settings to inputs on load,
  and saving writes the new form. The examples and blocks that ship with
  the app are already in it.

## [0.8.0] - 2026-10-18

Matching a control surface no longer means guessing: the Devices tab shows
what your devices actually send, and OSC nodes finally have their settings
in the Node tab.

### Added

- **Last messages** in the Devices tab. Under MIDI inputs, the newest
  messages from each port ("CC 74 = 127 · ch 1", "Note on C4 · vel 100 ·
  ch 1"); under OSC, the newest messages with their address exactly as the
  sender wrote it and their arguments. Both lists show each message's age,
  newest first, and clear with a button. Click an OSC address to copy it,
  or select an OSC In and press **Use** beside a message to set the node's
  address to it. A surface whose layout you'd rather not edit can be
  matched from what it already sends.

### Fixed

- An OSC In's or OSC Out's address and host fields in the Node tab show
  the default the node uses instead of an empty box, and a setting the
  node can't work with (an address without a leading `/`, a host that
  isn't an IP address or `localhost`) is called out under the settings.
- Text settings in the Node tab (an OSC address, a Comment's text, a File
  Out path) keep what you've typed until you leave the field. They were
  redrawn from the project every frame, so what was typed was lost before
  it could be saved.

### Changed

- The Devices tab's OSC section and the OSC In node's help now say what to
  check when no messages arrive: the surface's host and port, and the
  phone or tablet's local-network permission for its app, which silently
  stops every packet when refused. The node's help also notes that most
  surfaces name controls by page and name (`/1/fader1`) and that the
  address match is exact.
## [0.7.0] - 2026-10-18

The end of Phase 2: fourteen example projects to learn from, a user guide,
and an engine that spreads light patches and polyphonic synths across
cores.

### Added

- Nine new example projects, each a working patch with notes on the canvas
  saying what each part does and what to try. All of them play when you
  press Space, and render with `audioc render`, without any hardware:
  - **Sidechain Pump**: a pad that ducks under a four-on-the-floor kick
    through the Sidechain Ducker block, with the release set in beats so
    the pump follows the tempo, and a switch to hear it without.
  - **Drum Machine**: the Drum Machine block playing from the transport,
    and its per-drum triggers playing a second kit of Samplers cut from
    one audio file; a switch flips between the two kits.
  - **Build a Keyboard**: the Subtractive Synth Voice block made
    polyphonic and played from the computer keys (Play keys, `Ctrl+K`) and
    an on-screen Keyboard at once, with velocity, octave, and timbre
    knobs, all on a **Play** performance panel (`Ctrl+Tab`, `F11`). A demo
    phrase plays along through a Merge node; turn Demo off to play on your
    own.
  - **Gamepad Jam**: drums on a game controller's face buttons, strummed
    chords stepped by the D-pad, a filter on the left stick, and strum
    strength and echo on the analog triggers, with bindings for play/stop
    on Start and for the sticks and triggers, and a **Jam** panel. A demo
    pattern plays the same parts until you press Select on the controller
    to take over.
  - **Multiband Compressor**: a four-track loop through the Multiband
    Compressor block, with before and after meters, a bypass switch, and a
    meter per band showing how hard each band is turned down.
  - **Vocoder**: a saw chord made to talk by the Vocoder block, with a
    robot voice built from a buzz and two moving formant filters; switch
    on Audio In to vocode your own voice.
  - **Generative Patch**: two Euclidean rhythms, random notes snapped to a
    scale, a plucked string and an FM bass. It plays the same tune every
    time until you change the project's seed.
  - **Tape Loop**: phrases recorded onto a four-second loop that saturates,
    darkens, and wobbles with wow and flutter on every pass until they
    fade.
  - **Guitar-to-Synth**: a synth that follows the pitch and dynamics of a
    guitar on Audio In, with a recorded riff standing in until you switch
    the guitar on.
- `audioc bench` has a ninth stress project, `subtractive-voices-16`:
  sixteen voices of the shipped Subtractive Synth Voice block (two saws, a
  ladder filter, and an envelope each), all sounding at once. It is the
  synth the Phase 2 performance target is measured on; the results are in
  `bench/README.md`.
- A **user guide** in [docs/guide](docs/guide/README.md), written for
  musicians and sound designers rather than programmers: getting started
  and setting up your sound card, the editor, building a first synth and
  making it polyphonic, live and baked nodes and latency, blocks and
  packs, performing with panels, MIDI, the computer keyboard, gamepads,
  phones, and bindings, recording and rendering, every `audioc` command,
  a tour of every example, and troubleshooting.
- The **node reference** in [docs/reference](docs/reference/README.md) now
  opens with an index: every category with a sentence on what's in it and
  how many nodes it has, then every node under its category with a
  one-line summary, marking live sources and nodes that need their whole
  input. `audioc nodes --docs` writes it.
- **File ▸ Examples** lists every shipped example and opens the one you
  pick as an untitled copy, so saving never overwrites the example.
- A **Merge** node (Math & Logic) combines several MIDI or trigger streams
  into one, so a MIDI keyboard, the computer keys, and a pattern can all
  play one instrument at once. Its settings choose how many inputs it has
  (2 to 16) and whether it carries MIDI or triggers. A note played from
  two sources at once sounds once and ends when the last of them lets go,
  the sustain pedal stays down while any source holds it, and restarting
  the transport releases everything still held, so no note hangs.

### Changed

- **More engine threads now speed up patches built from many light
  nodes.** A run of nodes that only feed each other (an effect chain), and
  each voice of a Voices node, now runs as one job on one core, so the
  cores spend their time on audio instead of passing work back and forth.
  On four threads, eight chains of two dozen filters and delays render
  2.2–2.7× faster (they gained nothing before) and sixteen voices of the
  Subtractive Synth Voice about twice as fast (Engine threads in the
  Devices tab, `audioc run --threads`, `audioc bench --threads`). One
  thread is as fast as before, and the output is the same sample for
  sample on any number of threads. The Project tab's Schedule line counts
  the chains.
- **Sample & Hold**, **Random Walk**, and **Step Sequencer** are now named
  in words like every other node (they showed as SampleHold, RandomWalk,
  and StepSequencer). Projects that use them are unaffected.
- The **Reverse Delay** example is the real thing now, not a placeholder:
  a melody whose echoes play backwards, built once on a Clip, where
  Reverse bakes and plays, and once on Audio In, where it switches itself
  off while live, with notes on the canvas explaining why.

### Fixed

- Stopping the transport (`Space` or the stop button) now ends recording,
  as a bound Stop button already did, so a take ends when you stop instead
  of running on until you press `R` again.
- Gamepads are numbered from 1 in the Devices tab, `audioc devices`, and
  `audioc run`, as a Gamepad node's `pad` setting and bindings ("Pad 1")
  count them. They were listed from 0, one less than the number to use.
- The Keyboard node's help no longer says it plays from the computer
  keyboard: you play it with the mouse, and a Keyboard In node (with Play
  keys on) plays from the computer keys.
- A newly added OSC In, OSC Out, Gamepad, Keyboard In, Delay, or
  Oscillator showed none of its settings in the Node tab, so you couldn't
  set an OSC address, pick a gamepad, choose keys, put a Delay in beats,
  or make an Oscillator pure without editing the project file. They all
  have their fields under **Settings** now.

## [0.6.0] - 2026-10-18

Editor conveniences: tidy, align, and wire faster, find anything, keep
long wires out of the way, see where the time goes, and work in a light
theme at any size, with help for every node a key away.

### Added

- **Auto-layout** (`Ctrl+L`): tidy the selection, or the whole graph when
  nothing is selected, into columns that follow the signal from left to
  right. Loops through a Feedback node read from start to finish, sources
  and modulators sit next to what they drive, notes line up above or
  below the patch, frames move and resize with what's in them, and the
  patch stays where it was on the canvas. One undo step.
- **Align and distribute** (`Shift+Alt+A` at the pointer, Edit ▸ Align /
  Distribute, or a node's right-click menu with several selected): line
  the selected nodes up by their left, center, right, top, middle, or
  bottom, or space them out with equal gaps.
- **Lazy connect** (`F`): with two nodes selected, wire the left one into
  the right one: its first output into the first free input that takes
  it, with matching names like `out` → `in` first. Press it again for the
  next free input. It never makes a loop without a Feedback node, and the
  status line says what it wired or why it couldn't.
- **Wire selected nodes into a new one**: select two or more nodes and
  add a node with `Shift+A` (or from the Add menu): they're wired into it
  in the order you selected them, and it lands to their right. A Mix grows
  inputs to fit. If they don't all fit, the node is added unwired and the
  status line says why.
- **Find** (`Ctrl+F`): find nodes by name, kind, or id anywhere in the
  project, inside groups too, with the group each is in. Arrow keys
  choose, Enter jumps there (into the group, selected and centered), and
  Esc closes.
- **Light theme**: View ▸ Theme switches between Dark, Light, and Follow
  System (your operating system's light or dark mode) at once. The light
  theme keeps text easy to read everywhere (at least 4.5:1 contrast, the
  level accessibility guidelines ask of body text), and wires, sockets,
  node headers, meters, and scopes are deepened so they stand out on
  light cards. The editor remembers your choice.
- **UI scale**: View ▸ UI Scale makes the whole editor bigger or smaller,
  canvas text included, in steps from 75 % to 200 %. `Ctrl+=` and
  `Ctrl+-` step through the sizes and `Ctrl+0` goes back to 100 % (the
  canvas keeps its own zoom on `+`, `-`, and the wheel). The editor
  remembers the size too.
- **Node help**: `F1` on a node (or Node Help in its right-click menu, or
  the Help button in the Node tab) opens its reference in the side
  panel's new Help tab: what it does, its "Try this" tip highlighted,
  every socket with its unit, default, and range, and **Used in**: the
  blocks in your library and the example projects built with it. Click a
  block to add it to the canvas, or an example to open it as a new
  project (after asking about unsaved changes). With nothing selected,
  the Help tab lists every node and finds them by name or synonym, like
  the palette. The Node tab shows the tip too.
- **Portals**: turn a long wire into a pair of named tags, a sender by its
  source and a receiver by its destination, with no line across the canvas.
  Right-click a wire or a reroute and choose Make Portal, or select a
  reroute (or the two nodes a wire joins) and press `Shift+P`. A source has
  one sender and as many receivers as you like: make portals of its other
  wires, or copy and paste a receiver. Hover a tag to light up its other
  end, double-click it to jump there, and press `F2` to rename the whole
  portal; `Shift+P` on a tag turns it back into a plain wire. Portals work
  inside groups, pasted portals keep names of their own, and a portal plays
  exactly like the wire it replaced (older versions open it as two reroutes
  and a wire).
- **Minimap**: while your patch doesn't fit the window, a small map of it
  sits in the canvas's corner, with the nodes in their category colors, the
  frames, and the part you're looking at outlined. Click or drag in it to
  go there. It hides itself once everything fits; `Shift+M` or
  View ▸ Minimap turns it off and on.

### Changed

- Secondary text in the dark theme is a shade lighter, so it stays
  readable on buttons.
- **Node timing** (`Ctrl+T`) shows each node's processing time per block
  in microseconds, held steady over about half a second and colored from
  cool to hot by its share of the block's time budget, so the expensive
  parts of a patch stand out. Hover a time to see its share. Groups and
  Voices show the total of what's inside them (every voice), and inside a
  Voices body each node shows all its copies. The Node tab shows the
  selected node's time and share, with a Measure button while timing is
  off. Timing is still off until you ask for it and costs nothing then.
  Nodes that run in several steps, such as Feedback, and blocks split by
  bindings now count all their time.

### Fixed

- Jumping to a node inside a group from the Problems or Project tab now
  centers that node, and leaving the group returns to the view you had.
- A project opened right after the editor starts keeps its saved view
  instead of being reframed.

## [0.5.0] - 2026-10-18

Control surfaces: gamepads, the computer keyboard, and Open Sound Control
reach the patch as nodes and as bindings on any input, and performance
panels turn a patch into a playable surface.

### Added

- **Gamepads**: plug in a game controller and the editor and `audioc run`
  hear its buttons, sticks, and triggers. Controllers can come and go while
  you play; each keeps its number while it's connected, and unplugging one
  lets go of anything it was holding. The Devices tab and `audioc devices`
  list them; `audioc run --no-gamepad` ignores them.
- **OSC**: receive Open Sound Control from phone and tablet control apps or
  other programs. Turn on listening in the Devices tab (port 9000 unless
  you change it), which shows the address to send to and counts what
  arrives, or run `audioc run --osc-in 9000`. Every argument of a message
  is its own control, and a message without arguments works as a trigger.
  Patches can send OSC too, each message to the address in its settings.
- **Play keys**: the keyboard button in the transport bar (also
  Transport ▸ Play Keys, `Ctrl+K`) sends your computer keys to the patch
  instead of running canvas shortcuts. Escape and shortcuts with Ctrl, Cmd,
  or Alt still work, text fields still get your typing, holding a key
  doesn't repeat it, and keys never stick when you turn it off or switch
  windows. Performance mode turns it on for you and puts it back when you
  leave.
- **Gamepad** node: a game controller in the patch. Every button is a gate
  and a press trigger (release triggers too, if you ask), and the sticks
  and analog triggers are numbers with an adjustable dead zone and
  response curve. Each change lands on the exact sample it arrived on,
  and a button held through a transport restart stays held.
- **Keyboard In** node: the computer keyboard in the patch. Keys you list
  become gates and press triggers, and the middle row of letters plays
  notes like a piano (`A` to `'` for the white keys, the row above for
  the black ones, `Z` and `X` to change octave), so you can play a synth
  without a MIDI keyboard. A key always releases the note it started,
  even after an octave change.
- **OSC In** node: Open Sound Control messages to one address (a fader or
  button on a phone or tablet control surface, say) come in as up to four
  numbers that hold their last value, plus a trigger for each message.
- **OSC Out** node: sends up to four numbers from the patch as Open Sound
  Control messages to an address, host, and port, on a trigger or
  whenever they change. Changes are sent at most once per `interval`
  (20 ms by default), so a moving signal can't flood the network, and the
  latest values always arrive in the end.
- **Bindings**: drive any unwired number, switch, or trigger input from a
  MIDI controller or note, a computer key, a gamepad button or stick, or
  an OSC address. Right-click the input (or a control node's title) and
  choose **Bind…**, then move the control: the first one that really
  moves is bound, as one undo step (Esc cancels). Controllers and sticks
  sweep the input's range (on a log scale for frequencies, like its
  slider), notes and keys press it, and triggers fire once per press.
- Bound inputs show a small link on the canvas, and their widgets, on the
  canvas and on performance panels, follow the control while the patch
  plays without marking the project changed.
  The Project tab lists every binding (range, invert, momentary or toggle,
  jump to the node, remove); the Node tab lists the selected node's.
  Deleting a node removes its bindings in the same undo step, and grouping
  or ungrouping keeps them with their nodes.
- **Transport buttons**: the Devices tab sets up gamepad transport
  buttons in one click (A play/stop, B stop, X record) and learns a
  control for any transport action. They're stored in the project like
  other bindings.
- Bindings are part of the project, so `audioc run` and offline renders
  honor them too; `audioc inspect` lists them, and `audioc validate`
  warns about bindings whose input is missing or wired.
- **Performance panels.** Pin controls and displays (sliders, knobs,
  buttons, toggles, XY pads, keyboards, number fields, dropdowns, meters,
  scopes, analyzers, readouts) to a panel with `P` or a node's right-click
  menu, and `Ctrl+Tab` (or the Canvas | Panels switch in the transport bar)
  shows them as a grid of live widgets without the graph: a mixer window, a
  drum pad, or a pedalboard built from your own patch. The widgets play
  exactly as on the canvas. Add, rename, reorder, and delete panels from
  their tabs; move cards by dragging, make them bigger or smaller, or
  unpin them, each one undo step. Panels are saved with the project, and
  pinned nodes show a pin badge on the canvas.
- **Performance mode.** `F11` shows the current panel full screen with
  bigger cells and only play/stop, the position, and the panel tabs;
  `Esc` or `F11` leaves.
- **JACK and ASIO builds are covered**: the README explains how to build
  with JACK (Linux) or ASIO (Windows) and pick them, and every change is now
  built with both, with the JACK build playing a project through a JACK
  server.

### Changed

- Building from source on Linux needs the udev development files
  (`libudev-dev` on Debian and Ubuntu) as well as ALSA's; they let the
  editor find gamepads.
- Parameter smoothing now lands on the same samples whatever the block
  size, so a render with controller changes is identical at every block
  size.

## [0.4.0] - 2026-10-18

### Added

- **Blocks**: a group saved to its own `.acblock` file (name, description,
  version, author, tags), so a patch built once can be used in any project.
  Projects keep a copy of every block they use and remember where it came
  from, so a newer version in the library can update them.
- `audioc blocks` lists the block library, and `audioc block-update`
  brings a project's copies of library blocks up to date (`--check` only
  reports).
- **Packs**: `audioc pack song.acproj` bundles a project and the audio files
  it uses into one `.acpack` to share, copying in files from outside the
  project's folder; `audioc unpack` unpacks one, and never writes outside
  the folder it unpacks into.
- **Blocks in the editor.** The palette (Shift+A) and the Library tab find
  blocks by name, tag, or description; the Library tab lists shipped, your
  own, and imported blocks with origin filters and tag chips, and imports
  block files. A group's Node tab saves it to your library, and when the
  library has a newer version of a block a project uses, the Node and
  Project tabs offer to update it (one undo step; values you set carry
  over).
- **File ▸ Open** opens packs (unpacked into a folder next to them) and
  imports block files; **File ▸ Export Pack…** shares the current project
  with its audio. Blocks a pack carries join your library.
- Shipped blocks: **Tremolo**, **Compressor**, **Limiter** (keeps peaks under a
  ceiling, with lookahead), **Noise Gate**, **Sidechain Ducker**,
  **Multiband Compressor**, **Track** (volume, pan, mute, and solo for a
  mix), **Metronome**, and **Tape Stop**.
- Instrument blocks for Voices (tick **Polyphonic** and play MIDI into
  them): **Karplus-Strong String** (a plucked string, in tune across the
  keyboard), **Subtractive Synth Voice** (two detuned saws through a ladder
  filter with an envelope sweep), and **FM Operator Pair**. Rhythm and
  voice blocks: **Drum Machine** (a 16-step pattern playing synthesized
  kick, snare, and hats), **Euclidean Drummer**, and **Vocoder** (12 bands).
- Space and modulation blocks: **Plate Reverb** (a figure-eight tank),
  **Hall Reverb** (parallel damped combs into allpasses), **Chorus**,
  **Flanger**, **Phaser** (2–4 notches), **Vibrato** (depth in semitones),
  **Auto-Wah**, and **Stutter** (repeats a beat-sized slice).

### Changed

- File ▸ Open shows the file dialog first and asks about unsaved changes
  only when the file will replace the open project.

## [0.3.0] - 2026-10-18

Milestone M2.2 of the roadmap: the Phase 2 node library, about eighty new
nodes across every category, with editors for the ones you draw rather than
type.

### Added

- **Generators:** **Noise** (white, pink, brown; seeded), **Graph** (a
  hand-drawn curve over seconds or beats), **Sampler** (plays a buffer or an
  audio file at any pitch, with start, end, and crossfaded loop points),
  **Wavetable** (morphs through a table built in or taken from a buffer,
  band-limited), and **Impulse** (a one-sample click).
- **Level and dynamics:** **Crossfade**, **Envelope Follower**, **Gain
  Computer**, and **Lookahead** (whose undelayed `early` output feeds a
  detector so a limiter sees peaks before they arrive), **Shaper**
  (oversampled waveshaping with built-in or hand-drawn curves), and
  **Normalize** (to a target peak or loudness in LUFS, over the whole
  signal).
- **Filters:** **SVF** (lowpass, bandpass, highpass, and notch at once),
  **Ladder** (four-pole with drive; self-oscillates in tune), **Comb**,
  **Allpass**, and **DC Block**.
- **Time:** **Freeze** (holds a seamless loop of the input), **Pitch Shift**
  (with formant correction, so a voice still sounds like itself), and
  **Diffuser** (eight modulated allpasses, stereo).
- **Spectral:** **FFT** and **IFFT** (which line up exactly with a dry path:
  their latency is compensated through the spectrum), **Band Split**
  (crossover or FFT bands that sum back flat), **Spectral Gate**, **Spectral
  Shift**, and **Pitch Detect**.
- **Math:** **Clamp**, **Logic**, **Convert** (decibels, notes, ratios,
  beats, samples), **Smooth**, **Integrate**, **Differentiate**, **Random**,
  **Sample & Hold**, **Quantize**, **Counter**, **Accumulate**, and
  **Select** and **Switch**, which route any signal type and crossfade audio.
- **Modulation:** **LFO** (in hertz or locked to the beat), **Envelope**
  (breakpoints with sustain and loop points), **Step Sequencer**, and
  **Random Walk**.
- **Triggers and gates:** **Threshold Trigger**, **Frequency Trigger**,
  **Onset** (finds attacks in audio), **Clock Divide**, **Clock Multiply**,
  **Trigger Delay**, **Gate To Trigger**, **Trigger To Gate**,
  **Euclidean**, **Probability**, **Burst**, **Latch**, and **Timer**.
- **Sequencing and notes:** **MIDI CC**, **MIDI Filter**, **MIDI Transpose**
  (optionally snapped to a scale), **Pattern** (a grid of step buttons),
  **MIDI Pattern** (a piano-roll clip), **Note To MIDI**, **Arpeggiator**,
  **Chord**, and **Scale Quantize**. None of them leaves a note hanging.
- **Controls:** **Knob**, **XY Pad**, **Keyboard** (a playable piano on the
  canvas), **Number**, and **Dropdown**, each with its own widget on the
  canvas. Playing the Keyboard or touching the XY Pad isn't an edit: it adds
  no undo steps and isn't saved.
- **Display:** **Analyzer**, a 64-band spectrum on the canvas.
- **Utility:** **Mid/Side** encode and decode.
- **MIDI Out** sends MIDI to hardware and other programs, keeping each
  message's timing within the block. It can re-channel everything, ends the
  notes it started when the transport stops, and has a `panic` trigger. Pick
  the port in the Devices tab or with `audioc run --midi-out <name>`;
  `audioc devices` now lists MIDI outputs too.
- **File Out** renders its input to a file of its own during `audioc render`,
  so one render can produce stems, dry and wet versions, or a click track
  next to the mix. Paths are relative to the main output's folder.
- `audioc render --midi-out song.mid` writes what MIDI Out nodes sent to a
  standard MIDI file, on the project's beats and with its tempo changes.
- **Node editors** in the Node tab, with a larger editor in its own window:
  draw breakpoint curves for Graph, Envelope, and Shaper; paint steps on
  Pattern's grid; draw, move, and stretch notes in MidiPattern's piano roll;
  drag BandSplit's crossover frequencies; and draw StepSequencer's values.
  A whole drag is one undo step.
- The Node tab edits the new nodes' settings (lists of choices, optional
  limits, chord intervals, FFT sizes).
- `audioc bench` has stress projects for the expensive new nodes:
  `fft-chains-16`, `pitch-shift-16`, and `diffuser-16`.

## [0.2.0] - 2026-10-18

Milestone M2.1 of the roadmap, engine depth: polyphony, nodes that need
their whole input, latency compensation, and the use of every core.

### Added

- **Polyphony** with the **Voices** node: any group becomes a polyphonic
  instrument. Its body runs once per voice (1 to 64), a **Voice** node inside
  gives each copy its note, pitch, gate, velocity, and index, and the voices'
  outputs are summed. Busy voices are stolen by age or pitch (or new notes are
  ignored), the sustain pedal holds notes, and bend and controllers reach
  every voice.
- **Whole-input nodes and baking.** Nodes that need all of their input first,
  starting with the new **Reverse** node, render their upstream part of the
  patch in the background and play the result in time with the transport.
  Editing anything upstream bakes again while the old result keeps playing;
  a live source such as Audio In disables them, and offline renders always
  bake them first.
- **Latency compensation.** Where paths of different latency meet, the engine
  delays the faster ones so everything lines up, including sample-rate
  modulation, triggers, and MIDI. The new **Latency** node delays by a set
  number of samples and reports the latency at any point of the patch.
- **Info** node: sample rate, block size, CPU load, dropouts, and latency as
  signals.
- **Multi-core processing.** Independent parts of a patch (voices, parallel
  chains) can run on a pool of real-time worker threads, with exactly the
  same output as on one core, and per-core load is reported.
- **Spectrum and Buffer signal types**, for FFT frames and shared audio
  buffers, with their own socket colors and square sockets.
- **Record** has a `buffer` output holding the last finished take, so a
  recording can be played back straight away.
- **Editor**: a Voices node from the palette comes with a playable synth
  voice inside, any group can be made polyphonic from the Node tab, and
  double-clicking a Voices node edits its body. Node headers show when a
  node is disabled, baking, or baked, and how much latency it adds. The
  Project tab shows the output latency, and the Devices tab sets the number
  of engine threads and shows the load on each.
- **Command line**: `audioc run --threads N` runs the engine on several
  cores and bakes whole-input nodes in the background; `audioc validate`
  lists disabled nodes, latency, and compensation; `audioc bench --threads N`
  shows the multi-core speedup on two new stress projects (parallel chains
  and a 16-voice polyphonic synth).

## [0.1.0] - 2026-10-17

The first release: Phase 1 of the roadmap, the foundation. The engine, 32
nodes, the command line, and the desktop editor work on macOS, Windows, and
Linux. Use with real audio and MIDI hardware still needs checking by a
person (see the M1.7 table in `docs/ROADMAP.md`).

### Added

- **Projects** as JSON files with embedded group definitions; loading and
  saving round-trips exactly, and unknown node kinds are kept rather than
  dropped. Copy and paste between projects with fresh node ids.
- **Compiler**: flattens groups, infers rates and channel counts, inserts
  the few implicit conversions, allows cycles only through Feedback nodes,
  and reports problems by node name while the rest of the graph keeps
  running.
- **Engine**: block-based scheduling with buffer reuse, state that carries
  over while you edit, an exact transport and tempo map with tempo and
  meter changes, and the same samples offline and live at any block size.
- **32 nodes**: Audio In, Audio Out, MIDI In, Record, Oscillator, Clip,
  Transport, Gain, Pan, Mix, Biquad, Delay, Constant, Math, Unary, Compare,
  Map, ADSR, Clock, MIDI Note, Button, Toggle, Slider, Meter, Scope,
  Readout, Feedback, Reroute, Group, Split Channels, Merge Channels, and
  Comment. Generators share start, stop, reset, and loop controls. The
  reference in `docs/reference/` is generated from the nodes' own docs.
- **Devices**: audio in and out on CoreAudio, WASAPI, ALSA, and (as feature
  builds) ASIO and JACK, with sample rate and buffer size selection; MIDI
  input placed to the sample; recording to WAV through a disk thread;
  round-trip latency measurement with a loopback cable; and a null device
  for machines without audio hardware.
- **Command line** (`audioc`): `render`, `run`, `devices`, `latency`,
  `validate`, `inspect`, `nodes`, `fmt`, and `bench`.
- **Editor** (`audio-constructor`): a node canvas with pan and zoom, a
  searchable add palette, type-checked wiring, auto-wire from a dropped
  wire, insert on wire, delete with reconnect, duplicate, mute, collapse,
  frames, reroutes, groups with exposed sockets, and undo for all of it;
  inline controls, live meters, scopes, and flowing wires; Node, Library,
  Project, Devices, and Problems tabs; a transport bar; save, open, recent
  projects, and asset handling with missing-file reports.
- **Examples**: Start, Guitar Pedalboard, Subtractive Synth, Four Tracks,
  and Reverse Delay (a preview of a Phase 2 node).
- **Releases**: every merged merge request publishes a release with
  binaries for Linux (x86-64), macOS (Apple silicon), and Windows (x86-64).
  The binaries are unsigned for now.

[0.10.0]: https://github.com/Andorbal/audio-constructor/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/Andorbal/audio-constructor/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/Andorbal/audio-constructor/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/Andorbal/audio-constructor/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/Andorbal/audio-constructor/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/Andorbal/audio-constructor/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/Andorbal/audio-constructor/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/Andorbal/audio-constructor/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/Andorbal/audio-constructor/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Andorbal/audio-constructor/releases/tag/v0.1.0
