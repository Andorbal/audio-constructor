# Roadmap

Three phases, each a large chunk of work delivered through many small
commits and a merge request per milestone, and a fourth, drafted from the
gap studies in [GAPS.md](GAPS.md), for a human to confirm, reorder, or cut
before it starts. Finish a phase before starting the next; within a phase,
milestones are ordered but later ones can start when their inputs exist.

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
- [x] Graph, NodeInstance, Wire, GroupDef, Project types with serde and `schema_version`.
- [x] Signal types, units, ranges; `Signature` with dynamic sockets from config.
- [x] `Node` trait, `node!` macro, `Registry`.
- [x] Round-trip tests: load → save produces identical JSON; unknown node kinds preserved.
- [x] Clipboard format (subgraph as JSON) with id remapping on paste.

### M1.3 Compiler and offline engine
- [x] Group flattening with namespaced ids; `Feedback`-only cycles; compile errors as a report, never a panic.
- [x] Rate and channel inference; implicit conversions.
- [x] Topological schedule; buffer pool with lifetime reuse; state carry-over on recompile.
- [x] Transport and tempo map with exact beat ↔ sample conversion (tests at several rates and tempos, including tempo changes).
- [x] Offline renderer; `audioc render` to WAV (16/24/32f); `audioc validate`, `inspect`, `nodes`.
- [x] Golden-render harness under `tests/golden/`; determinism test (offline vs. null real-time host at block sizes 32 and 1024).
- [x] `assert_no_alloc` guard in debug builds around `process`.

*Verified:* `cargo test -p ac-engine` (25 engine tests: conversions, upmix,
channel and type errors, cycles named and isolated, Feedback delay, nested
groups, reroutes, carry-over, smoothing, start modes, loop/seek, buffer
reuse, and offline vs. null host bit-for-bit at blocks 32 and 1024, all
under the allocation guard); `cargo test -p ac-core` (tempo map at six
rates with tempo and meter changes); `cargo test -p ac-cli --test golden`
(golden hashes, live-vs-offline determinism, and the `audioc render` binary
matching the library).

### M1.4 Phase 1 nodes
Every node in NODES.md marked **P1**, with unit tests and at least one golden project per category:
AudioIn, AudioOut, MidiIn, Record, Oscillator, Clip, Transport, Gain, Pan, Mix, Biquad, Delay, Constant, Math, Unary, Compare, Map, ADSR, Clock, MidiNote, Button, Toggle, Slider, Meter, Scope, Readout, Feedback, Reroute, Group, SplitChannels, MergeChannels, Comment.
- [x] Generator contract implemented (start-on-play dropdown, stop/reset/loop/reverse/rate where applicable).
- [x] `audioc nodes --docs` generates `docs/reference/` from node doc comments.

*Verified:* all 32 kinds registered (`every_phase1_node_is_registered`);
`cargo test -p ac-nodes -p ac-dsp` (unit tests per node: known frequencies,
impulse responses, generator contract, edge cases, block size 1; a sweep
runs every node at block sizes 1/64/1024 under the allocation guard);
14 golden projects in `tests/golden/` cover every category with P1 nodes;
`docs/reference/` is generated and a test fails if it goes stale.

### M1.5 Real-time runtime and devices
- [x] cpal host with device enumeration, sample rate and buffer size selection, hot-swap of graphs, parameter ring with smoothing, telemetry triple-buffers, xrun counter.
- [x] midir input with timestamps → block offsets.
- [x] Null host for tests and headless CI.
- [x] Recording through a disk thread; files land in `<project>/assets/`.
- [x] `audioc run` (headless live); `audioc devices`.
- [x] Latency measurement (loopback test in the Devices panel, manual).

*Verified:* `cargo test -p ac-io` (duplex ring pass-through, underrun and
overrun; MIDI offsets and rebasing; disk recorder through the real Record
node; latency correlation on a simulated loopback to the exact sample;
paced null host; live hot-swap and parameters; all under the allocation
guard); `cargo test -p ac-cli` (`audioc devices`, `run --null` with and
without recording, error paths; `tests/acceptance.rs` records 30 s through
the live path, plays it back from a Clip after save and reopen, and gets
the same samples). The latency probe (noise burst + FFT cross-correlation)
finds a simulated loopback delay to the exact sample; it runs from the
Devices panel's "Measure latency" button and `audioc latency`. Real devices,
MIDI hardware, and a loopback-cable measurement need a human; steps are in
the merge request and results go in the M1.7 table.

### M1.6 Desktop editor
- [x] eframe shell, theme module, fonts, icons; dark theme.
- [x] Node canvas: pan/zoom/select/move, add palette with fuzzy search, wiring with type checking, drop-on-empty-canvas filtered palette with auto-wire, insert on wire, delete with reconnect, duplicate, mute, collapse, frames, reroute, undo/redo for everything.
- [x] Inline widgets per unit; wired-input live values; socket meters; wire flow for audio; event pulses.
- [x] Group create/enter/leave/ungroup with exposed sockets.
- [x] Side panel tabs: Node (with docs), Library, Project, Devices, Problems.
- [x] Transport bar; keyboard shortcuts from UX.md.
- [x] Save/load/recent projects; asset handling; missing-file handling.
- [x] egui_kittest tests for: add node via palette, wire two nodes, undo, group and enter.

*Verified:* `cargo test -p ac-app` (unit tests for every editing operation:
group/ungroup boundaries, delete-with-reconnect, duplicate with and
without incoming wires, insert on wire, paste, reroutes, nested groups,
interface sockets; egui_kittest tests on the null stream: add a node via the
palette, wire and disconnect, undo/redo, group and enter/leave/ungroup,
missing nodes and bad wires still draw, drop-wire auto-wire, splice on wire,
duplicate-and-place as one undo step; Export WAV is byte-identical to a
direct render). Look and live feedback checked from screenshots under Xvfb
with a software renderer (playing, meters, scope, wire flow, event pulses,
palette search, groups, panels). Real devices, native dialogs, and feel
need a human; steps are in the merge request.

### M1.7 End to end, examples, baseline
- [x] Example projects: Start, Guitar Pedalboard (AudioIn → Biquad → Delay → AudioOut), Subtractive Synth (MidiIn → MidiNote → Oscillator → Biquad → ADSR-driven Gain → AudioOut), Four Tracks (4 × Track block → Mix → AudioOut), Reverse Delay placeholder (documents the Phase 2 node).
- [x] `audioc bench` with three stress projects and a baseline file committed; CI bench job comparing against `main`.
- [ ] Manual verification on all three OSes recorded in this file (who, which device, buffer size, measured latency, xruns over 10 minutes).

*Verified:* `cargo test -p ac-cli --test examples` (all five examples open,
are canonical, compile without errors; Start, Subtractive Synth, and Four
Tracks make sound without clipping; the CLI render of the synth equals the
library render bit for bit). `audioc bench` baseline in `bench/`; the CI
bench job compares merge requests with their base branch.

Manual verification (to be filled in by a person with the hardware; the
merge request lists the exact steps):

| OS | Who | Audio device | Buffer | Measured round trip | Xruns in 10 min | Date |
| --- | --- | --- | --- | --- | --- | --- |
| macOS | | | 128 | | | |
| Windows | | | 128 | | | |
| Linux | | | 128 | | | |

**Phase 1 acceptance**
- Live guitar through the Pedalboard example at 128-frame buffers with zero engine-caused xruns over 10 minutes on each OS.
- Record 30 s, play back from a Clip, save, close, reopen, same result.
- `audioc render` of the Subtractive Synth example with a MidiPattern stand-in (a Clock-driven note) matches the in-app offline export bit-for-bit.
- All CI jobs green, including wasm checks and the bench comparison.

