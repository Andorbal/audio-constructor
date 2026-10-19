# Shipped blocks

Each `.acblock` file here is a block that ships with Audio Constructor: a
group definition built from the node library (docs/NODES.md, "Blocks that
ship in `blocks/`"), with a name, description, version, and tags. The app
lists them in the Library tab under **Shipped**; adding one copies its
definition into the project, which remembers where it came from so the
editor can offer an update when a newer version ships.

- The file name is the block's id (`compressor.acblock` → `shipped:compressor`).
- Raise `version` whenever a change should be offered to projects that use
  the block.
- Every block has a golden project, `tests/golden/block-<id>.acproj`, that
  plays it; `cargo test -p ac-cli --test golden` checks both that the
  render matches and that the project's copy of the block is this file's.
- Every block's preview (what the Library tab's play button plays; see
  DECISIONS D190) is golden-tested too: `cargo test -p ac-cli --test
  previews` renders them all and compares them with
  `tests/golden/block-previews.json`. A change to a block changes its
  preview's hash; regenerate with `GOLDEN_UPDATE=1` and review the diff.
- Format: ARCHITECTURE §12. Files are canonical JSON (`audioc fmt` keeps
  them that way).
