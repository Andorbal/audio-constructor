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

<a id="gamepad"></a>
## Gamepad

Kind: `gamepad` · live source · Also found by: controller, joystick, joypad, game, stick, buttons, device, input

Brings a game controller into the patch: every button as a gate and
a press trigger, the sticks and analog triggers as numbers.

The `pad` config picks the controller: 1 is the first one connected,
2 the second, and so on up to 16. A controller that isn't connected
reads as at rest: buttons up, sticks centered, triggers released.

Buttons are named by their place on the standard layout: `south`,
`east`, `west`, and `north` are the four face buttons (bottom, right,
left, top), then come the bumpers, the triggers pulled past halfway,
`select`, `start`, `mode` (the center button), the stick presses,
and the D-pad. Each has a gate that is high while it's held and a
`_press` trigger; turn on the `releases` config to add a `_release`
trigger for each as well. An analog button counts as held from
halfway down until it's back under 40 %, so one resting near the
middle doesn't chatter.

The sticks (`left_x`, `left_y`, `right_x`, `right_y`) run from −1
to 1, right and up positive; the analog triggers
(`left_trigger_axis`, `right_trigger_axis`) from 0 to 1.
`dead_zone` ignores small movements around rest and rescales the
rest of the travel, so a stick pushed all the way still reaches 1.
For a stick it's measured from the center with both axes together,
so a diagonal push behaves like a straight one. `curve` bends the
response: 1 is straight, 2 gives finer control near rest, 0.5 makes
small movements count for more.

Every change lands on the sample it arrived on, and values hold
until the controller moves again. Restarting the transport doesn't
forget what the controller is doing: a button held through a
restart stays held.

Gamepad is a live source: nodes that need their whole input before
they can play are disabled downstream of it.

**Try this:** wire `left_x` through a `Map` into a filter's `cutoff`
and `south` into an `ADSR`'s `gate` for a synth you play from the
couch, or wire `right_trigger_axis` into a `Crossfade`'s `mix` to
pull in an effect.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `dead_zone` | Float | 0.1 | 0 … 1 | Movement around rest to ignore, 0–1; full travel still reaches 1. |
| `curve` | Float | 1 | 0.25 … 4 | Response curve: 1 straight, above 1 finer near rest, below 1 touchier. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `left_x` | Float | Left stick, −1 (left) to 1 (right). |
| `left_y` | Float | Left stick, −1 (down) to 1 (up). |
| `right_x` | Float | Right stick, −1 (left) to 1 (right). |
| `right_y` | Float | Right stick, −1 (down) to 1 (up). |
| `left_trigger_axis` | Float | Left analog trigger, 0 (released) to 1 (pulled). |
| `right_trigger_axis` | Float | Right analog trigger, 0 (released) to 1 (pulled). |
| `south` | Bool | Bottom face button (often A), high while held. |
| `south_press` | Trigger | Fires when the button goes down. |
| `east` | Bool | Right face button (often B), high while held. |
| `east_press` | Trigger | Fires when the button goes down. |
| `west` | Bool | Left face button (often X), high while held. |
| `west_press` | Trigger | Fires when the button goes down. |
| `north` | Bool | Top face button (often Y), high while held. |
| `north_press` | Trigger | Fires when the button goes down. |
| `left_bumper` | Bool | Left bumper, high while held. |
| `left_bumper_press` | Trigger | Fires when the button goes down. |
| `right_bumper` | Bool | Right bumper, high while held. |
| `right_bumper_press` | Trigger | Fires when the button goes down. |
| `left_trigger` | Bool | Left trigger, high while pulled past halfway. |
| `left_trigger_press` | Trigger | Fires when the button goes down. |
| `right_trigger` | Bool | Right trigger, high while pulled past halfway. |
| `right_trigger_press` | Trigger | Fires when the button goes down. |
| `select` | Bool | Select (back), high while held. |
| `select_press` | Trigger | Fires when the button goes down. |
| `start` | Bool | Start, high while held. |
| `start_press` | Trigger | Fires when the button goes down. |
| `mode` | Bool | The center button, high while held. |
| `mode_press` | Trigger | Fires when the button goes down. |
| `left_stick` | Bool | Left stick pressed in, high while held. |
| `left_stick_press` | Trigger | Fires when the button goes down. |
| `right_stick` | Bool | Right stick pressed in, high while held. |
| `right_stick_press` | Trigger | Fires when the button goes down. |
| `dpad_up` | Bool | D-pad up, high while held. |
| `dpad_up_press` | Trigger | Fires when the button goes down. |
| `dpad_down` | Bool | D-pad down, high while held. |
| `dpad_down_press` | Trigger | Fires when the button goes down. |
| `dpad_left` | Bool | D-pad left, high while held. |
| `dpad_left_press` | Trigger | Fires when the button goes down. |
| `dpad_right` | Bool | D-pad right, high while held. |
| `dpad_right_press` | Trigger | Fires when the button goes down. |

<a id="keyboard-in"></a>
## Keyboard In

Kind: `keyboard_in` · live source · Also found by: computer keyboard, keys, typing, qwerty, hotkey, notes, midi, play, input

Brings the computer keyboard into the patch: chosen keys as gates
and press triggers, and a row of letter keys played like a piano.

