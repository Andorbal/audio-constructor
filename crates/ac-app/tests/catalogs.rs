//! The translation catalogs (docs/TRANSLATING.md): they parse cleanly,
//! every message the editor's code asks for exists in English, English
//! has no message nothing asks for, and no new text skips the catalogs.
//!
//! Message ids are found in the source as the first argument of `tr(…)`
//! and `tr_args(…)`, always a string literal: an id built at run time
//! would hide from this check, so the editor never builds one. The source
//! read is ac-app's and the web page's (`ac-web`), whose Devices section
//! and notices use the same catalogs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use ac_app::i18n;
use fluent_bundle::FluentResource;
use fluent_bundle::concurrent::FluentBundle;
use fluent_syntax::ast;

/// The source folders read, with the name each file is shown under.
fn src_dirs() -> [(&'static str, PathBuf); 2] {
    let app = Path::new(env!("CARGO_MANIFEST_DIR"));
    [
        ("ac-app/src", app.join("src")),
        ("ac-web/src", app.join("../ac-web/src")),
    ]
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// `text` without its `#[cfg(test)]` modules (unit tests ask for made-up
/// ids and show made-up text on purpose), as blank lines so line numbers
/// stay; and the files of the test modules it declares
/// (`#[cfg(test)] mod tests;`).
fn without_tests(text: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(text.len());
    let mut files = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        if line != "#[cfg(test)]" {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        out.push('\n');
        let Some(item) = lines.next() else { break };
        let decl = item
            .trim_start_matches("pub(crate) ")
            .trim_start_matches("pub ");
        if let Some(name) = decl.strip_prefix("mod ").and_then(|d| d.strip_suffix(';')) {
            files.push(format!("{name}.rs"));
            out.push('\n');
        } else if decl.starts_with("mod ") {
            out.push('\n');
            // rustfmt closes a top-level module with `}` alone on a line.
            for l in lines.by_ref() {
                out.push('\n');
                if l == "}" {
                    break;
                }
            }
        } else {
            out.push_str(item);
            out.push('\n');
        }
    }
    (out, files)
}

/// The editor's source without its tests, file by file: the name to show
/// and the text.
fn sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (label, dir) in src_dirs() {
        let mut files = Vec::new();
        rust_files(&dir, &mut files);
        files.sort();
        let mut test_files: BTreeSet<PathBuf> = BTreeSet::new();
        let mut kept = Vec::new();
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            let (text, tests) = without_tests(&text);
            // `edit/mod.rs` declares `edit/tests.rs`; `app.rs`, `app/….rs`.
            let base = match f.file_name().and_then(|n| n.to_str()) {
                Some("mod.rs" | "lib.rs" | "main.rs") => f.parent().map(Path::to_path_buf),
                _ => f.parent().zip(f.file_stem()).map(|(p, s)| p.join(s)),
            };
            if let Some(base) = base {
                test_files.extend(tests.iter().map(|t| base.join(t)));
            }
            kept.push((f, text));
        }
        for (f, text) in kept {
            if test_files.contains(&f) {
                continue;
            }
            let rel = f.strip_prefix(&dir).unwrap_or(&f).display().to_string();
            out.push((format!("{label}/{rel}"), text));
        }
    }
    out
}

/// Every message id the code asks for, with where.
fn ids_in_code() -> BTreeMap<String, Vec<String>> {
    let mut ids: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, text) in sources() {
        for id in calls(&text) {
            ids.entry(id).or_default().push(name.clone());
        }
    }
    ids
}

/// The string literals right after `tr(` and `tr_args(` in `text`.
fn calls(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for pat in ["tr(", "tr_args("] {
        let mut from = 0;
        while let Some(at) = text[from..].find(pat) {
            let start = from + at;
            from = start + pat.len();
            // `tr(` inside another name (`attr(`, `str(`) isn't a call.
            let before = text[..start].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            let rest = text[from..].trim_start();
            let Some(lit) = rest.strip_prefix('"') else {
                continue;
            };
            if let Some(end) = lit.find('"') {
                out.push(lit[..end].to_string());
            }
        }
    }
    out
}

fn english() -> Vec<(&'static str, FluentResource)> {
    i18n::ENGLISH
        .iter()
        .map(|(name, text)| {
            let r = FluentResource::try_new((*text).to_string())
                .unwrap_or_else(|(_, e)| panic!("locales/en/{name} doesn't parse: {e:?}"));
            (*name, r)
        })
        .collect()
}

