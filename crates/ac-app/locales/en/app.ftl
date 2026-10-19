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
