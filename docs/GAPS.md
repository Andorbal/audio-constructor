# Gaps: what familiar tools do that the nodes can't yet

Audio Constructor is meant to be flexible enough that a person can build
the tool they want: a multitrack recorder, a pattern studio, a sampler, a
clip launcher. This document measures how close today's nodes, blocks,
panels, and bindings get, by building the basics of each of those four
kinds of tool and writing down where it got hard, where it got silly, and
where it got impossible. It ends with the list of nodes, panel features,
editor changes, and engine changes the studies argue for, in priority
order, which `docs/ROADMAP.md` Phase 4 turns into milestones.

The bar is not a clone. It is: the basics work, and they feel good. If a
recorder can be built but recording a track takes twenty gestures, that
is a failure, and this document counts the gestures.

## Method

Four study projects live in [`studies/`](../studies/README.md). Each is a
real project: it compiles, renders, makes sound, and is kept working by
tests (`crates/ac-cli/tests/studies.rs`, and the editor's layout test).
Each was built the way a person would build it in the editor, with the
nodes that exist, then read back for three things:

- **Possible?** Could the feature be built at all, with any number of
  nodes, without changing the engine?
- **How many nodes?** The node and wire count per unit of the tool (one
  mixer channel, one drum pad, one clip slot). This is what a person
  wires and later reads.
- **How many gestures?** The clicks, key presses, drags, and typed values
  to do the tool's everyday tasks in today's editor, counted from the
  documented interactions (`docs/guide/editor.md`), against roughly what
  the same task takes in the tool being studied. A gesture count is an
  estimate of feel, not a benchmark; it is accurate to within a few
  gestures and the comparison holds at any reasonable counting.

What the studies stand in for:

| Study | Kind of tool | Everyday tasks measured |
| --- | --- | --- |
| `multitrack-recorder` | A linear multitrack recorder with a mixer | Add a track; record a take; punch in; place a clip at a bar; start from bar 9; loop a section; automate a fader; export stems |
| `pattern-studio` | A pattern-based studio: channel rack, step grid, piano roll, playlist | Add a channel; program a beat; vary velocity; swing it; make pattern B; arrange A and B into a song; route a channel to an effect |
| `pads-and-keys` | A sampler and drum machine | Put a sound on a pad; play pads from a keyboard with velocity; velocity layers; round robin; choke; slice a loop; build a keymapped instrument; resample |
| `scenes` | A clip launcher and a loop pedal | Launch a clip on the bar; stop it; launch a scene; see what's playing; record a loop; overdub |

## What already works well

Credit first, because several things came out better than expected and
should be protected:

- **The generator contract** (start/stop/reset/loop/rate, `playing`,
  `end`) makes clips, patterns, and samplers interchangeable. Chaining,
  retriggering, and the arrangement tricks below all come from it.
- **Record → `buffer` → Sampler** is live resampling with two wires, no
  file dialog. The sampler and looper studies lean on it.
- **The Track block** has real solo semantics (`soloed`/`any_solo`) and
  shared insides, so a mixer's channels stay identical.
- **Bindings** make any control playable from any hardware in two
  gestures, and the project remembers them.
- **File Out** gives sample-aligned stems from one render.
- **Tempo-synced nodes** (Clock, Delay in beats, Graph in beats) follow
  the tempo map exactly; nothing drifts.
- **Voices** turns any body into an instrument, and a custom body (the
  two-zone sampled keys) took eight nodes.

## What the first run showed

Two things stood out as soon as the studies were opened and played,
before any gesture was counted. They go first in the priorities below.

- **A panel doesn't say what belongs with what.** The Mixer panel's 39
  cards flow into equal cells in pin order. A toggle gets the same cell
  as a knob, a meter spans three cells and pushes the next track's first
  knob to the end of the line, and the only thing relating a pan to its
  fader is the word in front of it. Everything works and nothing reads.
  A mixer is strips; a drum rack is rows; a session grid is tracks by
  scenes. Panels have none of those shapes, and moving one card at a
  time can't make them.
- **Stop doesn't always stop.** Transport Stop fires a generator's
  `stop` only when that input is unwired, and only if the transport was
  playing. In the launcher study every clip's `stop` is wired (the slot
  logic needs it), so Stop leaves six clips looping; the slot stop
  buttons then wait for a bar clock that is quiet while the transport is
  stopped, so nothing short of closing the project ends the sound. In
  the recorder the input monitor is a toggle defaulting to on, so the
  microphone is live from the moment the project loads whatever the
  transport does. Nothing in the editor silences the graph as a whole,
  and nothing shows what is sounding.

