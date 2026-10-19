# Words the whole editor shares: languages, themes, and node categories.

## View ▸ Language

language-system = System
language-english = English
language-pseudo = Pseudo (for testing translations)

## View ▸ Theme

theme-dark = Dark
theme-light = Light
theme-high-contrast = High Contrast
theme-system = Follow System

## Library categories

category-io = Input / Output
category-generators = Generators
category-level = Level & Dynamics
category-filters = Filters
category-time = Time
category-spectral = Spectral
category-math = Math & Logic
category-modulation = Modulation
category-triggers = Triggers & Gates
category-sequencing = Sequencing & Notes
category-controls = Controls
category-display = Display
category-utility = Utility

## Status messages

status-copied =
    Copied { $count ->
        [one] { $count } node
       *[other] { $count } nodes
    }
status-language = Language: { $language }
## The window

# The window's title: a dot while there are unsaved changes, then the project.
app-window-title = { $dirty }{ $project } — Audio Constructor

## The status line: hints when nothing else is said

hint-panels = Drag or click a widget to play it · drag a card by its edge to move it · F11 performance mode · Ctrl+Tab canvas
hint-nothing-selected = Shift+A add · drag from a socket to wire · double-click empty canvas to search · Ctrl+F find · Ctrl+L lay out · Tab goes to a node
hint-several-selected = F connect two · Shift+A add a node wired from these · Ctrl+L lay out · Shift+Alt+A align · Ctrl+G group
hint-one-selected = G move · Shift+D duplicate · X delete · Ctrl+G group · M mute · H collapse · Tab next node

## The status line: what happened

status-couldnt-open-path = Couldn't open { $path }: { $error }
status-couldnt-open = Couldn't open: { $error }
status-theme-missing = The theme “{ $name }” isn't in your themes folder, so the editor is Dark
status-theme-files-unusable =
    { $count ->
        [one] { $count } theme file couldn't be used ({ $first })
       *[other] { $count } theme files couldn't be used ({ $first })
    }
status-themes-loaded =
    { $count ->
        [one] { $count } theme in your themes folder
       *[other] { $count } themes in your themes folder
    }
status-theme-low-contrast = “{ $name }” has text at { $ratio }:1 ({ $where }), under the 4.5:1 that keeps text readable
status-no-themes-folder = Themes aren't kept anywhere in this session
status-themes-folder = Themes folder: { $path }
status-couldnt-open-themes-folder = Couldn't open the themes folder: { $error }
status-block-files-unreadable =
    { $count ->
        [one] { $count } block file in your library couldn't be read ({ $first })
       *[other] { $count } block files in your library couldn't be read ({ $first })
    }
status-ui-scale = Interface at { $percent } %
status-learning = Binding { $target }: move a control (MIDI, key, gamepad, or OSC)… Esc cancels
status-cancel = Cancel
status-recorded = Recorded { $file }
status-clip-from-take = Create Clip from take
status-dismiss = Dismiss
status-more-takes = +{ $count } more
status-exporting-to = Exporting { $path }…
status-exporting = Exporting…
status-play-keys-on = Play keys: keys go to the patch · Esc and shortcuts with modifiers still work · { $stop } stops
status-audio-settings = Audio device settings
status-problems =
    { $count ->
        [one] { $count } problem: show the Problems tab
       *[other] { $count } problems: show the Problems tab
    }
status-exported = Exported { $seconds } s × { $channels } ch at { $rate } Hz to { $path }
status-export-failed = Export failed: { $error }
status-take-incomplete = Take { $file } is incomplete: { $error }
status-binding-cancelled = Binding cancelled
status-bound = Bound { $source } to { $target }
status-already-bound = { $source } already drives { $target }
status-node-gone = That node no longer exists.
status-audio = Audio: { $what }
status-a-selected-node = a selected node
status-no-output = { $node } has no output
status-no-free-input = it has no free input that takes the { $type } output of { $node }
# The find bar jumped to a node: its title and kind, and the group it's in.
status-found = { $title } ({ $kind })
status-found-in = { $title } ({ $kind }) in { $place }
status-block-gone = { $name } is no longer in the library
status-added-and-wired = Added { $name } and wired the { $count } selected nodes into it, in the order you selected them
status-added-without-wiring = Added { $name } without wiring: { $why }
status-pasted =
    { $count ->
        [one] Pasted { $count } node
       *[other] Pasted { $count } nodes
    }
