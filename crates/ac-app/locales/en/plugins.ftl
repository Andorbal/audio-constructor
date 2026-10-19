# Plugins installed on this computer, hosted as nodes: the Node tab's
# Plugin section, the Devices tab's Plugins section, the Library tab, the
# palette, and the status line. Format names (CLAP, VST3) stay as they are.

## What a plugin is: "CLAP instrument", "VST3 effect"

plugin-kind-instrument = { $format } instrument
plugin-kind-effect = { $format } effect
# The palette's and the Library tab's line under a plugin's name. $kind is
# one of the two above.
plugin-summary = A { $kind }, installed on this computer.
plugin-summary-by = A { $kind } by { $vendor }, installed on this computer.

## The Node tab, for a Plugin node

section-plugin = Plugin
plugin-none-chosen = No plugin chosen. Add one from the palette (Shift+A) or the Library tab, where the plugins installed on this computer are listed by name.
plugin-name = Name
plugin-vendor = Vendor
plugin-kind = Kind
plugin-version = Version
plugin-id = Id
plugin-found-at = Found at
# Under the reason a plugin can't run (it isn't installed, or failed to load).
plugin-missing-note = Its sockets and wires stay, so the project works again on a computer that has it. Installed it just now? Rescan in the Devices tab.
plugin-running = Running
plugin-not-hosted = Plugins don't run in this editor.
# The Node tab's button and the node's right-click menu.
plugin-show-window = Show Plugin Window
plugin-show-window-tip = The plugin's own interface, in a window of its own
plugin-no-window-tip = This plugin has no window of its own
plugin-not-running-tip = The plugin isn't running
plugin-params-note = Its parameters are the inputs above: moving one in the plugin's window moves it here too. Saving the project saves everything the plugin keeps.
# The same note for a Plugin node inside a group.
plugin-params-note-in-group = Its parameters are the inputs above: moving one in the plugin's window moves it here too. Inside a group the plugin's parameters are saved, but not the rest of its state (every instance of the group shares the node).

## The Devices tab

section-plugins = Plugins
# While plugins are being looked for (also in the status line).
plugins-looking = Looking for plugins…
# How many plugins were found, and of which format.
plugins-found = { $count } found: { $clap } CLAP, { $vst3 } VST3
# Where a format's plugins are looked for: "CLAP folders: ~/.clap, /usr/lib/clap".
plugins-folders = { $format } folders: { $folders }
plugins-rescan-tip = Look for plugins installed or changed since the last scan
plugins-rescan-all = Rescan all
plugins-rescan-all-tip = Open every plugin again, changed or not
# A list that opens to show each bundle that couldn't be opened, and why.
plugins-failed =
    { $count ->
        [one] { $count } couldn't be opened
       *[other] { $count } couldn't be opened
    }
plugins-separate-process = Each plugin is opened in a separate process while it's looked at, so one that crashes is listed here instead of closing the editor.

## The Library tab

library-plugins = Plugins
library-no-plugins = No plugins found yet. Install CLAP or VST3 plugins, then rescan in the Devices tab.

## The status line

status-plugins-found =
    { $count ->
        [one] Found { $count } plugin
       *[other] Found { $count } plugins
    }
status-plugins-found-some-failed =
    { $count ->
        [one] Found { $count } plugin ({ $failed } couldn't be opened: see the Devices tab)
       *[other] Found { $count } plugins ({ $failed } couldn't be opened: see the Devices tab)
    }
status-plugin-window-failed = Couldn't open the plugin's window: { $error }
