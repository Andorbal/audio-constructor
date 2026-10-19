//! The translation catalogs (docs/TRANSLATING.md): they parse cleanly,
//! every message the editor's code asks for exists in English, and English
//! has no message nothing asks for.
//!
//! Message ids are found in the source as the first argument of `tr(…)`
//! and `tr_args(…)`, always a string literal: an id built at run time
//! would hide from this check, so the editor never builds one.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use ac_app::i18n;
use fluent_bundle::FluentResource;
use fluent_bundle::concurrent::FluentBundle;
use fluent_syntax::ast;

fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
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

/// Every message id the code asks for, with where.
fn ids_in_code() -> BTreeMap<String, Vec<String>> {
    let mut files = Vec::new();
    rust_files(&src_dir(), &mut files);
    let mut ids: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        // Unit tests (at the end of a file) ask for made-up ids on purpose.
        let text = text
            .split("#[cfg(test)]\nmod tests")
            .next()
            .unwrap_or_default();
        let name = f
            .strip_prefix(src_dir())
            .unwrap_or(&f)
            .display()
            .to_string();
        for id in calls(text) {
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
