# Architecture

This document describes how Audio Constructor is built. It is written for
whoever implements it, human or agent, and it is opinionated: the choices here
were made deliberately (see [DECISIONS.md](DECISIONS.md)). Change them by
adding a decision record, not by drifting.

Contents

1. [Goals and constraints](#1-goals-and-constraints)
2. [Technology choices](#2-technology-choices)
3. [Workspace layout](#3-workspace-layout)
4. [The graph model](#4-the-graph-model)
5. [Signal types and the value model](#5-signal-types-and-the-value-model)
6. [Time, transport, and the clock](#6-time-transport-and-the-clock)
7. [Compilation and scheduling](#7-compilation-and-scheduling)
8. [The real-time runtime](#8-the-real-time-runtime)
9. [Causality: live mode vs. baked nodes](#9-causality-live-mode-vs-baked-nodes)
10. [Offline rendering and determinism](#10-offline-rendering-and-determinism)
11. [Device layer](#11-device-layer)
12. [Persistence: projects, blocks, assets](#12-persistence-projects-blocks-assets)
13. [Hosts: app, CLI, plugin, web](#13-hosts-app-cli-plugin-web)
14. [Testing and performance](#14-testing-and-performance)

---

## 1. Goals and constraints

- **Everything is a node or a wire.** There is no separate "track" or "clip
  lane" data model. Views are projections of the graph.
- **Every parameter is a socket.** Any node input can be driven by a wire, and
  an unwired input shows an inline control.
- **Real-time capable.** A guitar plugged into an input must pass through a
  patch with latency bounded by the device buffer size, with no dropouts
  caused by the engine itself.
- **Deterministic.** Rendering the same project twice, live or offline, gives
  the same samples. Tempo is exact to the sample.
- **Scales down and up.** Modest patches run on a Raspberry Pi class machine;
  big patches use every core available.
- **Headless first.** The engine has no dependency on the UI. The CLI, the app,
  the plugin build, and the web build are all thin hosts over the same engine.
- **Portable.** Native on macOS, Windows, Linux; the engine crates must compile
  for `wasm32-unknown-unknown` from day one so the web build stays reachable.

## 2. Technology choices

| Concern | Choice | Why |
| --- | --- | --- |
| Language | **Rust** (stable, 2024 edition) | No garbage collector on the audio thread, first-class WebAssembly target, strong audio ecosystem, memory safety for a long-running autonomous build. |
| UI | **egui** via **eframe** with the **wgpu** backend | Immediate mode makes live-updating meters and animated wires trivial, it renders through wgpu on all desktop platforms and the web, and the node canvas is fully custom-drawn so the app can have its own look. |
| Audio I/O | **cpal** | CoreAudio, WASAPI, ASIO (feature flag), ALSA, JACK, Web Audio behind one API. |
| MIDI | **midir** | Cross-platform, including Web MIDI. |
| Game controllers | **gilrs** | Cross-platform, including the browser Gamepad API. |
| OSC | **rosc** | Phones and tablets as control surfaces. Phase 2. |
| FFT | **realfft** / **rustfft** | Fast, pure Rust, wasm-friendly. |
| Serialization | **serde** + JSON | Human-readable, diffable project files. |
| Lock-free plumbing | **rtrb** (SPSC rings), **arc-swap**, **triple_buffer** | Audio thread never blocks. |
| Thread priority | **audio_thread_priority** | Real-time scheduling for worker threads. |
| Plugin build | **nih-plug** (CLAP + VST3) | Phase 3. Same engine, host-provided buffers. |
| Plugin hosting | **clack** (CLAP), **vst3-sys** (VST3) | Phase 3. See licensing notes in DECISIONS. |
| Benchmarks | **criterion** plus an `audioc bench` subcommand | Per-node and whole-graph throughput, tracked over time. |
| UI tests | **egui_kittest** | Drive the editor without a display; snapshot where a software renderer is available. |

Deliberately **not** used: a web front end plus IPC (an extra process boundary
in the hot path and two languages to keep in sync), JUCE (C++, weak wasm story,
licensing), platform-native toolkits (the app has its own look everywhere).

## 3. Workspace layout

```
audio-constructor/
  Cargo.toml              # workspace
  crates/
    ac-core/              # graph model, signal types, node trait, registry, serde. No I/O. wasm-safe.
    ac-dsp/               # pure DSP primitives: oscillators, filters, envelopes, FFT helpers, resampling. wasm-safe.
    ac-nodes/             # every built-in node, one file each, registered with ac-core. wasm-safe.
    ac-engine/            # compiler, scheduler, transport, real-time runtime, offline renderer, baking. wasm-safe.
    ac-io/                # device layer: cpal, midir, gilrs, rosc, file I/O. Feature-gated per backend.
    ac-cli/               # `audioc` binary
    ac-app/               # `audio-constructor` desktop binary (eframe)
    ac-plugin/            # Phase 3: nih-plug wrapper
    ac-web/               # Phase 3: wasm glue, AudioWorklet host
  examples/               # example projects shipped with the app
  blocks/                 # built-in reusable blocks (group definitions)
  tests/golden/           # reference projects and expected renders
  docs/
```

Dependency direction is strictly downward: `ac-app`/`ac-cli` → `ac-io` →
`ac-engine` → `ac-nodes` → `ac-dsp` → `ac-core`. CI runs
`cargo check --target wasm32-unknown-unknown` for `ac-core`, `ac-dsp`,
`ac-nodes`, and `ac-engine` on every change.

## 4. The graph model

Defined in `ac-core`.

```
Project
  ├── settings: sample_rate, length, seed, tempo_map, ...
  ├── graph: Graph                      # the root graph
  └── group_defs: Map<GroupId, GroupDef> # embedded group definitions

Graph
  ├── nodes: Map<NodeId, NodeInstance>
  └── wires: Vec<Wire>

NodeInstance
  ├── id: NodeId (stable, random u64; survives copy/paste by remapping)
  ├── kind: NodeKind (registry key, e.g. "osc", or Group(GroupId))
  ├── config: Config (serde Value; things that change the socket list, e.g. mixer input count)
  ├── params: Map<SocketName, Value> (values for unwired inputs)
  ├── ui: position, size, collapsed, color override, label
  └── muted: bool

Wire
  ├── from: (NodeId, OutputSocketName)
  └── to:   (NodeId, InputSocketName)

GroupDef
  ├── id, name, description, version, author, tags
  ├── graph: Graph
  └── interface: ordered inputs/outputs, each mapping to an inner socket, with type and default
```

Rules:

- Sockets are addressed by **name**, never by index, so files survive node
  evolution.
- A node's socket list is a function of its `config` (`describe(config) ->
  Signature`). Changing config re-describes and recompiles.
- An input accepts at most one wire. An output fans out freely.
- A `Group` node instance references a `GroupDef` by id. Editing a def updates
  every instance (Blender semantics). Projects embed every def they use, so
  they are self-contained; a def remembers the block it came from so the app
  can offer updates.
- Unknown node kinds are preserved on load and shown as a "missing node"
  placeholder so a project never fails to open.

### The node trait

Every built-in node implements one trait and lives in one file.

```rust
pub trait Node: Send {
    /// Static metadata: name, category, docs, keywords for search.
    fn meta() -> NodeMeta where Self: Sized;
    /// Sockets, given this instance's config. May be dynamic (N inputs).
    fn describe(config: &Config) -> Signature where Self: Sized;
    /// Build runtime state for a given sample rate and max block size.
    fn new(config: &Config, ctx: &PrepareCtx) -> Self where Self: Sized;
    /// Process one block. Inputs are typed accessors; outputs are typed buffers.
    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io);
    /// Samples of delay this node introduces (for latency compensation).
    fn latency(&self) -> u32 { 0 }
    /// Streaming (can run live) or WholeInput (needs the entire input first).
    fn causality(&self) -> Causality { Causality::Streaming }
    /// For source nodes: can the output be computed ahead of time?
    fn determinism(&self) -> Determinism { Determinism::Deterministic }
    /// Reset internal state (on transport stop/restart).
    fn reset(&mut self);
}
```

A small `node!` declarative macro keeps socket declarations terse. Nodes are
registered in a `Registry` that the compiler, the CLI (`audioc nodes`), the
add-node search, and the documentation generator all read from.

## 5. Signal types and the value model

Keep the type set small. Each type has a fixed socket/wire color (see UX.md).

| Type | Carries | Rate |
| --- | --- | --- |
| `Audio` | 1..N channels of `f32`, nominal range -1..1 | per-sample |
| `Float` | one `f32` with a unit hint (`Hz`, `dB`, `Seconds`, `Beats`, `Ratio`, `Semitones`, `Generic`) and a soft range | constant, per-block, or per-sample (see below) |
| `Int` | `i32` | constant or per-block |
| `Bool` | gate: `true`/`false` | constant, per-block, or per-sample |
| `Trigger` | list of events in this block, each with a sample offset | event |
| `Midi` | list of MIDI events (note on/off, CC, pitch bend, aftertouch, channel) with sample offsets | event |
| `Spectrum` | frames of complex bins from an FFT node (Phase 2) | frame |
| `Buffer` | shared, immutable handle to an audio buffer (clip, recording) (Phase 2) | constant |

### Rates

A `Float` or `Bool` input can be fed at three rates, and the engine decides
which at compile time:

- **Const**: the input is unwired; its value comes from `params`. Changing it
  from the UI goes through the parameter queue, not a recompile.
- **Block**: one value per block. Produced by UI controls, transport values,
  smoothed parameters, and anything else that doesn't need audio-rate
  resolution.
- **Sample**: one value per sample. Produced by oscillators, envelopes, audio
  → float conversions, and math on sample-rate inputs.

Nodes read inputs through an accessor that hides the rate:

```rust
let freq = io.input_float("freq");       // Const | Block | Sample
for i in 0..ctx.frames {
    let f = freq.at(i);                   // branch-free in the Sample case via a match hoisted out of the loop by the helper
    ...
}
```

Helpers like `for_each_frame!` hoist the rate `match` outside the inner loop so
the common cases cost nothing. Rate-polymorphic nodes (Math, Compare, Clamp)
produce output at the highest rate of their inputs; the compiler infers this.
A node's declared output rate is part of its `Signature`.

### Implicit conversions (the compiler inserts these; nothing else is implicit)

- `Float → Audio`: mono stream of the value.
- `Audio (mono) → Audio (N)`: upmix by copying.
- `Int → Float`, `Bool → Float` (0/1).
- `Audio → Float`: sample-rate float from channel 0 (for "use this audio as a modulator").

Everything else (`Trigger ↔ Bool`, `Midi → Float`, `Audio → Spectrum`) is an
explicit node, because the semantics involve choices the user should see.

### Multi-channel audio

Channel count is a compile-time property of each wire, propagated from
sources. Nodes declare whether they are channel-polymorphic (Gain: yes, Pan:
mono in / stereo out). Mismatches that aren't covered by the upmix rule are
compile errors shown on the wire.

## 6. Time, transport, and the clock

Ground truth is an integer: `sample_pos: u64` since transport start. Every
other notion of time is derived from it by a pure function, which is what
makes playback reproducible.

```
Transport
  ├── state: Stopped | Playing | Recording
  ├── sample_pos: u64
  ├── sample_rate: u32            # the engine's running rate
  ├── loop: Option<(u64, u64)>
  ├── length: u64                 # project length in samples; bounds baking and offline renders
  └── tempo_map: Vec<TempoSegment { start_sample, bpm: f64, time_sig: (u8, u8) }>
```

- `beat_at(sample)` and `sample_at(beat)` are computed per tempo segment in
  `f64` relative to the segment start, then rounded to the nearest sample. A
  120 BPM beat at 48 kHz lands on sample 24 000, every time.
- Tempo-synced nodes (Clock, Pattern, LFO in sync mode) ask the transport for
  the exact sample of the next beat boundary and emit their trigger at that
  offset inside the block. No accumulators, no drift.
- Events (`Trigger`, `Midi`) are sample-stamped within the block, so a note
  that lands mid-block starts mid-block.
- The project has a nominal sample rate; the engine runs at the device's
  rate. Nodes are written to be sample-rate independent (they take the rate
  from `PrepareCtx`). Clips are resampled on load if needed.
- There is no external sync (MIDI clock, Ableton Link) in scope.

## 7. Compilation and scheduling

`ac-engine` compiles a `Project` into a `CompiledGraph`, a flat schedule the
runtime can execute without any graph traversal, allocation, or hashing.

Steps:

1. **Flatten groups.** Instances are expanded in place; node ids inside an
   instance are namespaced by the instance path so state is addressable.
   `Voices` nodes (Phase 2) expand their body once per voice.
2. **Validate types and channels.** Insert implicit conversions. Report errors
   per wire/socket, never panic.
3. **Break feedback.** Cycles are legal only through a `Feedback` node, which
   introduces exactly one block of delay (Phase 1) and is the cut point for
   the topological sort. Single-sample feedback is a Phase 3 investigation
   (compile the strongly connected component to a per-sample sub-schedule).
4. **Infer rates** (Section 5) and **channel counts**.
5. **Causality analysis** (Section 9) marks `WholeInput` nodes as enabled,
   disabled, or baked.
6. **Latency compensation** (Phase 2): sum `latency()` along every path and
   insert delays on the shorter inputs of any node that merges paths, so
   an FFT-processed branch lines up with a dry branch.
7. **Topological sort** into an ordered list of steps, then **partition** the
   list into chains that only depend on earlier chains. Phase 1 runs chains
   sequentially on the audio thread; Phase 2 runs independent chains on a
   real-time worker pool with the audio thread participating.
8. **Allocate buffers** once: a pool of audio/float buffers sized to the
   maximum block size, with lifetimes computed from the schedule so buffers
   are reused.
9. **Prepare nodes**: call `new()` for new nodes, **carry over state** for
   nodes whose id and config are unchanged, so recompiling while playing
   doesn't reset a ringing delay line.

Compilation happens off the audio thread (UI thread or a background thread).
Swapping in the result is one atomic pointer exchange at a block boundary.

## 8. The real-time runtime

The audio callback thread obeys the usual rules, enforced by convention,
review, and a debug-mode allocation detector (`assert_no_alloc`):

- **No allocation, no locks, no I/O, no syscalls, no panics** inside
  `process()` or the scheduler.
- **Graph swap**: compiled graphs arrive through an SPSC ring and are
  installed at the start of a block; the retired graph goes back through
  another ring and is dropped on the control thread (see the engine API
  below).
- **Parameters in**: an SPSC ring (`rtrb`) of `(param slot, value)` updates
  (the control side resolves node path and socket name to a slot), drained
  at the start of each block. Values smooth over a few ms by
  default to avoid zipper noise (per-socket opt-out).
- **Events in**: MIDI, gamepad, keyboard, OSC arrive on their own threads and
  are pushed into rings with timestamps from one monotonic clock (one ring
  per source, so no producer ever waits on another; a full ring drops and
  counts). The callback drains them and assigns block offsets **one block
  late**: an event that arrived a quarter of the way through the previous
  callback period lands a quarter of the way into this block,
  `offset = clamp((t_recv − (t_callback − block)) × rate, 0, frames − 1)`.
  That costs one buffer of latency and removes the jitter of quantizing to
  callback boundaries.
- **Device input** comes from a separate input stream through an SPSC ring
  of interleaved samples, drained by the output callback (which owns the
  processor). The same path serves one duplex interface or two devices. The
  ring keeps a one-buffer cushion against callback jitter; when it runs dry
  or (with two clocks drifting) piles up, the callback zero-fills or skips
  back to the cushion and counts a device xrun.
- **Telemetry out**: meters, scope rings, per-socket levels, per-node
  timings, and the xrun counter are relaxed atomics the UI reads at frame
  rate (DECISIONS D22). Nothing waits on the UI.
- **Multi-core** (Phase 2): the schedule's independent chains are pushed to a
  fixed pool of real-time-priority workers; the callback thread works too and
  joins at a barrier before writing the output. If a block overruns, the xrun
  counter increments and the UI shows it; the engine never silently glitches.
- **Recording** writes to a lock-free ring drained by a disk thread; the
  disk thread, not the callback, owns the file.

The engine exposes one API to all hosts. Because the control side and the
audio side live on different threads, an `Engine` is two halves joined by
lock-free rings (`rtrb`):

```rust
pub struct Engine { pub handle: EngineHandle, pub processor: Processor }
impl Engine {
    pub fn new(project: Project, registry: Arc<Registry>, resources: Arc<dyn Resources>,
               options: EngineOptions) -> (Engine, CompileReport);   // never fails; problems are in the report
    pub fn split(self) -> (EngineHandle, Processor);
    pub fn process(&mut self, io: &mut HostIo);                       // offline: both halves on one thread
}
impl EngineHandle {                                                   // control thread (UI, CLI)
    pub fn edit(&mut self, edit: GraphEdit) -> CompileReport;         // recompiles, hot-swaps
    pub fn set_param(&mut self, node: &NodePath, socket: &str, value: Value) -> bool;
    pub fn fire(&mut self, node: &NodePath, socket: &str) -> bool;    // inline trigger button
    pub fn play(&mut self); pub fn stop(&mut self); pub fn seek(&mut self, sample: u64);
    pub fn set_recording(&mut self, on: bool); pub fn set_loop(&mut self, r: Option<(u64, u64)>);
    pub fn telemetry(&self) -> TelemetryReader;
    pub fn collect_garbage(&mut self);                                // drops retired graphs off the audio thread
}
impl Processor {                                                      // audio thread
    pub fn process(&mut self, io: &mut HostIo);
}
```

Graph swap: compiled graphs travel to the processor through a ring and are
installed at a block boundary; node state for unchanged nodes is moved into
the new graph by swapping boxes, and the retired graph travels back through
another ring to be dropped on the control thread. Telemetry is published
through relaxed atomics rather than `triple_buffer` (DECISIONS D22).

## 9. Causality: live mode vs. baked nodes

Some nodes can't stream: `Reverse`, `TimeStretch`, `Normalize`, anything that
must see all of its input first. The rule the user asked for is "disable them
when they can't work, with a UI indication, rather than forcing a mode."

- A source node is **Live** (`AudioIn`, `MidiIn`, `Gamepad`, `KeyboardIn`,
  `OscIn`) or **Deterministic** (`Clip`, `Oscillator`, `Pattern`, `Transport`,
  `Noise` with a seed). UI controls (`Slider`, `Button`) are Deterministic with
  a snapshot: their current value is used, and changing them invalidates the
  bake.
- A `WholeInput` node is **enabled** iff every source upstream of it is
  Deterministic. Otherwise it is **disabled**: it passes silence/zero, and the
  UI badges it ("needs a known input; disabled while live").
- Enabled `WholeInput` nodes are **baked**: a background thread renders their
  upstream subgraph over the project length (or the loop region) and stores
  the result; the node then streams the bake. Bakes are invalidated by any
  upstream edit and recomputed, debounced. The UI shows "baking…" / "baked".
- In an **offline render** (CLI or export) every `WholeInput` node is enabled,
  because the renderer runs the graph in passes: upstream subgraphs first.

This keeps a single project format and a single engine, with no mode switch.

## 10. Offline rendering and determinism

`Engine::process` with an offline `HostIo` renders as fast as the CPU allows.
`audioc render` uses it. Guarantees, enforced by tests:

- The same project at the same sample rate and block size renders
  **bit-identically** on the same machine, live or offline.
- Across machines/platforms, renders match within a small tolerance
  (floating-point library differences). Golden tests compare with a tolerance
  by default and bit-exactly on the reference CI platform.
- All randomness comes from a PRNG seeded from `(project.seed, node path)`.
- Block size must not change the output except for `Feedback` nodes, whose
  delay is defined in blocks, and signals that are block-rate by definition
  (an `Int` such as `Transport.bar`, or a block-rate value that changes
  over time), which update once per block. Sample-accurate sources
  (`Transport.time`, oscillators, envelopes, clocks, MIDI) are block-size
  independent; golden projects declare which kind they are and the test
  suite checks both.

## 11. Device layer

`ac-io` wraps every backend so the engine never names one. Audio goes
through two traits: an `AudioHost` is a configured backend that knows its
rate and buffer size before it starts (so the engine is built at the
device's rate), and `start` turns it into a running `AudioStream` that owns
the engine's `Processor`, reports counters, measures latency, and hands the
processor back when stopped (so switching devices at the same rate doesn't
rebuild the engine). The device backend (cpal) and a timer-paced null host
implement both. MIDI needs no trait: every source, a port or a virtual one
like the computer keyboard, pushes into the same per-source rings, and one
`MidiReceiver` drains them on the audio thread.

| Platform | Backends (cpal) | Notes |
| --- | --- | --- |
| macOS | CoreAudio | Already low latency; aggregate devices supported by the OS. |
| Windows | WASAPI shared + exclusive; **ASIO** (feature `asio`) | ASIO needs the Steinberg ASIO SDK at build time and a signed license to distribute binaries; it ships as an opt-in build. WASAPI exclusive gets most of the way for many interfaces. |
| Linux | ALSA; JACK (feature `jack`); PipeWire via its JACK/ALSA shims | Native PipeWire later if the shims prove limiting. |
| Web | Web Audio `AudioWorklet` | Requires cross-origin isolation headers for `SharedArrayBuffer`. |

- Buffer sizes from 32 to 2048 frames; the settings panel shows the measured
  round-trip latency and the xrun count. Latency is measured with a loopback
  cable: the stream pauses the engine for about a second, plays a burst of
  pseudo-random noise on every output, records every input, and finds the
  burst by cross-correlation.
- Device callback threads ask the OS for real-time priority; a refusal is
  reported, not fatal.
- MIDI via midir, hot-plug aware. Gamepads via gilrs (buttons as `Bool`/
  `Trigger`, axes as `Float`, with per-axis dead zone and curve in the node).
- The computer keyboard is a control source like any other: the host feeds
  key events into the same ring as MIDI.
- A **null host** (no devices, silent, clocked by the caller) exists for
  tests, CI, and the agent's environment, which has no audio hardware.

## 12. Persistence: projects, blocks, assets

- **Project**: `name.acproj`, a JSON document with `schema_version`. Node
  params by socket name; node ids stable. Formatted with stable key order so
  diffs are readable.
- **Block**: `name.acblock`, a JSON `GroupDef` with metadata. Blocks live in a
  per-user library directory and in `blocks/` in the repo (built-ins).
  Projects embed copies of the defs they use.
- **Assets**: audio clips are referenced by relative path plus content hash.
  Recordings land in `<project>/assets/` by default. `audioc pack` bundles a
  project and its assets into a zip (`.acpack`) for sharing; the app opens
  packs directly.
- **Migrations**: a `schema_version` bump comes with a migration function and
  a test fixture of the old format.
- **Clipboard**: copy serializes selected nodes and internal wires as JSON
  text; paste accepts that text. Patches can travel through chat messages.

## 13. Hosts: app, CLI, plugin, web

All hosts call the same `Engine`.

- **App** (`ac-app`): eframe shell. Owns the `Engine`, the device layer, the
  undo stack, and the editor. Edits flow `UI → GraphEdit → Engine::edit`.
- **CLI** (`ac-cli`, binary `audioc`):
  - `audioc render <project> -o out.wav [--from <time>] [--to <time>] [--sample-rate N] [--bit-depth 16|24|32f] [--set node.socket=value]...`
  - `audioc run <project> [--host NAME] [--output-device NAME] [--input-device NAME|none] [--midi NAME|all|none] [--sample-rate N] [--buffer-size N] [--duration TIME] [--no-play] [--record] [--null]` runs live, headless, until Ctrl+C or `--duration`, printing position, load, xruns, and dropped messages every second. Record nodes write takes into `<project>/assets/`. `--null` runs without devices (silent input, real-time paced) for headless smoke tests.
  - `audioc latency [device options] [--count N]` measures the round trip through a loopback cable.
  - `audioc validate <project>` prints the compile report (errors, disabled nodes, latency).
  - `audioc inspect <project>` prints nodes, wires, and groups.
  - `audioc nodes [--json] [--docs <dir>]` lists the registry; `--docs` generates the node reference.
  - `audioc devices` lists audio and MIDI devices.
  - `audioc bench [--project <p>]` renders stress projects and reports the real-time ratio per node and overall.
  - `audioc pack` / `audioc unpack`.
- **Plugin** (Phase 3, `ac-plugin`): nih-plug exposes the engine as a CLAP and
  VST3 effect and instrument. The plugin's audio input and output map to
  `AudioIn`/`AudioOut` nodes; host MIDI maps to `MidiIn`; a configurable set of
  sockets is exposed as host-automatable parameters. The editor UI runs inside
  the plugin window via `nih_plug_egui`.
- **Web** (Phase 3, `ac-web`): the engine runs inside an `AudioWorklet`; the
  UI is eframe's web target. The two communicate through `SharedArrayBuffer`
  rings, the same abstraction as native.

What must be true from day one so Phase 3 is a port, not a rewrite: the engine
crates compile to wasm; the engine never touches devices or files directly;
block size and sample rate are runtime values; the UI talks to the engine only
through the `Engine` API.

## 14. Testing and performance

- **Unit tests per node**: impulse responses, known frequencies, edge cases
  (zero-length input, extreme parameters, block size 1).
- **Golden renders**: `tests/golden/*.acproj` rendered by `audioc render` and
  compared against committed reference hashes/WAVs. Every new node category
  gets a golden project.
- **Determinism test**: render each golden project twice (offline, and through
  the null real-time host with a small block size) and compare.
- **Compiler tests**: cycles without Feedback are rejected; dynamic sockets;
  group flattening; state carry-over on recompile; implicit conversions.
- **Real-time hygiene**: `assert_no_alloc` in debug builds around `process`;
  a test runs every node through a block with the allocator guard active.
- **Benchmarks**: criterion benches in `ac-engine` and `ac-nodes` for per-node
  cost; `audioc bench` renders stress projects (e.g. 1 000 oscillators into a
  mixer; a 64-band FFT split; a 16-voice synth) and prints the real-time
  ratio. CI runs `audioc bench` on every merge request and compares against
  the baseline from `main` on the same runner tier, failing on a regression
  larger than 20 %. Results are also appended to a history file so trends are
  visible.
- **UI tests**: egui_kittest drives the editor (add node, wire, undo) without
  a display. Snapshot tests run where a software renderer is available.
- **Runtime instrumentation**: per-node CPU time and the xrun counter are
  always collected (cheap) and can be shown in the editor, Blender-style.
