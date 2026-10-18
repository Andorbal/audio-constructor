//! The editor's look and help, without a window (egui_kittest): View ▸
//! Theme and View ▸ UI Scale switch at once and are saved in the app's
//! settings file, and F1 opens a node's reference with the blocks and
//! examples that use it.

use std::path::PathBuf;

use ac_app::audio::AudioMode;
use ac_app::icons;
use ac_app::panels::Tab;
use ac_app::settings::AppSettings;
use ac_app::theme::{self, ThemeChoice};
use ac_app::{AppOptions, EditorApp};
use ac_core::{GROUP_KIND, NodeId, NodeInstance, Project};
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn harness_with(settings: AppSettings, project: Option<Project>) -> Harness<'static, EditorApp> {
    let ctx = egui::Context::default();
    let mut app = EditorApp::new(
        &ctx,
        AppOptions {
            audio: AudioMode::Null,
            settings,
            ..AppOptions::headless()
        },
    );
    if let Some(p) = project {
        app.open_unsaved(p);
    }
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .with_max_steps(64)
        .with_step_dt(1.0 / 30.0)
        .with_accessibility_check(false)
        .build_ui_state(|ui, app: &mut EditorApp| app.ui(ui), app);
    h.run_steps(4);
    h
}

fn steps(h: &mut Harness<'static, EditorApp>, n: usize) {
    h.run_steps(n);
}

fn press(h: &mut Harness<'static, EditorApp>, modifiers: Modifiers, key: Key) {
    h.key_press_modifiers(modifiers, key);
    steps(h, 3);
}

fn click(h: &mut Harness<'static, EditorApp>, label: &str) {
    h.get_by_label_contains(label).click();
    steps(h, 3);
}

/// A fresh settings file (not yet written) in its own temp folder.
fn settings_file(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    std::env::temp_dir()
        .join(format!("ac-app-{tag}-{}-{nanos}", std::process::id()))
        .join("settings.json")
}

fn dark_mode(h: &Harness<'static, EditorApp>) -> bool {
    h.ctx.global_style().visuals.dark_mode
}

#[test]
fn view_theme_light_switches_at_once_and_is_remembered() {
    let file = settings_file("theme");
    let mut h = harness_with(AppSettings::load_from(Some(file.clone())), None);
    assert!(dark_mode(&h), "dark by default");
    assert_eq!(theme::canvas_bg(), theme::DARK.canvas_bg);

    click(&mut h, "View");
    click(&mut h, "Theme");
    click(&mut h, "Light");
    assert!(!dark_mode(&h), "egui's visuals are light");
    let v = h.ctx.global_style().visuals.clone();
    assert_eq!(v.panel_fill, theme::LIGHT.panel_bg);
    assert_eq!(
        theme::canvas_bg(),
        theme::LIGHT.canvas_bg,
        "the canvas draws with the light palette"
    );
    assert_eq!(h.state().settings().prefs.theme, ThemeChoice::Light);
    assert!(file.is_file(), "saved as it changed");

    // The next start reads it back.
    let h2 = harness_with(AppSettings::load_from(Some(file.clone())), None);
    assert!(!dark_mode(&h2));
    assert_eq!(h2.state().settings().prefs.theme, ThemeChoice::Light);
    assert_eq!(theme::canvas_bg(), theme::LIGHT.canvas_bg);

    // And back to dark.
    let mut h = h2;
    click(&mut h, "View");
    click(&mut h, "Theme");
    click(&mut h, "Dark");
    assert!(dark_mode(&h));
    assert_eq!(theme::canvas_bg(), theme::DARK.canvas_bg);
    let again = AppSettings::load_from(Some(file.clone()));
    assert_eq!(again.prefs.theme, ThemeChoice::Dark);
    std::fs::remove_dir_all(file.parent().unwrap()).ok();
}

#[test]
fn a_bad_settings_file_starts_with_the_defaults() {
    let file = settings_file("bad");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "{ not json").unwrap();
    let h = harness_with(AppSettings::load_from(Some(file.clone())), None);
    assert!(dark_mode(&h));
    assert_eq!(h.ctx.zoom_factor(), 1.0);
    std::fs::remove_dir_all(file.parent().unwrap()).ok();
}

