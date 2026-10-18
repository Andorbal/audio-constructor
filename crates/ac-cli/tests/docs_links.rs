//! The documentation holds together: every relative link in the user
//! guide, the node reference, and the other Markdown files points at a
//! file in the repository (and `page.md#anchor` at a heading or an
//! `<a id>` on that page), every guide chapter is in the guide's index,
//! and the examples tour has an entry for every example project, so a new
//! example without one fails here.
//!
//! Everything is read from disk when the test runs, so examples and pages
//! added later are checked too.

use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The files in `dir` (relative to the root) with extension `ext`, sorted.
fn files_in(dir: &str, ext: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(root().join(dir))
        .unwrap_or_else(|e| panic!("reading {dir}: {e}"))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == ext))
        .collect();
    out.sort();
    out
}

/// Every Markdown page whose links are checked.
fn pages() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = [
        "README.md",
        "CHANGELOG.md",
        "blocks/README.md",
        "bench/README.md",
    ]
    .iter()
    .map(|p| root().join(p))
    .collect();
    for dir in ["docs", "docs/guide", "docs/reference"] {
        out.extend(files_in(dir, "md"));
    }
    out
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// `path` relative to the repository root, for messages.
fn shown(path: &Path) -> String {
    path.strip_prefix(root())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// The lines of `text` outside fenced code blocks, with inline code spans
/// taken out, so examples of syntax aren't read as links or headings.
fn prose(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let mut kept = String::new();
        let mut in_code = false;
        for c in line.chars() {
            if c == '`' {
                in_code = !in_code;
            } else if !in_code {
                kept.push(c);
            }
        }
        out.push(kept);
    }
    out
}

/// The targets of the inline links and images in `text`: the `url` of
/// `[label](url)` and `![alt](url "title")`.
fn links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in prose(text) {
        let mut rest = line.as_str();
        while let Some(at) = rest.find("](") {
            let after = &rest[at + 2..];
            let mut depth = 0;
            let mut end = after.len();
            for (i, c) in after.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' if depth == 0 => {
                        end = i;
                        break;
                    }
                    ')' => depth -= 1,
                    _ => {}
                }
            }
            let target = after[..end].split_whitespace().next().unwrap_or("");
            let target = target.trim_start_matches('<').trim_end_matches('>');
            if !target.is_empty() {
                out.push(target.to_string());
            }
            rest = &after[end.min(after.len())..];
        }
    }
    out
}

/// The anchor a heading gets on the repository's host: lowercase, with
/// everything but letters, digits, spaces, `-`, and `_` dropped and
/// spaces turned into `-`.
fn slug(heading: &str) -> String {
    // Link text stays; the URL of a link in a heading doesn't.
    let mut text = String::new();
    let mut rest = heading;
    while let Some(open) = rest.find('[') {
        text.push_str(&rest[..open]);
        let tail = &rest[open + 1..];
        match (tail.find("]("), tail.find(')')) {
            (Some(close), Some(paren)) if close < paren => {
                text.push_str(&tail[..close]);
                rest = &tail[paren + 1..];
            }
            _ => {
                text.push('[');
                rest = tail;
            }
        }
    }
    text.push_str(rest);
    text.trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// The anchors a page offers: its headings (repeats numbered `-1`, `-2`,
/// as the host does) and its `<a id="…">` tags.
fn anchors(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
            let base = slug(&line[hashes..]);
            let n = seen.entry(base.clone()).or_insert(0);
            out.insert(if *n == 0 {
                base.clone()
            } else {
                format!("{base}-{n}")
            });
            *n += 1;
        }
        let mut rest = line;
        while let Some(at) = rest.find("<a id=\"") {
            let tail = &rest[at + 7..];
            if let Some(end) = tail.find('"') {
                out.insert(tail[..end].to_string());
            }
            rest = tail;
        }
    }
    out
}

/// `base` joined with a relative `link`, with `.` and `..` worked out, or
/// `None` if it climbs out of the repository.
fn resolve(base: &Path, link: &str) -> Option<PathBuf> {
    let mut parts: Vec<String> = base
        .strip_prefix(root())
        .ok()?
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();
    for piece in link.split('/') {
        match piece {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p.to_string()),
        }
    }
    Some(parts.iter().fold(root(), |p, s| p.join(s)))
}

fn is_external(link: &str) -> bool {
    link.contains("://") || link.starts_with("mailto:")
}

#[test]
fn slugs_follow_the_host() {
    assert_eq!(slug(" JACK (Linux)"), "jack-linux");
    assert_eq!(slug(" A node says \"disabled\""), "a-node-says-disabled");
    assert_eq!(slug(" Sharing a project: packs"), "sharing-a-project-packs");
    assert_eq!(slug(" Input / Output"), "input--output");
    assert_eq!(slug(" block-update"), "block-update");
    assert_eq!(slug(" See [the guide](x.md) now"), "see-the-guide-now");
    let a = anchors("# Notes\n## Notes\n<a id=\"clip\"></a>\n```\n# not a heading\n```\n");
    assert_eq!(
        a.into_iter().collect::<Vec<_>>(),
        ["clip", "notes", "notes-1"]
    );
    assert_eq!(
        links("See [a](b.md#c), ![i](../i.png \"t\"), and `[x](not-a-link)`."),
        ["b.md#c", "../i.png"]
    );
}

#[test]
fn relative_links_point_at_files_and_anchors_that_exist() {
    let mut broken = Vec::new();
    let mut checked = 0;
    for page in pages() {
        let text = read(&page);
        let dir = page.parent().unwrap_or(Path::new("")).to_path_buf();
        for link in links(&text) {
            if is_external(&link) {
                continue;
            }
            checked += 1;
            let (path, anchor) = match link.split_once('#') {
                Some((p, a)) => (p, Some(a)),
                None => (link.as_str(), None),
            };
            let target = if path.is_empty() {
                Some(page.clone())
            } else {
                resolve(&dir, path)
            };
            let Some(target) = target.filter(|t| t.exists()) else {
                broken.push(format!("{}: {link} (no such file)", shown(&page)));
                continue;
            };
            if let Some(anchor) = anchor
                && target.extension().is_some_and(|x| x == "md")
                && !anchors(&read(&target)).contains(anchor)
            {
                broken.push(format!(
                    "{}: {link} (no heading or anchor \"{anchor}\" in {})",
                    shown(&page),
                    shown(&target)
                ));
            }
        }
    }
    assert!(
        checked > 100,
        "only {checked} links found; is the parser broken?"
    );
    assert!(broken.is_empty(), "broken links:\n{}", broken.join("\n"));
}

#[test]
fn every_guide_chapter_is_in_the_index() {
    let index = read(&root().join("docs/guide/README.md"));
    let linked: BTreeSet<String> = links(&index)
        .into_iter()
        .map(|l| l.split('#').next().unwrap_or("").to_string())
        .collect();
    for page in files_in("docs/guide", "md") {
        let name = page.file_name().unwrap().to_string_lossy().to_string();
        if name != "README.md" {
            assert!(
                linked.contains(&name),
                "docs/guide/{name} isn't linked from docs/guide/README.md"
            );
        }
    }
}

#[test]
fn every_example_is_in_the_tour() {
    let tour = read(&root().join("docs/guide/examples.md"));
    let examples = files_in("examples", "acproj");
    assert!(!examples.is_empty(), "no example projects found");
    let missing: Vec<String> = examples
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .filter(|name| !tour.contains(&format!("`{name}`")))
        .collect();
    assert!(
        missing.is_empty(),
        "examples without an entry in docs/guide/examples.md (add a section and a row \
         naming the file): {}",
        missing.join(", ")
    );
}