The `keys` config lists the keys to bring in by name, separated by
commas (default `1, 2, 3, 4`). Each gets a `key_<name>` output that
is high while it's held and a `key_<name>_press` trigger, so `Space`
becomes `key_space` and `key_space_press`. Keys go by their names:
letters, digits, `Space`, `Enter`, `Tab`, `Backspace`, `Escape`,
`Up`, `Down`, `Left`, `Right`, `Insert`, `Delete`, `Home`, `End`,
`PageUp`, `PageDown`, `F1`–`F12`, and `Minus`, `Equals`,
`LeftBracket`, `RightBracket`, `Backslash`, `Semicolon`, `Quote`,
`Backtick`, `Comma`, `Period`, `Slash`. Unknown names are skipped,
and a node takes up to 32 keys.

The `midi` output plays the middle row of letters like a piano:
`A S D F G H J K L ; '` are white keys from C up, and
`W E T Y U O P` the black keys between them. `Z` and `X` shift the
row down and up an octave, up to three each way. `A` plays MIDI note
`low` (config, 0–127, default 60, middle C), on channel `channel`
(1–16, default 1), with the `velocity` of the moment it's pressed. A
key keeps the note it started, so shifting octaves while holding a
key still releases the right note. Two keys that land on one note
sound it once, until both are up, and notes outside MIDI's range
don't sound. A key listed in `keys` that is also on the piano row
(or is `Z` or `X`) does both jobs.

Every press and release lands on the sample it arrived on, and a
key that repeats while held doesn't strike again. Restarting the
transport releases every note; keys still held sound again from
there, and the octave shift stays where it was.

Keyboard In is a live source: nodes that need their whole input
before they can play are disabled downstream of it.

**Try this:** wire `midi` into a `Voices` synth and play it from
the laptop keyboard, or wire `key_1_press` into a `Sampler`'s
`start` to fire a sample with the 1 key.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `velocity` | Float | 0.8 | 0 … 1 | How hard the piano row plays, 0–1. |

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `midi` | Midi | Notes from the piano row. |
| `key_1` | Bool | High while the key is held. |
| `key_1_press` | Trigger | Fires when the key goes down. |
| `key_2` | Bool | High while the key is held. |
| `key_2_press` | Trigger | Fires when the key goes down. |
| `key_3` | Bool | High while the key is held. |
| `key_3_press` | Trigger | Fires when the key goes down. |
| `key_4` | Bool | High while the key is held. |
| `key_4_press` | Trigger | Fires when the key goes down. |

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

<a id="osc-in"></a>
## OSC In

Kind: `osc_in` · live source · Also found by: open sound control, network, phone, tablet, touch, remote, control surface, wireless, input

Receives Open Sound Control messages, such as the faders and buttons
of a phone or tablet control surface, as numbers and a trigger.

The `address` config is the address to listen to, exactly as the
sender writes it (default `/fader/1`; patterns with wildcards aren't
matched). `args` (1–4, default 1) is how many of each message's
arguments to bring out: the first comes out of `value`, the others
out of `value_2` to `value_4`. Each holds the last number received
(true and false arrive as 1 and 0, and anything that isn't a finite
number is skipped), starting at 0. `received` fires on the sample
each message arrives on. A message without arguments sets `value`
to 1, so a plain button works as a trigger; a button that sends 1
and 0 works as a gate on `value`.

An address that isn't valid (it must start with `/`, contain no
spaces or any of `* ? [ ] { } # ,`, and be at most 64 characters)
receives nothing, and the node stays at 0. The host decides which
network port to listen on. Restarting the transport keeps the last
values, the way the control surface still shows them.

OSC In is a live source: nodes that need their whole input before
they can play are disabled downstream of it.

**Try this:** set `address` to a fader on your phone and wire
`value` through a `Map` into a filter's `cutoff`, or wire a button's
`received` into a `Clip`'s `start` to fire a sample from across the
room.

**Outputs**

| Socket | Type | Description |
| --- | --- | --- |
| `value` | Float | The message's first argument, held until the next. |
| `received` | Trigger | Fires on each message. |

<a id="osc-out"></a>
## OSC Out

Kind: `osc_out` · Also found by: open sound control, network, send, phone, tablet, remote, control surface, wireless, output

Sends numbers from the patch as Open Sound Control messages, to move
the faders of a phone or tablet control surface or to drive
anything else on the network that speaks OSC.

`host` and `port` (config) say where to send: an IP address such as
`192.168.1.20`, or `localhost` (default `127.0.0.1`, port 9000).
Names other than `localhost` aren't looked up. `address` is the OSC
address (default `/fader/1`), and `args` (1–4, default 1) how many
numbers each message carries, from `value` and then `value_2` to
`value_4`.

A message goes out on every `send` trigger, and, while `on_change`
is on (the default), whenever the values change, at most once every
`interval` seconds (default 0.02, so a moving signal sends 50
messages a second; never more often than every millisecond). The
latest values always go out once the interval has passed, so the far
end ends up in sync. With `on_change` on, the current values are
also sent when the patch starts and when the transport restarts from
zero. Values that aren't numbers are sent as 0.

A config it can't send to (an invalid address, host, or port) sends
nothing. The host passes the messages to the network.

**Try this:** wire an `LFO` into `value` and watch a fader on your
phone follow it, or wire a `Clock` into `send` and a `Transport`'s
`bar` into `value` to show the bar number on a remote display.

**Inputs**

| Socket | Type | Default | Range / choices | Description |
| --- | --- | --- | --- | --- |
| `value` | Float | 0 |  | The message's first number. |
| `send` | Trigger | — |  | Sends the values now. |
| `on_change` | Bool | true |  | Also send whenever the values change, at most once per interval. |
| `interval` | Float (s) | 0.02 | 0.001 … 1 | Shortest time between messages sent for changes. |

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

