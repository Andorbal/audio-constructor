//! The editor's look and help, without a window (egui_kittest): View ▸
//! Theme and View ▸ UI Scale switch at once and are saved in the app's
//! settings file, and F1 opens a node's reference with the blocks and
//! examples that use it.

use std::path::PathBuf;

use ac_app::audio::AudioMode;
use ac_app::settings::AppSettings;
use ac_app::theme::{self, ThemeChoice};
use ac_app::{AppOptions, EditorApp};
use ac_core::Project;
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