*Status:*
- Live guitar at 128 frames with zero engine-caused xruns: **needs manual
  verification** (table above). The engine side is covered by the
  allocation guard on every node and the processor, and by load
  measurement in `audioc run` and the editor.
- Record 30 s → Clip → save → reopen → same result: automated through the
  live code path in `crates/ac-cli/tests/acceptance.rs` (bit-identical);
  the same flow in the editor needs a manual check with a real input.
- `audioc render` of the Subtractive Synth (with its clock-driven demo
  note) matches the in-app export bit for bit: both call the same render
  function with default options; `tests/examples.rs` checks the CLI against
  the library and an `ac-app` unit test checks the export against the
  library.
- CI: fmt, clippy, tests on Linux/macOS/Windows, wasm32, and cargo deny are
  green; the bench comparison runs on merge requests.

---

## Phase 2 — The Library: every node, polyphony, baking, scale

**Goal.** The full starting catalog from NODES.md, polyphonic synths, whole-
input nodes with background baking, latency compensation, multi-core
scheduling, shareable blocks, and a dozen example projects that show what
the tool is for.

### M2.1 Engine depth
- [x] `Voices` group instancing and the `Voice` node; voice stealing policies.
- [x] Causality analysis, disabled badges, background baking with invalidation; offline multi-pass rendering for whole-input nodes.
- [x] Latency compensation across merge points; `Latency` and `Info` nodes.
- [x] Multi-core scheduler with real-time worker pool; per-core load telemetry; stress tests under `audioc bench`.
- [x] `Buffer` and `Spectrum` signal types; `Record` emits a `Buffer`.

*Verified:* `cargo test -p ac-engine --test phase2` (20 tests, all under the
allocation guard): compensation lines up merging audio and sample-rate
float paths at blocks 1/64/1000, survives recompiles without a dropout, and
probes report path latency; whole-input nodes bake first offline (blocks
1/64/256), stay silent live until a background job finishes and then
stream the bake, are disabled (with an Info diagnostic) behind Audio In but
bake offline from the render's input, rebake on upstream edits while the
old bake plays, bake nested nodes in order, and drop stale results; 8
parallel chains and a 16-voice `Voices` synth render bit-identically with
and without a 1- or 3-worker pool, and per-core load is published;
Spectrum frames keep their offsets and shapes through a pass-through node
at blocks 1/5/64, mistyped Spectrum wires are reported, and Buffer handles
persist across blocks; Voices count notes, fan inputs out to every voice
(also through `set_param`), steal or ignore notes, and namespace their
paths. `cargo test -p ac-nodes` (Voices allocation and stealing, sustain,
broadcast, Voice gate dips on retrigger, Reverse slices, Latency, Info,
Record's buffer output), `cargo test -p ac-engine --lib pool` (every job
runs exactly once over 2000 rounds), `cargo test -p ac-io` (the disk
recorder hands each take back as a buffer and releases it once unused; the
background baker bakes and debounces parameter changes),
`cargo test -p ac-app` (a palette Voices node gets a body that sounds,
groups toggle polyphony, hidden sockets stay hidden). Three golden projects
(`time-reverse`, `sequencing-voices`, `utility-latency`) pass the golden
and live-vs-offline determinism checks. `audioc bench --threads 4` on a
4-core VM: 1000 oscillators 2.4× faster, the 16-voice synth 2.0×, parallel
chains 1.6×, the serial effect chain 1.1×; on one thread the branch is
within noise of `main` on the same machine (+1 % to +10 %). The badges were
checked in a screenshot under Xvfb. Needs a human: worker threads on real
devices at 128 frames (xruns with `--threads` vs without), and real-time
priority for workers on macOS and Windows (only Linux asks for it).

### M2.2 Nodes
- [x] Every node in NODES.md marked **P2**, in category order: Generators, Level/Dynamics, Filters, Time, Spectral, Math, Modulation, Triggers, Sequencing, Controls, Display, I/O, Utility.
- [x] Node editors for `Graph` (curve editor), `Pattern` (grid), `MidiPattern` (piano roll), `BandSplit` (band editor), `StepSequencer`.
- [x] Golden projects for each category; benches for the expensive ones (FFT, PitchShift, Diffuser).

*Verified:* 73 new nodes. `Gamepad`, `KeyboardIn`, and `OscIn`/`OscOut` are
marked P2 but belong to M2.4 (control surfaces), which lists them. A test in
`crates/ac-nodes/tests/all_nodes.rs` reads NODES.md and fails if any other P2
node is missing from the registry. Every node passed these checks:
- `cargo test -p ac-nodes`: unit tests next to each node, covering known
  values, edge cases, NaN and extreme inputs, reset, and identical output at
  blocks 1, 64, and 512.
- The allocation-guard sweep at blocks 1, 64, and 1024, with defaults and
  with non-default configs (`configured_nodes.rs`, `buffer_nodes.rs`,
  `sequencing_nodes.rs`, `math_modulation_configs.rs`). The sequencing
  sweep also floods MIDI past the per-block capacity and checks that no
  note is left hanging.

The DSP in `ac-dsp` (FFT/STFT, crossovers, YIN, onset detection, phase
vocoder, SVF/ladder/one-pole/allpass, cepstrum, dynamics, oversampling,
waveshaping, band-limited wavetables, BS.1770 loudness, colored noise)
has its own tests (`cargo test -p ac-dsp`).

`cargo test -p ac-engine --test phase2`:
- Spectrum wires carry latency; this was a fix to M2.1's compensation.
- A dry path cancels against FFT → IFFT to below 1e-4.

`cargo test -p ac-cli --test render_outputs`:
- File Out writes the same samples as the main mix and refuses paths that
  leave the output folder.
- MIDI Out passes MIDI through with its timing.
- `render --midi-out` writes a standard MIDI file.

`cargo test -p ac-io`:
- MIDI Out reaches a port one block late, in order, through the live
  runtime.
- A full queue drops messages and counts them.

Editor tests:
- `cargo test -p ac-app --test node_editors` drives each node editor with
  the pointer: painting Pattern cells; drawing, moving, and deleting notes;
  moving and adding curve points; splitting and merging bands; drawing
  step values.
- `--test editor` turns the Knob, plays the Keyboard, touches the XY Pad,
  types into the Number field, and picks a Dropdown choice. Control
  gestures don't dirty the document.
- Library tests cover each editor's model and the coalesced undo of a drag.

Goldens: 21 new golden projects, at least one per category, pass the
hash, block-independence, and live-vs-offline checks. The Analyzer is in
`spectral-gate`. The golden test now also checks File Out files and MIDI
Out messages.

`audioc bench` (release, 128-frame blocks, this VM) renders the new stress
projects at these speeds:

| Project | Real time |
| --- | --- |
| `fft-chains-16` | 27× |
| `pitch-shift-16` | 9.4× |
| `diffuser-16` | 6.0× |

Each node costs per 128-frame block: IFFT ≈ 3 µs, FFT ≈ 1.6 µs,
PitchShift ≈ 22 µs, Diffuser ≈ 32 µs.

Screenshots under Xvfb (software rendering) showed every node editor and
control widget. Needs a human: listening tests (Ladder self-oscillation and
drive, Freeze seams, PitchShift warble and formants on a voice, Diffuser
tone, Sampler loop crossfades), the feel of the Knob, XY Pad, and
Keyboard, and MIDI Out timing against a hardware synth.

### M2.3 Blocks and sharing
- [x] Block library directory, `.acblock` save/load, metadata, versioning, "update instances from library".
- [x] All blocks listed in NODES.md built and tested as golden projects.
- [x] `audioc pack`/`unpack`; the app opens `.acpack`.
- [x] Library tab shows user, shipped, and imported blocks with search and tags.

