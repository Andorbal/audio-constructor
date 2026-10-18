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

[0.2.0]: https://github.com/Andorbal/audio-constructor/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Andorbal/audio-constructor/releases/tag/v0.1.0
