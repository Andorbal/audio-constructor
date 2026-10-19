# The editor in a browser: files the page keeps for it.

## Files, when the page keeps them (the status line)

status-saving-audio-stays =
    { $count ->
        [one] Saving… The project's { $count } audio file stays in this browser tab: Export Pack saves it with the project
       *[other] Saving… The project's { $count } audio files stay in this browser tab: Export Pack saves them with the project
    }
status-pack-left-out-audio =
    { $count ->
        [one] Left out { $count } missing audio file: { $files }
       *[other] Left out { $count } missing audio files: { $files }
    }
status-couldnt-make-pack = Couldn't make the pack: { $error }
status-rendered-saving = Rendered { $seconds } s × { $channels } ch at { $rate } Hz; saving…
status-opened-as-new = Opened { $name } as a new project
# Added after "Opened …" when the project's audio files aren't in this tab.
status-audio-files-to-locate =
    { $count ->
        [one] { $count } audio file to locate in the Project tab (or open the project's pack, which carries it)
       *[other] { $count } audio files to locate in the Project tab (or open the project's pack, which carries them)
    }
status-couldnt-use-named = Couldn't use { $name }: { $error }
# Why an audio file is missing (the Project tab, on hover).
asset-not-in-tab = not opened in this browser tab: Locate it
