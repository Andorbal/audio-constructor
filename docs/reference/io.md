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