*Verified:* blocks have three origins: shipped (built into the app from
`blocks/`), the person's own, and imported. Each has a source
`<origin>:<id>` and a version. DECISIONS D79–D84 cover the design.

`cargo test -p ac-core block`:
- Block files round-trip with nested definitions.
- A pasted instance keeps its source.
- Updating keeps parameter values and drops wires to sockets the new
  version no longer has.

`cargo test -p ac-io`:
- Library tests: every shipped block loads; the person's blocks save,
  bump their version, and reload; imports dedupe and rename; search
  matches names, tags, and descriptions; a project's stale and unknown
  blocks are found; someone else's block with the same name is adopted,
  not confused with one of ours.
- Pack tests: round trips with assets inside and outside the project
  folder; damaged packs are reported.
- ZIP tests: round trips; reads an archive made by another tool; refuses
  entry names that leave the folder (on write and on read); damage is
  reported, never panicked on.

`cargo test -p ac-cli --test blocks_cli` runs `audioc blocks`
(`--json` too), `block-update` (`--check` writes nothing), and
`pack`/`unpack`, refusing to unpack over a folder without `--force`. The
unpacked project then renders.

The 23 blocks: each `blocks/<id>.acblock` has a golden,
`tests/golden/block-<id>.acproj`. A golden test fails if a golden's copy
of its block differs from the shipped file or doesn't play it. The block
goldens pass the hash, block-independence, and live-vs-offline checks
like every other golden. Each golden plays its block the way it's meant to
be used: the gate on a tone over hiss, the ducker pumping a pad with a
kick, the instruments from golden MIDI, the drum blocks for a bar from
the transport. The Plate and Hall Reverb decay times were checked by
Schroeder integration of rendered impulse responses.

Editor (`cargo test -p ac-app --test blocks`):
- The palette adds a shipped block as one undo step.
- A block with a Voice inside comes in as Voices.
- Saving a group sets its source and bumps its version, and an older
  copy is offered the update from the Node and Project tabs.
- Removing a saved block leaves projects alone.
- File ▸ Open opens a pack's project and imports block files without
  touching the document.
- Export Pack saves first (asking where for an untitled project) and
  writes a pack that unpacks again.
- The library folder keeps saved blocks between runs.

`panels::library` unit tests cover the origin, tag, and search filters.

Needs manual verification (in the PR): listening to each block's golden
render, and how the Library tab feels with a large library.

### M2.4 Control surfaces
- [x] Gamepad (gilrs), KeyboardIn, OSC in/out nodes; binding/learn UI for MIDI, keys, gamepad; transport bindings.
- [x] ASIO build (feature `asio`) documented and tested on Windows; JACK on Linux.
- [x] Panels: pin nodes, multiple panels, full-screen performance mode.
- [x] Devices tab lists the last OSC and MIDI messages received, with
  addresses as the sender wrote them; Use sets a selected OSC In's address
  (D115).
- [x] `Text` signals (D116): OSC In's `address` and OSC Out's `address`,
  `host`, and `port` are inputs, so a block can expose them; `Constant`
  holds text; `Number to Text` and `Text to Number` build and read
  addresses. Project files are version 2 (OSC settings become params on
  load).

*Verified:* keys, gamepads, and OSC reach the engine as `ControlEvent`s,
like MIDI: one ring per source, one block late, with a sample offset.
Outgoing OSC is built on the audio thread without allocating (DECISIONS
D85–D88). The four nodes complete the P2 catalog; `all_nodes.rs` no longer
exempts them. Each device and feature is covered as follows.

Device layer (`cargo test -p ac-io`):
- Control rings deliver in order, keep arrival order at equal offsets,
  and drop and count when full; swapping a dropped sender's ring loses
  nothing.
- Gamepad mapping: each standard button maps once, analog triggers are
  both buttons and axes, pad numbers stay put, and unplugging releases
  held controls. The thread starts and stops on CI machines without a
  gamepad.
- OSC is tested over loopback with real packets: messages, bundles, and
  garbage. A message without arguments acts as a trigger, deeply nested
  bundles are refused, and a port already in use is an error, not a
  panic.
- OSC out reaches its target one block late, in order, and a full queue
  drops and counts.
- The listener's message log keeps each message's address and arguments
  as text (floats trimmed to four decimals), newest 64, and clears.

Nodes (`cargo test -p ac-nodes`):
- Unit tests next to each node check sample-accurate changes, state held
  across blocks, dead zone and curve, reset, block sizes 1/64/512,
  NaN, and bad configs.
- Keyboard In always releases the note it started, even after an octave
  change, including when MIDI floods past capacity.
- OSC Out rate-limits changes and still sends the last value.
- `configured_nodes.rs` floods all four nodes with events under the
  allocation guard.

Bindings (DECISIONS D92–D95):
- `cargo test -p ac-engine --test bindings`:
  - a MIDI CC moves a bound Gain on its own sample;
  - a key toggles a Bool, and a button fires a trigger;
  - transport bindings start, stop, and record;
  - bindings to wired, missing, or unbindable inputs are reported, not
    applied;
  - bound values survive a recompile;
  - group instances and nodes inside groups can be bound;
  - offline renders are deterministic and block-size independent;
  - the learn monitor hands raw events to the editor;
  - a flood of bound events stays allocation-free.
- Parameter smoothing now gives the same samples at every block size; no
  earlier golden hash changed.
- `--test bindings_cli`: `inspect` lists bindings and `validate` reports
  the ones that can't act.

Panels (DECISIONS D96–D99): `edit::pins` tests cover undo, pins removed
with deleted or grouped nodes, and dangling pins dropped on open.

Editor (egui_kittest):
- `--test bindings`:
  - Bind… learns a MIDI controller, a gamepad stick sent through the real
    control rings, and a key;
  - Esc cancels learning;
  - deleting a node removes its bindings in the same undo step;
  - the transport preset adds three gamepad buttons.
- `--test panels`:
  - `P` pins a Slider, `Ctrl+Tab` shows it, and a drag sets the document
    and the engine as one undo step;
  - panel Buttons are momentary;
  - several panels can be added, renamed, and switched;
  - cards move, resize, and unpin;
  - `F11` sends the full-screen command and hides the chrome, and Esc
    leaves;
  - a Meter follows live audio;
  - performance mode turns Play keys on and restores it on leaving.
- `--test editor`: with Play keys on, keys go to the patch instead of
  running shortcuts, and text fields keep their typing.

Goldens: four new golden projects drive the nodes and bindings from test
control events and test MIDI, and check the OSC sent like MIDI. They pass
the hash, block-independence, and live-vs-offline checks:
- `io-gamepad-keyboard-in`
- `io-osc-in-out`
- `bindings-midi-cc`
- `bindings-controls`

Backends (DECISIONS D100):
- The CI job *JACK build (Linux)* builds the editor and `audioc` with
  `jack`. It starts `jackd` with its dummy driver and runs
  `crates/ac-io/tests/backends.rs`, which checks JACK is listed, opens at
  the server's rate, and plays half a second through the engine. It then
  runs `audioc run examples/start.acproj --host jack`.
- *ASIO build (Windows)* builds both programs with `asio` (the build
  downloads the SDK). It checks the ASIO host is listed and fails cleanly
  without drivers; CI runners have none.

Needs manual verification (in the PR):
- an ASIO interface;
- physical gamepads;
- a phone OSC app;
- how learning feels;
- Play keys in a real window;
- full screen on each OS;
- touch on panels.

