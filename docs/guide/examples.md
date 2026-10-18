# Examples tour

The `examples/` folder holds working projects, each built to teach
something. Every one has notes on its canvas explaining each part and a
"Try this" note with changes to make. All of them play something when you
press `Space`, even without a microphone, MIDI keyboard, or gamepad
plugged in (Guitar Pedalboard needs an input to be heard), and all of them
render with `audioc render`.

Open one with **File ▸ Examples** (as an untitled copy), File ▸ Open, from a terminal
(`audio-constructor examples/vocoder.acproj`), or from **Used in** on a
node's Help page; see [Opening the examples](getting-started.md#opening-the-examples).
A good order for a first tour is the order below.

| Example | File | What it teaches |
| --- | --- | --- |
| [Start](#start) | `start.acproj` | The canvas, playing, and changing values |
| [Guitar Pedalboard](#guitar-pedalboard) | `guitar-pedalboard.acproj` | Live input through effects |
| [Subtractive Synth](#subtractive-synth) | `subtractive-synth.acproj` | A synth from oscillator, filter, and envelope |
| [Four Tracks](#four-tracks) | `four-tracks.acproj` | Clips, a shared Track group, and mixing |
| [Build a Keyboard](#build-a-keyboard) | `build-a-keyboard.acproj` | Playing a polyphonic synth from the computer keys or the screen |
| [Sidechain Pump](#sidechain-pump) | `sidechain-pump.acproj` | Sidechain ducking in time with the tempo |
| [Drum Machine](#drum-machine) | `drum-machine.acproj` | Patterns, synthesized drums, and Samplers |
| [Gamepad Jam](#gamepad-jam) | `gamepad-jam.acproj` | Playing drums and chords from a game controller |
| [Reverse Delay](#reverse-delay) | `reverse-delay.acproj` | Baked and disabled nodes |
| [Multiband Compressor](#multiband-compressor) | `multiband-compressor.acproj` | Compressing frequency bands separately |
| [Vocoder](#vocoder) | `vocoder.acproj` | A synth that talks, from filter banks |
| [Generative Patch](#generative-patch) | `generative-patch.acproj` | Euclidean rhythms and seeded random notes |
| [Tape Loop](#tape-loop) | `tape-loop.acproj` | A feedback loop with wow and flutter |
| [Guitar-to-Synth](#guitar-to-synth) | `guitar-to-synth.acproj` | Pitch and level tracking |

## Start

`examples/start.acproj` · opens when the editor starts

An Oscillator through a Gain, a Meter, and a Scope into Audio Out: the
smallest patch that makes a sound, with notes that walk you through
playing it, changing values, adding a node from the palette, splicing it
onto a wire, and grouping. A last note, "Now break it", invites you to
wire an input twice and to make a loop without a Feedback node, to see
how the editor handles mistakes.

**Try:** add a second Oscillator at 3 Hz and wire it into the first one's
`fm` for vibrato.

## Guitar Pedalboard

`examples/guitar-pedalboard.acproj`

A guitar (or any instrument or microphone) on input 1 of your audio
interface, through a tone filter and an echo, live. It shows Audio In, a
Biquad as a tone control, and a Delay. You don't need to press play: live
input always passes through. Pick your interface in the Devices tab with
the smallest buffer that doesn't crackle; 128 frames is a good start.

**Try:** set the echo's **Time in** to beats (Node tab, Settings) to lock the
repeats to the tempo, or add a second Biquad in peak mode for a mid
boost.

## Subtractive Synth

`examples/subtractive-synth.acproj`

The classic mono synth: oscillator → filter → amplifier, with one ADSR
opening both the filter and the amplifier through two Maps. It plays from
a MIDI keyboard, and a Clock plays a demo note every half beat while the
**Demo** toggle is on, so you hear it without one. It's the finished
version of the [first synth](building-sounds.md#a-first-synth) walkthrough.

**Try:** turn Demo off and play it from MIDI, or follow the Voices
node's own tip: group the voice, swap MIDI Note for a Voice node, and make
it polyphonic.

## Four Tracks

`examples/four-tracks.acproj`

Four looping clips, each through a Track group (gain, pan, and a mute
switch), summed by a Mix. It shows Clip nodes, a group used four times,
and how editing the inside of one copy changes all of them.

**Try:** `Tab` into one Track, change it, and watch every track follow;
swap a clip's file in the Node tab.

## Build a Keyboard

`examples/build-a-keyboard.acproj`

One polyphonic synth, the Subtractive Synth Voice block in a Voices node,
with three ways to play it: a demo phrase from a MIDI Pattern, the
computer keyboard through a Keyboard In node (turn on Play keys with
`Ctrl+K`), or the on-screen Keyboard. It shows Keyboard In, Voices,
choosing between MIDI sources, and a panel for playing without the wires
in view.

**Try:** pick the computer keys as the source (the canvas notes say
how), turn on Play keys, and hold several letter keys for chords; then
press `Ctrl+Tab` for the panel and `F11` to play it full screen.

## Sidechain Pump

`examples/sidechain-pump.acproj`

A pad that ducks under a four-on-the-floor kick, the dance-music pump. A
MIDI Pattern plays chords on eight voices of the Subtractive Synth Voice
block, a Euclidean Drummer plays the kick, and the Sidechain Ducker turns
the pad down on every kick. The release is set in beats and converted to
seconds, so the pump follows the tempo.

**Try:** click Pump to compare with and without; wire the hats and snare
into the ducker's sidechain instead of the kick.

## Drum Machine

`examples/drum-machine.acproj`

The Drum Machine block playing a sixteen-step pattern in time with the
transport, and its per-drum triggers playing a second kit of Samplers cut
from one audio file. A switch flips between the two kits. It shows the
Pattern node, Samplers with start and end points, and blocks that send
triggers out for your own sounds.

**Try:** `Tab` into the Drum Machine and click the Pattern's cells in the
Node tab to change the beat; put a Probability between a trigger and its
Sampler so some hits drop out.

## Gamepad Jam

`examples/gamepad-jam.acproj`

Drums and chords played from a game controller. A demo plays the same
parts until you take over, so it makes sound before a controller is
plugged in. It shows the Gamepad node, bindings from the sticks and
triggers to knobs in the patch, a transport button on the controller, and
a panel to play from.

**Try:** plug in a controller (the Devices tab lists it), take over from
the demo, and follow the canvas notes; then bind another stick to a knob
yourself with right-click ▸ Bind….

## Reverse Delay

`examples/reverse-delay.acproj`

Echoes that play backwards: a Reverse cuts its input into half-second
slices and plays each backwards, and a Delay of the same length lines
each reversed slice up after the one it came from. The patch is built
twice: on a Clip, where Reverse **bakes** and plays, and on Audio In,
where it is **disabled** while live. It's the example for
[Live and baked](live-and-baked.md).

**Try:** turn on Use Audio In, play, and watch the lower Reverse stay
disabled; set the Reverse's **Slice** (Settings) and the Delay's `time` to 1 s
together for longer phrases.

## Multiband Compressor

`examples/multiband-compressor.acproj`

A four-track loop through the Multiband Compressor block, which splits the
mix at 200 Hz and 2 kHz and compresses each band on its own. Meters show
the level before and after and how hard each band is turned down, and a
switch bypasses it.

**Try:** pull `threshold_low` down to −36 dB and hear the bass tighten
while the cymbals stay lively; `Tab` into the block to see the Band Split
and three Compressors inside.

## Vocoder

`examples/vocoder.acproj`

A saw chord made to talk by the Vocoder block, which splits a carrier
(the chord) and a modulator (a voice) into twelve matching bands. The
voice here is a little robot built from a buzz and two moving formant
filters; switch on **Use Audio In** to vocode your own voice from a
microphone.

**Try:** shorten the Vocoder's `release` to 0.01 s for crisper words, or
raise the syllable Clock's interval for a slow, singing robot.

## Generative Patch

`examples/generative-patch.acproj`

Music that writes itself, the same way every time. A Clock drives two
Euclidean rhythms; on each hit, a Random node picks a pitch, Scale
Quantize snaps it to A minor pentatonic, and Note To MIDI plays it on a
Karplus-Strong String melody and an FM Operator Pair bass. A Limiter
keeps the peaks in check. The random choices come from the project's
seed.

**Try:** change the seed in the Project tab for a new tune with the same
rules; try another scale on both Scale Quantize nodes.

## Tape Loop

`examples/tape-loop.acproj`

A tape loop built from parts: a Delay is the tape between the heads, a
Shaper saturates it, two filters wear it down, and a Gain and a Feedback
node record it back over itself. Two LFOs wobble the delay time for wow
and flutter. Each phrase comes round again darker and more worn until it
fades.

**Try:** raise Feedback amount towards −4 dB for a loop that lasts much
longer; set Loop length to 3 s so phrases land somewhere new each time.

## Guitar-to-Synth

`examples/guitar-to-synth.acproj`

A synth that follows a guitar: Pitch Detect hears which note you play and
tunes two oscillators to it, and an Envelope Follower hears how hard you
play and sets the synth's volume and brightness. A recorded riff stands
in until you switch on **Guitar in**. Play single notes: Pitch Detect
follows one note at a time.

**Try:** lengthen Glide to 0.1 s for portamento between notes; wire Pitch
Detect's `onset` into an ADSR for a fresh attack on every note.

Next: [Troubleshooting](troubleshooting.md).
