# The side panel: its tabs and what's in them.

## Tabs

tab-node = Node
tab-library = Library
tab-project = Project
tab-devices = Devices
tab-problems = Problems
tab-help = Help
# A tab with a count in it, as a screen reader says it ("Problems, 2").
tab-named-count = { $tab }, { $count }
## Section headings (shown in capitals)

section-inputs = Inputs
section-outputs = Outputs
section-settings = Settings
section-editor = Editor
section-sound = Sound
node-tab-no-sound = Choose a file, or wire a Sample into `buffer`, to see the sound here.
section-help = Help
section-group-definition = Group definition

## The Node tab

node-tab-selected = { $count } nodes selected
# Under the node's title: its kind as projects store it (`oscillator`), and its id.
node-tab-kind-id = { $kind } · id { $id }
node-tab-group = Group
node-tab-help = Help
node-tab-help-tip = The node's reference: sockets, tips, and what uses it (F1)
node-tab-missing = The node is kept in the project with its settings and wires, but does nothing.
node-tab-portal-sender = Portal sender: the wire continues at its receivers
node-tab-portal-receiver = Portal receiver: the wire arrives from its sender
node-tab-portal-name = Portal name
node-tab-label-hint = Label (default: { $name })
node-tab-muted = Muted
node-tab-enter-group = Enter group
node-tab-group-input = group input
node-tab-dbfs = { $db } dBFS
node-tab-name = Name
node-tab-group-summary =
    { $inputs } inputs · { $outputs } outputs · { $nodes } nodes inside · { $count ->
        [one] { $count } instance
       *[other] { $count } instances
    }. Editing the inside changes every instance.