### M2.5 Editor conveniences
- [x] Auto-layout, align/distribute, portals, find, lazy connect, connect-selected-to-new-node, per-node timing display, minimap.
- [x] Light theme; UI scale.
- [x] Node help with "try this" tips and links to blocks.

*Verified:* the editor's colors now come from a palette (DARK and LIGHT),
switched per UI thread. DECISIONS D115–D110 record the choices. Each part
is covered as follows.

Canvas editing (unit tests in `crates/ac-app/src/edit/`):
- Auto-layout (layered, after Sugiyama et al. 1981):
  - chains run left to right, the selection stays anchored, and results
    are deterministic;
  - fan-out and fan-in don't overlap, and socket order is kept;
  - Feedback and other loops are handled, frames and nested frames move
    with their contents, and loose cards are placed;
  - a 300-node tangle lays out without trouble.
- Align and distribute math.
- Quick wiring: lazy connect picks sockets by name and type and refuses
  to close a loop without a Feedback node; connect-to-new plans the
  wiring, widens a Mix to fit, and keeps selection order.
- Find matches nodes inside groups and group instances.
- Portals: a wire becomes a portal and goes back to a wire; one sender
  can have several receivers; names stay unique; copy, paste, and groups
  work. A patch renders identically before and after making a portal.
- Minimap: points map from the graph to the minimap and back, and it
  shows or hides by the rule in D106.

Node timing:
- `cargo test -p ac-engine --test engine` checks that timing sums every
  step of a host block. Nothing is measured while it's off; the
  allocation guard and wasm32 still pass.
- Editor unit tests cover smoothing, heat colors, and summing a group or
  Voices node's insides against a real engine.

Light theme and UI scale:
- Contrast tests hold text, muted text, accents, and header titles to
  WCAG AA (4.5:1) on every panel and card surface, in both themes. Faint
  text and signal colors are held to 3:1.
- Switching palettes changes the accessors and egui's visuals, and other
  threads keep their own palette.
- Settings round-trip and survive a bad file.

Node help:
- `all_nodes.rs` now requires every node to have a "Try this" paragraph.
- The "Used in" index follows nested definitions and survives cycles.

Editor (egui_kittest):
- `--test arrange`:
  - `Ctrl+L` lays out a scrambled chain left to right as one undo step,
    and every shipped example and its groups lay out without overlaps;
  - Align Left works from `Shift+Alt+A`, and `Alt+A` still clears the
    selection;
  - `F` wires two nodes;
  - `Shift+A` "mix" wires two selected oscillators in selection order;
  - `Ctrl+F` finds a node in a group and jumps in.
- `--test canvas_aids`:
  - `Ctrl+T` shows timing in headers with the null host running;
  - a wire becomes a portal and back, each one undo step, with no line
    drawn while it's a portal;
  - portals rename and paste inside groups;
  - the minimap pans the view, and `Shift+M` toggles it.
- `--test look_and_help`:
  - View ▸ Theme ▸ Light applies at once and is remembered;
  - UI scale steps and is remembered;
  - `F1` on Gain Computer lists the Compressor blocks under Used in, and
    adding one works;
  - an example link asks about unsaved changes;
  - every side-panel tab fits at the default width.

Needs manual verification (in the PR):
- how the light theme looks, by eye;
- UI scale on a HiDPI screen;
- how auto-layout and align feel on real patches;
- portals at different zoom levels;
- timing readability on a heavy patch;
- the minimap's placement.

### M2.6 Examples and docs
- [x] Examples: Sidechain Pump, Drum Machine, Build a Keyboard, Reverse Delay, Gamepad Jam, Multiband Compressor, Vocoder, Generative Patch (Euclidean + Random + ScaleQuantize), Tape Loop, Guitar-to-Synth (PitchDetect).
- [x] Generated node reference published with the repo; user guide in `docs/guide/`.

*Verified:* `examples/` holds 14 projects: the 10 above, plus Start,
Guitar Pedalboard, Subtractive Synth, and Four Tracks. Each one:
- has notes on the canvas saying what each part does and what to try;
- opens from File ▸ Examples as an untitled copy;
- plays without hardware. Examples that need a live input default to a
  stand-in (DECISIONS D111).

`cargo test -p ac-cli --test examples` checks every example:
- the expected examples ship;
- each is canonical and compiles cleanly;
- its block copies are exactly the shipped versions;
- panels and bindings point at real controls;
- examples without live inputs make sound and don't clip.

It also checks single examples:
- Generative Patch follows its seed;
- Reverse Delay bakes from a Clip and is disabled on Audio In;
- Guitar-to-Synth follows its riff and a guitar on Audio In within 2 %;
- Build a Keyboard plays from the demo, the computer keys, and the
  on-screen keys;
- Gamepad Jam plays its demo and follows a controller (buttons, D-pad
  chords, sticks, and its six bindings).

`cargo test -p ac-app --test examples_layout` lays every example out as
the editor draws it: no overlapping cards, and frames hold whole cards.

Docs:
- The user guide is `docs/guide/`, 11 chapters: getting started, the
  editor, building sounds, live and baked, blocks, performing, recording
  and rendering, the command line, an examples tour, and
  troubleshooting.
- The generated node reference (`audioc nodes --docs docs/reference`)
  opens with an index of categories and nodes. A test keeps it in step
  with the code.
- The README links both.
- `cargo test -p ac-cli --test docs_links` fails on a broken relative
  link or anchor, a guide chapter missing from the index, or an example
  missing from the tour.

Needs manual verification (in the PR):
- listen to every example;
- play Build a Keyboard and Gamepad Jam with the real keyboard and a
  gamepad;
- read the guide for accuracy, clicking through its first-synth
  walkthrough in a real window.

**Phase 2 acceptance**
- A 16-voice subtractive synth with per-voice filter and envelope runs at 128-frame buffers on a 4-core laptop under 30 % CPU.
- The Reverse Delay example shows the Reverse node disabled when fed from AudioIn and baked when fed from a Clip, and renders correctly offline.
- A block exported from one project imports into another and updates its instances when the library copy changes.
- The bench suite shows the multi-core scheduler scaling on independent chains.

*Verified* (figures are in `bench/README.md`, measured on a 4-vCPU
2.1 GHz VM with a release build and 128-frame blocks):
- **16-voice synth.** `audioc bench`'s `subtractive-voices-16` runs 16
  voices of the shipped Subtractive Synth Voice block, each with its own
  ladder filter and envelopes, all sounding. It takes 6–8.4 % of one core
  on one thread. With chain fusion (DECISIONS D114) each voice runs as its
  own job, so four threads make it 1.9–2.4× faster again.
- **Reverse Delay.** `reverse_delay_bakes_from_a_clip_and_is_disabled_from_audio_in`
  (`crates/ac-cli/tests/examples.rs`) checks three things: the Clip path
  bakes and the Audio In path reports itself disabled while live, the
  baked live output matches the offline render sample for sample, and the
  render is its input reversed slice by slice.
- **Block export, import, and update.**
  `a_block_exported_from_one_project_updates_its_instances_in_another`
  (`crates/ac-cli/tests/phase2_acceptance.rs`):
  - project A saves a group to a library as `user:trim`;
  - project B uses two instances of it;
  - A saves a changed version (v2: a different gain and a new input);
  - `audioc block-update --library` reports and applies the update;
  - B ends up with the v2 interface, keeps its wires, and renders at half
    the RMS.
- **Multi-core scaling.** Chain fusion runs each linear chain, and each
  voice of a Voices node, as one job. On four threads:
  - `parallel-chains-8` (8 independent chains, 24 nodes deep) is
    2.2–2.65× faster than on one thread, up from 0.96–1.10× before
    fusion, because its 27 waves per block become 3;
  - heavy chains scale too: `diffuser-16` up to 2.7×, `fft-chains-16`
    1.8–1.9×.

  Tests in `crates/ac-engine/tests/phase2.rs` check that output is
  bit-identical on 1, 2, and 4 threads and matches the unfused schedule.
  Golden hashes are unchanged.