## The studies, one by one

### 1. The multitrack recorder

**Built** (92 nodes, 105 wires): five tracks, each a Clip or an Audio In
through the Track block with a volume knob, a pan knob, mute and solo
toggles, a meter, a reverb send knob and gain, and a stem File Out; a
reverb bus and a delay bus with return knobs; a master Mix into the
Compressor and Limiter blocks, a fader, a meter, an analyzer, and the
output; a metronome on a second Audio Out; the chords entering at bar 5
and leaving at bar 13 and the lead entering at bar 9; a drawn volume ride
on the lead; a vocal track with input monitoring and a punch-in window
on bars 5 and 6; a Mixer panel of 39 cards; F5 bound to the punch toggle.

**What one track costs**: 9 nodes and 10 wires (source, Track, 2 knobs,
2 toggles, meter, send knob and gain, File Out), plus one more Mix input,
one more input on the reverb bus, and one more OR gate in the solo chain.
Five tracks need four Logic nodes just so solo works.

**Gestures**

| Task | Today | In a recorder | Why |
| --- | --- | --- | --- |
| Add a track that shows up on the mixer with fader, pan, mute, solo, and a meter | about 50 (9 nodes from the palette, 10 wires, 5 renames, a Mix input, 5 pins, the solo OR) | 1 or 2 | No strip; a block's inputs can't go on a panel, so every control is its own node |
| Record a take on a new track and hear it in the mix | about 25 (Audio In, Record, save the project first, `R`, `R`, Create Clip from take, a Track, wires, a Mix input) | 4 (new track, arm, record, stop) | Record has no place in a track; the take has to be wired in by hand |
| Hear yourself while recording | 8 (Switch, Toggle, 3 wires) | 1 (monitor button) | No monitor path |
| Tell which controls belong to which track on the mixer | read the label prefixes; 39 cards flow in pin order and each meter breaks the line | default: one strip per track | Panels have no strips, rows, or labels; every card takes the same cell |
| Make the room quiet | Stop, then find the Monitor toggle among 39 cards | Stop | Stop reaches generators and nothing else; a live input through a Switch plays on, and nothing shows what is sounding |
| Punch in on bars 5–6 | about 30 (2 Compares with typed beats, 2 Logics, a Toggle, 6 wires) | 2 drags on the ruler | No punch range in the transport |
| Place a clip at bar 5 | 11 (Compare with a typed beat, Gate to Trigger, `start` set to Never, 3 wires) | 1 drag | Generators start when the transport starts; nothing is placed in time |
| Start playing from bar 9 | impossible | 1 click on the ruler | The transport only seeks to 0 (`Rewind`); a seek restarts every generator from its own top (`Command::Seek` sets `start_flag`) |
| Loop bars 5–8 | 6 (menu, two typed beats) | 1 drag | Fine; no ruler to drag on |
| Ride the lead's volume | about 20 plus drawing (Graph in beats with a typed duration, Map, Math add, 3 wires) | 2 plus drawing, or just move the fader in write mode | No automation lane; no write/touch; the curve doesn't know where bars are |
| Export stems | 7 per track (File Out, a typed path, a wire) then `Ctrl+E` | 4 in one dialog | Acceptable; a "stem per track" option would make it 1 |
| Keep the click out of the export | impossible | default | Every Audio Out is summed into a render |
| Record MIDI | impossible | 2 | No MIDI recording of any kind |
| Comp takes, trim a clip, fade a clip, drop a marker | impossible | a few each | No take lanes, no clip editing, no markers |

**Verdict.** A mixer can be built and sounds right, but every track is a
page of nodes, and the arrangement half of a recorder isn't there:
nothing is placed in time, playback can't start in the middle, and there
is no view of the song as a whole.

### 2. The pattern studio

