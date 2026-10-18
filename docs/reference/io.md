# Input / Output

[← All nodes](README.md)

<a id="audio-in"></a>
## Audio In

Kind: `audio_in` · live source · Also found by: input, microphone, mic, line in, interface, device

Brings a device input channel, or a pair of them, into the patch.

The `channel` config picks the first device input (counting from 0)
and `channels` chooses mono (1) or a stereo pair (2). Inputs the
device doesn't have read as silence, so a project opened on a smaller
interface still runs.

Audio In is a live source: nodes that need their whole input before
they can play are disabled downstream of it.

**Try this:** wire `out` through a `Gain` into an `Audio Out` to
monitor a microphone, or into a `Record` node to capture takes.

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `out` | Audio | The device input. |

<a id="audio-out"></a>
## Audio Out

Kind: `audio_out` · Also found by: output, speakers, headphones, monitor, main, device

Sends audio to the device outputs.

The `channel` config picks the first device output (counting from 0)
and `channels` chooses mono (1) or a stereo pair (2, the default). A
mono signal wired into a stereo Audio Out plays on both channels.

Several Audio Out nodes can share outputs: their signals add up, so a
main mix and a click track can both reach the speakers. Outputs the
device doesn't have are skipped. Samples that aren't finite numbers
(the result of a runaway feedback loop or a division by zero) are
sent as silence rather than reaching the speakers.

The node's meter shows the peak level of its first two channels.

**Try this:** add a second Audio Out on outputs 3–4 (`channel` = 2)
for a headphone cue mix, or wire an `Oscillator` straight in at a low
`Gain` to check that sound comes out at all.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to play. |

<a id="file-out"></a>
## File Out

Kind: `file_out` · Also found by: export, bounce, stem, render, file, save, wav

Renders its input to a file of its own when the project is rendered
offline.

`path` names the file, relative to the folder the main render goes
to (`drums.wav`, `stems/bass.wav`); `.wav` is added if there's no
extension. The file covers the same span as the render, at the same
sample rate and bit depth. `channels` sets mono (1) or stereo (2, the
default); a mono signal into a stereo File Out is written to both.

One `audioc render` can produce many files this way: stems, a dry and
a wet version, a click track. While the patch plays live, File Out
does nothing; use `Record` to capture a performance.

**Try this:** put a File Out after each instrument's last effect,
named after the instrument, and render once to get a set of stems
that line up sample for sample with the full mix.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to write. |

<a id="midi-in"></a>
## MIDI In

Kind: `midi_in` · live source · Also found by: keyboard, controller, notes, device, input

Brings MIDI from the connected devices into the patch.

Every message keeps its timing within the block, so notes start on
the sample they arrived. `channel` filters by MIDI channel: 0 passes
all channels (omni), 1–16 passes only that one.

MIDI In is a live source: nodes that need their whole input before
they can play are disabled downstream of it.

**Try this:** wire `midi` into a `MIDI Note` and its `freq` into an
`Oscillator` for a one-voice synth; use two MIDI In nodes on
different channels to split a keyboard between two sounds.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `channel` | Int | 0 | 0 … 16 | MIDI channel to pass, 1–16; 0 passes all. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `midi` | Midi | Messages from the devices. |

<a id="midi-out"></a>
## MIDI Out

Kind: `midi_out` · Also found by: send, device, hardware, synth, output, controller, external

Sends MIDI to the connected devices.

Every message keeps its timing within the block. `channel` re-channels
everything: 0 keeps each message's own channel, 1–16 sends them all on
that one.

When the transport stops, the node ends every note it started and
hasn't ended yet, so a hardware synth isn't left droning. A `panic`
trigger does the same and also sends All Notes Off on all sixteen
channels, for notes something else left hanging.

The host chooses the device (`audioc run --midi-out`, or the Devices
tab in the editor). An offline render collects the messages instead
(`audioc render --midi-out song.mid`).

**Try this:** wire a `Pattern` or an `Arpeggiator` into a MIDI Out to
play a hardware synth in time with the patch, then route the synth's
audio back in through an `Audio In` to process it.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `midi` | Midi | — |  | Messages to send. |
| `channel` | Int | 0 | 0 … 16 | Send everything on this channel, 1–16; 0 keeps each message's channel. |
| `panic` | Trigger | — |  | Ends every note, on every channel. |

<a id="record"></a>
## Record

Kind: `record` · Also found by: recorder, capture, bounce, take, tape, file, save

Writes audio to a file, one take at a time.

A take runs while `arm` is on and the transport is recording, or from
a `start` trigger until a `stop` trigger, whichever keeps it going
longer. Each take is a new file; the host chooses where it goes and
names it after the node's label. Takes begin and end on block
boundaries (a few milliseconds).

`recording` is on while a take runs and `length` is the duration of
the current or last take. Where recording isn't available (some quick
renders) the node still reports its state but writes nothing. The
node's readout also shows how many samples were dropped because the
disk couldn't keep up.

The `channels` config sets how many channels are written (2 by
default); a mono signal into a stereo Record is written to both.

`buffer` holds the last finished take (up to ten minutes of it) as
soon as it is safely on disk, so a recording can feed a `Sampler`
right away. It is empty until the first take finishes, and where
recording isn't available.

**Try this:** wire an `Audio In` into `in` and press record on the
transport to capture a take in time with the song, or wire a
`Button`'s `trigger` and `release` into `start` and `stop` for a
hold-to-record pedal.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `in` | Audio | — |  | Signal to record. |
| `arm` | Bool | true |  | Record while the transport is recording. |
| `start` | Trigger | — |  | Starts a take, whatever the transport does. |
| `stop` | Trigger | — |  | Ends a take begun by `start`. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `recording` | Bool | True while a take runs. |
| `length` | Float (s) | Duration of the current or last take. |
| `buffer` | Buffer | The last finished take. |