---

## Phase 3 — Reach: plugins, web, and polish

**Goal.** The engine runs inside other hosts and in the browser, third-party
plugins run as nodes, and the product is packaged for people who are not
developers.

### M3.1 Plugin build
- [x] `ac-plugin` with nice-plug (D117): CLAP and VST3, effect and instrument variants, host-automatable parameters chosen from the project, editor in the plugin window.
- [x] State save/restore through the host; sample rate and block size from the host.

*Verified:* `crates/ac-plugin` builds one library holding **Audio
Constructor Effect** and **Audio Constructor Instrument**, each as CLAP and
VST3. `scripts/bundle_plugin.py` lays out the bundles, and CI's *Plugin
bundles* job builds them in release on Linux, macOS, and Windows and runs
the host tests against the binary inside them.

`crates/ac-plugin/tests/clap_host.rs` loads the plugin in a real CLAP host
(clack-host), with debug builds aborting on any allocation in `process`:
- `a_project_runs_as_a_clap_effect_with_automatable_macros`: a sine passes
  at 0 dB; automating Macro 1 to 0.25 at frame 100 leaves the earlier
  samples alone and settles at −12 dB (within 0.05 dB); uneven block sizes
  up to the host's maximum work. This is Phase 3 acceptance #1.
- `state_restores_the_project_and_macros_in_a_second_instance`: a state
  saved from one instance gives bit-identical output in a second; the
  restored patch measures the calculated −32.0 dB.
- `a_state_that_cant_be_used_leaves_a_working_default`: broken or newer
  states fall back to the default patch with a report.
- `the_patch_follows_the_hosts_transport` and
  `the_instrument_plays_host_notes_on_their_samples` (a note starts on its
  exact sample).
- `both_plugins_offer_an_embedded_resizable_editor` (the gui extension,
  the platform's embedded API, resizing, 1200×760), and
  `the_editor_opens_in_the_hosts_window_and_closes_again` (ignored by
  default; CI runs it under Xvfb against the release bundle: the editor
  opens in a real X11 parent three times while audio plays).
- `crates/ac-plugin/tests/vst3_factory.rs` checks the VST3 factory exports
  both classes with their categories and creates each one.

"Parameters chosen from the project" are sixteen fixed macros that the
project maps with ordinary bindings (D119), so hosts see a stable
parameter list and automation lands on the right sample. The editor
(`crates/ac-plugin/src/editor`, egui_kittest tests in `editor/tests.rs`)
borrows the session's engine per frame: palette edits reach the session,
a state reset reloads the document, and a macro dragged in the Devices tab
goes through the host's parameter setter as one gesture. The editor is in
the CLAP plugins only; the VST3 plugins have no window for now (D128).

Needs a human with a host and a screen: the steps are in the M3.1 merge
request.

### M3.2 Plugin hosting
- [x] CLAP plugin scanner and `Plugin` node with dynamic sockets from the plugin's ports and parameters; plugin GUI in a floating window.
- [x] VST3 hosting (licensing per DECISIONS); sandboxing or crash isolation investigated.

*Verified (CLAP):* the `Plugin` node (`crates/ac-nodes/src/utility/plugin.rs`)
builds its sockets from the description in its config; its unit tests,
against a fake processor, check the sockets, notes at their pitch at block
sizes 1, 64, and 1000, parameter events (still inputs once, moving ones
every 32 samples at the same frames for block sizes 7, 64, and 1000),
changes before notes on a shared frame, the transport, latency, a failing
plugin silenced, and a missing one keeping its sockets with a warning.
`crates/ac-host` hosts real CLAP plugins with clack:
- `tests/clap_hosting.rs` scans our own Audio Constructor Instrument and
  Effect (the `ac-plugin` build) and renders them as nodes through the
  engine, with the allocation guard aborting on any allocation of ours
  around the plugin: A3 and A4 come out strongest at 220 and 440 Hz
  (±3 Hz) and die away after the note-off; the level macro moves the
  output by 36 dB (±3); the effect passes a 330 Hz oscillator at unity;
  renders are deterministic; a state saved into the node's config
  restores in a fresh engine without rebuilding the running instance; a
  changed description rebuilds the node and the old instance goes; a
  missing plugin is silent with a warning.
- `tests/plugin_window.rs` and `tests/third_party_window.rs` (ignored;
  CI runs them under xvfb) open plugin windows embedded in the host's
  X11 window while audio plays, close them, and reopen them: our
  instrument's node editor, and the third-party Six Sines while it plays
  a note (checked by screenshot on a virtual display, both drawn inside
  the host window).
- `crates/ac-cli/tests/plugins_cli.rs`: `audioc plugins --json` scans a
  `CLAP_PATH` folder in child processes and lists both plugins; a file
  that isn't a plugin is listed as failed; `audioc render` of a project
  with the instrument comes out at 440 Hz; `validate` warns for a missing
  plugin.
- `crates/ac-app/tests/plugins.rs`: installed plugins are offered by name
  in the palette (the bare node isn't), adding one makes a Plugin node
  with its sockets, the Node tab shows a missing plugin's warning, and
  saving stores a running plugin's state in its node without rebuilding
  it.
- Phase 3 acceptance #2 ("a third-party CLAP synth runs as a node inside a
  project"): CI's *Third-party plugin as a node* job downloads Six Sines
  1.2.1 for Linux, checks its SHA-256, and renders it from a MIDI Pattern
  with `audioc`: notes come out at 220 and 440 Hz (D136); verified locally
  the same way.

*Verified (VST3):* `crates/ac-host` hosts VST3 plugins through the
MIT-licensed `vst3` bindings (no SDK code, D137) with the same node,
scanner, state, and window paths as CLAP:
- `tests/vst3_hosting.rs` hosts our own plugins' VST3 build: the scanner
  describes both (parameters 0 to 1); the instrument renders A3 and A4 at
  220 and 440 Hz (±3) under the allocation guard; a parameter input moves
  the level; the effect passes a 330 Hz oscillator at unity; a saved state
  restores in a fresh engine; a missing plugin is silent with a warning.
- CI's *Third-party plugin as a node* job also renders Six Sines' VST3
  build from a MIDI Pattern with `audioc` (220 and 440 Hz) and opens its
  window embedded in the host's X11 window while it plays (checked
  locally by screenshot on a virtual display).
- `cargo check -p ac-host` passes for `aarch64-apple-darwin` and
  `x86_64-pc-windows-gnu`; windows on macOS and Windows need a human
  (MR description).

Sandboxing: the scan runs each bundle in a child process, so a plugin
that crashes or hangs while it's looked at is listed as failed instead of
stopping the scan (a bundle that fails to open is checked in
`plugins_cli.rs`). Crash isolation for
running plugins was investigated and deferred: out-of-process hosting
costs a context switch per block and shared-memory audio, and is
written up with the plan in D135.

### M3.3 Web
- [x] `ac-web`: engine in an AudioWorklet, eframe UI, SharedArrayBuffer rings, Web MIDI, Gamepad API; static hosting with cross-origin isolation headers.
  Verified: natively, `cargo test -p ac-web` runs the editor with its mirror engine against the worklet's engine through the same rings, messages, and snapshots the browser uses, and the Start, Guitar Pedalboard (a 1 kHz input), and Subtractive Synth (Demo off, a MIDI note) examples play sample for sample what a local engine plays, with the editor's transport and meters following; ring tests cover two threads, wrapping past `u32::MAX`, full rings, and damaged records, the snapshot a writer racing a reader, and `ac-engine/tests/relay.rs` a relayed engine across edits, seeks, and parameter changes. In the browser, `tests/web/e2e.cjs` (CI's *Web build and browser test* job) serves the site built by `scripts/build_web.py` with the COOP/COEP headers and opens it in headless Chromium: Start plays a 220 Hz saw at the −18 dB level (analyser and master meter) with the position moving and the editor in step, a parameter change moves it to 440 Hz; the Pedalboard asks for the (fake) microphone and passes its 1 kHz tone; the Synth is silent until a MIDI note through the page's MIDI path sounds at 440 Hz and stops after note off; Four Tracks plays its clips; Reverse Delay's Reverse node bakes on the page and plays (natively, the worklet then plays sample for sample what a local engine that baked plays); a patch whose oscillator pitch comes from a Text constant read by Text to Number plays at 330 Hz (natively, sample for sample as locally); a MIDI In → MIDI Out patch sends its messages back to the page; no ring records are dropped and the console logs no errors. Web MIDI ports and gamepads are covered by unit tests of the mapping and by the script-driven MIDI path; real devices need a person (see the MR's manual steps). The service worker path (a host without headers) and the `_headers` file were not exercised against a real host.
- [x] Projects open from and save to the browser's file API; examples load from the site.
  Verified: `crates/ac-app/tests/host_files.rs` covers the editor's side (open, save as of a revision, Save As, packs with their audio both ways, Locate and a Clip's file kept in memory, examples, Export WAV, block files); in headless Chromium the e2e test opens a project through the File System Access API (pickers standing in for a person's choices), saves it back to the same file without asking and asks on Save As, then without the API takes an upload and downloads the save; every example in it loads from the site's `examples/index.json`, Four Tracks with its audio files; an unsaved edit shows in the page's title and leaving asks first.

### M3.4 Engine extras
- [x] Single-sample feedback for strongly connected components.
- [x] `TimeStretch`, `Convolution`, `Granular`, `Formant`, `Chaos`, `SpectralFreeze`, `Spectrogram`.
- [x] Native PipeWire backend if the shims prove limiting.

*Verified:* each part is covered as follows.

Single-sample feedback (DECISIONS D140): a Feedback node with `delay` set
to `sample` makes the compiler run its strongly connected component as one
loop step, one sample at a time (`crates/ac-engine/src/loops.rs`).
- `cargo test -p ac-engine --test loops`:
  - a loop through a Delay of `d` samples comes round every `d + 1`
    samples (block mode: `d` + the block), and two sample-mode Feedback
    nodes in one loop add a sample each;
  - the loop is bit-identical at block sizes 1, 2, 7, 64, 333, and 1024,
    and on 1, 2, and 4 threads, live matching offline;
  - MIDI and triggers from outside reach a member on their own sample,
    and a trigger leaves the loop on its sample, at every block size;
  - a parameter moved while playing ramps sample by sample, the same at
    block sizes 1, 8, 64, and 256;
  - the loop keeps ringing through a recompile (state carries over), a
    muted member passes through, and a Scope inside the loop records
    every sample;
  - each voice of a Voices node gets its own loop;
  - a sample loop inside a block loop keeps both delays, and a sample
    Feedback on no cycle delays by one sample;
  - FFT and IFFT, or a whole-input node, inside the loop are reported on
    each node and on the Feedback, and the loop keeps one block of delay;
    a cycle with no Feedback beside it is still an error; a nonsense
    `delay` value means a block. Nothing panics, and every block runs
    under the allocation guard.
- Golden `utility-feedback-sample`: a Karplus-Strong string whose loop is
  218.2 samples long. A test measures it at 220.0 Hz (within 0.5 Hz) at
  block sizes 32, 256, and 1024; with a block of delay the same patch
  plays near 100 Hz at 256 and higher at 32. The golden is
  block-independent; no existing golden hash changed.
- `audioc bench` (release, 128-frame blocks, 10 s per project, best of 3,
  five runs of the base and the new build alternated on the same 4-vCPU
  VM, which other jobs shared): no project got slower beyond run-to-run
  spread. Real-time ratios, base → new, best of five: oscillators-1000
  3.12 → 3.26, effect-chain-200 3.87 → 3.93, mono-synths-16 15.0 → 16.0,
  parallel-chains-8 8.33 → 8.58, poly-synth-16 25.2 → 23.5 (medians 20.4
  → 22.6), subtractive-voices-16 10.05 → 10.31, fft-chains-16 24.6 →
  26.6, pitch-shift-16 7.67 → 8.28, diffuser-16 4.35 → 4.50. Rerun at
  the milestone's last commit (the engine unchanged since, the new nodes
  in no bench project), four alternated runs: best-of-four changes from
  −6.7 % (mono-synths-16) to +10.8 % (diffuser-16), with the two builds'
  ranges overlapping on every project.

