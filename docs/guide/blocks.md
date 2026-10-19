# Blocks and sharing

A **block** is a group saved for reuse: a compressor, a reverb, a synth
voice, a drum machine. Audio Constructor ships with 23 of them, all built
from ordinary nodes, and you can save your own, hear them before adding
them, share them as files or whole libraries on the web, and pick up
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
- the **▶** button at a block's right plays its [preview](#previews)
  over whatever your patch is playing; click it again (**■**) to stop;
- drag a block onto the canvas, or click it to add it in the middle of
  the view; right-click it for **Add to canvas**, **Export block
  file…**, and **Remove from library**.

A block on the canvas is a group: double-click it to look inside, see how
it's built, and change it. Instrument blocks (the ones with a Voice node
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

## Previews

A block's **preview** is a few seconds of it playing, so you can hear
what it does before you add it. Every block that makes sound has one in
the Library tab, including the shipped ones: a block that comes with a
preview file plays that, and any other is rendered on the spot, which
takes a moment the first time (the button shows **…** meanwhile).

What plays a block depends on what it takes:

- an **effect** (a block with an audio input) gets the preview phrase:
  two bars at 120 BPM of drums, a chord pad, and an arpeggio, all made
  of ordinary nodes. A block with two audio inputs gets the chords and
  arpeggio on the first and the drums on the second, so a sidechain
  ducker pumps to the beat and a vocoder speaks drums;
- an **instrument** gets notes: a block with a Voice node inside plays
  the phrase's chords, and a MIDI input on any other block gets its
  arpeggio. A block with a `gate` and a `freq` input follows the
  arpeggio too;
- **trigger inputs** get the phrase's drum hits: one named `kick`,
  `snare`, or `hat` gets that drum, and others get the snare, then the
  kick, then the hi-hat;
- everything else keeps its default, so a block that plays by itself,
  like the **Drum Machine**, just plays.

A preview is five seconds at 48 kHz, rendered offline, so the same block
always gives exactly the same preview, on any computer. It's turned down
if it would clip, but never up: a quiet block sounds quiet.

> **Try this:** search the Library tab for `reverb`, then press ▶ on
> **Plate Reverb** and on **Hall Reverb** in turn to hear the difference.

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

**Export block file…** writes a block to share: in the Node tab under
**Group definition** for a group in your project, or on a library
block's right-click menu. It asks where to save the `.acblock` file and
writes its preview beside it, `<name>.preview.wav`. Send both; the block
file records its preview's name and checksum, so whoever imports it gets
the preview too. **Save to library** also keeps a preview beside the
block in your library folder.

To add a block file someone sent you, use **File ▸ Open** and pick it,
or **Import block file…** at the bottom of the Library tab. It goes into
your library under **Imported** (with its preview, if it's beside it),
and your open project is left alone. A block with the same name as one
you have is kept separately, so it never overwrites yours.

From the command line, `audioc block-export` does the same; see
[The command line](command-line.md#block-export).

## Importing from a link

**File ▸ Import from Link…** (also at the bottom of the Library tab's
Blocks section) takes a link to:

- **a block** (`.acblock`): it joins your library under **Imported**,
  as a block file would, with its preview;
- **a pack** (`.acpack`): it's saved to your Downloads folder and opened
  the way packs are (see below);
- **a shared library** (its `index.json`, or the link to its folder
  ending in `/`): it's listed in the Library tab under **Shared
  libraries**.

Downloads happen in the background; the status line says what's being
downloaded and what came of it. Links must start with `https://`.

## Shared libraries

A **shared library** is a folder of block files on the web, with an
`index.json` that lists them. Once you've added one from its link, the
Library tab's **Shared libraries** section lists its blocks every time
you open the editor (it's read again when you first look, or when you
press **Refresh**). The search field at the top searches them too.

- **▶** plays a block's preview: it's downloaded the first time and kept
  in your cache folder after that.
- **Install** downloads the block into your library under **Imported**.
  Every download is checked against the index's size and checksum, and
  one that doesn't match (changed since the index was made, or damaged
  on the way) isn't kept. A block you already have shows **Installed**.
- **Remove** stops listing the library. Blocks you installed from it
  stay in your library.

Installing a newer version of a block you installed before adds it
alongside the old one (as importing a file would), so projects that use
the old one are never changed behind your back.

### Publishing one

1. Put the blocks in a folder, each exported with **Export block file…**
   (or `audioc block-export`) so it has a preview. Your own library
   folder works as it is: **Save to library** keeps a preview beside each
   block.
2. Run `audioc library-index <folder> --name "My Blocks"`. It checks that
   every block loads, leaves out (and names) any that don't, and writes
   `index.json`.
3. Upload the folder to any web host that serves files. The link to its
   `index.json`, or to the folder, is what you share.

Run `audioc library-index` again whenever you add or change a block;
`--check` tells you whether the index is out of date. See
[The command line](command-line.md#library-index).

### The index format

`index.json` is plain JSON, so other tools can write or read it:

```json
{
  "format": "audio-constructor/library",
  "version": 1,
  "name": "Tape and Space",
  "description": "Delays and reverbs.",
  "blocks": [
    {
      "id": "warm-echo",
      "name": "Warm Echo",
      "version": 3,
      "description": "A tape echo that darkens as it repeats.",
      "tags": ["delay", "space"],
      "author": "Sam",
      "schema_version": 2,
      "file": "warm-echo.acblock",
      "sha256": "…",
      "size": 18211,
      "preview": { "file": "warm-echo.preview.wav", "sha256": "…", "size": 480044 }
    }
  ]
}
```

| Field | Means |
| --- | --- |
| `format`, `version` | Always `audio-constructor/library` and `1` for now; an index from a newer version is refused rather than misread |
| `name`, `description` | The library, as the Library tab shows it |
| `id` | The block file's name without `.acblock`; installing names the block after it |
| `name`, `version`, `description`, `tags`, `author` | The block's own, from its file |
| `schema_version` | The project format the block file is written in |
| `file`, `size`, `sha256` | The block file, relative to `index.json`, its size in bytes, and its SHA-256 as 64 hex digits |
| `preview` | Its preview, the same way (left out when there's none) |

Links are relative to the index, so the folder can move to another host
without changes. Entries that can't be used are skipped and counted; the
rest of the library still works.

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
