# Dialogs: file prompts, unsaved changes, keyboard shortcuts, and About.

## File dialogs (titles and file types)

dialog-open-title = Open project
dialog-save-as-title = Save project as
dialog-export-title = Export WAV
dialog-choose-clip-title = Choose an audio file
dialog-locate-title = Locate missing file
dialog-export-pack-title = Export pack
dialog-import-block-title = Import block file
dialog-export-block-title = Export block
dialog-filter-everything = Projects, packs, and blocks
dialog-filter-project = Audio Constructor project
dialog-filter-pack = Audio Constructor pack
dialog-filter-block = Audio Constructor block
dialog-filter-wav = WAV audio
# Shown when the system has no file dialog, above a field for the path.
dialog-type-path = Type the file's path.
dialog-ok = OK
dialog-cancel = Cancel

## Unsaved changes

dialog-save-changes = Save changes to “{ $project }”?
dialog-changes-lost = Your changes will be lost if you don't save them.
dialog-dont-save = Don't Save
dialog-save = Save

## Keyboard shortcuts

dialog-shortcuts-title = Keyboard shortcuts
gesture-pan = Pan
gesture-pan-how = Middle-drag · Space+drag · two-finger scroll
gesture-zoom = Zoom
gesture-zoom-how = Wheel · pinch
gesture-box-select = Box select
gesture-box-select-how = Drag on empty canvas (Shift adds)
gesture-snap = Snap to grid
gesture-snap-how = Hold Ctrl while moving
gesture-cut = Cut wires
gesture-cut-how = Ctrl+drag across them
gesture-reroute = Add reroute
gesture-reroute-how = Shift+right-drag across a wire
gesture-disconnect = Disconnect
gesture-disconnect-how = Drag a wire's end off its input
gesture-insert = Insert on wire
gesture-insert-how = Drop an unwired node onto a wire
gesture-wire-into-new = Wire nodes into a new one
gesture-wire-into-new-how = Select two or more, then Shift+A
gesture-fine-coarse = Fine / coarse values
gesture-fine-coarse-how = Ctrl / Shift while dragging a value
gesture-type-value = Type a value
gesture-type-value-how = Click a value
gesture-delete = Delete
gesture-delete-how = X or Delete
gesture-move-card = Move a panel card
gesture-move-card-how = Drag it by its edge
gesture-focus-nodes = Move between nodes
gesture-focus-nodes-how = Tab · Shift+Tab (Shift+Enter keeps one selected)
gesture-focus-sockets = Move between a node's sockets
gesture-focus-sockets-how = Alt+↓ · Alt+↑
gesture-keyboard-wire = Wire from the keyboard
gesture-keyboard-wire-how = Enter on a socket, then Enter on another node or socket
gesture-keyboard-value = Type a socket's value
gesture-keyboard-value-how = F2 on the socket
gesture-keyboard-move = Move the selection
gesture-keyboard-move-how = Arrow keys (Shift: further, Ctrl: finer)
gesture-keyboard-menu = Open the right-click menu
gesture-keyboard-menu-how = Shift+F10
gesture-switch-region = Canvas ↔ side panel
gesture-switch-region-how = F6
gesture-panel-keys = Play a panel control
gesture-panel-keys-how = Tab to it · arrows turn it · Enter types or flips · Space presses
gesture-leave-performance = Leave performance mode
gesture-leave-performance-how = Esc or F11

## About

dialog-about-version = Version { $version }
dialog-about-text = A node-based audio construction suite: build instruments, effects, recorders, and sequencers from small nodes and wires.
dialog-about-credits = Fonts: Inter and JetBrains Mono (SIL Open Font License). Icons: Phosphor (MIT).

## File ▸ Import from Link…

dialog-link-title = Import from Link
dialog-link-text = A link to a block (.acblock), a pack (.acpack), or a shared library's index.json. Blocks join your library under Imported; a pack is saved to your Downloads folder and opened; a library is listed in the Library tab, to preview and install from.
dialog-link-field = Link
dialog-link-import = Import
