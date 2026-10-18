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

## [0.5.0] - 2026-10-18

### Added

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

### Changed

- Building from source on Linux needs the udev development files
  (`libudev-dev` on Debian and Ubuntu) as well as ALSA's; they let the
  editor find gamepads.

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

[0.5.0]: https://github.com/Andorbal/audio-constructor/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/Andorbal/audio-constructor/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/Andorbal/audio-constructor/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/Andorbal/audio-constructor/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Andorbal/audio-constructor/releases/tag/v0.1.0
