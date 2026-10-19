//! `audioc nodes`: list the registry and generate the node reference.

use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use ac_core::{Category, NodeEntry, Registry, SignalType, SocketSpec, Value, empty_config};
use anyhow::{Context, Result};

pub fn nodes(json: bool, docs: Option<&Path>) -> Result<ExitCode> {
    let registry = ac_nodes::registry();
    if let Some(dir) = docs {
        write_docs(&registry, dir)?;
        println!(
            "wrote node reference for {} nodes to {}",
            registry.len(),
            dir.display()
        );
        return Ok(ExitCode::SUCCESS);
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&registry_json(&registry))?
        );
        return Ok(ExitCode::SUCCESS);
    }
    for cat in Category::ALL {
        let entries: Vec<&NodeEntry> = sorted(&registry, cat);
        if entries.is_empty() {
            continue;
        }
        println!("{}", cat.name());
        for e in entries {
            println!(
                "  {:<16} {:<16} {}",
                e.meta.kind,
                e.meta.name,
                e.meta.summary()
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn sorted(registry: &Registry, cat: Category) -> Vec<&NodeEntry> {
    let mut v: Vec<&NodeEntry> = registry.iter().filter(|e| e.meta.category == cat).collect();
    v.sort_by_key(|e| e.meta.name);
    v
}

fn socket_json(s: &SocketSpec) -> serde_json::Value {
    serde_json::json!({
        "name": s.name,
        "type": s.ty,
        "unit": s.unit,
        "default": s.default,
        "range": s.range.map(|(a, b)| [a, b]),
        "choices": s.choices,
        "doc": s.doc,
    })
}

pub fn registry_json(registry: &Registry) -> serde_json::Value {
    let nodes: Vec<serde_json::Value> = registry
        .iter()
        .map(|e| {
            let sig = (e.describe)(&empty_config());
            serde_json::json!({
                "kind": e.meta.kind,
                "name": e.meta.name,
                "category": e.meta.category.slug(),
                "keywords": e.meta.keywords,
                "summary": e.meta.summary(),
                "doc": e.meta.doc_text(),
                "live": e.meta.live,
                "whole_input": e.meta.whole_input,
                "inputs": sig.inputs.iter().filter(|s| !s.hidden).map(socket_json).collect::<Vec<_>>(),
                "outputs": sig.outputs.iter().filter(|s| !s.hidden).map(socket_json).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::Value::Array(nodes)
}

fn fmt_value(v: &Value, s: &SocketSpec) -> String {
    match (v, s.ty) {
        (_, SignalType::Audio | SignalType::Midi | SignalType::Spectrum | SignalType::Buffer) => {
            "—".into()
        }
        (_, SignalType::Trigger) if s.choices.is_empty() => "—".into(),
        (Value::Text(t), _) => format!("`{t}`"),
        (Value::Float(f), _) => format!("{f}"),
        (other, _) => format!("{other:?}"),
    }
}

fn type_label(s: &SocketSpec) -> String {
    let unit = s.unit.suffix();
    if s.ty == SignalType::Float && !unit.is_empty() {
        format!("Float ({unit})")
    } else {
        s.ty.name().to_string()
    }
}

fn socket_table(out: &mut String, title: &str, sockets: &[SocketSpec], inputs: bool) {
    let sockets: Vec<&SocketSpec> = sockets.iter().filter(|s| !s.hidden).collect();
    if sockets.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n**{title}**\n");
    if inputs {
        let _ = writeln!(
            out,
            "| Socket | Type | Default | Range / choices | Description |"
        );
        let _ = writeln!(out, "| --- | --- | --- | --- | --- |");
    } else {
        let _ = writeln!(out, "| Socket | Type | Description |");
        let _ = writeln!(out, "| --- | --- | --- |");
    }
    for s in sockets {
        let doc = s.doc.replace('|', "\\|");
        if inputs {
            let range = if !s.choices.is_empty() {
                s.choices
                    .iter()
                    .map(|c| format!("`{c}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                s.range
                    .map(|(a, b)| format!("{a} … {b}"))
                    .unwrap_or_default()
            };
            let _ = writeln!(
                out,
                "| `{}` | {} | {} | {} | {} |",
                s.name,
                type_label(s),
                fmt_value(&s.default, s),
                range,
                doc
            );
        } else {
            let _ = writeln!(out, "| `{}` | {} | {} |", s.name, type_label(s), doc);
        }
    }
}

/// What a category holds, in a sentence, for the reference index and the
/// top of its page.
fn blurb(cat: Category) -> &'static str {
    match cat {
        Category::Io => {
            "Sound, MIDI, and controls coming into and going out of the patch: audio \
             devices, MIDI ports, gamepads, the computer keyboard, OSC, and files."
        }
        Category::Generators => {
            "Sources of signal over time: oscillators, noise, audio files, samplers, \
             drawn curves, and the song position."
        }
        Category::Level => {
            "Volume, panning, and mixing, and the parts that compressors, limiters, and \
             gates are built from."
        }
        Category::Filters => {
            "Filters for shaping tone, for synth sounds, and for building reverbs and \
             resonators."
        }
        Category::Time => {
            "Effects that work with time: delays, reversing, freezing, pitch shifting, \
             and diffusion."
        }
        Category::Spectral => {
            "Work in frequencies instead of samples: transforms to and from spectra, \
             band splitting, spectral gating and shifting, and pitch detection."
        }
        Category::Math => {
            "Arithmetic, comparisons, unit conversions, smoothing, randomness, and \
             routing, for numbers and gates."
        }
        Category::Modulation => {
            "Envelopes, LFOs, step sequencers, and random movement, for moving other \
             nodes' inputs over time."
        }
        Category::Triggers => {
            "Clocks and rhythms, and tools that make, delay, count, filter, and reshape \
             triggers and gates."
        }
        Category::Sequencing => {
            "Notes and MIDI: turning MIDI into signals and back, patterns, piano rolls, \
             arpeggios, chords, scales, and polyphony."
        }
        Category::Controls => {
            "Widgets you play by hand, on the canvas or on a performance panel: knobs, \
             sliders, buttons, pads, and a keyboard."
        }
        Category::Display => {
            "Meters, scopes, readouts, and analyzers that show what a signal is doing."
        }
        Category::Utility => {
            "Wiring helpers and odds and ends: feedback loops, reroutes, groups, channel \
             tools, notes on the canvas, and the engine's vital signs."
        }
    }
}

/// The summary a node gets in the index, with its live and whole-input
/// marks (the things that decide whether it bakes or switches off).
fn index_summary(m: &ac_core::NodeMeta) -> String {
    let mut s = m.summary().replace('|', "\\|");
    if m.live {
        s.push_str(" *Live source.*");
    }
    if m.whole_input {
        s.push_str(" *Needs its whole input: baked, or off while live.*");
    }
    s
}

/// Write `README.md` (index) and one page per category.
pub fn write_docs(registry: &Registry, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut index = String::new();
    let _ = writeln!(index, "# Node reference\n");
    let _ = writeln!(
        index,
        "Generated by `audioc nodes --docs docs/reference` from the doc comment of each\nnode. Edit the node's source, not these files.\n"
    );
    let _ = writeln!(
        index,
        "Every node that ships with Audio Constructor, by category: what it does, a\n\"Try this\" tip, and every socket with its type, default, and range. The same\ntext is in the editor's Help tab (`F1` on a node). For how the nodes fit\ntogether, read the [user guide](../guide/README.md).\n"
    );
    let cats: Vec<(Category, Vec<&NodeEntry>)> = Category::ALL
        .into_iter()
        .map(|cat| (cat, sorted(registry, cat)))
        .filter(|(_, entries)| !entries.is_empty())
        .collect();
    let total: usize = cats.iter().map(|(_, e)| e.len()).sum();
    let _ = writeln!(index, "## Categories\n");
    let _ = writeln!(index, "{total} nodes in {} categories.\n", cats.len());
    let _ = writeln!(index, "| Category | Nodes | What's in it |");
    let _ = writeln!(index, "| --- | --- | --- |");
    for (cat, entries) in &cats {
        let _ = writeln!(
            index,
            "| [{}](#{}) | {} | {} |",
            cat.name(),
            cat.slug(),
            entries.len(),
            blurb(*cat)
        );
    }
    for (cat, entries) in &cats {
        let cat = *cat;
        let _ = writeln!(
            index,
            "\n<a id=\"{}\"></a>\n## {}\n",
            cat.slug(),
            cat.name()
        );
        let _ = writeln!(
            index,
            "{} All on one page: [{}]({}.md).\n",
            blurb(cat),
            cat.name(),
            cat.slug()
        );
        let _ = writeln!(index, "| Node | What it does |");
        let _ = writeln!(index, "| --- | --- |");
        let mut page = String::new();
        let _ = writeln!(page, "# {}\n", cat.name());
        let _ = writeln!(page, "[← All nodes](README.md)\n");
        let _ = writeln!(page, "{}\n", blurb(cat));
        for e in entries {
            let m = &e.meta;
            let anchor = m.kind.replace('_', "-");
            let _ = writeln!(
                index,
                "| [{}]({}.md#{}) | {} |",
                m.name,
                cat.slug(),
                anchor,
                index_summary(m)
            );
            let _ = writeln!(page, "<a id=\"{anchor}\"></a>\n## {}\n", m.name);
            let mut tags = format!("Kind: `{}`", m.kind);
            if m.live {
                tags.push_str(" · live source");
            }
            if m.whole_input {
                tags.push_str(" · whole input (baked)");
            }
            if !m.keywords.is_empty() {
                let _ = write!(tags, " · Also found by: {}", m.keywords.join(", "));
            }
            let _ = writeln!(page, "{tags}\n");
            let _ = writeln!(page, "{}", demote_headings(&m.doc_text()));
            let sig = (e.describe)(&empty_config());
            socket_table(&mut page, "Inputs", &sig.inputs, true);
            socket_table(&mut page, "Outputs", &sig.outputs, false);
            let _ = writeln!(page);
        }
        let path = dir.join(format!("{}.md", cat.slug()));
        std::fs::write(&path, page).with_context(|| format!("writing {}", path.display()))?;
    }
    let path = dir.join("README.md");
    std::fs::write(&path, index).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Keep node docs below the page's `##` node headings.
fn demote_headings(doc: &str) -> String {
    doc.lines()
        .map(|l| {
            if l.starts_with('#') {
                format!("##{l}")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed reference must match what the nodes' doc comments say.
    /// Regenerate with `cargo run -p ac-cli -- nodes --docs docs/reference`.
    #[test]
    fn committed_reference_is_up_to_date() {
        let dir = std::env::temp_dir().join(format!("ac-docs-{}", std::process::id()));
        write_docs(&ac_nodes::registry(), &dir).unwrap();
        let committed = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference");
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let fresh = std::fs::read_to_string(entry.path()).unwrap();
            let name = entry.file_name();
            let old = std::fs::read_to_string(committed.join(&name)).unwrap_or_default();
            assert!(
                fresh == old,
                "docs/reference/{} is stale; run `cargo run -p ac-cli -- nodes --docs docs/reference`",
                name.to_string_lossy()
            );
        }
        std::fs::remove_dir_all(dir).ok();
    }
}
