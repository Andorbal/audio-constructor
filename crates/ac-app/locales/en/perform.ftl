# The panels and performance mode.

perform-tab-tip = Double-click to rename · right-click for more
perform-rename = Rename
perform-move-left = Move Left
perform-move-right = Move Right
perform-delete-panel = Delete Panel
perform-delete-panel-tip = The nodes stay in the patch; Ctrl+Z brings the panel back
perform-new-panel = New panel
perform-perform = Perform
perform-perform-tip = Show this panel full screen (F11; Esc leaves)
perform-hint = P on the canvas pins the selected controls · Ctrl+Tab: canvas
perform-play-tip = Play (Space)
perform-exit = Exit
perform-exit-tip = Leave performance mode (Esc or F11)
perform-play-keys-on = Play keys is on: computer keys go to the patch
perform-play-keys-off = Play keys: send computer keys to the patch
perform-no-panels = No panels yet
perform-no-panels-hint = Select sliders, buttons, meters, or other controls on the canvas and press P to pin them to a panel.
perform-nothing-pinned = Nothing pinned to { $panel } yet
perform-nothing-pinned-hint = Select controls or displays on the canvas (sliders, knobs, buttons, the keyboard, meters, scopes) and press P, or right-click one and pin it here.
perform-missing-node = Missing node
perform-no-widget = no widget to show

## What screen readers say about the panels.

a11y-panel =
    { $count ->
        [one] Panel { $panel }, { $count } control
       *[other] Panel { $panel }, { $count } controls
    }
# A card's header button: "Shrink Cutoff".
a11y-panel-button = { $button } { $card }
a11y-panel-shrink = Shrink
a11y-panel-grow = Grow
a11y-panel-unpin = Unpin
# A meter's reading: "peak -12.5 dB".
a11y-peak = peak { $db } dB
a11y-silent = silent
a11y-recording = recording
a11y-not-recording = not recording
perform-compact = Compact (half height)
perform-color = Color
perform-color-default = Its category's
perform-color-amber = Amber
perform-color-green = Green
perform-color-red = Red
perform-color-blue = Blue
perform-color-purple = Purple
perform-color-grey = Grey
perform-make-strip = Make Strip
perform-new-strip = Strip
perform-out-of-strip = Take Out of Strip
perform-unpin = Unpin
perform-rename-strip = Rename Strip
perform-ungroup-strip = Ungroup Strip (Keep the Cards)
perform-unpin-strip = Unpin Strip
perform-edit-label = Edit Label
perform-label-wider = Wider
perform-label-narrower = Narrower
perform-remove-label = Remove
perform-add-label = Add Label Here
perform-new-label = Section
perform-add-separator = Add Separator Here
perform-mixer = Mixer
perform-pads = Pads
perform-grid = Grid
perform-tracks = Tracks
perform-track-n = Track { $n }
perform-scene-n = Scene { $n }
a11y-lit = lit
a11y-light-on = on
a11y-light-off = off