The seven nodes, one file each, with a "Try this" tip and the DSP source
cited in the code (`cargo test -p ac-nodes`):
- **Chaos** (Lorenz 1963, Rössler 1976, fourth-order Runge-Kutta):
  outputs stay within ±1 and use the range across `chaos`; Lorenz flips
  between its lobes and settles below the onset of chaos; Rössler at 0
  loops at `rate`; the motion matches at 48 and 96 kHz; block sizes 1, 64,
  and 1024 and a reset repeat exactly; seeds drift apart.
- **Formant** (Peterson and Barney's 1952 vowel formants, three
  state-variable band-passes): each vowel's impulse response peaks at its
  formants; a and i differ where they should; morphing glides in pitch;
  `shift` and `voice` move them; the shape is the same at 48 and 96 kHz;
  a saw comes out 11–18 dB down (documented); block size doesn't matter.
- **Spectral Freeze** (Dolson's phase vocoder with Laroche and Dolson's
  rigid phase locking): a held 1 kHz tone keeps its pitch and level a
  second after its input stops; frozen bins keep their levels and turn
  their phases; a frozen glide holds its level within 5 % (a test that
  fails without the locking); glide and block size are exact.
- **Spectrogram**: a tone lights its band in every column; time runs
  through the columns; short sounds stay in long columns; the picture
  starts silent; block size doesn't change it. The editor draws it as a
  mesh: a unit test checks each cell's place and color, and
  `cargo test -p ac-app --test editor a_spectrogram_draws_what_it_hears`
  plays a 1 kHz tone on the null stream and finds the lit row in the
  card's body (stable over repeated runs).