**Built** (56 nodes, 67 wires): a Clock in sixteenths into two 16-step,
4-row Patterns; four drum Samplers on slices of one loop, each picking
pattern A or B through a trigger Switch; a two-node playlist (bar modulo
8, compared with 4) that alternates A and B every four bars; a Step
Sequencer giving the hats sixteen velocities through a Map into the
Sampler's gain; two piano rolls (MIDI Patterns) for chords through a
MIDI Switch into a six-voice synth; one 64-beat piano roll holding the
bass for the whole song into an FM pair; a Track block with knobs and a
mute per channel; a reverb bus and a tempo-synced delay; a limiter and a
meter; a Rack panel; keys 1–6 bound to the mutes.

**What one drum channel costs**: 6 nodes (Switch for the pattern, Sampler,
Track, 2 knobs, Toggle) and 8 wires, plus a row in each Pattern and a Mix
input. A velocity row adds 2 nodes. A second pattern adds one Switch per
channel, so an eight-channel rack with three patterns needs sixteen
Switches and two Compares before any music is written.

**Gestures**

| Task | Today | In a pattern studio | Why |
| --- | --- | --- | --- |
| Put a sample on a new channel in the rack | about 25 (Sampler, Choose file, start and end points typed, Switch, Track, knobs, mute, wires, a Pattern row, a Mix input) | 1 drag | No channel strip; Sampler has no waveform to set points on |
| Program a beat | 16 clicks on the grid, after the Pattern and Clock exist (7) | 16 clicks | Same, once set up; but the grid lives in the Node tab, not on a panel |
| Accent some steps | 7 plus 16 typed values (Step Sequencer, Map, 2 wires) | 16 drags under the grid | Pattern steps are on or off; no velocity per step |
| Swing the beat | impossible | 1 drag | No swing anywhere: not on Clock, Pattern, or MIDI Pattern |
| Make pattern B | 3 plus painting, then one Switch and 3 wires per channel | 1 (new pattern) | A Pattern holds one grid; switching needs a node per channel |
| Arrange A and B into a song | 8 for an alternation; a real song needs a Compare and a Select per section | drag patterns onto the playlist | No playlist; no view of patterns in time |
| See the piano roll next to the drum grid | impossible | default | Editors are one at a time in the Node tab, and never on a panel |
| Route a channel to a send effect | 4 (Gain, 2 wires, a Mix input) | 2 (pick a mixer track, turn a send) | Acceptable |
| Mute a channel from the keyboard | 2 (Bind…, press) | 1 | Fine |

**Verdict.** The beat-making core is close: Pattern and Clock are the
right primitives and feel right once wired. What's missing is above and
around them: a step grid that lives on a panel with velocity under it,
swing, more than one grid per Pattern, and any way to lay patterns out in
time.

### 3. The sampler and drum machine

**Built** (83 nodes, 112 wires): MIDI In and the computer keys merged;
eight pads, each a MIDI Filter on one note, a MIDI Note for its trigger
and velocity, a Map from velocity to gain, a three-input trigger Merge
(MIDI, a pad Button, and a demo Pattern), and a Sampler on a slice of
one loop; a snare with a soft and a hard layer by velocity, each
alternating two samples, through two Switches, a Counter, a Compare, and
four more Switches into four Samplers; the closed hat choking the open
hat; a two-zone keymapped instrument as a custom Voices body (two
Samplers, a Compare, an audio Switch, an ADSR, a Map, a Gain); a Record
fed by the pads whose buffer a ninth pad plays; a Pads panel; keys 1–8
bound to the pads.

**What one pad costs**: 6 nodes and 7 wires from MIDI with velocity and a
button. Eight pads are 48 nodes; a 16-pad kit would be 96 before layers,
envelopes, or per-pad effects. The snare's two layers times two round
robins cost 11 routing nodes on top. A per-pad decay would be three more
nodes per pad (ADSR, Map, Gain), because a Sampler has no envelope and a
shared pad block can't hold a different sample per instance.

**Gestures**

