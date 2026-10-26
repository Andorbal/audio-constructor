//! The sampler nodes in the editor (ROADMAP M4.4): a Waveform's handles
//! drag, and audio files dropped on the window become a Clip or a Sampler,
//! a new sound for a card, or a pad on a panel.

use std::path::PathBuf;

use ac_app::audio::AudioMode;
use ac_app::canvas::scene::Body;
use ac_app::{AppOptions, EditorApp};
use ac_core::{NodeId, NodeInstance, Panel, PanelItem, Project, Value};
use egui::{Key, Modifiers, Pos2, vec2};
use egui_kittest::Harness;

fn harness(project: Project) -> Harness<'static, EditorApp> {
    let ctx = egui::Context::default();
    let mut app = EditorApp::new(
        &ctx,
        AppOptions {
            audio: AudioMode::Null,
            ..AppOptions::headless()
        },
    );
    app.open_unsaved(project);
    let mut h = Harness::builder()
        .with_size(vec2(1400.0, 900.0))
        .with_max_steps(64)
        .with_step_dt(1.0 / 30.0)
        .with_accessibility_check(false)
        .build_ui_state(|ui, app: &mut EditorApp| app.ui(ui), app);
    h.run_steps(4);
    h
}

fn drag(h: &mut Harness<'static, EditorApp>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.run_steps(2);
    h.drag_at(from);
    h.run_steps(2);
    for i in 1..=8 {
        h.hover_at(from + (to - from) * (i as f32 / 8.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(4);
}

fn node(h: &Harness<'static, EditorApp>, id: NodeId) -> Option<NodeInstance> {
    h.state().document().project.graph.node(id).cloned()
}

#[test]
fn a_waveforms_end_handle_drags_on_a_panel() {
    let mut p = Project::default();
    p.graph.nodes.push(NodeInstance::new(NodeId(1), "waveform"));
    let mut panel = Panel::new("Sound");
    panel.items.push(PanelItem::new(NodeId(1)));
    p.panels.push(panel);
    let mut h = harness(p);
    h.key_press_modifiers(Modifiers::CTRL, Key::Tab);
    h.run_steps(3);
    let cell = h
        .state()
        .perform
        .cells
        .iter()
        .find(|c| c.node == NodeId(1))
        .cloned()
        .expect("the card");
    let (body, r) = cell.body.expect("a body");
    assert_eq!(body, Body::Waveform);
    // The end handle starts at the right edge.
    let from = Pos2::new(r.max.x - 1.0, r.center().y);
    let to = Pos2::new(r.min.x + 0.6 * r.width(), r.center().y);
    drag(&mut h, from, to);
    let end = node(&h, NodeId(1))
        .and_then(|n| n.params.get("end_point").and_then(Value::as_f64))
        .expect("set");
    assert!((end - 0.6).abs() < 0.02, "{end}");
}

/// A saved project in a fresh folder, with a short WAV file beside it.
fn saved(h: &mut Harness<'static, EditorApp>, tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("ac-drop-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join("hit.wav");
    let tone: Vec<f32> = (0..4_800).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
    ac_io::wav::write_wav(&wav, &[tone], 48_000, ac_io::wav::BitDepth::Float32).unwrap();
    let file = dir.join("drops.acproj");
    h.state_mut().save_to(&file, 1.0);
    h.run_steps(2);
    (dir, wav)
}

fn kinds(h: &Harness<'static, EditorApp>) -> Vec<String> {
    h.state()
        .document()
        .project
        .graph
        .nodes
        .iter()
        .map(|n| n.kind.clone())
        .collect()
}

#[test]
fn a_dropped_file_becomes_a_clip_or_a_sampler_or_a_cards_new_sound() {
    let mut h = harness(Project::default());
    let (dir, wav) = saved(&mut h, "canvas");
    let center = h.state().canvas.transform().screen.center();
    h.state_mut().drop_audio(&wav, Some(center), false, 2.0);
    h.run_steps(3);
    assert_eq!(kinds(&h), ["clip"]);
    let clip = h.state().document().project.graph.nodes[0].clone();
    assert_eq!(clip.config["file"], "assets/hit.wav");
    assert!(
        dir.join("assets/hit.wav").exists(),
        "copied into the project"
    );
    // With Shift, a Sampler, further down.
    h.state_mut()
        .drop_audio(&wav, Some(center + vec2(0.0, 250.0)), true, 3.0);
    h.run_steps(3);
    assert_eq!(kinds(&h), ["clip", "sampler"]);
    // Onto the clip's card: the same clip, its sound replaced.
    let other = dir.join("other.wav");
    ac_io::wav::write_wav(
        &other,
        &[vec![0.25; 2_400]],
        48_000,
        ac_io::wav::BitDepth::Float32,
    )
    .unwrap();
    let card = h
        .state()
        .canvas
        .node_screen_rect(clip.id)
        .expect("drawn")
        .center();
    h.state_mut().drop_audio(&other, Some(card), false, 4.0);
    h.run_steps(3);
    assert_eq!(kinds(&h), ["clip", "sampler"], "no new node");
    assert_eq!(h.state().document().undo_label(), Some("Choose clip file"));
    assert_eq!(
        h.state().document().project.graph.nodes[0].config["file"],
        "assets/other.wav"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_dropped_on_a_panel_makes_a_pad_that_plays_it() {
    let mut p = Project::default();
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(1), "audio_out").at(800.0, 0.0));
    let mut h = harness(p);
    let (dir, wav) = saved(&mut h, "panel");
    h.key_press_modifiers(Modifiers::CTRL, Key::Tab);
    h.run_steps(3);
    h.state_mut().drop_audio(&wav, None, false, 2.0);
    h.run_steps(3);
    let doc = h.state().document();
    let g = &doc.project.graph;
    let pad = g.nodes.iter().find(|n| n.kind == "pad").expect("a pad");
    let sampler = g
        .nodes
        .iter()
        .find(|n| n.kind == "sampler")
        .expect("a sampler");
    assert_eq!(sampler.config["file"], "assets/hit.wav");
    assert!(
        doc.project.panels[0].contains(pad.id),
        "the pad is on the panel"
    );
    let wired = |from: NodeId, to: NodeId| {
        g.wires
            .iter()
            .any(|w| w.from.node == from && w.to.node == to)
    };
    assert!(wired(pad.id, sampler.id));
    assert!(wired(sampler.id, NodeId(1)), "heard through the output");
    let _ = std::fs::remove_dir_all(&dir);
}
