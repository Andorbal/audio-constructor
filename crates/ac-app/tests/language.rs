//! The editor's language (egui_kittest): View ▸ Language switches at once
//! and is remembered, and in the Pseudo language every menu label carries
//! Pseudo's brackets, so none of the menus' text skips the catalog.

use std::path::PathBuf;

use ac_app::audio::AudioMode;
use ac_app::i18n::{self, Language, LanguageChoice, tr};
use ac_app::settings::AppSettings;
use ac_app::{AppOptions, EditorApp};
use ac_core::{NodeId, NodeInstance, Project};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

fn harness_with(settings: AppSettings) -> Harness<'static, EditorApp> {
    let ctx = egui::Context::default();
    let mut app = EditorApp::new(
        &ctx,
        AppOptions {
            audio: AudioMode::Null,
            settings,
            ..AppOptions::headless()
        },
    );
    // Two nodes, both selected, so every Edit entry and submenu is there.
    let mut p = Project::default();
    for (i, kind) in [(1, "oscillator"), (2, "gain")] {
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(i), kind).at(i as f32 * 260.0, 0.0));
    }
    app.open_unsaved(p);
    app.doc.selection = [NodeId(1), NodeId(2)].into();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .with_max_steps(64)
        .with_step_dt(1.0 / 30.0)
        .with_accessibility_check(false)
        .build_ui_state(|ui, app: &mut EditorApp| app.ui(ui), app);
    h.run_steps(4);
    h
}

fn settings_file(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    std::env::temp_dir()
        .join(format!("ac-app-{tag}-{}-{nanos}", std::process::id()))
        .join("settings.json")
}

fn click(h: &mut Harness<'static, EditorApp>, label: &str) {
    h.get_by_label_contains(label).click();
    h.run_steps(3);
}

/// Open a menu of the menu bar (the side panel has a Help tab too).
fn open_menu(h: &mut Harness<'static, EditorApp>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).click();
    h.run_steps(3);
}

/// The text of every labelled thing inside an open menu, with its role.
fn menu_texts(h: &Harness<'static, EditorApp>) -> Vec<(Role, String)> {
    let mut out = Vec::new();
    for menu in h.query_all_by_role(Role::Menu) {
        for n in menu.children_recursive() {
            let a = n.accesskit_node();
            let text = a
                .label()
                .map(|s| s.to_string())
                .or_else(|| a.value().map(|s| s.to_string()))
                .unwrap_or_default();
            if !text.trim().is_empty() {
                out.push((a.role(), text));
            }
        }
    }
    out
}

/// Close whatever menu is open.
fn close_menus(h: &mut Harness<'static, EditorApp>) {
    for _ in 0..3 {
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
    }
}

/// Icons are text in the icon font (Private Use Area) and sit beside the
/// words; what's left after them must come from the catalog.
fn words(text: &str) -> String {
    text.chars()
        .filter(|c| !('\u{E000}'..='\u{F8FF}').contains(c))
        .collect::<String>()
        .trim()
        .to_string()
}

#[test]
fn view_language_switches_at_once_and_is_remembered() {
    let file = settings_file("language");
    let mut h = harness_with(AppSettings::load_from(Some(file.clone())));
    assert_eq!(h.state().language(), Language::English);
    click(&mut h, "View");
    click(&mut h, "Language");
    click(&mut h, "Pseudo");
    assert_eq!(h.state().language(), Language::Pseudo);
    assert_eq!(i18n::language(), Language::Pseudo);
    h.run_steps(2);
    // The menu bar is in Pseudo from the next frame.
    let file_menu = tr("menu-file");
    assert!(i18n::is_pseudo(&file_menu));
    let _ = h.get_by_label(&file_menu);
    assert!(h.query_by_label("File").is_none());
    assert!(
        h.state().status().is_some_and(i18n::is_pseudo),
        "{:?}",
        h.state().status()
    );
    let saved = AppSettings::load_from(Some(file.clone()));
    assert_eq!(saved.prefs.language, LanguageChoice::Pseudo);

    // The next start speaks Pseudo, and English comes back from the menu.
    let mut h = harness_with(AppSettings::load_from(Some(file.clone())));
    assert_eq!(h.state().language(), Language::Pseudo);
    open_menu(&mut h, &tr("menu-view"));
    click(&mut h, &tr("menu-language"));
    click(&mut h, &tr("language-english"));
    h.run_steps(2);
    assert_eq!(i18n::language(), Language::English);
    let _ = h.get_by_label("File");
    std::fs::remove_dir_all(file.parent().unwrap()).ok();
}

#[test]
fn every_menu_label_is_translated_in_pseudo() {
    let mut settings = AppSettings::in_memory();
    settings.prefs.language = LanguageChoice::Pseudo;
    let mut h = harness_with(settings);
    assert_eq!(i18n::language(), Language::Pseudo);

    // The menu bar itself.
    let bar = [
        "menu-file",
        "menu-edit",
        "menu-view",
        "menu-add",
        "menu-transport",
        "menu-help",
    ];
    let mut checked = 0;
    let mut untranslated = Vec::new();
    let mut check = |h: &Harness<'static, EditorApp>, skip: &[String]| {
        for (role, text) in menu_texts(h) {
            let w = words(&text);
            // Content, not the editor's words: example names, node names,
            // and numbers on their own.
            // The logo says the product's name, which isn't translated.
            if w.is_empty()
                || w == "Audio Constructor"
                || skip.contains(&w)
                || w.chars().all(|c| !c.is_alphabetic())
            {
                continue;
            }
            checked += 1;
            if !i18n::is_pseudo(&w) {
                untranslated.push(format!("{role:?} {text:?}"));
            }
        }
    };
    for id in bar {
        let label = tr(id);
        assert!(i18n::is_pseudo(&label));
        open_menu(&mut h, &label);
        check(&h, &[]);
        // Each submenu, opened from its menu.
        let subs: &[&str] = match id {
            "menu-file" => &["menu-open-recent", "menu-examples"],
            "menu-edit" => &["action-arrange-menu"],
            "menu-view" => &["menu-theme", "menu-language", "menu-ui-scale"],
            "menu-add" => &["category-filters"],
            _ => &[],
        };
        for sub in subs {
            // Each submenu from a freshly opened menu.
            close_menus(&mut h);
            open_menu(&mut h, &label);
            click(&mut h, &tr(sub));
            // The examples are named by their projects, and the Add menu's
            // categories list nodes by their own names.
            let skip: Vec<String> = match *sub {
                "menu-examples" => h
                    .state_mut()
                    .side
                    .help
                    .examples()
                    .iter()
                    .map(|e| e.name.clone())
                    .collect(),
                "category-filters" => h
                    .state()
                    .registry
                    .iter()
                    .map(|e| e.meta.name.to_string())
                    .collect(),
                _ => Vec::new(),
            };
            check(&h, &skip);
        }
        close_menus(&mut h);
    }
    assert!(checked > 100, "the menus were open ({checked} labels)");
    assert!(
        untranslated.is_empty(),
        "menu text without Pseudo's brackets:\n{}",
        untranslated.join("\n")
    );
}
