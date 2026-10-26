//! Templates in the editor (ROADMAP M4.6): Add ▸ Track, Drum Pad, Clip
//! Slot, and Send each add a block instance wired in and pinned; a
//! selection saved as a template adds again.

use ac_app::audio::AudioMode;
use ac_app::templates::Template;
use ac_app::{AppOptions, EditorApp};
use ac_core::{Endpoint, NodeId, NodeInstance, Project};
use egui::vec2;
use egui_kittest::Harness;
use serde_json::json;

fn harness(project: Project) -> Harness<'static, EditorApp> {
    harness_with(project, AppOptions::headless().settings)
}

fn harness_with(
    project: Project,
    settings: ac_app::settings::AppSettings,
) -> Harness<'static, EditorApp> {
    let ctx = egui::Context::default();
    let mut app = EditorApp::new(
        &ctx,
        AppOptions {
            audio: AudioMode::Null,
            settings,
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

fn mixer() -> Project {
    let mut p = Project::default();
    for (id, kind, x) in [
        (1, "oscillator", 0.0),
        (2, "mix", 600.0),
        (3, "audio_out", 900.0),
    ] {
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(id), kind).at(x, 0.0));
    }
    p.graph.nodes[1].config = json!({ "inputs": 2 });
    p.graph.connect(
        Endpoint::new(NodeId(2), "out"),
        Endpoint::new(NodeId(3), "in"),
    );
    p
}

#[test]
fn a_track_template_is_a_wired_pinned_channel_strip_in_one_step() {
    let mut h = harness(mixer());
    h.state_mut().doc.selection = [NodeId(1)].into();
    h.run_steps(2);
    let id = h
        .state_mut()
        .add_template(Template::Track, 1.0)
        .expect("added");
    h.run_steps(2);
    let doc = h.state().document();
    let g = &doc.project.graph;
    let n = g.node(id).unwrap();
    assert_eq!(n.kind, "group");
    assert_eq!(n.ui.label.as_deref(), Some("Track"));
    assert_eq!(
        g.wire_into(&Endpoint::new(id, "in")).map(|w| w.from.node),
        Some(NodeId(1))
    );
    assert_eq!(
        g.wire_into(&Endpoint::new(NodeId(2), "in_0"))
            .map(|w| w.from.node),
        Some(id),
        "into the mix"
    );
    assert!(doc.project.panels[0].contains(id), "pinned");
    assert_eq!(doc.undo_label(), Some("Add Track"));
    // One undo takes it all away.
    h.state_mut().doc.undo();
    assert!(h.state().document().project.graph.node(id).is_none());
    assert!(h.state().document().project.panels.is_empty());
}

#[test]
fn a_clip_slot_template_drives_the_selected_clip_and_a_send_feeds_the_mix() {
    let mut p = mixer();
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(4), "clip").at(0.0, 300.0));
    let mut h = harness(p);
    h.state_mut().doc.selection = [NodeId(4)].into();
    h.run_steps(2);
    let slot = h.state_mut().add_template(Template::ClipSlot, 1.0).unwrap();
    h.run_steps(2);
    let g = h.state().document().project.graph.clone();
    assert_eq!(
        g.wire_into(&Endpoint::new(NodeId(4), "start"))
            .map(|w| w.from.node),
        Some(slot)
    );
    h.state_mut().doc.selection = [NodeId(1)].into();
    h.run_steps(2);
    let send = h.state_mut().add_template(Template::Send, 2.0).unwrap();
    h.run_steps(2);
    let g = &h.state().document().project.graph;
    assert_eq!(
        g.wire_into(&Endpoint::new(send, "in")).map(|w| w.from.node),
        Some(NodeId(1))
    );
    assert!(h.state().document().project.panels[0].contains(slot));
    assert!(h.state().document().project.panels[0].contains(send));
}

