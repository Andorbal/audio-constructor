# The canvas: right-click menus, socket tooltips, hints, and badges.

## Right-click menus

canvas-disconnect = Disconnect
canvas-add-reroute = Add Reroute
canvas-add-reroute-here = Add Reroute Here
canvas-expose-input = Expose as Group Input
canvas-expose-output = Expose as Group Output
canvas-rename-socket = Rename Socket
canvas-remove-socket = Remove from Group Interface
canvas-bind = Bind…
# A control with two values binds each: "Bind X…", "Bind Y…".
canvas-bind-axis = Bind { $axis }…
canvas-remove-bindings = Remove Bindings
canvas-group-interface = Group interface
canvas-group-interface-tip =
    Drag to its empty socket to add one;
    right-click a socket to rename or remove it.
canvas-make-portal = Make Portal
canvas-enter-group = Enter Group
canvas-leave-group = Leave Group
canvas-choose-file = Choose File…
canvas-rename-frame = Rename Frame
canvas-remove-frame = Remove Frame (keep nodes)
canvas-arrange-selection = Arrange the selection
canvas-portal-sender = sender
canvas-portal-receiver = receiver
# A portal tag's menu title: its name and which end it is.
canvas-portal-title = Portal “{ $name }” · { $role }
canvas-jump-receiver = Jump to Receiver
canvas-jump-next-receiver = Jump to Next Receiver (of { $count })
canvas-jump-sender = Jump to Sender
canvas-rename-portal = Rename Portal
canvas-back-to-wire = Back to Wire
canvas-pin-top-level-only = Pin to Panel: top-level nodes only
canvas-pin-to = Pin to { $panel }
canvas-pin-to-new = Pin to New Panel
canvas-unpin-from = Unpin from { $panel }

## A socket's tooltip

canvas-new-socket = new socket
canvas-tip-wired = wired
canvas-tip-unwired = unwired: set inline (Const rate)
# How often an output changes: "sample rate", "block rate".
canvas-tip-rate = { $rate } rate
canvas-tip-rate-follows = rate follows inputs
canvas-tip-channels = { $count } ch
canvas-tip-channels-follow = channels follow input
canvas-tip-channels-up-to = up to { $count } ch
canvas-tip-range = range { $from } – { $to }
canvas-tip-silent = silent
canvas-tip-peak = peak { $db } dBFS
canvas-tip-events = { $count } events
canvas-tip-on = on
canvas-tip-off = off
canvas-tip-new-socket = Drop a wire here to add a socket to the group's interface.
## On the canvas

# Next to the group path at the top left, inside a group.
canvas-leave-hint = Ctrl+Enter to leave
canvas-drop-to-add = drop to add
canvas-bound = Bound to { $sources }
canvas-bound-but-wired = Bound to { $sources }, but the input is wired, so the binding does nothing.

## Badges in node headers (shown on hover)

canvas-badge-missing = Missing node: { $kind }. It's kept in the project and its wires are preserved.
canvas-badge-file-missing =
    File not found: { $file }
    Locate it from the Project tab.
canvas-badge-voices = Polyphonic: the inside runs once per voice ({ $count } voices), and notes from `midi` are shared out among them.
canvas-badge-latency = Adds { $samples } samples ({ $ms } ms) of latency; parallel paths are delayed to match.
canvas-badge-pinned = Pinned to { $panels }. Ctrl+Tab shows the panels; right-click to unpin.
canvas-badge-muted = Muted: passes its input through where the types match (M to unmute)
canvas-badge-disabled = Disabled: needs a known input, and { $sources } is live. It plays silence while live; offline renders still work.
canvas-badge-rebaking = Baking: something upstream changed; the previous bake plays until the new one is ready.
canvas-badge-baking = Baking: rendering its input in the background.
canvas-badge-waiting = Waiting to bake.
canvas-badge-baked = Baked: plays a render of its whole input, in time with the transport.

## Hints at the bottom of the canvas during a gesture

hint-add-reroutes = Drag across wires to add reroutes
hint-cut-wires = Drag across wires to cut them
hint-drop-wire = Drop on a socket to connect · on empty canvas to add a node
hint-fine-coarse = Ctrl: fine · Shift: coarse
hint-grab = Move the pointer (or the arrow keys) to place · click or Enter to confirm · Esc to cancel

## Portals

canvas-back-to-wires = Back to wires
canvas-back-to-a-wire = Back to a wire
canvas-portal-how = To make a portal, select a reroute, or the two nodes a wire joins
canvas-portal-nothing = Nothing there could become a portal
canvas-portal-made = Made portal “{ $name }”. Double-click a tag to jump to the other end; F2 renames it.
canvas-portal-no-receivers = No receivers: copy and paste one of this portal's receivers, or turn another wire from the same output into a portal.
canvas-portal-sends =
    { $count ->
        [one] Sends to { $count } receiver.
       *[other] Sends to { $count } receivers.
    }
