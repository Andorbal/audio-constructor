# The transport bar: play, record, loop, position, tempo, load, and the
# canvas and panels switch.

transport-go-to-start = Go to start
transport-play = Play
transport-stop = Stop
transport-play-tip = Play (Space) · from start: Shift+Space
transport-stop-tip = Stop (Space)
transport-record = Record
transport-record-tip = Record (R)
transport-loop = Loop
transport-host-playing = The host is playing
transport-host-stopped = The host is stopped
transport-host-tip = { $state }. The patch follows the host's transport: play, stop, and loop it in the host.
transport-play-keys = Play keys
transport-play-keys-on-tip = Play keys is on: computer keys go to the patch ({ $shortcut })
transport-play-keys-tip = Play keys: send computer keys to the patch instead of shortcuts ({ $shortcut })
# The song position as a screen reader says it.
transport-position = Position: bar { $bars }, time { $time }
# The tempo button: "120.0 BPM  4/4", with "  +2" for two more tempo changes.
transport-tempo = { $bpm } BPM  { $beats }/{ $unit }{ $more }
transport-tempo-tip = Tempo and time signature (click to edit; more changes in the Project tab)
transport-tempo-label = Tempo
transport-bpm-suffix = { " " }BPM
transport-beats-per-bar = Beats per bar
transport-beat-unit = Beat unit
transport-loop-range-tip = Loop range (click to edit)
transport-loop-beats = Beats
transport-xruns = Xruns: { $count }
transport-xruns-tip = Xruns: blocks that missed their deadline
transport-load = Audio load: { $percent } %
transport-load-tip = Audio load: the last block's processing time against its budget
transport-master-silent = Master: silent
transport-master-peak = Master peak { $db } dBFS
transport-performance-tip = Performance mode: the current panel full screen (F11)
transport-panels = Panels
transport-panels-tip = Panels: the pinned controls without the graph (Ctrl+Tab)
transport-canvas = Canvas
transport-canvas-tip = The node canvas (Ctrl+Tab)
