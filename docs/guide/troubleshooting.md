# Troubleshooting

Problems and what to check, roughly in the order to check it. The
editor's status line and the Problems tab often say what's wrong already;
look there first.

## No sound

1. **Is the transport playing?** Oscillators, Clips, and other generators
   start when you press `Space` (unless their `start` is set to On load).
   Live input (Audio In) passes through without pressing play.
2. **Is there a sound card?** The right end of the status line names the
   device in use. "no device (null)" means none could be opened; the
   Devices tab says why. Pick a device there and click **Apply**.
3. **Is it the right output?** Check **Output** in the Devices tab. An
   Audio Out node's `channel` setting picks the first output it plays on;
   outputs your device doesn't have are skipped.
4. **Does anything reach Audio Out?** Its meter, and the master meter in
   the transport bar, show the level. Follow the chain back with Meter
   nodes, or hover the sockets: an audio socket's tooltip says "silent" or
   gives its peak level.
5. **Is something turned all the way down or muted?** A Gain at −60 dB or
   below is silent. A muted node (crossed-out speaker badge) is bypassed.
6. **Is a node in trouble?** A red badge in a node's title, or a number on
   the Problems tab, means part of the patch can't run as wired. Click the
   problem to jump to the node.
7. **Is a whole-input node disabled?** A grey crossed circle on Reverse or
   Normalize means a live source feeds it, and it plays silence while live
   (see [below](#a-node-says-disabled)).
8. **Is a file missing?** A Clip with a crossed-out file badge plays
   silence (see [Missing files](#missing-files)).
9. **Did a feedback loop run away?** When a loop's gain is too high, its
   level climbs until the numbers stop being numbers, and Audio Out sends
   silence instead of passing them on. Lower the gain inside the loop.

## Crackles, dropouts, and xruns

An **xrun** is a buffer the computer didn't finish in time, heard as a
click or a gap. The count is in the transport bar (the lightning icon)
and in the Devices tab, which also shows the device's own underruns and
overruns.

- **Raise the buffer size** in the Devices tab (Buffer, then Apply): 256
  or 512 frames gives the computer more time per buffer. See
  [Buffer size and latency](getting-started.md#buffer-size-and-latency).
- **Find what's heavy.** The transport bar shows the audio load; above
  about 80 % dropouts become likely. Turn on node timing (`Ctrl+T`) and
  look for the hottest chips; see [High CPU](#high-cpu).
- **"real-time scheduling was denied"** in the Devices tab means the
  system wouldn't let the audio thread run ahead of everything else. The
  editor still works, but dropouts are more likely when the computer is
  busy. On Linux with ALSA, your user needs permission to use real-time
  priority: on most distributions, joining the `audio` group grants it
  (`sudo usermod -aG audio $USER`, then log out and back in). JACK and
  PipeWire handle this themselves.
- **Close other programs** that use the processor or the sound card
  heavily, and on laptops, plug in the power.
- **Two devices** (separate input and output interfaces) drift apart over
  time, which shows up as device underruns or overruns. One interface
  for both is steadier.

## Too much latency

- **Lower the buffer size** until it crackles, then go one step back up.
  At 48 kHz, 128 frames is 2.7 ms each way and 64 frames 1.3 ms.
- **Measure** the real round trip with a loopback cable and **Measure
  latency** (or `audioc latency`).
- **Check what the patch adds.** The Project tab's **Output latency**
  (and `audioc validate`) shows the delay nodes in the patch add, such as
  FFTs and pitch shifters. Keep those out of the path you play through
  live, or make them smaller; see [Latency](live-and-baked.md#latency).
- **On Windows**, an interface's ASIO driver may give lower latency than
  WASAPI; that needs an [ASIO build](getting-started.md#asio-windows).

## MIDI not seen

- The Devices tab's **MIDI inputs** lists what's connected. New devices
  should appear within a couple of seconds; click **Rescan** if not.
- On Windows, a MIDI port can be used by one program at a time. Close
  other music software that might have it open.
- On Linux, "MIDI support could not be initialized" means the system's
  MIDI sequencer isn't available. Loading it usually fixes that:
  `sudo modprobe snd-seq`.
- If the device is listed but nothing happens, check the patch: is there
  a MIDI In node, and is its `channel` 0 (all channels) or the channel
  your keyboard sends on? Hover MIDI In's `midi` output while you play: it
  counts the events.
- MIDI Out sends nowhere until you pick a port under **MIDI outputs**.

## Gamepad not seen on Linux

Gamepads are read from the system's input devices
(`/dev/input/event…`), through udev.

- Check the Devices tab's **Gamepads** section: it lists connected
  controllers, or says why gamepads aren't available.
- Most desktop distributions let the person logged in at the screen read
  game controllers. If yours doesn't, check that your user can read the
  device files (`ls -l /dev/input/event*`). Adding yourself to the
  `input` group is a common fix: `sudo usermod -aG input $USER`, then log
  out and back in.
- Programs started inside a sandbox or a container may not see input
  devices at all.
- `audioc run` listens to gamepads too (unless `--no-gamepad`) and prints
  when one connects.

Remember that a Gamepad node's `pad` setting counts from 1, the number the
Devices tab shows.

## OSC messages don't arrive

- Listening is off until you tick **Listen on UDP port** in the Devices
  tab, each time the editor starts (or `audioc run --osc-in 9000`).
- The sending app must use the address and port the Devices tab shows,
  and both devices must be on the same network. A firewall on the
  computer may block the port.
- The Devices tab counts messages that arrive ("12 messages"), and how
  many weren't OSC at all. If the count goes up but nothing happens, the
  OSC In node's `address` doesn't match exactly: addresses are compared
  letter for letter, with no wildcards.

## A node says "disabled"

A grey crossed circle on a Reverse or Normalize node, and an entry in the
Problems tab, mean a live source (Audio In, MIDI In, Gamepad, Keyboard
In, OSC In, or Info) feeds it. These nodes need their whole input before
they can play, which a live source can't give. It isn't an error: the
node plays silence while live and works in offline renders. To hear it in
the editor, record the live part first and feed the node from a Clip.
[Live and baked](live-and-baked.md) explains it in full.

A clock badge means it's still **baking**; give it a moment. A long
project takes longer to bake.

## Missing files

When a project opens and an audio file it uses isn't where it should be,
the status line says how many files have problems. Then:

- the Clip (or Sampler) shows a crossed-out file badge and plays silence;
- the Project tab's **Audio files** list marks the file; click
  **Locate…** next to it and pick the file where it is now. Every node
  using it is relinked.

A warning sign instead means the file is there but has changed since it
was added to the project; it plays as it is now.

To avoid missing files: save projects (saving copies outside audio files
into the project's `assets/` folder), and share them as
[packs](blocks.md#sharing-a-project-packs).

A node with a **question mark** badge is a kind of node this version
doesn't know, probably from a newer version. It's kept, with its settings
and wires, and does nothing; a version that knows it plays it again.

## High CPU

- The transport bar shows the audio load, and the Devices tab the load per
  core.
- **Node timing** (`Ctrl+T`) puts each node's cost in its title, colored
  from cool to hot; groups and Voices nodes show their total. The Node tab
  shows the selected node's share of the budget.
- Usual suspects: many voices (lower `voices` on a Voices node), large FFT
  sizes, Pitch Shift, Diffuser, and reverbs. LFOs are much cheaper than
  Oscillators for slow modulation.
- **Engine threads** in the Devices tab (or `audioc run --threads N`)
  spreads independent parts of the patch (voices, parallel chains) over
  several cores. It keeps those cores busy while playing, so leave it at 1
  unless a patch needs it.
- `audioc bench --project my-song.acproj` measures how much faster than
  real time a project renders on this computer, and which nodes take the
  time.

## Keys do nothing, or shortcuts don't work

- **Keys don't reach the patch:** turn on Play keys (`Ctrl+K` or the
  keyboard button in the transport bar). A Keyboard In node only hears
  keys while it's on.
- **Shortcuts stopped working:** Play keys may be on (the status line says
  so); `Ctrl+K` turns it off. While the panels show (`Ctrl+Tab`),
  shortcuts that change the patch are off. While you type in a text
  field, keys go to the field.
- **`Space` doesn't play in performance mode:** Play keys is on there, so
  `Space` goes to the patch. Use the play button.

## The editor window doesn't open

On Linux, the editor needs a desktop session (X11 or Wayland) and a
graphics driver with Vulkan or OpenGL. `audioc` doesn't: on a computer
without a screen, use `audioc run` to play projects and `audioc render`
to render them.

## A render doesn't match what I heard

Renders and live playing give the same samples at the same sample rate
and block size. The editor plays at the sound card's rate, while renders
use the project's **Sample rate** and **Render block** (Project tab); if
they differ, the render sounds the same but isn't identical sample for
sample. Live input (Audio In, MIDI In, controllers) isn't in a render
unless you pass `--input` for Audio In; see
[What plays in a render](recording-and-rendering.md#what-plays-in-a-render).

## Still stuck

- Press `F1` on the node in question: its Help page explains every socket
  and setting.
- `audioc validate my.acproj` prints every problem in a project, and
  `audioc inspect` prints everything in it.
- [Report a problem](https://github.com/Andorbal/audio-constructor/issues)
  with the project file (or a pack) and what you expected to hear.