- **Granular** (Roads' "Microsound" grains): onsets follow `density`;
  `pitch` transposes and `position` picks the slice; a buffer at another
  rate keeps its pitch; overlap keeps the level within 3 dB; `stop` lets
  grains finish; reset repeats the seeded scatter; block sizes match.
- **Convolution** (uniformly partitioned overlap-save, Wefers 2015):
  matches the direct sum one partition late; a file response is ready at
  once; the latency it reports follows `partition`; a stereo response
  feeds each side; a new response takes over without cutting the old
  tail; a response at another rate is converted; long responses are cut
  at `max_length`. In the engine (`cargo test -p ac-engine --test
  extras`) a dry path mixed with it arrives on the same sample at block
  sizes 1, 64, and 1024.
- **Time Stretch** (phase vocoder with identity phase locking): a stretch
  of 1 returns the input within −60 dB; stretches of 0.5, 2, and 3 change
  the length and keep the pitch and level; a changing stretch moves
  through the input at its rate; it bakes offline and is disabled live
  when fed by Audio In.

Every node also runs in the allocation-guarded sweep at block sizes 1,
64, and 1024 (`all_nodes.rs`; Granular and Convolution with buffers and
files in `buffer_nodes.rs`), and a test checks all seven have NODES.md
rows and are registered. Goldens, all block-independent and passing the
live-vs-offline check: `modulation-chaos`, `filters-formant`,
`spectral-freeze`, `display-spectrogram`, `generators-granular`, and
`time-convolution-stretch` (its impulse response,
`tests/golden/assets/plate-ir.wav`, was rendered with `audioc` from the
shipped Plate Reverb block). The streaming nodes also run inside a
single-sample loop, and a Spectral Freeze there is refused.

PipeWire (DECISIONS D141): evaluated, and no native backend added, since
neither PipeWire layer limits the engine. The evaluation, from the code
and the libraries:
- The engine takes whatever rate and buffer size a server gives it and
  opens input and output as separate streams (D36).
- The device layer lists and picks cpal's hosts by name
  (`ac-io/src/audio.rs`), so nothing in it is PipeWire-specific.
- The JACK route is what CI exercises for real (D100): it plays through
  `jackd`'s dummy driver with the same libjack interface that PipeWire's
  JACK library implements.
- The ALSA route uses PipeWire's ALSA plugin, shared with other programs.
  It has PipeWire's quantum on top of the ALSA buffer (documented, with
  the low-latency JACK route, in the guide's getting-started and
  troubleshooting chapters).
- cpal 0.18.2, already in `Cargo.lock`, has a PipeWire host behind its
  `pipewire` feature. Adopting it would mean linking `libpipewire-0.3`,
  so like JACK it could only be an opt-in build. That is a feature flag
  and a CI job away if a limitation turns up.

Example: `examples/plucked-strings.acproj` plays a string built on a
single-sample loop into a Convolution hall, with Chaos and a Spectrogram
(`cargo test -p ac-cli --test examples`: its loop and latency, and the
same render at block sizes 64 and 1024). Its impulse response,
`examples/assets/hall-ir.wav`, was rendered with `audioc` from the
shipped Hall Reverb block. Every example still lays out without
overlaps.

Needs manual verification (in the PR):
- listening: the new goldens, the Plucked Strings example (in tune, with
  its Feedback set to sample; flat with it set to block), Formant's
  vowels, Granular clouds, Spectral Freeze holding a chord, Time Stretch
  at 0.5 and 2, and Convolution with a real room's impulse response;
- by eye: the Spectrogram on a card and on a performance panel, in both
  themes;
- on a PipeWire desktop, a release build plays and records through the
  default device, and `audioc latency` with a loopback cable reports the
  round trip;
- also on PipeWire, a JACK build under `pw-jack` with
  `PIPEWIRE_QUANTUM=128/48000` runs with 128-frame buffers (the Devices
  tab shows the size) and a lower measured round trip.

### M3.5 Product
- [x] Installers and signed builds (cargo-dist), auto-update check, crash reporting opt-in.
  Verified: installers are made by `scripts/package.py` with each platform's own tools rather than cargo-dist, which can't lay out an app bundle or put plugins where hosts look (D170). CI's *Bundles and installers* job builds the `.deb` and the tarball, the `.pkg`, and the NSIS setup program from release builds on Linux, macOS, and Windows, installs each on its runner (`apt-get install`, `installer -pkg`, `setup.exe /S`; the tarball's `install.sh` into a scratch home), runs `audioc --version` and `audio-constructor --version` from where they landed, checks the plugins, examples, menu entry, and (Windows) the uninstall entry are in place, renders the Start example with the installed `audioc`, and uninstalls again, checking the files are gone; the packages are kept as the run's artifacts. Signing (D171) runs only with the project's certificates, which don't exist yet, so the signed paths are untested: listed under the MR's manual checks. The update check (D173): `ac-io`'s `update` tests read the feed format, compare versions, and report a feed that's down, against a local server; `crates/ac-app/tests/product.rs` shows nothing is fetched before the person answers, a daily check offers a newer release with its download page and notes, a skipped version stays quiet while Help ▸ Check for Updates… still answers, a check within the day makes no request, and a failed check is silent unless asked for; `crates/ac-cli/tests/version.rs` covers `audioc version --check`, and run here against the real feed it reported 0.9.0 as the latest. Crash reports (D174): `ac-io`'s `crash` tests build a report from a real panic (thread, message, location), write `~` for the home folder, keep twenty, offer each once, and make an issue link that fits; `product.rs` shows a saved report offered once at start, Show Report, Report… opening the prefilled form, and "Don't offer again" kept; `ac_io::net` (bounded https fetching) refuses plain http beyond this machine, oversized bodies, and silent servers.
- [x] Block sharing: export with preview render, import from URL, library index format.
  Verified: `cargo test -p ac-io` covers the preview's wiring (an effect gets the built-in phrase, a sidechain its drums, a Voices instrument the chords, a gate/freq voice the arpeggio, triggers by name, a drum machine plays alone), that a preview is the same bytes every time, and that a silent block or one with no audio output gives a reason; the index (broken and unknown-node blocks left out, previews hashed, round trip, malformed entries skipped, a newer version refused, relative links); and links (a block brings its preview, an index lists and installs checked blocks, an altered or oversized block is refused, previews cached by hash, packs recognised, 404 and plain http reported). `crates/ac-cli/tests/previews.rs` checks every shipped block's preview against `tests/golden/block-previews.json` (SHA-256 on Linux x86-64, levels elsewhere), that `audioc block-export` writes what the editor renders, that `library-index` leaves out a broken block, and `block-import` from a local server (listing, choosing, a changed block refused, a pack). `crates/ac-app/tests/blocks.rs` plays a preview in the Library tab and from the cache, exports from the Node tab, imports a block and a library from a link, installs, plays a remote preview, opens a pack from a link, and removes a library.
- [x] Theming, localization scaffold, full accessibility pass.
  Verified: the theme tests check every palette in every theme (body text at least 4.5:1, 7:1 in High Contrast; signals 3:1; the focus ring 3:1 on canvas, cards, and panels) and that bad theme files, colours, and fields are reported without panics; the editor tests choose, reload, and save themes by name, fall back to Dark, and warn about low contrast, and the themes guide must list every palette field. `crates/ac-app/tests/catalogs.rs` checks the Fluent catalogs parse and that every id the code uses exists and none is unused; `tests/language.rs` opens every menu in the pseudo-locale and fails on any untranslated label. `tests/accessibility.rs` audits the AccessKit tree (the Start example with every side tab, the panel view, performance mode, every menu) and fails on an interactive node without a name; finds cards and sockets by their spoken labels; builds and plays a patch with the keyboard alone (a Gain from the palette, Oscillator → Gain → Audio Out wired, −12 dB typed with F2, Space); plays panel controls from keys and from screen-reader actions; and checks Reduce Motion and the size of every pointer target at 100 %. Real screen readers (NVDA, VoiceOver, Orca) are manual checks in the MR.

**Phase 3 acceptance**
- A project runs as a CLAP effect in a host, with its exposed parameters automatable.
- A third-party CLAP synth runs as a node inside a project.
- The web build runs the Start, Pedalboard (with browser mic), and Synth examples.

*Verified* (all three in CI, on Linux, with the whole of Phase 3 on one
branch):
- **CLAP effect, automatable.** `crates/ac-plugin/tests/clap_host.rs`
  (`a_project_runs_as_a_clap_effect_with_automatable_macros`) loads the
  plugin in a real CLAP host: a sine passes at 0 dB, Macro 1 automated to
  0.25 at frame 100 leaves the earlier samples alone and settles at
  −12 dB within 0.05 dB, at uneven block sizes. CI's bundle job runs it
  against the release `.clap` the installers ship.
