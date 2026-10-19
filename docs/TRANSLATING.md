# Translating

The editor's own words (its menus, panel and tab titles, the transport
bar, dialogs, the status line, and the help and settings chrome) come from
[Fluent](https://projectfluent.org) catalogs, one folder per language:

```
crates/ac-app/locales/
  en/           English, the source every language translates
    app.ftl     status messages, the window, the find bar and palette, themes
    canvas.ftl  right-click menus, socket tooltips, badges, hints
    dialogs.ftl file prompts, unsaved changes, shortcuts, About
    menus.ftl   the menu bar and every command's name
    panels.ftl  the side panel's tabs, and the node editors
    perform.ftl the panels and performance mode
    plugins.ftl plugins on this computer: the Node, Devices, and Library tabs
    product.ftl the update check and crash reports
    transport.ftl
    undo.ftl    what Edit ▸ Undo names each step
```

Not translated yet, and so left out of the catalogs: node names, node
documentation and the Help tab's text about each node, socket names and
types, the settings each node lists in the Node tab, value units, and
anything in a project (names, notes, panel and frame titles). Those come
with the nodes and the projects themselves; translating them is future
work. Plugin format names (CLAP, VST3) and what a plugin says about
itself (its name, vendor, and why it failed to load) stay as they are,
and so do the details of errors the file and device code reports.

## Adding a language

1. Copy `crates/ac-app/locales/en/` to a folder named after the language's
   tag, such as `locales/de/`, and translate each message's text. Keep the
   message ids (left of `=`) and the `{ $name }` placeables as they are;
   plurals use Fluent's selectors:

   ```ftl
   status-pasted =
       { $count ->
           [one] { $count } Knoten eingefügt
          *[other] { $count } Knoten eingefügt
       }
   ```

   A message left out falls back to English, so a catalog can be
   translated a file at a time.
2. In `crates/ac-app/src/i18n.rs`, add the language to `Language`, its tag
   to `Language::id` and `Language::for_locale` (so View ▸ Language ▸
   System picks it on systems set to it), its catalog next to `ENGLISH`
   (embedded with `include_str!`, so the browser build needs no files),
   a bundle for it in `bundles()`, an entry in `LanguageChoice` with its
   name (`language-…` in `app.ftl`, written in the language itself), and
   extend the catalog tests in `crates/ac-app/tests/catalogs.rs` to it.
3. Run `cargo test -p ac-app`. The catalog tests check that every file
   parses, that no message is defined twice, that every id the code asks
   for exists in English, and that English has no message nothing asks
   for.

## Finding text that isn't translated

View ▸ Language ▸ **Pseudo** shows a made-up language generated from
English: every letter accented, each message about a third longer, and
wrapped in brackets, `[Åďď Ñöďé…··]`. Text without brackets didn't come
from a catalog, and text that's cut off or overflows shows where a longer
language won't fit. A test opens every menu in Pseudo and fails on any
label without brackets.

## Adding a message

In code, ask for a message by its id, always as a string literal (the
catalog tests find ids by reading the source, so an id built at run time
would hide from them):

```rust
use crate::i18n::{tr, tr_args};

ui.button(tr("menu-reload-themes"));
self.say(tr_args("status-pasted", &[("count", ids.len().into())]), now);
```

`tr_args` takes text (`&str`, `String`) and counts (integers, which pick
plural forms). Format other numbers yourself (`format!("{:.1}", db)`) and
pass them as text, so precision and units stay where they're decided.
Then add the message to the right English file, with a comment above it
when its meaning or placeables aren't obvious from the text.
