# Roadmap

Three phases, each a large chunk of work delivered through many small
commits and a merge request per milestone. Finish a phase before starting the
next; within a phase, milestones are ordered but later ones can start when
their inputs exist.

Check boxes as milestones land. Update acceptance notes with what was
actually verified and how.

---

## Phase 1 — Foundation: it runs, it records, it plays, the editor is ready

**Goal.** A person can open the app on macOS, Windows, or Linux, plug in a
guitar, hear it through a filter and a delay in real time, record the
result, play it back from a Clip node, build a small synth from an
Oscillator, ADSR, and Biquad driven by a MIDI keyboard, save the project,
and reproduce the same audio from the command line.

### M1.1 Workspace and CI
- [x] Cargo workspace with the crates in ARCHITECTURE §3 (plugin and web crates as empty placeholders).
- [x] CI: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` on Linux, macOS, Windows; `cargo check --target wasm32-unknown-unknown` for `ac-core`, `ac-dsp`, `ac-nodes`, `ac-engine`.
- [x] `rust-toolchain.toml`, `deny.toml` (license and advisory checks), `.editorconfig`.
- [x] `audioc --version` and `audio-constructor` binaries build and run (empty window).

### M1.2 Core model and persistence
- [ ] Graph, NodeInstance, Wire, GroupDef, Project types with serde and `schema_version`.
- [ ] Signal types, units, ranges; `Signature` with dynamic sockets from config.
- [ ] `Node` trait, `node!` macro, `Registry`.
- [ ] Round-trip tests: load → save produces identical JSON; unknown node kinds preserved.
- [ ] Clipboard format (subgraph as JSON) with id remapping on paste.

### M1.3 Compiler and offline engine
- [ ] Group flattening with namespaced ids; `Feedback`-only cycles; compile errors as a report, never a panic.
- [ ] Rate and channel inference; implicit conversions.
- [ ] Topological schedule; buffer pool with lifetime reuse; state carry-over on recompile.
- [ ] Transport and tempo map with exact beat ↔ sample conversion (tests at several rates and tempos, including tempo changes).
- [ ] Offline renderer; `audioc render` to WAV (16/24/32f); `audioc validate`, `inspect`, `nodes`.
- [ ] Golden-render harness under `tests/golden/`; determinism test (offline vs. null real-time host at block sizes 32 and 1024).
- [ ] `assert_no_alloc` guard in debug builds around `process`.

### M1.4 Phase 1 nodes
Every node in NODES.md marked **P1**, with unit tests and at least one golden project per category:
AudioIn, AudioOut, MidiIn, Record, Oscillator, Clip, Transport, Gain, Pan, Mix, Biquad, Delay, Constant, Math, Unary, Compare, Map, ADSR, Clock, MidiNote, Button, Toggle, Slider, Meter, Scope, Readout, Feedback, Reroute, Group, SplitChannels, MergeChannels, Comment.
- [ ] Generator contract implemented (start-on-play dropdown, stop/reset/loop/reverse/rate where applicable).
- [ ] `audioc nodes --docs` generates `docs/reference/` from node doc comments.

### M1.5 Real-time runtime and devices
- [ ] cpal host with device enumeration, sample rate and buffer size selection, hot-swap of graphs, parameter ring with smoothing, telemetry triple-buffers, xrun counter.
- [ ] midir input with timestamps → block offsets.
- [ ] Null host for tests and headless CI.
- [ ] Recording through a disk thread; files land in `<project>/assets/`.
- [ ] `audioc run` (headless live); `audioc devices`.
- [ ] Latency measurement (loopback test in the Devices panel, manual).

### M1.6 Desktop editor
- [ ] eframe shell, theme module, fonts, icons; dark theme.
- [ ] Node canvas: pan/zoom/select/move, add palette with fuzzy search, wiring with type checking, drop-on-empty-canvas filtered palette with auto-wire, insert on wire, delete with reconnect, duplicate, mute, collapse, frames, reroute, undo/redo for everything.
- [ ] Inline widgets per unit; wired-input live values; socket meters; wire flow for audio; event pulses.
- [ ] Group create/enter/leave/ungroup with exposed sockets.
- [ ] Side panel tabs: Node (with docs), Library, Project, Devices, Problems.
- [ ] Transport bar; keyboard shortcuts from UX.md.
- [ ] Save/load/recent projects; asset handling; missing-file handling.
- [ ] egui_kittest tests for: add node via palette, wire two nodes, undo, group and enter.

### M1.7 End to end, examples, baseline
- [ ] Example projects: Start, Guitar Pedalboard (AudioIn → Biquad → Delay → AudioOut), Subtractive Synth (MidiIn → MidiNote → Oscillator → Biquad → ADSR-driven Gain → AudioOut), Four Tracks (4 × Track block → Mix → AudioOut), Reverse Delay placeholder (documents the Phase 2 node).
- [ ] `audioc bench` with three stress projects and a baseline file committed; CI bench job comparing against `main`.
- [ ] Manual verification on all three OSes recorded in this file (who, which device, buffer size, measured latency, xruns over 10 minutes).

**Phase 1 acceptance**
- Live guitar through the Pedalboard example at 128-frame buffers with zero engine-caused xruns over 10 minutes on each OS.
- Record 30 s, play back from a Clip, save, close, reopen, same result.
- `audioc render` of the Subtractive Synth example with a MidiPattern stand-in (a Clock-driven note) matches the in-app offline export bit-for-bit.
- All CI jobs green, including wasm checks and the bench comparison.

---

## Phase 2 — The Library: every node, polyphony, baking, scale

**Goal.** The full starting catalog from NODES.md, polyphonic synths, whole-
input nodes with background baking, latency compensation, multi-core
scheduling, shareable blocks, and a dozen example projects that show what
the tool is for.

### M2.1 Engine depth
- [ ] `Voices` group instancing and the `Voice` node; voice stealing policies.
- [ ] Causality analysis, disabled badges, background baking with invalidation; offline multi-pass rendering for whole-input nodes.
- [ ] Latency compensation across merge points; `Latency` and `Info` nodes.
- [ ] Multi-core scheduler with real-time worker pool; per-core load telemetry; stress tests under `audioc bench`.
- [ ] `Buffer` and `Spectrum` signal types; `Record` emits a `Buffer`.

### M2.2 Nodes
- [ ] Every node in NODES.md marked **P2**, in category order: Generators, Level/Dynamics, Filters, Time, Spectral, Math, Modulation, Triggers, Sequencing, Controls, Display, I/O, Utility.
- [ ] Node editors for `Graph` (curve editor), `Pattern` (grid), `MidiPattern` (piano roll), `BandSplit` (band editor), `StepSequencer`.
- [ ] Golden projects for each category; benches for the expensive ones (FFT, PitchShift, Diffuser).

### M2.3 Blocks and sharing
- [ ] Block library directory, `.acblock` save/load, metadata, versioning, "update instances from library".
- [ ] All blocks listed in NODES.md built and tested as golden projects.
- [ ] `audioc pack`/`unpack`; the app opens `.acpack`.
- [ ] Library tab shows user, shipped, and imported blocks with search and tags.

### M2.4 Control surfaces
- [ ] Gamepad (gilrs), KeyboardIn, OSC in/out nodes; binding/learn UI for MIDI, keys, gamepad; transport bindings.
- [ ] ASIO build (feature `asio`) documented and tested on Windows; JACK on Linux.
- [ ] Panels: pin nodes, multiple panels, full-screen performance mode.

### M2.5 Editor conveniences
- [ ] Auto-layout, align/distribute, portals, find, lazy connect, connect-selected-to-new-node, per-node timing display, minimap.
- [ ] Light theme; UI scale.
- [ ] Node help with "try this" tips and links to blocks.

### M2.6 Examples and docs
- [ ] Examples: Sidechain Pump, Drum Machine, Build a Keyboard, Reverse Delay, Gamepad Jam, Multiband Compressor, Vocoder, Generative Patch (Euclidean + Random + ScaleQuantize), Tape Loop, Guitar-to-Synth (PitchDetect).
- [ ] Generated node reference published with the repo; user guide in `docs/guide/`.

**Phase 2 acceptance**
- A 16-voice subtractive synth with per-voice filter and envelope runs at 128-frame buffers on a 4-core laptop under 30 % CPU.
- The Reverse Delay example shows the Reverse node disabled when fed from AudioIn and baked when fed from a Clip, and renders correctly offline.
- A block exported from one project imports into another and updates its instances when the library copy changes.
- The bench suite shows the multi-core scheduler scaling on independent chains.

---

## Phase 3 — Reach: plugins, web, and polish

**Goal.** The engine runs inside other hosts and in the browser, third-party
plugins run as nodes, and the product is packaged for people who are not
developers.

### M3.1 Plugin build
- [ ] `ac-plugin` with nih-plug: CLAP and VST3, effect and instrument variants, host-automatable parameters chosen from the project, editor in the plugin window.
- [ ] State save/restore through the host; sample rate and block size from the host.

### M3.2 Plugin hosting
- [ ] CLAP plugin scanner and `Plugin` node with dynamic sockets from the plugin's ports and parameters; plugin GUI in a floating window.
- [ ] VST3 hosting (licensing per DECISIONS); sandboxing or crash isolation investigated.

### M3.3 Web
- [ ] `ac-web`: engine in an AudioWorklet, eframe UI, SharedArrayBuffer rings, Web MIDI, Gamepad API; static hosting with cross-origin isolation headers.
- [ ] Projects open from and save to the browser's file API; examples load from the site.

### M3.4 Engine extras
- [ ] Single-sample feedback for strongly connected components.
- [ ] `TimeStretch`, `Convolution`, `Granular`, `Formant`, `Chaos`, `SpectralFreeze`, `Spectrogram`.
- [ ] Native PipeWire backend if the shims prove limiting.

### M3.5 Product
- [ ] Installers and signed builds (cargo-dist), auto-update check, crash reporting opt-in.
- [ ] Block sharing: export with preview render, import from URL, library index format.
- [ ] Theming, localization scaffold, full accessibility pass.

**Phase 3 acceptance**
- A project runs as a CLAP effect in a host, with its exposed parameters automatable.
- A third-party CLAP synth runs as a node inside a project.
- The web build runs the Start, Pedalboard (with browser mic), and Synth examples.