- **A third-party CLAP synth as a node.** CI's *Third-party plugin as a
  node* job downloads Six Sines 1.2.1 for Linux (checked by SHA-256,
  D136), renders it from a MIDI Pattern with `audioc` (notes at 220 and
  440 Hz), its VST3 build too, and opens both windows while they play;
  the process exits cleanly (D139).
- **The web build.** CI's *Web build and browser test* job builds the site
  and plays it in headless Chromium (`tests/web/e2e.cjs`): Start sounds at
  220 Hz and follows a parameter change, the Guitar Pedalboard takes the
  browser's (fake) microphone and passes its 1 kHz tone, and the
  Subtractive Synth plays a MIDI note at 440 Hz and stops after note off.
  A real microphone, MIDI keyboard, and browsers other than Chromium are
  manual checks.

---

## Phase 4 — Tools: what the studies found missing (draft)

> Drafted from [GAPS.md](GAPS.md), which built the basics of a multitrack
> recorder, a pattern studio, a sampler, and a clip launcher from today's
> nodes (`studies/`) and counted nodes and gestures. The order follows its
> priorities: first what every study hit, then what each tool can't do
> without. Nothing here is settled until a human confirms the milestones;
> cut or reorder freely, and keep each milestone's "rebuild the study" box,
> which is how a milestone proves it closed a gap.

**Goal.** A person can build those four tools from nodes and panels, and
each feels like the tool: adding a track is one gesture, a clip sits at a
bar and playback can start anywhere, a pad lights up and feels velocity, a
loop records on the bar. Every milestone rebuilds at least one study the
new way and updates its node count in `studies/README.md` and its gesture
table in `GAPS.md`.

### M4.1 Stop always stops
- [ ] Transport Stop fires every generator's `stop`, wired or not, unless its dropdown says *Never* (the dropdown stays beside a wired `stop` as a second source), and fires with the transport already stopped (GAPS A).
- [ ] A **Panic** action: a transport-bar button, `Escape` twice, and a bindable transport action, that resets every generator, releases every voice, and mutes every Audio Out for one block; the status bar shows which outputs are sounding.
- [ ] In `studies/scenes`, Stop ends the loops with the transport playing or not; the recorder's input monitor defaults to off.

### M4.2 Panels have structure
- [ ] **Strips**: a named vertical group of cards with one header, kept together as a column and moved, resized, and unpinned as one; **section labels and row breaks** between strips; **compact cards** (toggles and buttons at half height, two to a cell; a meter narrow beside a fader); a card keeps its place when something new is pinned (GAPS C).
- [ ] Pin a group or block instance to a panel: a card with a widget per unwired input (by type and unit) and a meter or light per output, so the Track block pins as a channel strip (GAPS C).
- [ ] `lit: Bool` input on Button and Toggle; a **Light** display node (`Bool` or level, with a color); a **Fader** control (vertical, in dB, with a `meter` audio input beside it); a **Pad** control with velocity and pressure; a **Pads** grid control (rows × columns → `midi` or per-pad triggers).
- [ ] Panel layout: named rows and columns, labels and separators, a color per card, multi-select drag; **panel from selection** (Mixer from these Tracks, Pads from these Samplers, Grid from these Clips).
- [ ] Rebuild `studies/multitrack-recorder` and `studies/scenes` with pinned blocks; update their node counts and the Mixer and Session rows in GAPS.md.

### M4.3 Song-locked time
- [ ] An `at: Float(Beats)` on Clip, MIDI Pattern, Pattern, and Graph (a new `start` choice, *At beat…*); `seek` on the Node trait, defaulting to a reset; a transport seek lands every song-locked generator at the right place inside itself; `audioc render --from` and a live seek agree (GAPS A).
- [ ] Transport: click or drag the position to seek, a ruler with bars, loop set by dragging on the ruler, markers (in the project, on the ruler, jumpable), a metronome button on the monitor path, a count-in in bars, a punch range applied to armed Record nodes.
- [ ] Audio Out `role` config, *main* or *monitor*: monitor outputs play live and are left out of renders and File Outs.
- [ ] Create Clip from take places the clip at the bar the take started and wires it where the Record's input went.
- [ ] Rebuild the recorder study: play from bar 9 works, the Compare/Gate to Trigger arrangement logic is gone, the click stays out of the export.

### M4.4 Sampler nodes
- [ ] **Sample** (a file as a `Buffer` output); **Note Split** (one note or a list → per-note `trigger`, `gate`, `velocity`, with choke groups); **Route** (the mirror of Select: `index` picks an output, and a gate mode that passes while open; triggers, MIDI, audio, and Buffer); Buffer in Select and Switch; **Zones** (key and velocity ranges → zone index and per-zone triggers, with a map editor); **Slicer** (equal, transient, or marked slices of a Buffer, played by index); **Velocity** (scale, curve, fixed); **Waveform** display (a Buffer or a Clip's file with the playhead and drag handles for start, end, and loop points; the same widget is the Node-tab editor for Clip and Sampler); **MIDI Monitor** and **Tuner** displays (GAPS B).
- [ ] Drop an audio file on the canvas (a Clip; with `Shift` a Sampler), on a Clip or Sampler card to assign it, on a panel to make a pad.
- [ ] Rebuild the pads study: a pad from MIDI with velocity is two nodes; the keys are Zones + Route + Samplers; the slices come from a Slicer.

### M4.5 Patterns and loops
- [ ] Pattern banks (`bank: Int` input, banks added in the editor) and cell velocity (painted by drag height; `velocity_k` per row); MIDI Pattern banks; `swing` on Clock and MIDI Pattern; **Humanize** (seeded timing and velocity jitter).
- [ ] **Looper** node: record, play, stop, overdub, undo triggers, quantized to beats, length free or from the first pass; `out`, `buffer`, `playing`, `recording`, `position`.
- [ ] Logic (and, or) and Math (add, multiply, min, max) with an `inputs` config; Mix and Merge grow when wired into their last input, and the palette offers a Merge when a second wire is dropped on a wired trigger input.
- [ ] The drawn editors as panel cards, live and editable: Pattern grid, Step Sequencer, piano roll, Graph, Waveform; several editor windows open at once; one docked under the canvas.
- [ ] Rebuild the pattern and scenes studies: the pattern Switches are gone, the hats have accents and swing, the looper is one node and overdubs.

### M4.6 Recording and arranging
- [ ] **MIDI Record** node (a take becomes a MIDI Pattern beside it, optionally quantized); automation write and touch (moving a knob while recording writes a song-locked Graph into that input); a **Tempo** node (`bpm` in, `tap` trigger; an edit to the tempo map, not a signal).
- [ ] An **Arrangement** panel card: lanes of song-locked Clips and MIDI Patterns drawn in time, draggable and trimmable, with the markers and the loop; a view onto nodes, nothing of its own.
- [ ] Templates: Add ▸ Track, Drum Pad, Clip Slot, Send, each a block instance wired in and pinned; user-saved templates from a selection.
- [ ] Rebuild all four studies; the gesture tables in GAPS.md get their "after" column.

**Phase 4 acceptance** (each measured against the gesture tables in GAPS.md)
- Press Stop in any study, with the transport playing or already stopped, and nothing sounds; the Mixer panel reads as one strip per track with a label over each section.
- From the Four Tracks example, add a track and record a take in five gestures or fewer, then play the song from bar 9.
- A 4×4 pad bank from a MIDI drum pad in at most 20 nodes, each pad lit by the pattern and playing with velocity.
- A three-track, two-scene session grid in at most 12 nodes besides the clips, launched on the bar, each slot showing its state; a loop recorded and overdubbed with one node.
- A pattern studio with two patterns alternating every four bars in at most 20 nodes, with swing and accents, its grid and piano roll on one panel.