node-tab-polyphonic = Polyphonic (one copy per voice)
node-tab-polyphonic-tip = Runs the inside once per voice and shares notes from the `midi` input among the copies. Put a Voice node inside to read each copy's note.
node-tab-help-link = Sockets, and the blocks and examples that use it
node-tab-time = Time
node-tab-measure = Measure
node-tab-measure-tip = Measure how long every node takes each block ({ $shortcut }). Measuring costs a little, so it's off until asked for.
node-tab-not-timed = — (doesn't run in the engine)
node-tab-time-share = { $time } · { $share } of budget
node-tab-origin-shipped = a shipped block
node-tab-origin-user = one of your blocks
node-tab-origin-imported = an imported block
node-tab-block-from = From { $origin }, “{ $name }” · this copy is v{ $version }
node-tab-block-unknown = From a block your library doesn't have ({ $source }) · v{ $version }
node-tab-save-to-library = Save to library
node-tab-save-to-library-tip = Save this definition as one of your blocks, so any project can add it from the Library tab. Saving it again makes a new version.
node-tab-update-from-library = Update from library (v{ $from } → v{ $to })
node-tab-update-from-library-tip = Replace this definition with the library's newer version. Every instance changes; values set on inputs that still exist stay.
node-tab-nothing-selected = Nothing selected
node-tab-empty-hint = Press Shift+A or double-click the canvas to add a node.
node-tab-select-hint = Click a node (or Tab to it) to see its settings and help.
node-tab-fire = Fire
node-tab-unwired = unwired
node-tab-shaper-hint = The curve shapes the signal when `shape` is custom.
node-tab-larger-editor = Larger editor
node-tab-none = none
node-tab-no-file = no file
node-tab-file-missing = { $file } (missing)
node-tab-choose = Choose…
node-tab-locate = Locate…
node-tab-channels =
    { $count ->
        [one] { $count } channel
       *[other] { $count } channels
    }
node-tab-group-input-title = Group Input
node-tab-group-output-title = Group Output
node-tab-interface-of = Interface of “{ $name }”
node-tab-interface-empty = None yet. Drag a wire to the empty socket, or right-click an inner socket and choose Expose.
node-tab-socket-name = Name of the socket “{ $name }”
node-tab-remove-socket = Remove from the interface
node-tab-remove-named-socket = Remove “{ $name }” from the interface
## The Devices tab

section-running = Running
section-computer-keys = Computer keys
section-macros = Macros
section-device = Device
section-midi-inputs = MIDI inputs
section-midi-outputs = MIDI outputs
section-gamepads = Gamepads
section-osc = OSC
devices-sample-rate = Sample rate
devices-hz = { $rate } Hz
devices-buffer = Buffer
devices-buffer-size = { $frames } frames ({ $ms } ms)
devices-frames = { $frames } frames
devices-channels = Channels
devices-channels-in-out = { $inputs } in · { $outputs } out
devices-load = Load
devices-percent = { $percent } %
devices-per-core = Per core
devices-core-audio = audio
devices-core-worker = worker { $number }
devices-core-load = { $core } { $percent } %
devices-xruns = Xruns
devices-xruns-tip = Engine blocks over budget plus device underruns/overruns
devices-audio = Audio
devices-host-provides = The host provides audio and MIDI. Set the sample rate, the buffer size, and MIDI routing in the host.
devices-computer-keys = Turn on Play keys (the keyboard button in the transport bar) to send key presses to the patch instead of running shortcuts.
devices-macros = The host automates these. To drive an input from one, right-click the input, choose Bind…, and move the macro here or in the host.
devices-macro-drives = Drives { $target }
devices-macro-drives-nothing = Drives nothing yet
devices-off = off
devices-null = no device (null)
devices-output = Output
devices-input = Input
devices-default = default
devices-none = none
devices-device-default = device default
devices-device = Device
devices-stream-problems = { $under } under · { $over } over · { $errors } errors
devices-priority = Priority
devices-realtime-denied = real-time scheduling was denied
devices-status = Status
devices-disconnected = device disconnected
devices-latency = Latency
devices-not-measured = not measured
devices-measuring = measuring…
devices-round-trip = { $ms } ms round trip
devices-on-null = Running on the silent null stream so the editor keeps working.
devices-measure-latency = Measure latency
devices-measure-latency-tip = Plays a short burst on every output and listens on the inputs. Connect an output to an input with a cable first (turn speakers down).
devices-measure-latency-needs = Needs an audio device with an input.
devices-no-hosts = No audio hosts on this machine.
devices-host = Host
devices-threads = Engine threads
devices-threads-one = 1 (audio thread only)
devices-threads-tip = Run independent parts of the patch (voices, parallel chains) side by side. Extra threads keep a core busy while playing.
devices-apply = Apply
devices-refresh = Refresh
devices-rescan = Rescan
devices-midi-off = MIDI is off (no device runtime).
devices-no-midi-inputs = No MIDI inputs found.
devices-no-midi-outputs = No MIDI outputs found.
devices-events-dropped = { $count } events dropped
devices-messages-dropped = { $count } messages dropped
devices-control-events-dropped = { $count } control events dropped
devices-last-messages = Last messages
devices-clear = Clear
devices-no-midi-yet = None yet. Play or turn something.
devices-midi-out-to = MIDI Out nodes send to:
devices-nowhere = nowhere
devices-all-ports = all ports
devices-age-now = now
devices-age-seconds = { $seconds } s ago
devices-age-minutes = { $minutes } min ago
# A MIDI message in the lists of last messages: "Note on C4 · vel 100 · ch 1".
devices-midi-note-on = Note on { $note } · vel { $velocity } · { $channel }
devices-midi-note-off = Note off { $note } · { $channel }
devices-midi-cc = CC { $controller } = { $value } · { $channel }
devices-midi-pitch-bend = Pitch bend { $value } · { $channel }
devices-midi-pressure = Pressure { $value } · { $channel }
devices-midi-poly-pressure = Pressure { $note } { $value } · { $channel }
devices-midi-program = Program { $program } · { $channel }
devices-midi-channel = ch { $number }
devices-gamepads-off = Gamepads are off (no device runtime).
devices-no-gamepads = No gamepads connected. Plug one in any time.
devices-gamepad-tip = Pad { $number } in Gamepad nodes and bindings
devices-osc-listen = Listen on UDP port
devices-osc-port = OSC port
devices-osc-listen-tip = Receive OSC from phone and tablet apps or other programs on the network. Each message argument is a control for OSC In nodes and bindings.
devices-osc-send-to = Send to { $address }:{ $port }
devices-osc-send-here = Send to this computer, port { $port }
devices-osc-messages = { $count } messages
devices-osc-not-osc = { $count } not OSC
devices-osc-dropped = { $count } dropped
devices-osc-nothing-yet = Nothing yet? Check the surface's host and port, and that the device lets its app use the local network (a phone or tablet asks once; if refused, the app sends nothing and shows no error).
devices-no-osc-yet = None yet. Move a control on the surface.
devices-osc-use-tip = Use sets the selected OSC In's address.
devices-osc-use = Use
devices-click-to-copy = Click to copy
devices-osc-out-off = OSC Out is off (no device runtime).
devices-osc-out-sent = OSC Out nodes send to the address in their settings · { $count } sent
devices-osc-out-failed = { $count } failed

## The Problems tab

problems-none = No problems
problems-project = project
problems-click-to-show = Click to show the node
## The Library tab

section-project-groups = Groups in this project
section-blocks = Blocks
section-shared-libraries = Shared libraries
library-shipped = Shipped
library-mine = Mine
library-imported = Imported
library-all = All
library-search = Search nodes and blocks
library-no-nodes-match = No nodes match.
library-hint = Drag a node or block onto the canvas, or click to add it in the middle.
library-tag-on-tip = Click again to show every tag
library-tag-tip = Only blocks with this tag
library-no-user-blocks = None yet. Select a group and press Save to library in the Node tab.
library-no-imported-blocks = None yet. Blocks arrive with packs and block files you open.
library-no-blocks-match = No blocks match.
library-import = Import block file…
library-import-tip = Add a block file someone shared to your library, under Imported.
# A block in the list, as a screen reader says it.
library-block-row = { $name }, { $origin } block, version { $version }: { $about }
library-add-to-canvas = Add to canvas
library-shipped-stay = Shipped blocks stay in the library.
library-remove = Remove from library
library-no-description = No description.
library-block-tip = { $origin } block · version { $version } · right-click for more
library-block-tip-by = { $origin } block · version { $version } · by { $author } · right-click for more
library-import-link-tip = Import a block or a pack from a link, or add a shared library by its index.json link to preview and install its blocks.
library-export = Export block file…
library-export-tip = Write it to a block file to share, with its preview beside it
# A block's play button: what it says to screen readers ("Play preview of
# Plate Reverb"), and its tooltips.
library-preview-play = Play preview of { $name }
library-preview-stop = Stop preview of { $name }
library-preview-cancel = Stop getting the preview of { $name }
library-preview-tip = Play its preview
library-preview-render-tip = Play a preview (rendered now: what an export would write)
library-preview-remote-tip = Play its preview (downloaded the first time)
library-preview-stop-tip = Stop the preview
library-preview-loading-tip = Getting the preview… (click to cancel)
library-no-shared = None yet. Import from Link… with a library's index.json link lists its blocks here.
library-reading = Reading the library…
library-shared-empty = This library has no blocks.
library-shared-refresh = Refresh
library-shared-refresh-tip = Read { $link } again
library-shared-remove = Remove
library-shared-remove-tip = Stop listing this library. Blocks you installed from it stay.
library-installed = Installed
library-installed-tip = This version is in your library
library-install = Install
library-install-named = Install { $name }
library-install-tip = Download it into your library, under Imported, checked against the library's index
library-shared-from = From { $library } · { $file }
# A shared library's block, to screen readers: "Plate Reverb, version 2: A
# bright plate."
library-remote-row = { $name }, version { $version }: { $about }
## The Project tab

section-tempo-map = Tempo map
section-loop = Loop
section-engine = Engine
section-audio-files = Audio files
project-no-blocks = This project uses no library blocks.
project-blocks-current = The library blocks this project uses are up to date.
project-update = Update
project-update-tip = Take the library's version ({ $source })
project-update-all = Update all
project-update-all-tip = Update every block above, as one undo step.
# A block's update, from the version the project has to the library's.
project-update-versions = v{ $from } → v{ $to }
project-name = Name
project-length = Length
project-seconds-suffix = { " " }s
project-render-block = Render block
project-seed = Seed
project-rates-note = Offline renders use the sample rate and block size; the live engine runs at the device's rate.
project-at-beat = at beat
project-bpm = BPM
project-time-sig = time sig
project-change-beat = Tempo change { $number }: at beat
project-change-bpm = Tempo change { $number }: BPM
project-change-beats = Tempo change { $number }: beats per bar
project-change-unit = Tempo change { $number }: beat unit
project-remove-tempo-change = Remove this tempo change
project-add-tempo-change = Add tempo change
project-loop-between = Loop between beats
project-loop-start = Loop start (beats)
project-loop-end = Loop end (beats)
project-output-latency = Output latency
project-latency = { $samples } samples ({ $ms } ms)
project-latency-tip = Delay that latency-reporting nodes add on the way to the outputs. Parallel paths are delayed to match where they meet.
project-whole-input = Whole-input nodes
project-whole-input-states = { $baked } baked · { $baking } baking · { $disabled } disabled
project-schedule = Schedule
project-schedule-stats = { $steps } steps in { $chains } chains, { $waves } waves · { $buffers } audio buffers
project-schedule-tip = A chain's steps run in order on one core; the chains of one wave can run on different cores (Devices tab).
project-no-audio-files = No audio files. Clip nodes add them.
project-file-changed = The file changed since it was added (its content hash differs).
# What a screen reader says for a file the project found.
project-file-found = Found
project-relative-paths = Relative paths resolve against { $folder }
project-save-to-copy = Save the project to copy its audio files into its assets folder.
## Bindings (the Project, Node, and Devices tabs)

section-bindings = Bindings
section-transport-buttons = Transport buttons
bindings-remove = Remove this binding
bindings-remove-named = Remove the binding of { $source }
bindings-show-node = Show the node
bindings-from = from
bindings-to = to
bindings-last-value = Where the control last set it
bindings-mode = Momentary or toggle
bindings-fires-on-press = fires on press
bindings-on-press = on press
bindings-does-nothing = does nothing (see Problems)
bindings-momentary = Momentary
bindings-toggle = Toggle
bindings-invert = invert
bindings-invert-tip = Turn the control around
bindings-none = No bindings. Right-click an input (or a control's title) and choose Bind… to drive it from a MIDI controller, a key, a gamepad, or OSC.
bindings-node-none = Right-click an unwired input and choose Bind… to drive it from a control.
bindings-instance = (instance { $instance })
bindings-add-gamepad-transport = Add gamepad transport buttons
bindings-add-gamepad-transport-tip = A plays and stops, B stops, X records, on any gamepad.
bindings-transport-already = The gamepad transport buttons are already set up.
bindings-transport-added = Added { $count } gamepad transport buttons (A play/stop, B stop, X record).
bindings-transport-action = Transport action to bind
bindings-learn-transport-tip = Press the control (MIDI, key, gamepad, or OSC) that should do this.
bindings-no-transport = No transport buttons yet.
bindings-transport-stored = Transport buttons are stored in the project, like other bindings.
## The Help tab

section-used-in = Used in
help-search = Search help
help-unknown-kind = No help for `{ $kind }`: this version doesn't have that node.
help-no-match = No node matches.
help-index-hint = Press F1 on a node for its help, or choose one here.
help-all-nodes = All nodes
help-live-source = live source
help-whole-input = needs its whole input (baked)
help-try-this = Try this
# A socket's default value ("440 Hz by default").
help-default = { $value } by default
help-unused = No block or example uses it yet. Build something with it and save it to the library from the Node tab.
help-blocks = Blocks
help-click-to-add = Click to add it to the canvas.
help-examples = Examples
# What a screen reader says for a link to a block or an example.
help-block-link = { $name }, block
help-example-link = { $name }, example
help-open-example = Open this example (as a new, untitled project)
bindings-transport-target = Transport: { $action }
bindings-missing-node = missing node { $node }

## Node editors (curves, grids, piano rolls, bands, steps)

editor-curve-hint = Drag points; double-click to add or remove one; right-click a point to bend the segment after it.
editor-pattern-hint = Click or drag across cells to switch them.
editor-piano-roll-hint = Click or drag to draw a note, drag one to move it or its end to stretch it, right-click to delete. Shift: off the grid.
editor-bands-hint = Drag an edge; double-click to split a band; right-click an edge to merge its bands.
editor-steps-hint = Click or drag to draw values. Shift: fine values.
editor-zones-hint = Drag a zone to move it, an edge to resize it. Double-click to add a zone, right-click one to remove it.
editor-sound-hint = Drag the white handles to set where the sound starts and ends; the blue bar is the loop.
editor-markers-hint = Click to add a slice, drag a line to move it, right-click to remove it.
editor-segment-after = Segment after this point
editor-shape-straight = Straight
editor-shape-hold = Hold (step)
editor-shape-smooth = Smooth
editor-shape-slow-start = Slow start
editor-shape-fast-start = Fast start
editor-remove-point = Remove point
node-tab-export-block-tip = Write this definition to a block file to share, with a few seconds of it playing (its preview) beside it.