| Task | Today | In a sampler | Why |
| --- | --- | --- | --- |
| Put a sound on a pad and play it from note 36 | about 30 (6 nodes, a file chosen, low and high typed, 7 wires) | 1 drag | No note-to-trigger node; Sampler needs a chain to listen to one note |
| Set a slice's start and end | 2 typed fractions, by ear | 2 drags on the waveform | No waveform anywhere in the editor |
| Slice a loop onto eight pads | 8 Samplers with 16 typed points | 1 (slice at transients) and 1 drag | No slicer; Onset can't cut a buffer |
| Two velocity layers | about 15 (Compare, 2 Switches, a second Sampler, wires) | 1 drag to split a zone | No zone map; no trigger routing node, so Switch is misused twice |
| Round robin | about 15 (Counter, Compare, 2 Switches per layer) | 1 click | Same |
| Choke the open hat | 1 wire | 1 setting | Fine |
| Keymap two samples with an envelope | about 40 (a Voices body of 8 nodes) | 2 drags | No zone node; a Sample-to-Buffer node would let a shared body take its sound as an input |
| Resample the pads onto a new pad | 8 (Record, 2 Buttons, Sampler, 4 wires) | 2 | Good |
| Play a pad with velocity from the mouse | impossible | default | Buttons have no velocity or pressure |
| See a pad light when the pattern hits it | impossible | default | No light display; Buttons take no `lit` input |

**Verdict.** Sample playback itself is complete and sounds right, and
resampling is better than most tools. Everything around it is missing:
a waveform to look at, a way to hear a note and fire a pad in one node, a
zone map, a slicer, and velocity in the controls. A drum machine is
possible but is a wall of routing nodes.

### 4. The clip launcher and looper

**Built** (99 nodes, 123 wires): three tracks with two clip slots each.
A slot is a Button, an arm Merge (the button and the scene), a disarm
Merge (the other slot's button and the track's stop), a Latch for the
armed state, a Sample & Hold clocked by a bar Clock, a Compare, a rising
and a falling Gate to Trigger into the Clip's `start` and `stop`, and a
Readout of `playing`. Two scene buttons, each through a Merge per slot.
A per-track Mix, knob, and gain. A looper: a Button toggling a Latch,
the same bar-quantized Sample & Hold, Record start and stop, a Trigger
Delay of 50 ms before the Sampler starts looping the take. A Session
panel of 23 cards; keys Q W A S Z X bound to slots, 1 and 2 to scenes,
the gamepad's south button to the looper.

**What one slot costs**: 10 nodes and 13 wires. Six slots are 60 nodes.
Exclusive slots on a track and scenes each add a Merge input per slot,
because a trigger input takes one wire.

**Gestures**

| Task | Today | In a launcher | Why |
| --- | --- | --- | --- |
| Add a clip slot that launches on the bar | about 50 (10 nodes, 13 wires, `start` Never, `edge` falling, a typed Compare) | 1 drag | No launch quantization anywhere; it's built from Latch, Sample & Hold, and edges |
| Launch and stop it | 1 and 1 | 1 and 1 | Fine, once built |
| Stop everything | impossible once the transport is stopped: the clips keep looping and the stop buttons wait for a bar that never comes | 1 (Stop) | Transport Stop fires only an unwired `stop`, and only while playing; the Clock is quiet while stopped; no panic |
| Make slots on a track exclusive | 2 Merges and 4 wires per track | default | A trigger input takes one wire; every extra source is a Merge |
| Add a scene | 1 Button plus a Merge and 2 wires per slot | 1 | Same |
| See which slot is playing | a Readout showing 0 or 1 | default | No light; Buttons don't light up |
| Lay the slots out as a grid of tracks and scenes | drag 23 cards by hand; it flows in pin order | default | Panels have no rows, columns, or labels |
| Record a loop at the next bar | about 35 (12 nodes) | 1 | No looper |
| Overdub on it | impossible in practice (a second Record and Sampler pair and a buffer swap on every pass) | 1 | Record writes one take at a time; Sampler latches its buffer at start |
| Set the tempo from the first loop's length | impossible | default | Nothing writes the tempo map from the patch |

**Verdict.** The launch logic works, and bar-quantized launching built
from primitives is a nice demonstration of the model, but no one should
build a 60-node grid to launch six clips, and a looper is the one thing
the loop-pedal crowd needs most.

## The gaps, grouped

The same shortfalls appear in several studies. Grouped by what has to
change, with the studies each one helps (R recorder, P pattern studio,
S sampler, L launcher).

### A. Time and arrangement (engine, transport, model)

