//! The shipped examples are lessons as much as patches (docs/UX.md,
//! "Onboarding and examples"), so they have to read well on the canvas: no
//! card covers another, and a frame holds whole cards, never half of one.
//! The cards are laid out the way the editor draws them. The gap studies in
//! `studies/` (docs/GAPS.md) are read the same way, so they get the same
//! check.

use std::path::{Path, PathBuf};

use ac_app::canvas::scene::{self, Class};
use ac_app::document::Document;
use egui::{Rect, pos2, vec2};

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn studies_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../studies")
}

fn projects_in(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{} exists: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "acproj"))
        .collect();
    v.sort();
    v
}

/// The examples, then the studies.
fn examples() -> Vec<PathBuf> {
    let mut v = projects_in(&examples_dir());
    v.extend(projects_in(&studies_dir()));
    v
}

#[test]
fn example_cards_do_not_overlap_and_frames_hold_whole_cards() {
    let registry = ac_nodes::registry();
    let mut problems = Vec::new();
    let list = examples();
    assert!(!list.is_empty());
    for path in list {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string());
        let name = name.unwrap_or_default();
        let project = ac_io::load_project(&path).expect("example loads");
        let frames = project.graph.frames.clone();
        let dir = path.parent().expect("dir").to_path_buf();
        let doc = Document::new(project, Some(path.clone()), dir, 1);
        let scene = scene::build(&doc, &registry, None);
        // Cards may touch; a pixel of shared edge isn't covering.
        let cards: Vec<(String, Rect)> = scene
            .nodes
            .iter()
            .filter(|n| n.class != Class::Reroute)
            .map(|n| (format!("{} ({})", n.id, n.title), n.rect.shrink(1.0)))
            .collect();
        for (i, (a, ra)) in cards.iter().enumerate() {
            for (b, rb) in &cards[i + 1..] {
                if ra.intersects(*rb) {
                    problems.push(format!("{name}: {a} {ra:?} overlaps {b} {rb:?}"));
                }
            }
        }
        let frames: Vec<(String, Rect)> = frames
            .iter()
            .map(|f| {
                let r = Rect::from_min_size(pos2(f.rect[0], f.rect[1]), vec2(f.rect[2], f.rect[3]));
                (f.label.clone(), r)
            })
            .collect();
        for (label, fr) in &frames {
            for (card, r) in &cards {
                if fr.intersects(*r) && !fr.contains_rect(*r) {
                    problems.push(format!(
                        "{name}: frame \"{label}\" {fr:?} cuts through {card} {r:?}"
                    ));
                }
            }
        }
        for (i, (a, ra)) in frames.iter().enumerate() {
            for (b, rb) in &frames[i + 1..] {
                if ra.shrink(1.0).intersects(rb.shrink(1.0)) {
                    problems.push(format!("{name}: frames \"{a}\" and \"{b}\" overlap"));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
