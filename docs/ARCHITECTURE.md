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
15. [Packaging and releases](#15-packaging-and-releases)

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
| UI | **egui** via **eframe** with the **wgpu** backend (on the web, WebGL2 through glow, D152) | Immediate mode makes live-updating meters and animated wires trivial, it renders through wgpu on all desktop platforms and through WebGL in a browser, and the node canvas is fully custom-drawn so the app can have its own look. |
| Audio I/O | **cpal** | CoreAudio, WASAPI, ASIO (feature flag), ALSA, JACK behind one API. The web build uses Web Audio's `AudioWorklet` directly (D150). |
| MIDI | **midir** | Cross-platform. The web build uses Web MIDI directly (D154). |
| Game controllers | **gilrs** | Cross-platform, including the browser Gamepad API. |
| OSC | **rosc** | Phones and tablets as control surfaces. Phase 2. |
| FFT | **realfft** / **rustfft** | Fast, pure Rust, wasm-friendly. |
| Serialization | **serde** + JSON | Human-readable, diffable project files. |
| Lock-free plumbing | **rtrb** (SPSC rings), **arc-swap**, **triple_buffer** | Audio thread never blocks. |
| Thread priority | **audio_thread_priority** | Real-time scheduling for worker threads. |
| Plugin build | **nice-plug** (CLAP + VST3) | The maintained, crates.io-published successor of nih-plug (DECISIONS D117). Same engine, host-provided buffers. VST3 through the MIT/Apache `vst3` bindings (D118). |
| Plugin hosting | **clack** (CLAP), **vst3** (VST3) | Phase 3 (M3.2): `ac-host`, §13. MIT/Apache, so nothing of a plugin standard's license reaches our code (D118, D130). |
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
    ac-app/               # the editor; `audio-constructor` desktop binary (eframe), and the plugin window (egui-baseview)
    ac-plugin/            # the engine as a CLAP/VST3 effect and instrument (nice-plug)
    ac-host/              # hosting CLAP/VST3 plugins as nodes: scanning, loading, their threads, their windows. Native only.
    ac-web/               # the web build: the page (editor, mirror engine, browser devices and files) and the audio worklet's module
  examples/               # example projects shipped with the app
  scripts/release.py      # version bumps, changelog checks, release notes (CLAUDE.md)
  scripts/build_web.py    # the web build's static site (D152); serve_web.py serves it locally
  CHANGELOG.md            # one section per release; the release notes
  blocks/                 # built-in reusable blocks (group definitions)
  tests/golden/           # reference projects and expected renders
  tests/web/              # the web build's end-to-end test in a headless browser
  docs/
```

Dependency direction is strictly downward: `ac-app`/`ac-cli` → `ac-io` →
`ac-engine` → `ac-nodes` → `ac-dsp` → `ac-core`. `ac-plugin` runs its
engine on `ac-engine` directly: a host provides its devices. Its window
(the `editor` feature) is `ac-app` without the desktop shell, which still
links `ac-io` and so the device libraries, though in a plugin it opens no
device (D124). `ac-host` needs only `ac-core`; `ac-cli` and `ac-app` (its
`plugins` feature, part of `desktop`) use it, and the editor inside our own
plugin builds without it, so a plugin binary never hosts plugins. `ac-web`
builds twice for wasm32: its `worklet` build is `ac-engine` and what's
below it behind a few exported functions, and its `ui` build is `ac-app`
without the desktop shell on eframe's web target (which links `ac-io` for
files and opens no device; D150, D152). Natively it builds what both
share, so the rings and the link are tested there. CI runs `cargo check
--target wasm32-unknown-unknown` for `ac-core`, `ac-dsp`, `ac-nodes`, and
`ac-engine` on every change, and builds and plays the web build in a
browser.

## 4. The graph model

Defined in `ac-core`.

```
Project
  ├── settings: sample_rate, length, seed, tempo_map, ...
  ├── graph: Graph                      # the root graph
  ├── group_defs: Map<GroupId, GroupDef> # embedded group definitions
  └── panels: Vec<Panel>                # performance panels: pins to root-graph nodes (views only)

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
    /// Whole-input nodes: every output from every input at once, off the
    /// audio thread; the engine bakes and streams the result (§9).
    fn render_whole(config: &Config, ctx: &WholeCtx, inputs: &WholeInputs) -> WholeOutputs { .. }
    /// Latency probes: the path latency at this node's output (§7).
    fn path_latency(&mut self, samples: u32) {}
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
| `Spectrum` | the FFT frames that completed during the block, each stamped with its sample offset; complex bins `re, im` | frame |
| `Buffer` | shared, immutable handle to an audio buffer (clip, recording) | constant |
| `Text` | a short string (an OSC address, a label): up to 128 bytes of UTF-8 kept inline (`TextBuf`), so the audio thread copies and compares it without allocating | constant or per-block |

### Rates

A `Float` or `Bool` input can be fed at three rates, and the engine decides
which at compile time:

- **Const**: the input is unwired; its value comes from `params`. Changing it
  from the UI goes through the parameter queue, not a recompile. (A `Text`
  input is the exception: the queue carries numbers, so editing typed text
  recompiles, with node state carried over, and the node reads the new text
  on its next block.)
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

### Spectrum and Buffer

A `Spectrum` output declares its frame layout (FFT size and hop) in its
socket spec, or follows its first wired `Spectrum` input, as audio channels
follow. The compiler sizes each spectrum's storage for the most frames a
block can hold, so producers push frames without allocating (DECISIONS
D53). A `Buffer` output holds an `Arc` handle that persists from block to
block; the node that creates a buffer (a host's recorder, an asset loader)
keeps its own handle off the audio thread, so the audio thread only ever
clones and drops handles, never frees audio.

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
- There is no external sync (MIDI clock, network tempo sync) in scope,
  except that the plugin build follows its host's play, stop, and position
  (§13, DECISIONS D121).

## 7. Compilation and scheduling

`ac-engine` compiles a `Project` into a `CompiledGraph`, a flat schedule the
runtime can execute without any graph traversal, allocation, or hashing.

Steps:

1. **Flatten groups.** Instances are expanded in place; node ids inside an
   instance are namespaced by the instance path so state is addressable.
   `Voices` nodes expand into a voice allocator, one copy of their body per
   voice (paths `voices/<voice>/…`), and a summing node per Audio or Float
   output; `Voice` nodes inside each copy get that voice's MIDI stream
   (DECISIONS D59).
2. **Validate types and channels.** Insert implicit conversions. Report errors
   per wire/socket, never panic.
3. **Break feedback.** Cycles are legal only through a `Feedback` node, which
   introduces exactly one block of delay and is the cut point for the
   topological sort. A wired input marked `late` (a display input such as
   a button's `lit`) gets a hidden one-block Feedback of its type in front
   of it first, so a wire into one never closes a cycle (DECISIONS D205). A Feedback node whose `delay` config is `sample`
   closes its loop with one sample instead (DECISIONS D140): the compiler
   finds the strongly connected component of the wire graph that holds it
   (edges out of block-mode Feedback nodes don't count, since those loops
   are already cut) and compiles it to a per-sample sub-schedule, one
   *loop step* in the outer schedule. The loop step's inputs are the wires
   into the component and its outputs the member outputs read outside it;
   each block it runs the members one sample at a time against a
   one-frame store of its own, in the component's topological order with
   each Feedback member split into its read (first) and write (last), so
   the read and the write are a sample apart. To the rest of the compiler
   the loop step is an ordinary step, so chains, waves, buffer reuse, and
   the worker pool are unchanged, and a loop runs on one thread. Members
   stay in the graph's node list (state carries over, resets reach them)
   and are moved into the loop's runner for its step. Their unwired inputs
   are ordinary params that the loop reads at each sample (a ramp at its
   own sample, an inline button only at the first), events from outside
   arrive on their own sample, and block-rate floats computed inside become
   sample-rate. Latency compensation doesn't apply inside a loop. A
   component holding a node that can't run one sample at a time (a
   spectrum socket, whole input, or a device) is reported and keeps one
   block of delay.
4. **Infer rates** (Section 5) and **channel counts**.
5. **Causality analysis** (Section 9) marks whole-input nodes disabled or
   baked.
6. **Prepare nodes**: call `new()` for new nodes, **carry over state** for
   nodes whose id and config are unchanged, so recompiling while playing
   doesn't reset a ringing delay line. A node rebuilt in place because its
   config changed (same path and kind) may **take over** from the one it
   replaces when the new graph is installed (`Node::take_over`, DECISIONS
   D196): a step grid edited while playing keeps its step and keeps
   playing. Nodes are created before steps are built because their latency
   is needed next.
7. **Latency compensation**: sum `latency()` along every path and insert
   delays on the shorter inputs of any node that merges paths, so an
   FFT-processed branch lines up with a dry branch. Audio, sample-rate
   floats, triggers, and MIDI are delayed; block-rate values aren't
   (DECISIONS D56). The delays are internal nodes, so they carry over too.
8. **Topological sort** into an ordered list of steps, then group the steps
   into **chains** and the chains into **waves** (DECISIONS D57, D114). A
   chain is a run of steps one thread works through in order. A step joins
   the chain of what it reads when exactly one chain holds the latest of its
   inputs and either each step of that chain it reads feeds nothing else (a
   linear run) or both belong to the same voice copy of a Voices node;
   device-output steps and a Feedback node's second step never join. A
   chain's wave is one more than the latest wave it reads from, so the
   chains of a wave never depend on each other and can run on different
   cores, while each step reads what earlier steps of its own chain wrote
   straight from the chain's work space. Eight chains of two dozen nodes
   are one wave, not two dozen, so handing waves to the workers costs a
   few hand-offs per block instead of one per node.
9. **Allocate buffers** once: a pool of audio/float buffers sized to the
   maximum block size, with lifetimes counted in waves so buffers are reused
   without two steps of one wave sharing a buffer. A sample buffer that only
   its own chain reads stays out of the pool: the chain hands a few buffers
   of its own from step to step as it runs.

Compilation happens off the audio thread (UI thread or a background thread).
Swapping in the result is one atomic pointer exchange at a block boundary.

## 8. The real-time runtime

The audio callback thread obeys the usual rules, enforced by convention,
review, and a debug-mode allocation detector (`assert_no_alloc`):

- **No allocation, no locks, no I/O, no syscalls, no panics** inside
  `process()` or the scheduler. A hosted plugin's own process call is the
  one exception the allocation guard makes: the plugin is someone else's
  code with its own allocator; everything of ours around it stays under
  the guard (§13, D133).
- **Graph swap**: compiled graphs arrive through an SPSC ring and are
  installed at the start of a block; the retired graph goes back through
  another ring and is dropped on the control thread (see the engine API
  below).
- **Parameters in**: an SPSC ring (`rtrb`) of `(param slot, value)` updates
  (the control side resolves node path and socket name to a slot), drained
  at the start of each block. Values smooth over a few ms by
  default to avoid zipper noise (per-socket opt-out). A ramp counts back
  from its target, so it gives the same samples however blocks split it.
- **Bindings** (D92–D95): the project's bindings (a MIDI controller or
  note, key, gamepad control, OSC address, or a plugin's macro parameter
  (D119) → an input or a transport action) compile to a table inside the
  compiled graph, resolved through the same (path, socket) → slot map as
  parameter changes; bindings to
  missing, wired, or unbindable inputs are compile warnings. Each block,
  the processor matches its MIDI and control events against the table
  (allocation-free, into a preallocated list in offset order) and splits
  the block where a bound event lands, so the parameter change, trigger,
  or transport action happens on that sample at any block size (at most
  64 splits a block). What a binding last set carries over to the next
  graph and is published through relaxed atomics
  (`EngineHandle::binding_values`) for the editor's widgets; a parameter
  message from the editor to the same slot wins. While the editor learns
  a binding, the processor also copies raw events into a small ring
  (`set_input_monitor`, `take_input_events`). Offline renders and the
  null host feed test MIDI and control events through the same path, so
  bindings are deterministic like everything else.
- **Events in**: MIDI, gamepad, keyboard, OSC arrive on their own threads and
  are pushed into rings with timestamps from one monotonic clock (one ring
  per source, so no producer ever waits on another; a full ring drops and
  counts). The callback drains them and assigns block offsets **one block
  late**: an event that arrived a quarter of the way through the previous
  callback period lands a quarter of the way into this block,
  `offset = clamp((t_recv − (t_callback − block)) × rate, 0, frames − 1)`.
  That costs one buffer of latency and removes the jitter of quantizing to
  callback boundaries. MIDI reaches the engine in `HostIo::midi`; keys,
  gamepad buttons and axes, and OSC arguments share one fixed-size event
  type, `ControlEvent`, in `HostIo::controls` (nodes read them with
  `HostAccess::controls`). Both kinds use the same ring code
  (`ac-io::rings`); a stream takes all of its sources and sinks as one
  `EventPorts` (MIDI in and out, controls in, OSC out) and hands them back
  when it stops (D85).
- **Events out**: `MidiOut` nodes run on the device-output path and write
  into a MIDI buffer the host hands the engine with each block
  (`HostIo::midi_out`). The callback stamps each message with the time it
  should leave, one block after the callback that produced it (mirroring
  events in), and pushes it into a lock-free ring; a sender thread writes it
  to the chosen ports when its time comes (D62). `OscOut` nodes write OSC
  messages (address and arguments inline, so building one never allocates)
  into `HostIo::osc_out` the same way; a sender thread encodes each when
  it's due and sends it over UDP to the message's own target (D85).
- **Device input** comes from a separate input stream through an SPSC ring
  of interleaved samples, drained by the output callback (which owns the
  processor). The same path serves one duplex interface or two devices. The
  ring keeps a one-buffer cushion against callback jitter; when it runs dry
  or (with two clocks drifting) piles up, the callback zero-fills or skips
  back to the cushion and counts a device xrun.
- **Telemetry out**: meters, scope rings, per-socket levels, per-node
  timings, and the xrun counter are relaxed atomics the UI reads at frame
  rate (DECISIONS D22). Nothing waits on the UI. Per-node timing runs only
  while `EngineStatus::timing_enabled` is set (the editor's `Ctrl+T`): each
  step adds its time (two clock reads) to its node's running sum, and when
  the host block ends the processor publishes every sum and starts over, so
  a node's reading covers all its steps and sub-blocks and a reader never
  sees a half-counted block. `EngineStatus::block_frames` gives the block's
  budget; `TelemetryReader::node_times` lists every node's time by engine
  path, and the editor smooths them and sums them for groups and voices
  (D105).
- **Multi-core**: each wave's chains are split into jobs (runs of
  consecutive chains of about equal length, a few per thread) that a fixed
  pool of real-time workers and the callback thread claim from one
  lock-free counter;
  the callback thread waits only for jobs another thread has claimed, so a
  sleeping worker costs parallelism, never an xrun. Steps that write the
  device outputs run last in their wave, one by one, so output sums the same
  way on any number of cores: results are bit-identical with or without the
  pool. Per-thread busy time is published as per-core load. The pool is
  opt-in (DECISIONS D57, D58). If a block overruns, the xrun counter
  increments and the UI shows it; the engine never silently glitches.
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
    pub fn binding_values(&self) -> Arc<BindingValues>;               // what each binding last set
    pub fn set_input_monitor(&mut self, on: bool);                    // copy raw events for learning
    pub fn take_input_events(&mut self) -> Vec<InputEvent>;
    pub fn play(&mut self); pub fn stop(&mut self); pub fn seek(&mut self, sample: u64);
    pub fn set_recording(&mut self, on: bool); pub fn set_loop(&mut self, r: Option<(u64, u64)>);
    pub fn telemetry(&self) -> TelemetryReader;
    pub fn collect_garbage(&mut self);                                // drops retired graphs off the audio thread
}
impl Processor {                                                      // audio thread
    pub fn process(&mut self, io: &mut HostIo);
    pub fn install_pending(&mut self) -> u64;                         // swap in a waiting graph now; its generation
    pub fn follow_transport(&mut self, playing: bool, position: Option<u64>, tolerance: u64); // a plugin host's
    pub fn reset(&mut self);                                          // reset every node at the next block
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
  upstream subgraph over the project length and stores the result; the node
  then streams the bake at the transport position. A node author writes only
  `render_whole` (whole inputs in, whole outputs out); the engine captures
  the inputs and streams the result (DECISIONS D54). Bakes are keyed by a
  hash of everything upstream, invalidated by any upstream edit (parameter
  changes debounced), and recomputed while the previous bake keeps playing.
  Bakes render from zero, without looping, at the project's render block
  size, so a bake is identical live and offline (DECISIONS D55). The UI
  shows "baking" / "baked" badges.
- The engine exposes bakes as self-contained jobs (`EngineHandle::
  take_bake_jobs`, `BakeJob::run`, `EngineHandle::finish_bake`): the engine
  crate spawns no threads (it builds for wasm), and hosts run jobs on a
  background thread.
- In an **offline render** (CLI or export) every `WholeInput` node is enabled,
  because the renderer runs the graph in passes: upstream subgraphs first,
  fed the same input audio and MIDI as the render.

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
- Block size must not change the output except for `Feedback` nodes in
  block mode, whose delay is defined in blocks (a sample-mode loop runs one
  sample at a time and doesn't depend on it), and signals that are block-rate by definition
  (an `Int` such as `Transport.bar`, or a block-rate value that changes
  over time), which update once per block. Sample-accurate sources
  (`Transport.time`, oscillators, envelopes, clocks, MIDI) are block-size
  independent; golden projects declare which kind they are and the test
  suite checks both.
- A render can be fed device input, MIDI, and control events (keys,
  gamepads, OSC) at absolute frames, and bakes replay the same input.
- Besides the device outputs, a render collects what `FileOut` nodes
  receive (into memory reserved from the render length, so nothing is ever
  dropped) and what `MidiOut` and `OscOut` nodes send. `audioc render` writes the files
  next to the main output, refusing paths that would leave its folder, and
  `--midi-out` writes the messages as a standard MIDI file (D61).

## 11. Device layer

`ac-io` wraps every backend so the engine never names one. Audio goes
through two traits: an `AudioHost` is a configured backend that knows its
rate and buffer size before it starts (so the engine is built at the
device's rate), and `start` turns it into a running `AudioStream` that owns
the engine's `Processor`, reports counters, measures latency, and hands the
processor back when stopped (so switching devices at the same rate doesn't
rebuild the engine). The device backend (cpal) and a timer-paced null host
implement both. Event sources need no trait: every MIDI source, a port or a
virtual one, pushes into its own ring and one `MidiReceiver` drains them on
the audio thread; every control source (the computer keys, the gamepad
thread, the OSC listener) owns a `ControlSender` and one `ControlReceiver`
drains those.

| Platform | Backends (cpal) | Notes |
| --- | --- | --- |
| macOS | CoreAudio | Already low latency; aggregate devices supported by the OS. |
| Windows | WASAPI shared + exclusive; **ASIO** (feature `asio`) | ASIO needs the Steinberg ASIO SDK at build time and a signed license to distribute binaries; it ships as an opt-in build. WASAPI exclusive gets most of the way for many interfaces. |
| Linux | ALSA; JACK (feature `jack`); PipeWire via its ALSA and JACK layers | The default build's ALSA `default` device is PipeWire's ALSA plugin on a PipeWire desktop (shared with other programs, with PipeWire's quantum on top of the ALSA buffer); a `jack` build run on PipeWire's JACK library (`pw-jack`) is a graph client at PipeWire's quantum. No native PipeWire backend for now (DECISIONS D141); cpal's `pipewire` host would add one as an opt-in feature. |
| Web | Web Audio `AudioWorklet` (the page's own, not cpal) | The engine runs in the worklet (D150); `SharedArrayBuffer` needs cross-origin isolation (D152). Web MIDI, the Gamepad API, and the microphone come in on the page and go through the ring (D154). |

The optional backends are built on every change (DECISIONS D100): CI
builds the editor and `audioc` with `jack` on Linux and plays a project
through a JACK server's dummy driver (`crates/ac-io/tests/backends.rs`),
and builds both with `asio` on Windows, where it checks the ASIO host is
listed and fails cleanly without drivers. Release archives are built
without either feature.

- Buffer sizes from 32 to 2048 frames; the settings panel shows the measured
  round-trip latency and the xrun count. Latency is measured with a loopback
  cable: the stream pauses the engine for about a second, plays a burst of
  pseudo-random noise on every output, records every input, and finds the
  burst by cross-correlation.
- Device callback threads ask the OS for real-time priority; a refusal is
  reported, not fatal.
- MIDI via midir, hot-plug aware.
- Gamepads via gilrs, on their own thread, hot-plug aware. A gamepad's index
  is the first one free when it connects and stays put while it's
  connected; buttons are 0–1, sticks −1 to 1 (right and up positive), and
  the analog triggers are both buttons and 0–1 axes. Unplugging releases
  what was held. Dead zones and curves beyond gilrs's own are the node's.
  Linux builds link libudev. No gamepad system is reported, never fatal
  (D86).
  The Gamepad node gives buttons as `Bool` gates and `Trigger` presses
  and axes as `Float`s, with a radial dead zone for the sticks and a
  response curve (D89).
- OSC via rosc: a UDP listener on a chosen port (off until asked for) turns
  every argument of every message into a control event, stamped on arrival
  (bundles flattened, time tags ignored); OSC out sends from a sender
  thread (D87). The listener and each MIDI port also keep the newest
  messages, with addresses and port names as text, in a mutex-guarded log
  for the editor's Devices tab (D115); the engine never reads it.
- The computer keyboard is a control source like any other: with Play keys
  on, the editor sends key presses and releases into its own ring instead
  of running shortcuts (D88).
- A **null host** (no devices, silent, clocked by the caller) exists for
  tests, CI, and the agent's environment, which has no audio hardware.

## 12. Persistence: projects, blocks, assets

- **Project**: `name.acproj`, a JSON document with `schema_version`. Node
  params by socket name; node ids stable. Formatted with stable key order so
  diffs are readable. Bindings are a `bindings` list (source, target by node
  path and socket or transport action, optional `min`/`max`/`invert`/`mode`),
  left out when empty.
- **Block**: `name.acblock`, a JSON `GroupDef` with metadata (name,
  description, version, author, tags) and the project `schema_version`, plus
  `uses`: the definitions its own group nodes reference. Blocks live in
  `blocks/` in the repo (shipped, compiled into `ac-io` by its build script),
  in a per-user library directory (the person's own), and under its
  `imported/` folder (other people's, from packs or files). A block's id is
  its file name; its **source** is `<origin>:<id>` (`shipped:compressor`).
  Projects embed copies of the defs they use, and a copy records its source,
  so the editor (and `audioc block-update`) can offer the library's newer
  version: the definition is replaced, instance parameters carry over by
  socket name, and wires to sockets the new version lacks are dropped
  (DECISIONS D79–D80).
- **Block previews**: `<id>.preview.wav` beside a block file, a 5 s,
  48 kHz, 16-bit render of the block playing, which the block file
  records as `preview: {file, sha256}` (a record of the file, not part of
  the definition: projects don't copy it, and blocks that differ only
  there are the same block). `ac_io::preview` builds a small project
  around the block, an Audio Out on its first audio output and the
  preview phrase (`crates/ac-io/src/preview/phrase.acblock`, two bars of
  drums, chords, and an arpeggio from ordinary nodes, seeded noise), wired
  by what the block takes: audio inputs get the phrase (the first of
  several the music, the rest the drums), a Voices block the chords,
  other MIDI inputs the arpeggio, trigger inputs the drum hits, and a
  `gate`/`freq` pair on a sound maker the arpeggio. It renders offline at
  a fixed rate, block size, and seed, so a preview is golden-testable by
  hash (every shipped block's is) (D190). `audioc block-export` and the
  editor's Export block file… write a block with its preview; the editor
  plays previews over the patch through an audition tap the device
  callback mixes in after the engine, lock-free (D193, D194).
- **Shared libraries**: a folder of block files and their previews with
  an `index.json`: `format` `audio-constructor/library`, `version` 1,
  `name`, `description`, and `blocks`, each with `id`, `name`,
  `version`, `description`, `tags`, `author`, `schema_version`, `file`,
  `size`, `sha256`, and an optional `preview` (`file`, `size`,
  `sha256`). Links are relative to the index (percent-encoded), so the
  folder works on any static host; hashes are SHA-256. `audioc
  library-index` writes it, compiling every block and leaving out the
  ones that don't load; readers skip entries they can't use and refuse a
  newer `version` (D191).
- **Imports from links** (`ac_io::remote`, downloads through
  `ac_io::net`: https only, except on this machine, and size-limited): a
  link to an `.acblock` is a block, to an `.acpack` a pack, and anything
  else a library index (a folder link means its `index.json`) or, failing
  that, whatever the bytes are. A library's blocks and previews must
  match its index's size and SHA-256 before they're kept. Blocks are
  imported exactly as block files are (D83, `imported:` ids, the preview
  kept beside the block); packs open as pack files do. The editor keeps
  the libraries it lists as links in its settings and caches their
  previews by hash in the platform cache folder; `audioc block-import`
  imports into the editor's library unless told another (D192, D194).
- **Assets**: audio clips are referenced by relative path plus content hash.
  Recordings land in `<project>/assets/` by default. `audioc pack` bundles a
  project and its assets into a zip (`.acpack`) for sharing: a
  `manifest.json`, the project at the root, and its assets at their relative
  paths; assets from outside the project's folder are copied in under
  `assets/` and the packed project points at the copies. Unpacking refuses
  names that would leave its folder (absolute paths, `..`). The app opens
  packs directly, unpacking them into a new folder next to the pack, and
  imports the blocks they carry that its library doesn't know (D81).
- **Migrations**: a `schema_version` bump comes with a migration function and
  a test fixture of the old format.
- **Clipboard**: copy serializes selected nodes and internal wires as JSON
  text; paste accepts that text. Patches can travel through chat messages.

## 13. Hosts: app, CLI, plugin, web

All hosts call the same `Engine`.

- **App** (`ac-app`): the editor, and the desktop shell around it. The
  editor (`EditorApp`) has two hosts, which both call `EditorApp::ui` once
  a frame with the frame's root `Ui` (DECISIONS D124):
  - *The desktop* (`desktop` feature, on by default: eframe on winit and
    wgpu, and the system's file dialogs through `native-dialogs`). The
    editor owns the `Engine`, the device layer, and the undo stack. Edits
    flow `UI → GraphEdit → Engine::edit`.
  - *A plugin window* (`embed` feature, `ac_app::embed`): egui-baseview
    with its OpenGL renderer draws the editor into a child of the host's
    window (D125). With `AudioMode::Hosted` the editor opens no devices
    (no audio stream, MIDI ports, gamepad thread, or OSC sockets); the
    host runs the processor. One `EditorApp` outlives the windows that
    show it (`EditorApp::embedded`, shared as `embed::SharedEditor`), so
    closing and reopening keeps its document, history, and view; it must
    stay `Send` (a compile-time check), since on X11 each window draws on
    a thread of its own. A host that keeps the project itself
    (`EditorApp::host_keeps_project`, D126) lends the editor its engine
    handle each frame (`with_engine`), takes engine edits
    (`take_engine_edit`) and other changes (`take_changes`), and answers
    `HostRequest`s (New, Open, a picked audio file); its automatable
    parameters show in the Devices tab through `host::HostControls`
    (D127). The editor leaves the window's title, size, and closing to
    the host, offers no Quit, and fills the window in performance mode.
- **CLI** (`ac-cli`, binary `audioc`):
  - `audioc render <project> -o out.wav [--from <time>] [--to <time>] [--sample-rate N] [--bit-depth 16|24|32f] [--set node.socket=value]... [--input in.wav] [--midi-out notes.mid]` also writes each `FileOut` node's file next to `out.wav`.
  - `audioc run <project> [--host NAME] [--output-device NAME] [--input-device NAME|none] [--midi NAME|all|none] [--midi-out NAME|all|none] [--sample-rate N] [--buffer-size N] [--duration TIME] [--no-play] [--record] [--null] [--threads N] [--osc-in PORT] [--no-gamepad]` runs live, headless, until Ctrl+C or `--duration`, printing position, load (per core with `--threads`), xruns, and dropped messages every second, and baking whole-input nodes in the background. Record nodes write takes into `<project>/assets/`. `--null` runs without devices (silent input, real-time paced) for headless smoke tests. Gamepads are listened to unless `--no-gamepad` (connects and disconnects are printed); `--osc-in PORT` listens for OSC on that UDP port; OSC Out nodes always send.
  - `audioc latency [device options] [--count N]` measures the round trip through a loopback cable.
  - `audioc validate <project>` prints the compile report (errors, disabled nodes, latency).
  - `audioc inspect <project>` prints nodes, wires, and groups.
  - `audioc nodes [--json] [--docs <dir>]` lists the registry; `--docs` generates the node reference.
  - `audioc devices` lists audio devices, MIDI inputs and outputs, and connected gamepads, and says how to receive and send OSC.
  - `audioc plugins [--json] [--rescan] [--in-process]` lists the CLAP and VST3 plugins in the standard folders (and `CLAP_PATH`, `VST3_PATH`), describing each bundle in a child `audioc scan-bundle` process and caching the result. `render`, `run`, and `validate` host the plugins a project's `Plugin` nodes use (§13, Plugin hosting).
  - `audioc bench [--project <p>] [--threads N]` renders stress projects and reports the real-time ratio per node and overall, and with `--threads` the speedup on N threads.
  - `audioc pack <project> [-o out.acpack]` / `audioc unpack <pack> [-o dir] [--force]`.
  - `audioc blocks [--library DIR] [--json]` lists the block library; `audioc block-update <project>... [--library DIR] [--force] [--check]` updates projects' copies of library blocks.
  - `audioc fmt` canonicalizes `.acproj` and `.acblock` files.
- **Plugin** (`ac-plugin`, DECISIONS D117–D123): nice-plug exports two
  plugins from one library, as CLAP and VST3: **Audio Constructor Effect**
  (stereo, mono, or mono-to-stereo audio, with notes in) and **Audio
  Constructor Instrument** (notes in, stereo out, notes out).
  - *Engine hosting.* Activation builds the engine at the host's sample
    rate and largest block and starts the instance's control thread
    (D122). Each host block goes through `ac_plugin::bridge::Bridge`,
    without allocating (debug builds abort if it does): the host's
    channels are copied into channel-major `HostIo` scratch sized at
    activation (a mono bus widened to a pair), host notes and MIDI become
    `MidiEvent`s at their offsets (for MIDI In nodes and MIDI bindings),
    macros that moved become `Control::Macro` events at the start of the
    framework's sub-block, the transport follows the host's
    (`Processor::follow_transport`, D121), and MIDI Out nodes' messages go
    back to the host. Blocks of any size up to the maximum work; longer
    ones are split. Latency the graph adds is reported to the host.
  - *Parameters.* Sixteen macros (`macro_1` … `macro_16`) are the host's
    parameters; the project's bindings from `Source::Macro` say what each
    drives, with the binding's range and mapping, sample-accurately, and
    the host's readout shows the value it sets (D119).
  - *State.* The host saves the macro values and one JSON document: the
    project, its audio files, and where it came from (D120). Restoring,
    loading a project file, and every edit recompile on the host's main
    thread or the editor's and hot-swap at a block boundary; bad state
    falls back to the default project with a report. New instances start
    with a working patch: a Gain on Macro 1 (effect), or MIDI In into an
    eight-voice Subtractive Synth Voice with Macros 1–4 on its sound
    (instrument).
  - *Editor.* The CLAP plugins' window is the app's editor
    (`ac_plugin::editor`, the `editor` feature, on by default): a
    resizable window, 1200 × 760 points to start, opened inside the
    host's through `ac_app::embed`. Each frame, on the window's thread,
    `editor::frame` locks the session, brings a project the session
    replaced (state restore, file load, default) into the document, runs
    the editor on the session's `EngineHandle`, and hands changes back
    with `Session::set_project` (what the engine hears at once, layout at
    most every quarter second and when the window closes). New, Open, and
    picked audio files go to `reset_to_default`, `load_project_file`, and
    `add_asset`. The macros move through the framework's `ParamSetter` as
    gestures the host records (D127); the transport is the host's. The
    VST3 plugins have no window yet (D128).
  - *Bundles.* `scripts/bundle_plugin.py` builds `Audio Constructor.clap`
    and `Audio Constructor.vst3` (D123).
- **Plugin hosting** (`ac-host`, DECISIONS D130–D138): third-party CLAP
  and VST3 plugins run as `Plugin` nodes in the editor, `audioc render`,
  `run`, and `validate`.
  - *The node.* `ac-nodes`'s `Plugin` node knows its plugin only through a
    `PluginDesc` in its config (`config.plugin`: format, id, name, vendor,
    where it was found, audio ports and their channels, note in and out,
    parameters with the socket each became), written when it's inserted
    from the scanner's catalog. Its sockets come from that alone (D130).
    When it's prepared it asks `Resources::plugin(&PluginRequest)` for a
    `Box<dyn PluginProcessor>`; hosts that run plugins put an
    `Arc<dyn PluginLoader>` (an `ac_host::PluginHost`) in their
    `FileResources`. Without one, or with the plugin missing, the node
    outputs silence and `Node::warning` puts the reason in the compile
    report. The processor's latency is the node's.
  - *A block.* The node copies its audio inputs into buffers allocated
    when it was made, turns parameter inputs into sample-accurate events
    (a still input when it changes, a moving one every 32 samples counted
    from the node's start, D131), merges them before the block's MIDI,
    adds the transport, and calls `PluginProcessor::process`. The CLAP
    processor copies into buffers shaped like the plugin's own ports (a
    description that no longer matches them still works), fills a
    preallocated event list, and calls the plugin; what the plugin sends
    back is parsed straight into fixed lists: notes and MIDI to
    `midi_out`, parameter moves into a ring for the main thread. Nothing
    of ours allocates, locks, or panics there; only the plugin's own call
    runs with the allocation guard off (D133). `start_processing`,
    `reset`, and `stop_processing` run marked as the audio thread for the
    plugin's thread check.
  - *Threads.* CLAP's main-thread calls happen on the thread that created
    the instance, which is where the engine compiles: the editor's and
    `audioc`'s main thread (a bake's copy, or an export's, lives on that
    job's thread). That thread keeps the instance's main half in a
    per-thread registry; the audio half carries an id and atomics, and
    everything a plugin may call from another thread just sets a flag.
    `ac_host::poll()` on the main thread (every editor frame, every tick
    of `audioc run`) runs main-thread callbacks, timers, and file
    descriptors, reports parameter moves (`HostEvent::ParamChanged`: the
    editor sets the node's input as a drag would), state changes, closed
    windows, and restarts (`HostEvent::Restart`: the host keeps the
    instance's state and `EngineHandle::rebuild` builds the node anew).
    The engine drops retired graphs on the thread that edits them, so an
    instance is destroyed at once where it was made; an audio half
    dropped elsewhere marks it orphaned for the next poll (D132).
  - *State.* Saving (and exporting) asks each running top-level plugin
    for its state and stores it in its node's config (`state`, base64).
    `NodeMeta::state_keys` keeps that key out of carry-over's comparison,
    so storing it doesn't rebuild the running plugin (D138).
  - *Windows.* **Show Plugin Window** embeds the plugin's view in a
    top-level window the host makes on the main thread (X11 with its own
    connection, Win32, or Cocoa), or asks a plugin that only floats to
    float its own (D134).
  - *VST3.* The same, through the `vst3` bindings (D137): the module's
    entry, the factory's audio classes, a component and controller on the
    main thread (connected when they're separate), an `IAudioProcessor`
    in the node fed preallocated `IParameterChanges` and `IEventList`
    objects and a `ProcessContext`, normalized (0–1) parameter sockets,
    window edits carried to the processor by the host, MIDI controllers
    through `IMidiMapping`, the component's and controller's states in one
    blob, and `IPlugView` windows with the host's `IPlugFrame` (and
    `IRunLoop` on Linux).
  - *Libraries.* Every plugin binary is opened once through a
    process-wide list and never unloaded (`RTLD_NODELETE` on Unix, no
    `FreeLibrary` on Windows); the formats' deinit and module exit still
    run when the last user goes. Plugin code left behind (exit handlers,
    thread-local destructors, threads) would otherwise crash the process
    when it runs after the code is unmapped (D139).
  - *Finding plugins.* `ac_host::scan` walks the standard CLAP and VST3
    folders (and `CLAP_PATH`, `VST3_PATH`) and describes each bundle in a
    child process (`audioc scan-bundle`, `audio-constructor
    --scan-plugin`), caching the catalog by each bundle's size and time.
    Running plugins stay in process; out-of-process hosting was weighed
    and deferred (D135). `PluginHost::locate` finds a node's plugin by its
    id in the catalog, or at the path its description records only if
    that path, links resolved, is inside one of those folders; anywhere
    else the path is just where the plugin was last seen, for the
    report (D197).
- **Web** (`ac-web`, D150–D155): the engine runs inside an `AudioWorklet`;
  the editor is eframe's web target. They are two wasm instances that share
  no memory, joined by `SharedArrayBuffer` rings, the same abstraction as
  native:
  - *The page* (`ui` feature) runs the editor (`AppOptions::web`:
    `AudioMode::Hosted` at the context's rate and 128-frame quantum, and
    `host_files`), a third caller of `EditorApp::ui`. Its engine is a
    *mirror*: the editor uses its `EngineHandle` as on the desktop, and
    each frame `PageLink::sync` posts the project (and its decoded audio
    files) whenever the mirror compiled a new one, with a barrier in the
    ring, then relays what the handle sent (`Processor::relay`: transport
    commands, then parameter changes resolved to slots). MIDI and control
    events go into the ring as they arrive. `PageLink::import` copies the
    worklet's telemetry snapshot into the mirror
    (`TelemetryReader::import`), so meters, scopes, socket values, binding
    values, and the transport follow what plays.
  - *The worklet* (`worklet.js` running the `worklet` feature's module,
    `ac_audio.wasm`) holds a whole engine (`AudioSide`). Its message
    handler compiles a posted project between render quanta; `process()`
    takes ring records up to a barrier for a project not compiled yet,
    applies relayed messages (`EngineHandle::apply_relayed`, dropping
    changes tagged for another project), runs the processor with the
    quantum's input, sends MIDI Out messages back, and publishes
    telemetry every fourth quantum. Nothing in it allocates; posted
    messages free retired graphs. Whole-input nodes bake on the page (one
    job a frame) and the worklet adopts each result
    (`EngineHandle::adopt_bake`, D156).
  - *Devices and files* (D153, D154): Web MIDI, the Gamepad API, and the
    microphone; files through the File System Access API or an upload and
    a download, audio files in memory, examples fetched from the site.
    OSC, recording takes, plugins, and device choice aren't available, and
    the Devices tab says so.
  - *The site* (D152): `scripts/build_web.py` builds it into `target/web/`;
    it needs cross-origin isolation, from the host's headers (`_headers`)
    or the site's own service worker (`sw.js`).

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
  cost; `audioc bench` renders stress projects (1 000 oscillators into a
  mixer, a long effect chain, parallel chains, a 16-voice `Voices` synth,
  and parallel copies of the expensive nodes: FFT → IFFT, PitchShift,
  Diffuser)
  and prints the real-time ratio and, with `--threads N`, the multi-core
  speedup. CI runs `audioc bench` on every merge request and compares against
  the baseline from `main` on the same runner tier, failing on a regression
  larger than 20 %. Results are also appended to a history file so trends are
  visible.
- **UI tests**: egui_kittest drives the editor (add node, wire, undo) without
  a display. Snapshot tests run where a software renderer is available.
- **Runtime instrumentation**: the xrun counter and per-thread load are
  always collected; per-node CPU time is collected while the editor shows
  it (`Ctrl+T`) and by `audioc bench`, and costs nothing otherwise.

## 15. Packaging and releases

Every merge to `main` is a release (CLAUDE.md): once CI passes on a
`main` commit whose version has no tag, `.github/workflows/release.yml`
builds on Linux (x86-64), macOS (Apple silicon), and Windows (x86-64)
with `python3 scripts/package.py --build --target <triple>`, tags the
commit, and publishes the packages with the changelog section as the
notes and a `SHA256SUMS.txt` (D170).

| Platform | Packages | Installs |
| --- | --- | --- |
| Linux | `audio-constructor_X.Y.Z_amd64.deb`; `audio-constructor-vX.Y.Z-<triple>.tar.gz` with `install.sh` | programs in `/usr/bin`, examples in `/usr/share/audio-constructor`, plugins in `/usr/lib/clap` and `/usr/lib/vst3`, a menu entry, icons, and file types; `install.sh` puts the same under `~/.local`, `~/.clap`, `~/.vst3` |
| macOS | `audio-constructor-vX.Y.Z-<triple>.pkg`; the `.tar.gz` | `Audio Constructor.app` in Applications (both programs inside, the examples in `Contents/Resources`), `audioc` linked into `/usr/local/bin`, and the plugins (optional) in `/Library/Audio/Plug-Ins` |
| Windows | `audio-constructor-vX.Y.Z-<triple>-setup.exe`; the `.zip` | Program Files, a Start-menu entry, the plugins (optional) in `Common Files\CLAP` and `\VST3`, file types, and an uninstaller in Installed apps; `/S` installs silently |

Package sources live in `packaging/`: the icon set (drawn by
`scripts/make_icon.py`), the Linux menu entry, file types, and install
script, the macOS entitlements, and the Windows NSIS script. The editor
looks for the examples beside its executable, in `Resources/` (the app
bundle), and in `share/audio-constructor/` beside a `bin/` (D172).

Signing (D171) is switched on by repository secrets; without them every
package is built unsigned. `release.yml` reads:

| Secret | Use |
| --- | --- |
| `MACOS_CERTIFICATE_P12`, `MACOS_CERTIFICATE_PASSWORD` | a base64 `.p12` holding the Developer ID Application and Installer identities, imported into a temporary keychain |
| `MACOS_SIGNING_IDENTITY` | the Developer ID Application identity's name: signs the programs, the app, and the plugin bundles (hardened runtime, `packaging/macos/entitlements.plist`) |
| `MACOS_INSTALLER_IDENTITY` | the Developer ID Installer identity's name: signs the `.pkg` |
| `MACOS_NOTARY_APPLE_ID`, `MACOS_NOTARY_TEAM_ID`, `MACOS_NOTARY_PASSWORD` | notarize the signed `.pkg` (an app-specific password) and staple the ticket |
| `WINDOWS_CERTIFICATE_PFX`, `WINDOWS_CERTIFICATE_PASSWORD` | a base64 Authenticode `.pfx`: signs both programs, the plugin library, the setup program, and its uninstaller (timestamped) |

CI's *Bundles and installers* job builds the same packages on every
change, installs each one on its runner, runs both programs from where
they landed, renders an example with the installed `audioc`, and
uninstalls; the packages are kept as the run's artifacts for two weeks,
so a change can be tried before it's released.