status-play-keys-unavailable = Play keys isn't available here: the host plays the patch
status-nothing-to-undo = Nothing to undo
status-nothing-to-redo = Nothing to redo
status-laid-out = Laid out { $count } nodes · Ctrl+Z undoes it
status-select-two-to-align = Select two or more nodes to align or distribute
status-grouped-unpinned =
    { $count ->
        [one] Grouped. { $count } pinned node left the panels: nodes inside groups can't be pinned (Ctrl+Z to undo).
       *[other] Grouped. { $count } pinned nodes left the panels: nodes inside groups can't be pinned (Ctrl+Z to undo).
    }
status-grouped = Grouped. Ctrl+Enter goes inside; Ctrl+Alt+G ungroups.
status-select-group-to-ungroup = Select a group to ungroup
status-not-a-group = That node isn't a group
status-play-keys-turned-on = Play keys on: keys go to the patch ({ $stop } or the keyboard button turns it off)
status-play-keys-off = Play keys off
status-pin-top-level-only = Only top-level nodes can be pinned: leave the group (Ctrl+Enter) first
status-select-something-to-pin = Select a control or display to pin: a slider, knob, button, keyboard, meter, scope…
status-only-controls-pin = Only controls and displays can be pinned
status-pinned =
    { $count ->
        [one] Pinned { $count } node to { $panel } · Ctrl+Tab shows the panels
       *[other] Pinned { $count } nodes to { $panel } · Ctrl+Tab shows the panels
    }
status-already-pinned = Already pinned to { $panel }
status-every-panel = every panel
status-unpinned =
    { $count ->
        [one] Unpinned { $count } node from { $panel }
       *[other] Unpinned { $count } nodes from { $panel }
    }
status-new-project = New project
status-opened = Opened { $path }
status-unpacked = Unpacked { $pack } into { $folder }
status-pack-blocks-added =
    { $count ->
        [one] added { $count } block from it to your library
       *[other] added { $count } blocks from it to your library
    }
status-unpacked-but-not-opened = Unpacked into { $folder }, but couldn't open the project: { $error }
status-couldnt-unpack = Couldn't unpack { $pack }: { $error }
status-project-blocks-added =
    { $count ->
        [one] added { $count } block it uses to your library
       *[other] added { $count } blocks it uses to your library
    }
status-audio-file-problems = { $count } audio file problem(s); see the Project tab
status-opened-example-file = Opened the example { $file }
status-opened-example = Opened the example “{ $name }” as a new project: Save As keeps your changes
status-couldnt-open-example = Couldn't open the example: { $error }
status-couldnt-copy-audio = Couldn't copy audio files: { $error }
status-saved = Saved { $path }
status-saved-with-audio =
    { $count ->
        [one] Saved { $path } (copied { $count } audio file into assets/)
       *[other] Saved { $path } (copied { $count } audio files into assets/)
    }
status-couldnt-save = Couldn't save: { $error }
status-relinked =
    { $count ->
        [one] Relinked { $count } node
       *[other] Relinked { $count } nodes
    }
status-clip-plays = Clip plays { $file }
status-couldnt-use-file = Couldn't use that file: { $error }

## The add-node palette

palette-feed-note = The { $count } selected nodes will be wired into it, in the order you selected them
## Blocks and packs

status-couldnt-read = Couldn't read { $path }: { $error }
status-block-already-there = { $name } is already in your library
status-block-imported = Imported { $name } into your library
status-block-imported-with-preview = Imported { $name } into your library with its preview
status-downloading = Downloading { $link }…
status-exporting-block = Exporting the block and its preview…
status-couldnt-import = Couldn't import { $name }: { $error }
status-block-saved = Saved { $name } to your library (v{ $version })
status-couldnt-save-block = Couldn't save { $name } to your library: { $error }
status-block-up-to-date = That block is up to date
status-block-updated = Updated { $name } to v{ $version }
status-blocks-up-to-date = Every block is up to date
status-blocks-updated =
    { $count ->
        [one] Updated { $count } block
       *[other] Updated { $count } blocks
    }
status-update-dropped-wires =
    { $count ->
        [one] removed { $count } wire to sockets the new version doesn't have
       *[other] removed { $count } wires to sockets the new version doesn't have
    }
status-block-removed = Removed { $name } from your library; projects that use it keep their copy
status-block-removed-file = Removed { $name } from your library and deleted its file; projects that use it keep their copy
status-couldnt-remove-block = Couldn't remove { $name }: { $error }
status-save-before-pack = Save the project first; the pack is made from the saved file
status-save-before-packing = Save the project before packing it
status-packed =
    { $count ->
        [one] Packed the project and { $count } audio file into { $path }
       *[other] Packed the project and { $count } audio files into { $path }
    }
