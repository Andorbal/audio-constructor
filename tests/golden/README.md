# Golden renders

Every `*.acproj` here is rendered offline by `crates/ac-cli/tests/golden.rs`
and compared with its `*.expected.json`:

- **Reference platform (Linux x86-64):** the FNV-1a hash of every output
  sample must match bit for bit.
- **Other platforms:** peak and RMS per channel must match within a small
  tolerance (math library differences).
- **Determinism:** each project also runs through the null real-time host at
  block sizes 32 and 1024 and must equal the offline render at the same
  block size. Projects with `"block_independent": true` must also render the
  same at both block sizes (projects using `Feedback` can't, by design).

Test inputs live in a top-level `"golden"` object in the project (preserved
by the file format): `seconds`, `block_independent`, an `input` sine fed to
`AudioIn`, `midi` events (`[frame, [status, data1, data2]]`), and `controls`,
control-surface events as if from the computer keys, a gamepad, or OSC
(`[frame, control, value]`):

```json
"controls": [
  [0,    {"key": "A"}, 1],
  [4800, {"key": "A"}, 0],
  [960,  {"gamepad": 0, "button": "south"}, 1],
  [960,  {"gamepad": 0, "axis": "left_x"}, -0.5],
  [2400, {"osc": "/fader/1", "arg": 0}, 0.25]
]
```

Keys go by name (`A`, `7`, `Space`, `Up`, `F1`, …), gamepad buttons and
axes by their socket names (`south`, `left_bumper`, `dpad_up`, `left_x`,
`right_trigger_axis`, …; `gamepad` defaults to 0), and OSC by address
(`arg` defaults to 0). Events on the same frame play in the order listed.
What `MIDI Out` and `OSC Out` nodes send is part of the expectation.

Test MIDI and controls reach the project's bindings too, the way a real
controller would: `bindings-midi-cc` sweeps a bound frequency (log scale)
and level with MIDI controllers and fires a bound trigger with a note, and
`bindings-controls` moves bound inputs with a gamepad stick, a key, and an
OSC fader.

Every node category has at least one project here. After an intentional
change to a node's output, regenerate expectations and review the diff:

```
GOLDEN_UPDATE=1 cargo test -p ac-cli --test golden
```
