# The command line

`audioc` does from a terminal what doesn't need the editor's window:
rendering projects to files, playing them live on a computer without a
screen, checking them, and managing blocks and packs. It's handy for
batch jobs, for a music box running a patch in a corner, and for scripts
that rebuild a set of renders whenever a project changes.

`audioc --help` lists the commands, and `audioc <command> --help` lists a
command's options. `audioc --version` prints the version.

If you built from source rather than downloading a release,
`cargo run --release -p ac-cli -- <command> …` runs it without installing
it, and `cargo install --path crates/ac-cli` installs it.

Times are given in seconds (`12.5`) or minutes and seconds (`1:30`).

## render

Render a project offline to a WAV file, as fast as the computer can.

```
audioc render my-song.acproj -o my-song.wav
audioc render my-song.acproj -o intro.wav --to 0:30
audioc render my-song.acproj -o hi-res.wav --sample-rate 96000 --bit-depth 32f
audioc render examples/tape-loop.acproj -o loop.wav --set 9.gain=-4 --set 11.value=3
audioc render examples/reverse-delay.acproj -o rd.wav --input take.wav --set c.value=true
```

| Option | Does |
| --- | --- |
| `-o`, `--output FILE` | The WAV file to write (required) |
| `--from TIME`, `--to TIME` | Render only this part (default: from 0 to the project's length) |
| `--sample-rate N` | Render at this rate (default: the project's) |
| `--block-size N` | Render in blocks of this many frames (default: the project's render block) |
| `--bit-depth D` | `16`, `24` (default), or `32f` |
| `--set NODE.SOCKET=VALUE` | Set an input for this render only; repeat for more |
| `--input FILE` | Play this WAV into Audio In nodes as if from the sound card |
| `--midi-out FILE` | Write what MIDI Out nodes send to a standard MIDI file |
| `-q`, `--quiet` | Print nothing but errors |

It prints the length, the speed (how many times faster than real time),
and each channel's peak and RMS level:

```
rendered 10.000 s × 2 ch at 48000 Hz (24) in 0.023 s (441× real time) → start.wav
  ch0: peak 0.1247 (-18.1 dBFS), rms 0.0722
  ch1: peak 0.1247 (-18.1 dBFS), rms 0.0722
```

File Out nodes write their own files next to the output file. For
`--set`, `--input`, and what plays in a render, see
[Recording and rendering](recording-and-rendering.md#from-the-command-line).

## run

Play a project live on the sound card, without the editor, until you
press `Ctrl+C` or a set time is up. Whole-input nodes bake in the
background, as in the editor.

```
audioc run examples/guitar-pedalboard.acproj
audioc run my-song.acproj --duration 3:00
audioc run my-patch.acproj --output-device "USB Audio" --buffer-size 128
audioc run my-patch.acproj --record
audioc run examples/start.acproj --null --duration 5
audioc run big-patch.acproj --threads 4
audioc run my-patch.acproj --osc-in 9000
```

| Option | Does |
| --- | --- |
| `--host NAME` | The audio system, such as `ALSA`, `JACK`, `CoreAudio`, `WASAPI`, or `ASIO` (see `audioc devices`) |
| `--output-device NAME`, `--input-device NAME` | The devices, by exact name or part of it; `--input-device none` for no input |
| `--sample-rate N` | The rate (default: the device's) |
| `--buffer-size N` | Frames per buffer |
| `--midi NAME` | MIDI inputs to open: `all` (default), `none`, or part of a port name |
| `--midi-out NAME` | Where MIDI Out nodes send: `none` (default), `all`, or part of a port name |
| `--duration TIME` | Stop after this long |
| `--no-play` | Don't start the transport (Audio In still passes through) |
| `--record` | Turn on transport recording: armed Record nodes write takes into the project's `assets/` folder |
| `--null` | Run without a sound card: silent input, output thrown away, in real time. For testing |
| `--threads N` | Spread the patch over N processor cores (default 1) |
| `--osc-in PORT` | Listen for OSC on this UDP port |
| `--no-gamepad` | Don't listen to gamepads (they're on by default and can be plugged in any time) |

Every second it prints the transport position, the audio load (per core
with `--threads`), dropouts (xruns), dropped messages, and the output
peak:

```
   1.0s  playing    pos 0:01.013  load   0.4%  xruns 0 (device 0)  dropped 0  peak -18.1/-18.1 dB
```

Bindings in the project work as they do in the editor.

## devices

List audio systems and their devices (with channels, rates, and buffer
sizes), MIDI inputs and outputs, connected gamepads, and how to send OSC
to this computer.

```
audioc devices
```

## latency

Measure the round-trip latency through a loopback cable: connect an
output to an input, turn the speakers down, and run it. It takes the same
device options as `run`.

```
audioc latency
audioc latency --output-device "USB Audio" --input-device "USB Audio" --buffer-size 64 --count 5
```

`--count N` measures N times.

## validate

Check a project without playing it: print every problem, the live
sources, which whole-input nodes are disabled or bake, and the latency the
patch adds (and which nodes add it).

```
audioc validate examples/reverse-delay.acproj
```

```
info [8]: needs a known input; disabled while live (fed by Guitar or mic)
examples/reverse-delay.acproj: 12 nodes, 12 steps, 7 audio buffers, 19 parameters
live sources: 7
disabled nodes (live): 8 (fed by Guitar or mic)
whole-input nodes baked when played: 1
latency: 0 samples (0.00 ms)
ok
```

Nodes are named by their ids (here, node `8`). It exits with status 2 if
the project has errors, which makes it useful in scripts.
`--sample-rate N` checks it at another rate.

## inspect

Print a project's settings, every node (with its id, label, settings, and
values), every wire, the groups, and the bindings.

```
audioc inspect examples/start.acproj
```

```
Start
  48000 Hz, 10 s, block 256, seed 0
  tempo: 120 BPM 4/4 from beat 0
nodes (8):
       2  Oscillator  freq=220 shape="saw"
       3  Gain  gain=-18
  …
wires (4):
  2:out → 3:in
  …
```

The first column is each node's id, the one `render --set` takes.

## nodes

List every node: its kind, name, and summary, by category.

```
audioc nodes
audioc nodes --json > nodes.json
audioc nodes --docs docs/reference
```

`--json` prints everything about each node (sockets, types, defaults,
ranges, and documentation) for other programs. `--docs DIR` writes the
[node reference](../reference/README.md) as Markdown pages.

## bench

Measure how fast this computer runs patches: renders a set of stress
projects (a thousand oscillators, long effect chains, a 16-voice synth,
FFTs, pitch shifters, diffusers) and prints how many times faster than
real time each runs, and which nodes take the time.

```
audioc bench
audioc bench --project my-song.acproj --seconds 30
audioc bench --threads 4
```

| Option | Does |
| --- | --- |
| `--project FILE` | Bench this project instead of the built-in ones |
| `--seconds N` | Seconds of audio per project (default 10) |
| `--block-size N` | Block size (default 128) |
| `--threads N` | Also run on N cores and report the speedup |
| `--repeat N` | Run each N times and keep the fastest |
| `--json` | Print the results as JSON |
| `--compare FILE`, `--threshold X` | Compare with earlier JSON results and fail on a slowdown larger than X (default 0.2, 20 %) |
| `--history FILE` | Append the results to a history file |

## blocks

List the block library: every shipped block with its id, version, name,
and tags.

```
audioc blocks
audioc blocks --library ~/.local/share/audioconstructor/blocks
audioc blocks --json
```

`--library DIR` also lists your own and imported blocks in that library
folder (where it is on each system is in
[Blocks and sharing](blocks.md#saving-your-own)).

## block-update

Update the copies of library blocks inside projects to the library's
newer versions, as the editor's **Update** buttons do.

```
audioc block-update song.acproj other-song.acproj --library ~/.local/share/audioconstructor/blocks
audioc block-update *.acproj --check
```

| Option | Does |
| --- | --- |
| `--library DIR` | Also use your own and imported blocks in this folder |
| `--check` | Only report what would change; exits with status 1 if anything is out of date |
| `--force` | Also refresh copies that differ from the library at the same version |

It prints a line per updated block (`shipped:compressor v1 → v2`), and
says when wires to sockets the new version doesn't have were removed.

## pack and unpack

Bundle a project and its audio files into one `.acpack` file to share,
and unpack one.

```
audioc pack my-song.acproj
audioc pack my-song.acproj -o ~/Desktop/my-song.acpack
audioc unpack my-song.acpack
audioc unpack my-song.acpack -o my-song-copy --force
```

`pack` writes `<name>.acpack` next to the project unless `-o` says where.
Audio files from outside the project's folder are copied into the pack;
missing ones are left out with a warning. `unpack` makes a new folder
next to the pack, named after it, unless `-o` names one; it won't
overwrite files already in that folder unless you add `--force`.

## fmt

Rewrite project and block files in their standard layout, so changes
show up cleanly in version control.

```
audioc fmt my-song.acproj blocks/*.acblock
audioc fmt --check *.acproj
```

`--check` only reports files that aren't in the standard layout, and
exits with status 1 if any aren't.

Next: [Examples tour](examples.md).