/// Every message id in the English catalog, with its file.
fn ids_in_catalog() -> BTreeMap<String, &'static str> {
    let mut ids = BTreeMap::new();
    for (name, r) in english() {
        for entry in r.entries() {
            if let ast::Entry::Message(m) = entry {
                let id = m.id.name.to_string();
                assert!(
                    ids.insert(id.clone(), name).is_none(),
                    "{id} is defined twice (again in {name})"
                );
            }
        }
    }
    ids
}

#[test]
fn the_catalogs_parse_cleanly() {
    let resources = english();
    assert!(!resources.is_empty());
    let mut bundle = FluentBundle::new_concurrent(vec!["en".parse().unwrap()]);
    for (name, r) in &resources {
        if let Err(e) = bundle.add_resource(r) {
            panic!("locales/en/{name}: {e:?}");
        }
    }
    // Every file on disk is embedded, and every embedded file is on disk.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("locales/en");
    let on_disk: BTreeSet<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".ftl"))
        .collect();
    let embedded: BTreeSet<String> = i18n::ENGLISH.iter().map(|(n, _)| n.to_string()).collect();
    assert_eq!(on_disk, embedded, "i18n::ENGLISH lists every .ftl file");
    // Every message has a value (an id with only attributes can't be shown).
    for (_, r) in &resources {
        for entry in r.entries() {
            if let ast::Entry::Message(m) = entry {
                assert!(m.value.is_some(), "{} has no text", m.id.name);
            }
        }
    }
}

#[test]
fn every_message_the_code_asks_for_is_in_english() {
    let catalog = ids_in_catalog();
    let code = ids_in_code();
    assert!(code.len() > 50, "the scan finds the editor's messages");
    let missing: Vec<String> = code
        .iter()
        .filter(|(id, _)| !catalog.contains_key(*id))
        .map(|(id, files)| format!("{id} (in {})", files.join(", ")))
        .collect();
    assert!(
        missing.is_empty(),
        "not in locales/en:\n{}",
        missing.join("\n")
    );
    for id in code.keys() {
        assert!(i18n::has_message(id), "{id}");
    }
}

#[test]
fn english_has_no_unused_messages() {
    let code = ids_in_code();
    let unused: Vec<String> = ids_in_catalog()
        .into_iter()
        .filter(|(id, _)| !code.contains_key(id))
        .map(|(id, file)| format!("{id} (in {file})"))
        .collect();
    assert!(
        unused.is_empty(),
        "nothing asks for these:\n{}",
        unused.join("\n")
    );
}

#[test]
fn the_scan_finds_calls_and_skips_lookalikes() {
    let found = calls(
        r#"tr("a-b") + &i18n::tr_args(
            "c-d", &[]) + attr("x-y") + str("z") + tr(name)"#,
    );
    assert_eq!(found, ["a-b", "c-d"]);
}

// ---------------------------------------------------------------- the guard

/// What counts as untranslated in a guarded call's text argument.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Rule {
    /// Any literal: the status line only ever shows messages.
    Any,
    /// A literal with a letter in it: glyphs, numbers, and `{}` alone
    /// aren't words.
    Words,
}

/// Calls whose text a person reads, and the rule for the string literal
/// (or `format!` of one) they're given. `button(` covers `small_button(`.
const GUARDED: &[(&str, Rule)] = &[
    ("say(", Rule::Any),
    ("warn(", Rule::Any),
    ("notify(", Rule::Any),
    (".label(", Rule::Words),
    ("RichText::new(", Rule::Words),
    ("Label::new(", Rule::Words),
    ("button(", Rule::Words),
    ("Button::new(", Rule::Words),
    ("on_hover_text(", Rule::Words),
    ("on_disabled_hover_text(", Rule::Words),
    ("heading(", Rule::Words),
    ("hint_text(", Rule::Words),
    ("section(ui,", Rule::Words),
];

/// Literals the guard lets through, by file and text, each with why. A
/// stale entry fails the test too, so the list stays short and true.
const ALLOWED: &[(&str, &str)] = &[
    // A theme file's name and its first note (the note is translated in
    // user_themes.rs): only punctuation is ours, as with every
    // "file: problem" line.
    ("ac-app/src/app.rs", "{}: {note}"),
    // Two messages joined by the status line's separator.
    ("ac-app/src/app/sharing.rs", "{text} · {}"),
    // The shape of a link the Import from Link field takes, not words.
    ("ac-app/src/app/sharing.rs", "https://…"),
    // The About box's title: the product's name, which isn't translated.
    ("ac-app/src/dialogs.rs", "{}  Audio Constructor"),
];

/// An untranslated literal: where, which call, and its text.
#[derive(Debug, PartialEq)]
struct Hit {
    line: usize,
    call: &'static str,
    text: String,
}