| Gap | Helps | What's needed |
| --- | --- | --- |
| **Nothing is placed in time; playback can't start mid-song.** A seek restarts every generator from its own top, and the editor only seeks to 0. | R P L | **Song-locked generators**: an `at: Float(Beats)` on Clip, MIDI Pattern, Pattern, and Graph (a new choice in the `start` dropdown, *At beat…*), and a seek that lands each of them at the right place inside themselves (`seek(sample)` on the Node trait, defaulting to reset). Then a **seekable transport**: click or drag the position, a ruler, `Home`/`End`, bar jumps. |
| **Stop doesn't always stop.** Transport Stop fires a generator's `stop` only when it is unwired (wiring it replaces *On transport stop*), and only if the transport was playing; anything started by a button, a clock, or a live input while stopped plays on. | L S R | **Stop reaches everything**: a wired `stop` still fires on transport stop unless its dropdown says *Never*, so the dropdown stays beside a wired `stop` as a second source; Stop also fires with the transport already stopped. A **Panic** action (a transport-bar button, `Escape` twice, bindable) that resets every generator, releases every voice, and mutes every Audio Out for one block. The status bar shows which outputs are sounding. |
| **No view of the song.** | R P | An **Arrangement panel card**: lanes of song-locked Clips and MIDI Patterns drawn in time, draggable; a view onto nodes, in keeping with UX principle 1. Patterns with banks (below) show as blocks on the lane. |
| **No markers, punch range, or count-in.** | R | Markers in the tempo map section of settings, shown on the ruler and jumpable; a punch range the transport applies to armed Record nodes; a count-in setting (bars) that the metronome plays before recording starts. |
| **No swing.** | P S | A `swing` input (0–1) on Clock, shifting every other subdivision late, so Pattern, Euclidean, Step Sequencer, and Arpeggiator all swing from the clock. A `swing` on MIDI Pattern for drawn notes. |
| **No metronome in the transport.** | R P | A click button in the transport bar driving a built-in metronome on the monitor path; the Metronome block stays for custom clicks. |
| **Every Audio Out is summed into a render.** | R | A `role` config on Audio Out: *main* or *monitor*; monitor outputs play live and are left out of renders and File Outs. Fixes click, cue mixes, and headphone feeds. |
| **No MIDI recording.** | R P | A **MIDI Record** node: `midi` in, writes notes during a take into a MIDI Pattern it creates next to itself (as Create Clip from take does), quantize optional. |
| **No automation write.** | R | Write/touch modes: moving a knob while recording writes a Graph in beats wired into that input, song-locked. Bindings already follow hardware; this is the recording of it. |
| **No tempo from the patch.** | L | A `set_tempo` command path from a node (a **Tempo** node with `bpm` in and `tap` trigger), with the determinism caveat that it's an edit, not a signal. |

### B. New nodes

