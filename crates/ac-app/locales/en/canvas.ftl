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