/// The guarded calls in `text` given untranslated literals.
fn untranslated(text: &str) -> Vec<Hit> {
    let mut out = Vec::new();
    for &(call, rule) in GUARDED {
        let mut from = 0;
        while let Some(at) = text[from..].find(call) {
            let start = from + at;
            from = start + call.len();
            // `say(` inside another name (`essay(`) isn't the call.
            let before = text[..start].chars().next_back();
            if call.starts_with(char::is_alphanumeric) && before.is_some_and(char::is_alphanumeric)
            {
                continue;
            }
            let Some((value, formatted)) = first_literal(&text[from..]) else {
                continue;
            };
            let words = if formatted {
                without_placeholders(&value)
            } else {
                value.clone()
            };
            if rule == Rule::Any || words.chars().any(char::is_alphabetic) {
                out.push(Hit {
                    line: text[..start].matches('\n').count() + 1,
                    call,
                    text: value,
                });
            }
        }
    }
    out.sort_by_key(|h| h.line);
    out
}

/// The string literal an argument list starts with, directly or as
/// `format!("…")` (then `true`), with escapes decoded and Unicode escapes
/// (icons) dropped.
fn first_literal(args: &str) -> Option<(String, bool)> {
    let rest = args.trim_start();
    let rest = rest.strip_prefix('&').unwrap_or(rest).trim_start();
    let (rest, formatted) = match rest.strip_prefix("format!(") {
        Some(r) => (r.trim_start(), true),
        None => (rest, false),
    };
    let mut chars = rest.strip_prefix('"')?.chars().peekable();
    let mut value = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some((value, formatted)),
            '\\' => match chars.next()? {
                // A Unicode escape is an icon here, not a word.
                'u' => while chars.next().is_some_and(|c| c != '}') {},
                // A continued line: the next line's indent isn't text.
                '\n' => while chars.next_if(|c| c.is_whitespace()).is_some() {},
                'n' => value.push('\n'),
                't' => value.push('\t'),
                other => value.push(other),
            },
            _ => value.push(c),
        }
    }
    None
}

/// A format string without its `{…}` placeholders.
fn without_placeholders(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

#[test]
fn no_text_a_person_reads_skips_the_catalogs() {
    let mut found = Vec::new();
    let mut allowed_hits = BTreeSet::new();
    for (name, text) in sources() {
        for hit in untranslated(&text) {
            let allowed = ALLOWED
                .iter()
                .position(|(file, lit)| *file == name && *lit == hit.text);
            match allowed {
                Some(i) => {
                    allowed_hits.insert(i);
                }
                None => found.push(format!("{name}:{}: {}\"{}\"", hit.line, hit.call, hit.text)),
            }
        }
    }
    assert!(
        found.is_empty(),
        "text that doesn't come from the catalogs (docs/TRANSLATING.md: add a \
         message and use tr or tr_args):\n{}",
        found.join("\n")
    );
    let stale: Vec<String> = ALLOWED
        .iter()
        .enumerate()
        .filter(|(i, _)| !allowed_hits.contains(i))
        .map(|(_, (file, lit))| format!("{file}: \"{lit}\""))
        .collect();
    assert!(
        stale.is_empty(),
        "allowed but no longer there (remove from ALLOWED):\n{}",
        stale.join("\n")
    );
}

#[test]
fn the_guard_finds_literals_and_skips_messages() {
    let code = r#"
        self.say("Saved", now);
        self.warn(format!("{}: {e}", file), now);
        ui.label(tr("menu-file"));
        ui.label("Plugins");
        ui.label(RichText::new("Running").small());
        ui.small_button(format!("{}  Rescan", icons::X));
        ui.label(format!("  {name}"));
        ui.label("·");
        ui.add(egui::Button::new("\u{E5DA}"));
        r.on_hover_text(
            "Two lines, \
             one text",
        );
        essay("not a call");
    "#;
    let hits: Vec<(usize, &str, String)> = untranslated(code)
        .into_iter()
        .map(|h| (h.line, h.call, h.text))
        .collect();
    assert_eq!(
        hits,
        [
            (2, "say(", "Saved".to_string()),
            (3, "warn(", "{}: {e}".to_string()),
            (5, ".label(", "Plugins".to_string()),
            (6, "RichText::new(", "Running".to_string()),
            (7, "button(", "{}  Rescan".to_string()),
            (11, "on_hover_text(", "Two lines, one text".to_string()),
        ]
    );
    // Test modules are left out, and keep the line numbers after them.
    let (kept, files) = without_tests(
        "a\n#[cfg(test)]\nmod tests {\n    say(\"x\");\n}\nb\n#[cfg(test)]\nmod more;\n",
    );
    assert_eq!(kept.lines().nth(5), Some("b"));
    assert!(!kept.contains("say"));
    assert_eq!(files, ["more.rs"]);
}
