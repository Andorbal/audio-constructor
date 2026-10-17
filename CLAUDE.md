# Working agreements for agents

You are building Audio Constructor, a node-based audio construction suite.
Read, in this order: `README.md`, `docs/ROADMAP.md`, `docs/ARCHITECTURE.md`.
Open `docs/NODES.md` and `docs/UX.md` when working on nodes or the editor.
`docs/DECISIONS.md` lists settled choices; don't re-litigate them, extend
them.

## Where to start

Pick up the first unchecked milestone in `docs/ROADMAP.md`. Milestones are
ordered. A milestone is done when every box is checked, tests cover it, CI is
green, and the acceptance notes say what was verified and how.

## Workflow

- **Branch per milestone**: `phase-1/m1.3-compiler`, etc. Never commit to
  `main`. Never force-push a shared branch.
- **Commit small and often**, after every coherent step that builds and
  passes tests. Push at least every few commits. A long run with no pushes is
  a failure mode; a hundred small commits is not.
- **One merge request per milestone**, opened early as a draft so progress is
  visible, with a description that lists what's done and what's verified.
  Merging is a human decision; never merge your own MR.
- Commit messages: imperative subject, a short body saying what and why. End
  with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Check boxes in `docs/ROADMAP.md` in the same commit as the work.
- When you make a design choice the docs don't cover, add a row to
  `docs/DECISIONS.md`. When the docs are wrong, fix the docs in the same MR.

## Non-negotiables

1. **Audio-thread rules** (ARCHITECTURE §8): no allocation, locks, I/O,
   syscalls, or panics in `process()` or the scheduler. Debug builds assert
   this. `unwrap()`/`expect()` are not allowed on that path.
2. **wasm32 builds**: `ac-core`, `ac-dsp`, `ac-nodes`, `ac-engine` must pass
   `cargo check --target wasm32-unknown-unknown`. Don't add a dependency to
   those crates that breaks it; put device or file code in `ac-io`.
3. **Determinism**: same project, same rate, same block size → same samples.
   Randomness only through the seeded PRNG. Golden tests enforce this.
4. **Every node**: one file, doc comment with a "try this" tip, unit tests,
   sample-rate independent, follows the generator contract where applicable
   (NODES.md). New nodes must be primitives; composites are blocks.
5. **No panics on user input**: bad projects, missing files, type mismatches
   produce reports and placeholders, never crashes.
6. **`unsafe`** needs a `// SAFETY:` comment and a reason it can't be avoided.
7. **Dependencies**: prefer well-maintained crates with permissive licenses;
   `cargo deny` must pass. No GPL dependencies in the engine crates.

## Your environment

You can't hear audio or see the window. That's expected. Use:

- `cargo test` (golden renders, determinism, compiler, node unit tests).
- `cargo run -p ac-cli -- render examples/<name>.acproj -o /tmp/out.wav`
  and inspect the WAV with a test helper (peak, RMS, spectrum peaks) to check
  what a patch does.
- The null host for runtime tests; `egui_kittest` for editor tests.
- `cargo run -p ac-cli -- bench` to check performance; compare with the
  committed baseline.

Things only a human can verify (device latency, listening tests, feel) go in
the MR description under "Needs manual verification", with exact steps.

## Commands

```
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check -p ac-core -p ac-dsp -p ac-nodes -p ac-engine --target wasm32-unknown-unknown
cargo deny check
cargo run -p ac-cli -- nodes --docs docs/reference
cargo run -p ac-cli -- bench
cargo run -p ac-app
```

## Style

- Rust 2024 edition, stable toolchain pinned in `rust-toolchain.toml`.
- Small modules, clear names, no clever macros beyond `node!`.
- Comments explain why, not what. DSP code cites its source (paper, book,
  well-known design) in a comment.
- Public-facing text (README, docs, in-app help) never references other
  products by name.