status-pack-copied =
    { $count ->
        [one] copied { $count } file from outside the project folder into the pack's assets/
       *[other] copied { $count } files from outside the project folder into the pack's assets/
    }
status-pack-left-out =
    { $count ->
        [one] left out { $count } missing file: { $files }
       *[other] left out { $count } missing files: { $files }
    }
status-couldnt-pack = Couldn't write the pack: { $error }

## The audio device, in the status bar ("no device (null) · 48000 Hz · 256 frames")

audio-describe = { $device } · { $rate } Hz · { $frames } frames
audio-off = audio off
audio-default-output = default output

## Node timing (Ctrl+T), on hover

timing-per-block = { $time } a block
timing-per-block-share = { $time } a block, { $share } of the { $budget } budget
timing-sum-of-voices = The sum of every voice and what's inside them.
timing-sum-of-nodes = The sum of the nodes inside.

## A plugin's window after the editor failed in it

embed-stopped = The editor stopped
embed-stopped-safe = Your project is safe with the plugin. Close and reopen this window.

## The find bar (Ctrl+F)

find-group = Group { $name }
find-portal-sender = Portal sender
find-portal-receiver = Portal receiver
find-hint = Find nodes by name, kind, or id
find-none = none
# Which result is chosen: "2 of 5".
find-count = { $index } of { $count }
find-keys = ↑ ↓ choose · Enter jumps to it · Esc closes
find-in = { $kind } · in { $place }

## The add-node palette (Shift+A)

palette-group-instance = An instance of the group “{ $name }”.
palette-nodes-that-take = Nodes that take { $type }
palette-nodes-that-feed = Nodes that feed { $type }
palette-hint = Search nodes and blocks  (volume, lpf, tremolo…)
palette-name = Add a node: search nodes and blocks
palette-nothing-matches = Nothing matches.
palette-block = Block
palette-group = Group
## What's wrong with a theme file (View ▸ Theme, and the status line)

theme-file-name-taken = another theme file is already named “{ $name }”
theme-file-not-json = isn't a theme file (not JSON: { $error })
theme-file-not-object = isn't a theme file (it should be one JSON object)
theme-file-no-name = has no "name"
theme-file-reserved-name = is named “{ $name }”, like a built-in theme: give it a name of its own
theme-file-no-base = no "base": built on Dark
theme-file-bad-base = "base" { $value } isn't dark, light, or high_contrast: built on Dark
theme-file-bad-header-mix = "header_mix" { $value } isn't a number from 0 to 1 (kept the base's)
theme-file-bad-colors = "colors" should be an object of colors (ignored)
theme-file-unknown-field = unknown field "{ $field }" (ignored)
theme-file-unknown-color = unknown color "{ $field }" (ignored)
theme-file-unknown-color-near = unknown color "{ $field }" (ignored, did you mean "{ $near }"?)
theme-file-bad-color = "{ $field }": { $value } isn't a color like "#RRGGBB" or "#RRGGBBAA" (kept the base's)
theme-file-too-big = is { $kb } KB, too big to be a theme
theme-file-unreadable = couldn't be read ({ $error })
status-reduce-motion-on = Reduce Motion on: no fades, eases, flowing wires, or flashes
status-reduce-motion-off = Reduce Motion off

## Sharing blocks: exports, links, shared libraries, and previews

status-export-block-gone = That block is gone
status-exporting-name = Exporting { $name }…
status-library-removed = Removed { $name } from the Library tab; blocks you installed from it stay
status-preview-needs-output = Previews play through the editor's own audio output; in a plugin, hear them in the desktop editor
status-preview-no-output = { $name } has no audio output, so nothing to hear
status-library-added =
    { $count ->
        [one] Added the library { $name } ({ $count } block) to the Library tab
       *[other] Added the library { $name } ({ $count } blocks) to the Library tab
    }
status-library-skipped =
    { $count ->
        [one] { $count } entry in its index couldn't be used
       *[other] { $count } entries in its index couldn't be used
    }
status-couldnt-import-link = Couldn't import from the link: { $error }
status-couldnt-install = Couldn't install it: { $error }
status-couldnt-preview = Couldn't play the preview: { $error }
status-exported-block-preview = Exported { $name } to { $path } with its preview
status-exported-block-no-preview = Exported { $name } to { $path }, without a preview: { $why }
status-exported-block = Exported { $name } to { $path }
status-couldnt-export-block = Couldn't export { $name }: { $error }
status-pack-downloaded = Downloaded the pack to { $path }
status-couldnt-save-pack = Couldn't save the pack to { $path }: { $error }