| Node | Helps | What it is | Study evidence |
| --- | --- | --- | --- |
| **Sample** | S L P | A file as a `Buffer` output (config `file`), so a sound can be wired into a Sampler, Granular, or Convolution from outside a block, and a shared pad or keys block takes its sound as an input. The smallest node with the biggest effect on blocks. | The keys body sets its files inside because nothing produces a Buffer from a file |
| **Note Split** (or *Drum Map*) | S P | `midi` in; config: a list of notes (or a range); per note *k*: `trigger_k`, `gate_k`, `velocity_k`. One node replaces MIDI Filter + MIDI Note + (often) Map per pad. Optional choke groups in config. | 6 nodes per pad → 2 |
| **Route** (trigger and event demux) | S L | `in` (trigger, MIDI, or audio by config), `index: Int` → `out_0..n`, the mirror of Select. Also a **Gate** mode: pass while `open: Bool`. | Velocity layers and round robin misuse four Switches each |
| **Zones** | S | `note`, `velocity` in → `zone: Int` and a per-zone `trigger`, from a list of key and velocity ranges (edited as a map in the Node tab). With Route and Sample, a keymapped instrument is Zones + Route + N Samplers. | 40 gestures for two zones |
| **Looper** | L S | `in: Audio`, `record`, `play`, `stop`, `overdub`, `undo` triggers, `length: Float(Beats)` (0 = free, first pass sets it), quantized to `quantize` beats; outputs `out`, `buffer`, `playing`, `recording`, `position`. Overdub mixes into the ring; undo drops the last layer. | 12 nodes for record-then-play; overdub impossible |
| **Slicer** | S | A `Buffer` in; slices by `count` (equal), by transients (`sensitivity`), or by markers set in a waveform editor; `trigger` + `slice: Int` plays one; outputs `out`, and per-slice `buffer_k` optional. | Eight Samplers with sixteen typed points |
| **Pattern banks and velocity** (change to Pattern) | P S | Pattern holds several grids (`bank: Int` input, banks added in the editor); cells carry a level painted by drag height (`velocity_k` output per row). MIDI Pattern gets `bank` too. | One Switch per channel per extra pattern; two nodes per velocity row |
| **Logic and Math with N inputs** | R P L | `inputs` config on Logic (and/or) and Math (add, multiply, min, max), like Mix. | Four OR gates for five solos |
| **Merge that grows** (editor change) | S L | Wiring into a Merge's last input adds one, the way select-then-add grows a Mix. Same for Mix. | Two Merges per slot |
| **Light** (display) | L S P | `in: Bool` (or `Float` level) shown as a lamp with a `color` config; pinnable. Buttons and Toggles gain a `lit: Bool` input so a pad shows the pattern hitting it. | Readouts showing 0 or 1 |
| **Waveform** (display) | S R L | Shows a `Buffer` or a Clip's file with the playhead, start/end and loop points as drag handles when wired to a Sampler (the node tab's editor for Clip and Sampler uses the same widget). | Points typed by ear |
| **Pad** (control) | S L | A Button with velocity (from where it's hit, or pressure where the device has it), aftertouch as `pressure`, and `lit`. A **Pads** grid control (rows × columns → `midi` or per-pad triggers) for a 4×4 in one node. | Eight Buttons with no velocity |
| **Fader** (control) | R P | A vertical Slider in dB with a meter beside it (`meter: Audio` in), so a strip's level and level reading are one card. | Two cards per channel for level and meter |
| **Velocity** | S P | Scales and curves MIDI velocity (`scale`, `curve`, `min`, `max`, `fixed`). | Map from velocity on every pad |
| **Humanize** | P | Timing and velocity jitter on triggers or MIDI, seeded. | Not built; every pattern study wanted it |
| **MIDI Monitor** (display) | S P | Shows the last events on a MIDI wire. | Debugging the pad studies was blind |
| **Tuner** (display) | R | Pitch Detect shown as a needle with the note name. | Not built; obvious for the guitar examples |

### C. Panels

| Gap | Helps | What's needed |
| --- | --- | --- |
| **Nothing says what belongs with what.** Cards flow in pin order into equal cells; a toggle takes a knob's cell, a meter spanning cells breaks the line, and the only thing relating a pan to its fader is its label prefix. The first thing anyone sees. | R P S L | **Strips**: a named vertical group of cards with one header, kept together as a column and moved, resized, and unpinned as one; a pinned block (next row) is a strip by default. **Section labels and row breaks** between strips (Tracks, Buses, Master). **Compact cards**: toggles and buttons at half height, two to a cell; a meter narrow beside a fader. A card keeps its place: pinning something new never reflows the others. |
| **A block's controls can't go on a panel.** Only top-level nodes with widgets pin, so every fader is a Knob node. | R P S L | **Pin a group or block**: its card shows a widget per unwired input (knob, toggle, dropdown by type and unit) and a meter or light per output; pinning a Track gives a strip. Inputs that are wired show the wire's value. Pinned group inputs are the group's `params`, already in the project. |
| **No grid.** A slot grid is tracks × scenes and a drum rack is rows, but a panel is a flow; no color; moving is one card at a time. | L P R | A **grid with named rows and columns**, a **color per card or strip**, multi-select drag. A panel can be **generated from a selection**: "Mixer from these Tracks", "Pads from these Samplers", "Grid from these Clips". |
| **The drawn editors never leave the Node tab.** | P S R | Pattern grids, Step Sequencer rows, piano rolls, Graph curves, and the waveform as **panel cards**, live and editable, sized by span. The whole pattern studio front becomes a panel. |
| **No state on controls.** | L S | `lit` inputs (above) and a playing/armed/stopped look for Clip and Sampler cards on panels. |
| **Panels in the plugin window and the web** already work; keep them in the test matrix as cards are added. | all | |

### D. Editor and workflow

| Gap | Helps | What's needed |
| --- | --- | --- |
| **Adding a unit of a tool is many additions.** A track, a pad, a slot, a channel is 6–10 nodes every time. | all | **Templates**: a few shipped and user-saved "add this set, wired, with its controls, pinned": Add ▸ Track (source → Track → the master Mix, knobs and toggles pinned to the Mixer panel), Add ▸ Drum Pad, Add ▸ Clip Slot, Add ▸ Send. A template is a block instance plus its pinned controls; with pinned blocks (C) most templates collapse to "add the block, pin it". |
| **Wiring a new source into the mix needs the Node tab.** | R P | Mix (and Merge) **grow when wired into their last input**; dropping a node on a Mix adds an input. |
| **Files are chosen in the Node tab.** | S R L | **Drop an audio file** on the canvas for a Clip (or Sampler with `Shift`), on a Pad or Sampler card to assign it, on a panel to make a pad; the Clip and Sampler cards show a small waveform. |
| **Record has no home.** | R | A Record node created next to an Audio In is wired and named after it; Create Clip from take places the clip at the bar the take started (song-locked, A) and wires it where the Record's monitor went. |
| **Editors one at a time.** | P | Larger editor windows already exist; allow several open at once, and dock one under the canvas. |
| **The transport bar is thin.** | R P L | Seek by click and drag, a ruler with bars, loop by drag, markers, a metronome button, count-in, tap tempo. Mostly A's work surfacing. |

### E. Model

| Gap | Helps | What's needed |
| --- | --- | --- |
| **Buffers don't route.** Select and Switch take audio, numbers, triggers, and MIDI, not `Buffer`. | S L | Buffer in Route, Select, and Switch; Sampler re-latches a changed buffer at the next start (it already does). |
| **A block instance can't differ in config from its siblings.** | S | Sidestepped by the Sample node (a sound becomes an input). Keep D82; revisit only if Sample isn't enough. |
| **A trigger input takes one wire.** | L S | Keep the rule (UX: an input takes one wire) and make Merge cheap: grows when wired, and the palette offers it when a second wire is dropped on a wired trigger input ("Merge here?"). |

## Priorities

Scored by how many of the four studies a change helps and how badly, then
by cost. The first group is what makes each tool *feel* right; the second
makes each one *complete*; the third is polish.

**First: the things every study hit.**

1. **Stop always stops, and a panic key** (A). A fix before a feature,
   and a small one: the launcher study traps its player with sound that
   nothing in the editor can end. Anything built from wired generators
   has the same hole.
2. **Panels with structure** (C): strips, section labels, compact cards,
   and blocks pinned as strips. Every study's mixer, rack, pad bank, and
   session grid is a flow of equal cards related only by their labels,
   and it was the first thing seen on opening them. Pinning a block as a
   strip also removes 4–6 nodes per unit in all four studies.
3. **Song-locked generators and a seekable transport** (A). Without it
   there is no recorder and no playlist. It is also the engine's biggest
   missing semantic, so it belongs early, before an arrangement view is
   drawn on top.
4. **Sample, Note Split, Route** (B). Three small nodes that cut the pad
   chain from 6 nodes to 2, make zones and round robins two nodes, and let
   blocks take their sounds as inputs.
5. **Light, `lit` on buttons, Pad with velocity, Fader** (B, C). Panels
   that show state and feel pressure are what make a pad bank or a session
   grid playable.
6. **Templates and growing Mix/Merge** (D). Add ▸ Track in one gesture is
   the direct answer to "twenty controls to record a track."

**Second: the thing each tool can't do without.**

7. **Looper** node (L).
8. **Pattern banks and velocity, swing on Clock** (P).
9. **Arrangement panel card, markers, punch, count-in, monitor outputs,
   MIDI recording** (R).
10. **Zones, Slicer, Waveform display, file drop** (S).
11. **Panel grid, colors, generated panels; editors as panel cards** (C).

**Third: polish.**

12. Logic and Math with N inputs; Velocity; Humanize; MIDI Monitor;
    Tuner; automation write; Tempo node; several editors at once.

## How to use this document

- `docs/ROADMAP.md` Phase 4 schedules the first two groups as milestones,
  in this order, for a human to reorder or cut.
- When a gap closes, rebuild the study that showed it the new way and
  shorten its canvas notes. The node count in `studies/README.md` and the
  gesture table here are the before; the diff is the after.
- A new kind of tool (a modular synth rack, a guitar rig, a radio
  automation desk, a live-coding surface) gets a new study before new
  nodes, so the nodes come from a measured need.
