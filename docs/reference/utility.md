# Utility

[← All nodes](README.md)

<a id="comment"></a>
## Comment

Kind: `comment` · Also found by: note, text, label, sticky

A sticky note on the canvas, for explaining a patch to others or to
yourself later.

Its `text` setting holds the note. A Comment has no sockets and does
nothing to the sound.

To label a whole area of a patch, use a frame instead: select the
nodes and press `Ctrl+J`. Frames are a canvas feature rather than a
node, and they move their contents with them.

**Try this:** leave a Comment beside the part of a patch worth playing
with, saying what to try ("turn `cutoff` down slowly"), the way the
example projects do.

<a id="feedback"></a>
## Feedback

Kind: `feedback` · Also found by: loop, cycle, recursion

Sends a signal back to an earlier point in the patch. It is the only
way to make a loop: a wire path that leads back to where it started.

What comes out of `out` is what went into `in` one block earlier. The
delay is exactly one processing block (the engine's block size, shown
in the node's header), so **results depend on the block size**: a
feedback patch sounds slightly different at a different buffer size,
and offline renders match live playback only at the same block size.
For a loop with a delay you choose, put a `Delay` inside it; the loop
then takes the `Delay` time plus one block.

The `channels` setting (1 to 32, default 1) sets how many channels the
loop carries; a mono input is copied to every channel. `out` is silent
for the first block and after a transport restart. NaN or infinite
samples are stored as 0, so one bad value can't circulate forever.

**Try this:** build an echo from parts: `Mix` → `Delay` → `Gain`
(−6 dB) → `Feedback` → back into a second input of the `Mix`. Each
trip round the loop comes back 6 dB quieter. Keep the gain around a
loop below 0 dB, or it grows until it distorts.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to send back round the loop. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | What went into `in` one block ago. |

<a id="group"></a>
## Group

Kind: `group` · Also found by: subpatch, subgraph, block, container

A piece of patch packed into one node, with its own sockets.
Double-click it (or select it and press `Tab`) to go inside and edit
it; `Tab` again comes back out.

Select some nodes and press `Ctrl+G` to make a group: wires that
crossed the edge of the selection become the group's sockets. To add a
socket later, go inside, right-click an input, and choose *expose as a
group input*. `Ctrl+Alt+G` ungroups, putting the nodes back in place.

Each Group node is an instance of a definition, and every instance of
a definition shares it: change the inside of one and all of them
change. A project carries the definitions it uses, so it opens
anywhere. The blocks in the library are saved groups; open one up to
see how it works.

Groups cost nothing when playing: the engine flattens them into the
surrounding patch.

**Try this:** build an `Oscillator` → `Biquad` → `Gain` voice, group
it, and duplicate the group (`Shift+D`) a few times with different
`freq` values wired in. Tweak the filter inside one copy and hear
every copy follow.

<a id="info"></a>
## Info

Kind: `info` · live source · Also found by: sample rate, block size, cpu, load, xruns, dropouts, latency, stats

The engine's vital signs as signals: `sample_rate` (Hz), `block_size`
(the largest block, in frames), `cpu_load` (how much of each block's
time the patch used last block, 0 to 1 and beyond), `xruns` (dropouts
so far, from overruns and the audio device), and `latency` (the
patch's output latency in samples).

`cpu_load` and `xruns` depend on the machine, not the patch, so Info
counts as a live source: a whole-input node fed by them is disabled.
In offline renders they read 0.

**Try this:** wire `cpu_load` into a `Readout` and a `Meter` to watch
how hard a patch works while you build it, or feed `sample_rate`
through a `Math` node to make a filter cutoff that is always a
fixed fraction of Nyquist.

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `sample_rate` | Float (Hz) | The engine's sample rate. |
| `block_size` | Int | The largest block, in frames. |
| `cpu_load` | Float (×) | Last block's processing time over its budget. |
| `xruns` | Int | Dropouts so far. |
| `latency` | Int | Output latency in samples. |

<a id="latency"></a>
## Latency

Kind: `latency` · Also found by: pdc, delay compensation, samples, align, debug

Delays audio by a fixed number of samples and tells the engine so,
which makes it a handy way to see latency compensation at work.

Some nodes (FFT, Lookahead, PitchShift) delay what passes through
them and report it. Wherever paths with different delays meet, the
engine delays the faster inputs so everything lines up: a dry signal
mixed with an FFT-processed copy stays in time. Latency does the same
as those nodes, by exactly `samples` (config, up to 10 seconds' worth
at 96 kHz), with `channels` channels (config, 1 to 32, default 2).

`path_latency` reports how far behind the sources this point runs, in
samples: the slowest path into the node plus its own delay. That is
the figure the engine lines other paths up to where they meet. With
`samples` at 0, Latency is a pure probe: drop it anywhere to read the
latency at that point. The project's total output latency shows in
`audioc validate` and in the editor.

**Try this:** split an Oscillator into two paths, one through a
Latency of 480 samples, and mix them: they stay in phase instead of
comb-filtering, because the engine delays the direct path too.
`audioc validate` lists the delay it added to the Mix input.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to delay. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The input, `samples` later. |
| `path_latency` | Int | How far behind the sources this point runs, in samples. |

<a id="merge-channels"></a>
## Merge Channels

Kind: `merge_channels` · Also found by: stereo, mono, channels, join, combine

Combines mono signals into one multichannel signal.

`in_0` becomes the first channel (left, for stereo), `in_1` the
second, and so on. The `channels` setting (1 to 32, default 2) sets how
many inputs there are. An unwired input is a silent channel.

**Try this:** merge two `Oscillator`s tuned a few cents apart into a
stereo pair for a wide, moving sound. Or rebuild a stereo signal after
processing its sides separately with `Split Channels`.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in_0` | Audio | — |  | One channel. |
| `in_1` | Audio | — |  | One channel. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The merged signal. |

<a id="reroute"></a>
## Reroute

Kind: `reroute` · Also found by: dot, wire, route, tidy

A dot on a wire, for tidy wiring. Whatever goes in comes straight out,
of any type.

Add one by `Shift`+right-dragging across a wire, then drag the dot to
bend the wire around other nodes. A Reroute takes the type of the wire
connected to it. It costs nothing when playing: the engine removes
every Reroute and connects the wires straight through.

**Try this:** fan one signal out from a Reroute to several inputs, so
a single wire leaves the source node and the patch reads like a bus.
Or steer a long wire round the edge of a busy patch with two Reroutes
at the corners.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Any | 0 |  | Any signal. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Any | The same signal. |

<a id="split-channels"></a>
## Split Channels

Kind: `split_channels` · Also found by: stereo, mono, channels

Splits a multichannel signal into one mono output per channel.

`out_0` is the first channel (left, for stereo), `out_1` the second,
and so on. The `channels` setting (1 to 32, default 2) sets how many
outputs there are. A mono input is copied to every output.

**Try this:** split a stereo signal, give each side a different
`Delay` time, and join them again with `Merge Channels` for a wide
echo. Or split a stereo `AudioIn` to process a guitar on the left
input and a microphone on the right separately.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | The signal to split. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out_0` | Audio | One channel. |
| `out_1` | Audio | One channel. |

