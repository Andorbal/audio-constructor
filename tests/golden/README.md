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
`AudioIn`, and `midi` events (`[frame, [status, data1, data2]]`).

Every node category has at least one project here. After an intentional
change to a node's output, regenerate expectations and review the diff:

```
GOLDEN_UPDATE=1 cargo test -p ac-cli --test golden
```
