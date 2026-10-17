# Working agreements for agents

You are building Audio Constructor, a node-based audio construction suite.
Read, in this order: `README.md`, `docs/ROADMAP.md`, `docs/ARCHITECTURE.md`.
Open `docs/NODES.md` and `docs/UX.md` when working on nodes or the editor.
`docs/DECISIONS.md` lists settled choices; don't re-litigate them, extend
them.

## Where to start

Pick up the first unchecked milestone in `docs/ROADMAP.md`. Milestones are
ordered. A milestone is done when every box is checked, tests cover it, CI is
green, the acceptance notes say what was verified and how, and its
`CHANGELOG.md` section says what changed.

## Workflow

- **Branch per milestone**: `phase-1/m1.3-compiler`, etc. Never commit to
  `main`. Never force-push a shared branch.
- **Commit small and often**, after every coherent step that builds and
  passes tests. Push at least every few commits. A long run with no pushes is
  a failure mode; a hundred small commits is not.
- **One merge request per milestone**, opened early as a draft so progress is
  visible, with a description that lists what's done and what's verified.
  Merging is a human decision; never merge your own MR. Every merge request,
  milestone or not, is a release (below).
- Commit messages: imperative subject, a short body saying what and why. End
  with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Check boxes in `docs/ROADMAP.md` and add `CHANGELOG.md` entries in the
  same commit as the work.
- When you make a design choice the docs don't cover, add a row to
  `docs/DECISIONS.md`. When the docs are wrong, fix the docs in the same MR.

## Versions, changelog, and releases

Every merge request is a release, and merging it is what ships it. All
crates share the workspace version in `Cargo.toml`; `CHANGELOG.md` has a
section per version, newest first.

- **Bump in the branch's first commit**: `python3 scripts/release.py bump
  minor` (or `patch`). It sets the version, updates `Cargo.lock`, and opens
  a `## [X.Y.Z] - YYYY-MM-DD` section at the top of `CHANGELOG.md`. CI's
  *Version and changelog* job fails until the version is newer than
  `main`'s and the section has entries, so bump first and the draft stays
  green.
- **Which part**: before 1.0, `minor` for a milestone, new nodes or
  features, or a change to project files, the command line, or rendered
  output (a golden hash changes); `patch` for fixes, docs, CI, and internal
  changes. Never `major`: 1.0.0 is a human decision. Changed your mind?
  Run `bump` again; it renames the branch's section instead of adding one.
- **Write entries as you go**, under `### Added`, `### Changed`,
  `### Fixed`, or `### Removed` (`### Deprecated` and `### Security` when
  they apply). Write for someone using the editor or `audioc`: what
  changed and why it matters, not commit subjects, file names, or
  refactors nobody can see. A milestone's section can open with a sentence
  or two summing it up. The section becomes the release notes.
- **Before marking the MR ready**, run `bump` again with the same part to
  date the section today, then `python3 scripts/release.py check`.
- **If `main` takes your version first** (another MR merged), merge `main`,
  keep its sections exactly as they are, run `bump` again, and move your
  entries into the section it opens.
- **Releasing is automatic**: when CI passes on a `main` commit whose
  version has no tag, `.github/workflows/release.yml` tags it `vX.Y.Z`,
  builds Linux, macOS, and Windows archives, and publishes a release with
  the section as its notes. Never tag or publish releases by hand, and
  don't edit a released section except to correct it.

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
python3 scripts/release.py bump minor   # or patch; first commit on a branch
python3 scripts/release.py check        # version and changelog, as CI checks them
```

## Style

- Rust 2024 edition, stable toolchain pinned in `rust-toolchain.toml`.
- Small modules, clear names, no clever macros beyond `node!`.
- Comments explain why, not what. DSP code cites its source (paper, book,
  well-known design) in a comment.
- Public-facing text (README, docs, in-app help) never references other
  products by name.