#[test]
fn ui_scale_steps_change_the_zoom_factor_and_are_remembered() {
    let file = settings_file("scale");
    let mut h = harness_with(AppSettings::load_from(Some(file.clone())), None);
    assert_eq!(h.ctx.zoom_factor(), 1.0);
    let canvas_zoom = h.state().canvas.view.zoom;

    press(&mut h, Modifiers::COMMAND, Key::Equals);
    assert_eq!(h.ctx.zoom_factor(), 1.1);
    press(&mut h, Modifiers::COMMAND, Key::Equals);
    assert_eq!(h.ctx.zoom_factor(), 1.25);
    press(&mut h, Modifiers::COMMAND, Key::Minus);
    assert_eq!(h.ctx.zoom_factor(), 1.1);
    assert_eq!(
        h.state().canvas.view.zoom,
        canvas_zoom,
        "Ctrl+= and Ctrl+- scale the interface, not the canvas"
    );
    press(&mut h, Modifiers::COMMAND, Key::Num0);
    assert_eq!(h.ctx.zoom_factor(), 1.0);
    // The ends hold.
    for _ in 0..4 {
        press(&mut h, Modifiers::COMMAND, Key::Minus);
    }
    assert_eq!(h.ctx.zoom_factor(), 0.75);

    // The menu lists every step.
    click(&mut h, "View");
    click(&mut h, "UI Scale");
    click(&mut h, "150 %");
    assert_eq!(h.ctx.zoom_factor(), 1.5);
    assert_eq!(h.state().settings().prefs.ui_scale, 1.5);
    assert_eq!(h.state().status(), Some("Interface at 150 %"));

    let h2 = harness_with(AppSettings::load_from(Some(file.clone())), None);
    assert_eq!(h2.ctx.zoom_factor(), 1.5, "the next start reads it back");
    std::fs::remove_dir_all(file.parent().unwrap()).ok();
}

#[test]
fn every_side_panel_tab_fits_at_the_default_width() {
    let mut h = harness_with(AppSettings::in_memory(), None);
    let window = 1400.0;
    // Narrow: the tabs not shown keep only their icons.
    let help = h.get_by_label(icons::BOOK_OPEN).rect();
    assert!(help.max.x <= window, "{help:?}");
    let node = h.get_by_label(&format!("{} Node", icons::SLIDERS)).rect();
    assert!(node.min.x > window - 400.0 && node.max.x < help.min.x);
    // Shown, a tab has its name.
    h.state_mut().side.show_help(None);
    steps(&mut h, 2);
    let help = h.get_by_label(&format!("{} Help", icons::BOOK_OPEN)).rect();
    assert!(help.max.x <= window, "{help:?}");
}

/// One node of `kind`, selected.
fn one_node(kind: &str) -> Harness<'static, EditorApp> {
    let mut p = Project::default();
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(1), kind).at(0.0, 0.0));
    let mut h = harness_with(AppSettings::in_memory(), Some(p));
    h.state_mut().doc.selection = [NodeId(1)].into();
    steps(&mut h, 2);
    h
}

/// Scroll a label into view and click it.
fn click_exact(h: &mut Harness<'static, EditorApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    steps(h, 3);
    h.get_by_label(label).click();
    steps(h, 3);
}

fn block_link(name: &str) -> String {
    format!("{}  {name}", icons::STACK)
}

fn example_link(name: &str) -> String {
    format!("{}  {name}", icons::FOLDER_OPEN)
}

#[test]
fn f1_on_a_gain_computer_lists_the_blocks_that_use_it() {
    let mut h = one_node("gain_computer");
    press(&mut h, Modifiers::NONE, Key::F1);
    assert!(h.state().side.open);
    assert_eq!(h.state().side.tab, Tab::Help);
    assert_eq!(h.state().side.help.kind.as_deref(), Some("gain_computer"));
    // Name, the highlighted tip, the sockets with their units, Used in.
    let _ = h.get_by_label("Gain Computer");
    let _ = h.get_by_label_contains("Try this");
    let _ = h.get_by_label("threshold");
    assert!(
        h.query_all_by_label("Float (dB)").count() >= 2,
        "threshold and knee are in decibels"
    );
    let used_in = h.get_by_label("USED IN").rect();
    for name in ["Compressor", "Multiband Compressor", "Limiter"] {
        let link = h.get_by_label(&block_link(name)).rect();
        assert!(link.min.y > used_in.max.y, "{name} is listed under Used in");
    }
    assert!(
        h.query_by_label(&block_link("Tremolo")).is_none(),
        "only blocks that hold it"
    );

    // Clicking a block adds it to the canvas.
    let before = h.state().document().graph().nodes.len();
    click_exact(&mut h, &block_link("Compressor"));
    let doc = h.state().document();
    assert_eq!(doc.graph().nodes.len(), before + 1);
    let added = doc.graph().nodes.last().unwrap();
    assert_eq!(added.kind, GROUP_KIND);
    let def = &doc.project.group_defs[&added.group_def().unwrap()];
    assert_eq!(def.name, "Compressor");
    assert_eq!(def.source.as_deref(), Some("shipped:compressor"));
}