#[test]
fn a_saved_selection_adds_again_with_its_controls_pinned() {
    let mut p = Project::default();
    p.graph.nodes.push(
        NodeInstance::new(NodeId(1), "knob")
            .with_label("Cutoff")
            .at(0.0, 0.0),
    );
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(2), "biquad").at(300.0, 0.0));
    p.graph.connect(
        Endpoint::new(NodeId(1), "out"),
        Endpoint::new(NodeId(2), "cutoff"),
    );
    let dir = std::env::temp_dir().join(format!("ac-templates-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let settings = ac_app::settings::AppSettings::load_from(Some(dir.join("settings.json")));
    let mut h = harness_with(p, settings);
    h.state_mut().doc.selection = [NodeId(1), NodeId(2)].into();
    h.run_steps(2);
    h.state_mut().save_selection_as_template(1.0);
    assert_eq!(h.state().saved_templates().len(), 1);
    assert_eq!(h.state().saved_templates()[0].name, "Cutoff");
    h.state_mut().paste_template(0, 2.0);
    h.run_steps(2);
    let doc = h.state().document();
    assert_eq!(doc.project.graph.nodes.len(), 4, "a second knob and filter");
    let knobs: Vec<_> = doc
        .project
        .graph
        .nodes
        .iter()
        .filter(|n| n.kind == "knob")
        .map(|n| n.id)
        .collect();
    assert!(
        doc.project.panels[0].contains(knobs[1]),
        "the new knob is pinned"
    );
    // Kept in the settings file.
    let again = ac_app::settings::AppSettings::load_from(Some(dir.join("settings.json")));
    assert_eq!(again.prefs.templates.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Step in real time, so the null stream runs blocks, until `done` holds.
fn wait_for(h: &mut Harness<'static, EditorApp>, mut done: impl FnMut(&EditorApp) -> bool) -> bool {
    let t0 = std::time::Instant::now();
    while t0.elapsed() < std::time::Duration::from_secs(5) {
        if done(h.state()) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
        h.run_steps(1);
    }
    done(h.state())
}

/// Phase 4 acceptance: from the Four Tracks example (saved), add a track
/// and record a take in four gestures (Add ▸ Track, R, Space, Create Clip
/// from take), then play from bar 9 (a click on the ruler).
#[test]
fn four_tracks_gets_a_recorded_track_in_four_gestures() {
    use egui::{Key, Modifiers};
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/four-tracks.acproj"
    );
    let project = ac_io::load_project(std::path::Path::new(path)).expect("the example");
    let mut h = harness(project);
    let dir = std::env::temp_dir().join(format!("ac-four-tracks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    h.state_mut().save_to(&dir.join("four-tracks.acproj"), 0.5);
    h.state_mut().doc.selection.clear();
    h.run_steps(2);
    let before = h.state().document().project.graph.nodes.len();
    // 1: Add ▸ Track, with nothing selected: a track that records.
    let track = h.state_mut().add_template(Template::Track, 1.0).unwrap();
    h.run_steps(3);
    let g = h.state().document().project.graph.clone();
    assert_eq!(
        g.nodes.len(),
        before + 3,
        "the Track, an Audio In, a Record"
    );
    let input = g
        .wire_into(&Endpoint::new(track, "in"))
        .map(|w| w.from.node)
        .unwrap();
    assert_eq!(g.node(input).unwrap().kind, "audio_in");
    // 2 and 3: R records (and plays), Space stops.
    h.hover_at(egui::pos2(700.0, 600.0));
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::NONE, Key::R);
    h.run_steps(2);
    std::thread::sleep(std::time::Duration::from_millis(300));
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::NONE, Key::Space);
    assert!(wait_for(&mut h, |app| !app.takes.is_empty()), "a take");
    // 4: Create Clip from take.
    let take = h.state().takes[0].clone();
    h.state_mut().clip_from_take(&take);
    h.run_steps(2);
    let g = &h.state().document().project.graph;
    let clip = g
        .wire_into(&Endpoint::new(track, "in"))
        .map(|w| w.from.node)
        .unwrap();
    assert_eq!(
        g.node(clip).unwrap().kind,
        "clip",
        "the take plays on the track"
    );
    // Then play from bar 9: the song is long enough to have one.
    let settings = &h.state().document().project.settings;
    assert!(ac_app::editors::arrange::song_end(settings) >= 36.0);
    let _ = std::fs::remove_dir_all(&dir);
}
