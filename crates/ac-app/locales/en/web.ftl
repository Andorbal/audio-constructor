# The editor in a browser: files the page keeps for it, the browser's
# section of the Devices tab, and what the page says over the editor.

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

## The browser's file pickers and downloads: what kind of file

web-file-openable = Audio Constructor project, pack, or block
web-file-project = Audio Constructor project
web-file-pack = Audio Constructor pack
web-file-block = Audio Constructor block
web-file-wav = WAV audio
web-couldnt-open-file = Couldn't open the file: { $error }
web-couldnt-load-examples = Couldn't load the examples: { $error }

## What the page says once, about a patch

web-osc-quiet = This patch uses OSC, which needs UDP: browsers can't send or receive it, so its OSC nodes stay quiet here
web-record-no-takes = Record nodes can't write takes in the browser; Export WAV renders the project

## Over the editor, while there's no sound

web-audio-stopped = The audio stopped: { $error }. Reload the page to start it again.
web-audio-waiting = Click anywhere or press a key to start the sound (browsers wait for you).

## The Devices tab

web-devices-output = The browser plays through the system's output; pick another output in the system's sound settings.
web-microphone = Microphone
web-mic-on = On: Audio In nodes hear it
# $device is the microphone's name, as the browser reports it.
web-mic-on-named = On ({ $device }): Audio In nodes hear it
web-mic-stop = Stop using the microphone
web-mic-asking = Waiting for the browser to allow the microphone…
web-mic-blocked = The microphone is blocked. Allow it in the browser's site settings, then try again.
web-mic-none = No microphone: { $error }
web-try-again = Try again
web-mic-off = Audio In nodes hear the microphone or an audio interface's input once you allow it. Echo cancellation and noise suppression stay off.
web-mic-use = Use the microphone
web-midi = MIDI
web-midi-asking = Asking the browser for MIDI…
web-midi-unsupported = This browser has no MIDI (Web MIDI). The on-screen keyboard and the computer keys still play.
web-midi-refused = MIDI was refused: { $error }
web-midi-no-inputs = No MIDI inputs. Plug one in; it's picked up at once.
web-midi-inputs = Inputs (all are heard):
web-midi-no-outputs = No MIDI outputs.
web-gamepads-press = Press a button on a gamepad to connect it (browsers show gamepads only after a press).
web-not-in-browser = Not in the browser
web-not-in-browser-text = OSC needs UDP, which pages can't use: OSC In and OSC Out nodes stay quiet here (use the desktop editor or audioc run). Record nodes can't write takes to disk, and plugins can't load. Export WAV and Export Pack download their files.
