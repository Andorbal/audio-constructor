# In the browser

Audio Constructor also runs in a web browser: the same editor, the same
nodes and blocks, and the same sound, with nothing to install. A patch
you build there opens in the desktop editor and the other way around.

The browser version suits trying things out, sketching a patch on a
machine that isn't yours, and sharing a link to an example. For playing
an instrument through it, or for the lowest latency, the desktop editor
is better: the browser adds its own buffering, and some things a browser
doesn't allow (see [what's different](#whats-different)).

## Open it

Open the site where it's published (if this copy of Audio Constructor
has one), or build and serve it yourself:

```
python3 scripts/build_web.py --serve
```

and open <http://localhost:8000>. Building needs the Rust toolchain (see
the [README](../../README.md)) and wasm-bindgen's command line at the
version the build names; the script tells you the exact `cargo install`
line if it's missing.

The page opens the **Start** example. To open another example straight
away, add its file name to the address:
`?example=guitar-pedalboard`, `?example=subtractive-synth`.

**Click anywhere or press a key to start the sound.** Browsers keep a
page quiet until you do; a note at the top of the editor says so until
then.

The page needs a reasonably recent browser with audio worklets, shared
memory, and WebGL 2. If something's missing it says what, instead of
starting.

## What works as on the desktop

- **The editor**: everything in [The editor](editor.md) and
  [Building sounds](building-sounds.md): wiring, groups, blocks (the
  shipped ones are built in), panels and performance mode, bindings and
  learning, undo, Find, the light theme and UI scale (remembered by the
  browser).
- **Playing**: the transport, the computer keys (Play keys,
  `Ctrl+K`), the on-screen keyboard, live meters and scopes.
- **Examples**: File ▸ Examples lists every shipped example; each opens
  as a new, untitled copy, with its audio files.
- **MIDI keyboards and controllers**, in browsers with Web MIDI: the
  browser asks once whether the page may use MIDI. Every MIDI input is
  heard, as on the desktop. MIDI Out nodes send to the outputs you check
  in the **Devices** tab.
- **Gamepads**: press a button on the gamepad once (browsers show a
  gamepad to a page only after a press); then it works like on the
  desktop, as `pad 1`, `pad 2`, ….
- **Audio In**: the microphone, or an audio interface's input, once you
  allow it. A patch with an Audio In node (the Guitar Pedalboard) asks
  for it when it opens; the Devices tab has **Use the microphone** too.
  The browser's echo cancellation, noise suppression, and automatic gain
  are turned off, as an instrument wants. Which input is used is the
  browser's choice: pick another in its site settings or the system's.

## Opening and saving

The File menu works with files on your computer through the browser:

- **Open** (`Ctrl+O`) opens a project (`.acproj`), a pack (`.acpack`),
  or a block file (`.acblock`, into your library).
- **Save** (`Ctrl+S`) and **Save As**. In browsers that let a page write
  files, Save writes back to the file you opened or saved last, and Save
  As asks where. Elsewhere, both download the project.
- **Export WAV** renders the project (the editor waits while it does) and
  saves or downloads the WAV, exactly what `audioc render` makes.
- **Export Pack** saves the project and its audio files in one file.

**Audio files** are the one thing to keep in mind. A page can't look
next to a project file for its `assets/` folder, so:

- A project opened on its own shows its audio files as missing: locate
  them in the Project tab (each one you pick is kept for this tab), or
- open the project's **pack** instead, which carries them.
- Files you pick for a Clip are kept in the page while it's open. Saving
  the project alone doesn't save them: use **Export Pack** to keep the
  project with its sound. The status line reminds you when you save a
  project that has audio files.

There's no Open Recent in the browser.

## What's different

- **No OSC.** Browsers can't use UDP, so OSC In and OSC Out nodes stay
  quiet (a patch that uses them says so). Use the desktop editor or
  `audioc run` for OSC.
- **No recording to disk.** Record nodes can't write takes. Render with
  Export WAV instead.
- **No plugins.** Plugin nodes can't load in a browser. A project that
  has one opens with the node and its wires in place; the node plays
  silence, and the Problems tab says plugins can't run in a browser.
- **The output is the system's.** The browser plays through the output
  your system uses; change it in the system's sound settings. There's no
  latency measurement, and the CPU load and per-node timing read 0
  (the page can't time the audio thread).
- **Baking pauses the editor.** A Reverse or Normalize node fed from a
  Clip bakes in the page, so the editor stops for a moment while it does
  (on the desktop it bakes in the background).
- **Latency** is the browser's: usually a little more than the desktop
  editor at a moderate buffer. MIDI and keys land at the start of the
  next 128-sample block.
- **Leaving the page** with unsaved changes asks first, like closing the
  desktop editor's window.

## Putting it on a website

The site is a folder of static files (`target/web/` after the build). Any
static host can serve it, over https. The page shares memory with its
audio engine, which browsers allow only on a *cross-origin isolated*
page:

- Hosts that read a `_headers` file (it's in the folder) send the two
  headers that need (`Cross-Origin-Opener-Policy: same-origin` and
  `Cross-Origin-Embedder-Policy: require-corp`). Elsewhere, set those two
  headers in the host's settings.
- Where a host can't send headers at all, the site's own service worker
  adds them: the page reloads itself once on the first visit, and after
  that it's isolated.

Don't embed other sites' images, fonts, or scripts in the page: an
isolated page refuses what isn't served for it.

## Troubleshooting

- **"needs a cross-origin isolated page"**: the host isn't sending the
  headers, and the service worker isn't allowed (some private modes block
  service workers). See [Putting it on a website](#putting-it-on-a-website).
- **No sound**: click the page first. Then check the system's output and
  the transport, as in [Troubleshooting](troubleshooting.md#no-sound).
- **"The audio stopped"**: the audio engine hit something it couldn't
  recover from. Reload the page; your patch is lost unless you saved it,
  so save often. Please report it with the project.
- **The microphone is blocked**: allow it in the browser's site settings
  (the icon at the left of the address bar), then **Try again** in the
  Devices tab.
- **No MIDI**: the browser may not have Web MIDI, or MIDI was refused for
  the page; the Devices tab says which.
