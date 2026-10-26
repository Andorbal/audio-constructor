# The menu bar, its menus, and the commands they run (each command's label
# is also what the shortcut list and right-click menus show).

## The menu bar

menu-file = File
menu-edit = Edit
menu-view = View
menu-add = Add
menu-transport = Transport
menu-help = Help

## File

menu-open-recent = Open Recent
menu-no-recent = No recent projects
# A recent project whose file is gone.
menu-recent-missing = { $name } (missing)
menu-remove-from-list = Remove from list
menu-clear-list = Clear list
menu-examples = Examples
menu-no-examples = No examples found
menu-example-tip = Opens a copy: saving asks where to put it

## Edit

# Edit ▸ Undo with what it undoes ("Undo Add Gain").
menu-undo-named = Undo { $what }
menu-redo-named = Redo { $what }
# Under Edit ▸ Align / Distribute.
menu-arrange-at-pointer = { $shortcut } opens this at the pointer

## View

menu-theme = Theme
menu-theme-low-contrast = low text contrast ({ $ratio }:1)
menu-theme-low-contrast-tip = Text contrast { $ratio }:1 ({ $where }), under 4.5:1: text may be hard to read
menu-open-themes-folder = Open Themes Folder
menu-reload-themes = Reload Themes
menu-ui-scale = UI Scale
# A step of View ▸ UI Scale.
menu-ui-scale-step = { $percent } %
menu-language = Language

## Add

menu-reroute-hint = Reroute: Shift+right-drag across a wire

## Transport

menu-play = Play
menu-stop = Stop
menu-check-for-updates = Check for Updates…
menu-check-for-updates-daily = Check for Updates Daily
menu-report-problem = Report a Problem…

## Commands

action-new = New
action-open = Open…
action-import-from-link = Import from Link…
action-save = Save
action-save-as = Save As…
action-export-wav = Export WAV…
action-export-pack = Export Pack…
action-quit = Quit
action-undo = Undo
action-redo = Redo
action-copy = Copy
action-paste = Paste
action-delete = Delete
action-delete-reconnect = Delete and Reconnect
action-duplicate = Duplicate
action-duplicate-with-inputs = Duplicate with Inputs
action-select-all = Select All
action-select-none = Select None
action-select-linked = Select Linked
action-grab = Move
action-rename = Rename
action-add-node = Add Node…
action-frame = Frame Selection
action-group = Make Group
action-ungroup = Ungroup
action-enter-leave-group = Enter / Leave Group
action-portal = Make Portal / Back to Wire
action-auto-layout = Auto-Layout
action-arrange-menu = Align / Distribute…
action-align-left = Align Left
action-align-center = Align Center
action-align-right = Align Right
action-align-top = Align Top
action-align-middle = Align Middle
action-align-bottom = Align Bottom
action-distribute-horizontally = Distribute Horizontally
action-distribute-vertically = Distribute Vertically
action-lazy-connect = Connect Selected
action-find = Find…
action-mute = Mute
action-collapse = Collapse
action-hide-unwired = Hide Unwired Sockets
action-node-help = Node Help
action-fit-all = Frame All
action-fit-selection = Frame Selected
action-zoom-in = Zoom In
action-zoom-out = Zoom Out
action-ui-scale-up = Bigger Interface
action-ui-scale-down = Smaller Interface
action-ui-scale-reset = Interface at 100 %
action-toggle-side-panel = Side Panel
action-show-timing = Node Timing
action-minimap = Minimap
action-reduce-motion = Reduce Motion
action-pin = Pin to Panel
action-toggle-panels = Canvas / Panels
action-performance-mode = Performance Mode
action-play-stop = Play / Stop
action-stop = Stop
action-panic = Panic (Esc Esc)
action-play-from-start = Play from Start
action-record = Record
action-toggle-loop = Loop
action-rewind = Go to Start
action-go-to-end = Go to End
action-add-marker = Add Marker
action-previous-marker = Previous Marker
action-next-marker = Next Marker
action-metronome = Metronome
action-play-keys = Play Keys
action-shortcuts = Keyboard Shortcuts
action-about = About Audio Constructor