#[test]
fn the_node_tab_and_the_menu_open_the_same_help() {
    let mut h = one_node("oscillator");
    assert_eq!(h.state().side.tab, Tab::Node);
    // The Node tab shows the tip and a button to the full reference.
    let _ = h.get_by_label_contains("Try this");
    click_exact(&mut h, &format!("{}  Help", icons::BOOK_OPEN));
    assert_eq!(h.state().side.tab, Tab::Help);
    assert_eq!(h.state().side.help.kind.as_deref(), Some("oscillator"));
    // Help ▸ Node Help with nothing selected lists every node.
    h.state_mut().doc.selection.clear();
    click_exact(&mut h, "Help");
    click(&mut h, "Node Help");
    assert_eq!(h.state().side.tab, Tab::Help);
    assert_eq!(h.state().side.help.kind, None);
    let _ = h.get_by_label_contains("Press F1 on a node");
}

#[test]
fn help_search_finds_nodes_by_their_synonyms() {
    let mut h = one_node("gain");
    h.state_mut().side.show_help(None);
    steps(&mut h, 2);
    let field = h
        .get_by_role(egui::accesskit::Role::TextInput)
        .rect()
        .center();
    h.hover_at(field);
    h.drag_at(field);
    h.drop_at(field);
    steps(&mut h, 2);
    h.event(egui::Event::Text("compressor".into()));
    steps(&mut h, 3);
    assert_eq!(h.state().side.help.query, "compressor");
    click_exact(&mut h, "Gain Computer");
    assert_eq!(h.state().side.help.kind.as_deref(), Some("gain_computer"));
    assert!(h.state().side.help.query.is_empty());
    let _ = h.get_by_label(&block_link("Compressor"));
}

#[test]
fn an_example_link_opens_the_example_after_asking_about_changes() {
    let mut h = one_node("oscillator");
    // An unsaved change.
    let ctx = h.ctx.clone();
    h.state_mut().run(&ctx, ac_app::actions::Action::Mute, 0.0);
    steps(&mut h, 2);
    assert!(h.state().document().is_dirty());
    let nodes = h.state().document().graph().nodes.len();

    press(&mut h, Modifiers::NONE, Key::F1);
    let _ = h.get_by_label("Examples");
    click_exact(&mut h, &example_link("Start"));
    // Cancel keeps everything as it was.
    h.get_by_label("Cancel").click();
    steps(&mut h, 3);
    assert_eq!(h.state().document().graph().nodes.len(), nodes);
    assert!(h.state().document().is_dirty());

    click_exact(&mut h, &example_link("Start"));
    h.get_by_label("Don't Save").click();
    steps(&mut h, 3);
    let doc = h.state().document();
    assert_eq!(doc.project.name, "Start");
    assert!(
        doc.path.is_none(),
        "an untitled copy, so saving can't overwrite it"
    );
    assert!(!doc.is_dirty());
    assert!(
        doc.graph().nodes.iter().any(|n| n.kind == "oscillator"),
        "the example's patch"
    );
    assert!(
        h.state()
            .status()
            .is_some_and(|s| s.contains("Opened the example “Start”")),
        "{:?}",
        h.state().status()
    );
}

#[test]
fn file_examples_opens_an_untitled_copy() {
    let mut h = one_node("oscillator");
    click(&mut h, "File");
    click(&mut h, "Examples");
    click_exact(&mut h, "Drum Machine");
    let doc = h.state().document();
    assert_eq!(doc.project.name, "Drum Machine");
    assert!(doc.path.is_none(), "a copy: saving asks where to put it");
    assert!(
        doc.graph().nodes.len() > 1,
        "the example's patch, not the one node from before"
    );
}