canvas-portal-no-sender = No sender is linked to it.
canvas-portal-receives = Receives from its sender.
canvas-portal-tip =
    Portal “{ $name }”. { $what }
    Double-click to jump to the other end · F2 renames · Shift+P turns it back into a wire

## Arranging and quick wiring

canvas-layout-nothing = Nothing to lay out yet: add a few nodes first
canvas-layout-select = Select two or more nodes to lay out (or none, for the whole graph)
canvas-distribute-select = Select three or more nodes to distribute
canvas-align-select = Select two or more nodes to align
canvas-connect-select = Select two nodes to connect them (F)
canvas-connect-missing = Can't wire a missing node
canvas-connect-nothing = Nothing to connect: no free input of { $right } takes an output of { $left }, or the other way round without making a loop
# What F wired: "Connected Oscillator out → Gain in".
canvas-connected = Connected { $from } { $output } → { $to } { $input }
canvas-couldnt-connect = Couldn't connect: { $error }

## Frames, notes, and node bodies

canvas-frame = Frame
canvas-frame-numbered = Frame { $number }
canvas-button-held = held
canvas-button-press = press
canvas-choose-a-file = Choose a file…
canvas-recording = REC
canvas-record-idle = idle
canvas-note = Note
canvas-note-empty = Double-click to write a note.
canvas-not-a-number = “{ $text }” isn't a number
canvas-save-to-record = save the project to record

## Why a wire or a paste didn't happen

error-no-such-output = that output doesn't exist
error-no-such-input = that input doesn't exist
error-same-node = a node can't feed itself directly
error-incompatible = { $from } can't connect to { $to } without a conversion node
error-missing-node = can't wire a missing node
error-clipboard-not-nodes = the clipboard doesn't hold nodes
error-group-inside-itself = can't paste a group inside itself
error-group-definition-missing = group definition `{ $def }` is missing
error-group-no-definition = group node has no definition
error-unknown-kind = unknown node kind `{ $kind }`

## The canvas to screen readers

a11y-canvas =
    { $count ->
        [one] Node canvas, { $count } node
       *[other] Node canvas, { $count } nodes
    }
a11y-canvas-group =
    { $count ->
        [one] Node canvas inside { $group }, { $count } node
       *[other] Node canvas inside { $group }, { $count } nodes
    }
# A card: "Bass, Oscillator, 6 inputs, 1 output".
a11y-node =
    { $title }, { $kind }, { $inputs ->
        [one] { $inputs } input
       *[other] { $inputs } inputs
    }, { $outputs ->
        [one] { $outputs } output
       *[other] { $outputs } outputs
    }
a11y-node-plain =
    { $title }, { $inputs ->
        [one] { $inputs } input
       *[other] { $inputs } inputs
    }, { $outputs ->
        [one] { $outputs } output
       *[other] { $outputs } outputs
    }
a11y-kind-group = group
a11y-kind-missing = missing node
a11y-kind-reroute = reroute
a11y-kind-portal-sender = portal sender
a11y-kind-portal-receiver = portal receiver
a11y-kind-note = note
a11y-selected = selected
a11y-muted = muted
a11y-collapsed = collapsed
# A socket: "Gain gain: Float input, -6 dB".
a11y-socket = { $node } { $socket }: { $side }, { $state }
a11y-input = { $type } input
a11y-output = { $type } output
a11y-from = from { $source }
a11y-to = to { $targets }
a11y-not-connected = not connected
hint-keyboard-wire = Wiring from { $node } { $socket }: Tab to a card, Alt+↓ to a socket, Enter connects · Esc cancels
status-keyboard-wire-nothing = Nothing there takes that wire: pick a socket with Alt+↓ and ↑

# The last row of a plugin's card when it has more knobs than the card shows.
canvas-more-knobs = { $count ->
    [one] +1 more knob
   *[other] +{ $count } more knobs
}
canvas-more-knobs-tip = Every knob is in the Node tab. Wire one and it shows here; Show All Knobs in the node's menu puts them all on the card.
canvas-all-knobs = Show All Knobs
canvas-fewer-knobs = Show Fewer Knobs
canvas-light-on = on
canvas-light-off = off
canvas-pin-as-strip = Pin as Strip
canvas-mixer-from-selection = Mixer from Selection
canvas-pads-from-selection = Pads from Selection
canvas-grid-from-selection = Launch Grid from Selection
