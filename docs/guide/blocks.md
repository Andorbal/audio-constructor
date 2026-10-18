# Blocks and sharing

A **block** is a group saved for reuse: a compressor, a reverb, a synth
voice, a drum machine. Audio Constructor ships with 23 of them, all built
from ordinary nodes, and you can save your own, share them, and pick up
improved versions later. This chapter also covers **packs**, for sharing
a whole project with its audio files.

## Using blocks

Blocks are in the add palette (`Shift+A`) and the side panel's
**Library** tab, next to the nodes. In the Library tab's **Blocks**
section:

- **All**, **Shipped**, **Mine**, and **Imported** filter by where blocks
  came from, with a count for each;
- the **#tag** chips show only blocks with that tag (click again for all);
- the search field at the top searches names, tags, and descriptions;
- hover a block for its description, version, and author;
- drag a block onto the canvas, or click it to add it in the middle of
  the view; right-click it for **Add to canvas** and **Remove from
  library**.

A block on the canvas is a group: press `Tab` to look inside, see how it's
built, and change it. Instrument blocks (the ones with a Voice node
inside) come in as [Voices](building-sounds.md#polyphony-with-voices)
nodes, ready to play chords.

When you add a block, the project gets its own copy of the block's
definition. The project then opens anywhere, even on a computer whose
library doesn't have the block, and changing your library never changes a
project behind your back. Every copy of the block within a project shares
that one definition: change the inside of one and all of them follow.

### The shipped blocks

| Kind | Blocks |
| --- | --- |
| Dynamics | **Compressor**, **Limiter**, **Noise Gate**, **Sidechain Ducker**, **Multiband Compressor** |
| Space | **Plate Reverb**, **Hall Reverb** |
| Modulation | **Chorus**, **Flanger**, **Phaser**, **Tremolo**, **Vibrato**, **Auto-Wah** |
| Instruments | **Subtractive Synth Voice**, **Karplus-Strong String**, **FM Operator Pair** |
| Rhythm | **Drum Machine**, **Euclidean Drummer**, **Metronome** |
| Mixing | **Track** (volume, pan, mute, and solo for one channel) |
| Other effects | **Stutter**, **Tape Stop**, **Vocoder** |

A block's insides are shared by all its copies, so blocks that need their
own sound per use take it from outside: wire a Clip into a **Track**
rather than looking for one inside it, and use the **Drum Machine**'s
per-drum trigger outputs to play your own Samplers.

`audioc blocks` lists the library from the command line.

## Saving your own

1. Build the part you want to keep, select it, and make a group
   (`Ctrl+G`). Give it the inputs and outputs it should have (see
   [Groups](editor.md#groups)).
2. Name it: in the Node tab, under **Group definition**, type a **Name**.
   The name is how you'll find it.
3. Click **Save to library**. The status line says "Saved … to your library
   (v1)", and the block appears in the Library tab under **Mine**.

Saving a block with the same name again makes a new version of it (v2,
v3, …). The project you saved from remembers where its copy came from, so
other projects that use an older version can be offered the update.

Your blocks are files in a library folder in your user data folder:

| System | Folder |
| --- | --- |
| Linux | `~/.local/share/audioconstructor/blocks/` |
| macOS | `~/Library/Application Support/Audio-Constructor/blocks/` |
| Windows | `%APPDATA%\Audio Constructor\data\blocks\` |

Blocks other people gave you are in its `imported/` folder.
`audioc blocks --library <that folder>` lists them alongside the shipped
ones.

Removing a block from the library (right-click ▸ **Remove from library**)
deletes its file. Projects that use it keep their copy. Shipped blocks
can't be removed.

## Updating

When the library has a newer version of a block than a project's copy:

- the block's Node tab shows **Update from library (v1 → v2)**;
- the Project tab's **Blocks** section lists every block with an update,
  each with **Update**, and **Update all** does them all as one undo step.

Updating replaces the definition in every copy in the project. Values you
set on its inputs stay, as long as the new version still has those
inputs; wires to inputs or outputs the new version doesn't have are
removed, and the status line says how many.

For projects on disk, `audioc block-update` does the same from the command
line (with `--check` to only report); see
[The command line](command-line.md#block-update).

## Sharing a block

Send the block's `.acblock` file from your library folder. To add a block
file someone sent you, use **File ▸ Open** and pick it, or **Import block
file…** at the bottom of the Library tab. It goes into your library under
**Imported**, and your open project is left alone. A block with the same
name as one you have is kept separately, so it never overwrites yours.

## Sharing a project: packs

A **pack** (`.acpack`) is one file holding a project and every audio file
it uses, ready to send.

- **File ▸ Export Pack…** saves the project first (asking where, if it
  has never been saved), then asks where to write the pack. Audio files
  from outside the project's folder are copied into the pack's `assets/`
  folder. The status line says how many files went in, and names any that
  were missing and left out.
- **File ▸ Open** on a pack unpacks it into a new folder next to the pack
  and opens the project from there. Blocks the project carries that your
  library doesn't have are added to your library under **Imported**.
- From the command line: `audioc pack my.acproj` and
  `audioc unpack my.acpack`; see
  [The command line](command-line.md#pack-and-unpack).

Unpacking never writes outside its folder, so a pack can't put files
anywhere else on your computer.

## Sharing a few nodes

`Ctrl+C` copies the selected nodes, and the wires between them, as text.
Paste it into another project with `Ctrl+V`, or into a message to send
someone a piece of a patch.

Next: [Performing](performing.md).
