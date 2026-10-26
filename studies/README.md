# Studies

Projects that try to rebuild the basics of familiar kinds of music software
from the nodes, blocks, panels, and bindings that exist today, to find out
what is missing. They are measurements, not examples: each one works (it
compiles, renders, and makes sound), and each one is also as awkward as
today's tools make it. The findings, and the list of nodes, panel
features, and editor changes they argue for, are in
[docs/GAPS.md](../docs/GAPS.md).

| Study | Stands in for | Nodes | What it builds |
| --- | --- | --- | --- |
| `multitrack-recorder.acproj` | A linear multitrack recorder | 52 | Five tracks, each a Track block pinned as a channel strip with a reverb send and a stem file; reverb and delay buses; a master with a compressor, a limiter, and a Fader; clips placed in the song at bars 1, 5, and 9, so it plays from any bar; a volume ride; a punch range, markers, and a count-in on the transport; a Mixer panel of strips under labels |
| `pattern-studio.acproj` | A pattern-based studio | 56 | Four drum channels from a 16-step grid, a chord channel and a bass channel from piano rolls, two patterns and a four-node playlist that alternates them, a velocity row for the hats, channel strips, sends, a Rack panel |
| `pads-and-keys.acproj` | A sampler and drum machine | 83 | Eight pads from MIDI, the computer keys, and buttons, with velocity; a snare with two velocity layers and round robin; a choke; a two-zone keymapped instrument in a Voices body; resampling into a pad; a Pads panel |
| `scenes.acproj` | A clip launcher and loop pedal | 89 | Three tracks with two clip slots each, launched on the bar and lit while they play, with exclusive slots, stop buttons, and two scenes; a bar-quantized looper; a Track block per track; a Session panel laid out as tracks by scenes |

Open one in the editor (`cargo run --release -p ac-app -- studies/scenes.acproj`)
and read the notes on its canvas; the frames mark each part. Render one
with `audioc render studies/<name>.acproj -o out.wav`. The audio files
come from `../examples/assets/`.

A test (`crates/ac-cli/tests/studies.rs`) keeps every study loading,
canonical, compiling without errors, making sound, and pinning and binding
real controls, and the editor's layout test keeps their cards from
covering one another, so the studies stay honest as the nodes change. When
a gap they show is closed, rebuild the study the new way and shorten its
notes; the diff is the proof.
