//! The update check and crash reports in the editor (egui_kittest, a local
//! feed): nothing is asked for before the person decides, a newer release
//! is offered with its download, a skipped one stays quiet, Help asks on
//! demand, and a saved crash report is offered once.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ac_app::audio::AudioMode;
use ac_app::product::ProductOptions;
use ac_app::settings::{AppSettings, UpdateChecks};
use ac_app::{AppOptions, EditorApp};
use ac_io::update::Version;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// A feed answering every request with `body`, counting them.
fn feed(body: &'static str) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://127.0.0.1:{}/releases/latest",
        listener.local_addr().unwrap().port()
    );
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            c.fetch_add(1, Ordering::SeqCst);
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
                line.clear();
            }
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (url, count)
}

const NEWER: &str = r#"{"tag_name":"v0.99.0","html_url":"https://example.org/releases/v0.99.0","body":"Added:\n- Everything"}"#;
const OLDER: &str = r#"{"tag_name":"v0.1.0","html_url":"https://example.org/releases/v0.1.0"}"#;

fn temp(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    std::env::temp_dir().join(format!("ac-product-{tag}-{}-{nanos}", std::process::id()))
}

fn editor(
    settings: AppSettings,
    feed: &str,
    crash_dir: Option<PathBuf>,
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
    app.enable_product(
        &ctx,
        ProductOptions {
            feed: feed.to_string(),
            crash_dir,
            current: Version(0, 14, 0),
        },
    );
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .with_max_steps(64)
        .with_step_dt(1.0 / 30.0)
        .with_accessibility_check(false)
        .build_ui_state(|ui, app: &mut EditorApp| app.ui(ui), app);
    h.run_steps(4);
    h
}

fn click(h: &mut Harness<'static, EditorApp>, label: &str) {
    h.get_by_label_contains(label).click();
    h.run_steps(3);
}

/// The menu bar's Help menu (the side panel has a Help tab too).
fn open_help(h: &mut Harness<'static, EditorApp>) {
    h.query_all_by_role_and_label(egui::accesskit::Role::Button, "Help")
        .next()
        .expect("the Help menu")
        .click();
    h.run_steps(3);
}

/// Runs frames until `label` shows (the check runs on its own thread).
fn wait_for(h: &mut Harness<'static, EditorApp>, label: &str) {
    for _ in 0..300 {
        h.run_steps(1);
        if h.query_by_label_contains(label).is_some() {
            // A new window takes a few frames to size and place itself.
            h.run_steps(8);
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("{label:?} never showed");
}

fn wait_idle(h: &mut Harness<'static, EditorApp>) {
    for _ in 0..300 {
        h.run_steps(1);
        if !h.state().product().is_some_and(|p| p.is_checking()) {
            h.run_steps(2);
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the check never finished");
}

fn settings_at(dir: &std::path::Path) -> AppSettings {
    AppSettings::load_from(Some(dir.join("settings.json")))
}

#[test]
fn nothing_is_asked_for_until_the_person_decides() {
    let dir = temp("ask");
    let (url, requests) = feed(NEWER);
    let mut h = editor(settings_at(&dir), &url, None);
    h.get_by_label("Check for updates?");
    std::thread::sleep(Duration::from_millis(100));
    h.run_steps(3);
    assert_eq!(
        requests.load(Ordering::SeqCst),
        0,
        "no request before a yes"
    );
    click(&mut h, "Don't Check");
    assert!(h.query_by_label("Check for updates?").is_none());
    assert_eq!(settings_at(&dir).prefs.update_checks, UpdateChecks::Off);
    // Next start: not asked again, nothing checked.
    let h = editor(settings_at(&dir), &url, None);
    assert!(h.query_by_label("Check for updates?").is_none());
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn checking_daily_offers_a_newer_release_and_its_download() {
    let dir = temp("daily");
    let (url, requests) = feed(NEWER);
    let mut h = editor(settings_at(&dir), &url, None);
    click(&mut h, "Check Daily");
    wait_for(&mut h, "Audio Constructor 0.99.0 is out. You have 0.14.0.");
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    let saved = settings_at(&dir).prefs;
    assert_eq!(saved.update_checks, UpdateChecks::Daily);
    assert!(
        saved.last_update_check > 1_700_000_000,
        "the check's time is kept"
    );
    click(&mut h, "What's New");
    h.get_by_label_contains("- Everything");
    click(&mut h, "Download");
    assert_eq!(
        h.state().product().and_then(|p| p.opened()),
        Some("https://example.org/releases/v0.99.0")
    );
    assert!(
        h.query_by_label_contains("is out").is_none(),
        "closed after Download"
    );
    // Started again within the day: no second request.
    let mut h = editor(settings_at(&dir), &url, None);
    wait_idle(&mut h);
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_skipped_version_stays_quiet_but_help_still_answers() {
    let dir = temp("skip");
    let (url, requests) = feed(NEWER);
    let mut s = settings_at(&dir);
    s.set_update_checks(UpdateChecks::Daily);
    let mut h = editor(s, &url, None);
    wait_for(&mut h, "0.99.0 is out");
    click(&mut h, "Skip This Version");
    assert_eq!(settings_at(&dir).prefs.skip_version, "0.99.0");
    // A day later the feed still has 0.99.0: the check runs, quietly.
    let mut s = settings_at(&dir);
    s.set_last_update_check(0);
    let mut h = editor(s, &url, None);
    wait_idle(&mut h);
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    assert!(h.query_by_label_contains("is out").is_none());
    // Asking from Help shows it anyway.
    open_help(&mut h);
    click(&mut h, "Check for Updates…");
    wait_for(&mut h, "0.99.0 is out");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn help_says_when_this_is_the_newest_version() {
    let dir = temp("newest");
    let (url, _) = feed(OLDER);
    let mut s = settings_at(&dir);
    s.set_update_checks(UpdateChecks::Off);
    let mut h = editor(s, &url, None);
    open_help(&mut h);
    click(&mut h, "Check for Updates…");
    wait_for(&mut h, "You have the newest version, 0.14.0.");
    click(&mut h, "OK");
    // And turning daily checks on from the menu is kept.
    open_help(&mut h);
    click(&mut h, "Check for Updates Daily");
    assert_eq!(settings_at(&dir).prefs.update_checks, UpdateChecks::Daily);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_failed_check_is_explained_only_when_asked_for() {
    let dir = temp("offline");
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{port}/");
    let mut s = settings_at(&dir);
    s.set_update_checks(UpdateChecks::Daily);
    let mut h = editor(s, &url, None);
    wait_idle(&mut h);
    assert!(
        h.query_by_label_contains("Couldn't check").is_none(),
        "quiet when nobody asked"
    );
    open_help(&mut h);
    click(&mut h, "Check for Updates…");
    wait_for(&mut h, "Couldn't check for updates");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_saved_crash_report_is_offered_once() {
    let dir = temp("crash");
    let crashes = dir.join("crashes");
    let text = ac_io::crash::format_report(
        "audioc",
        "2026-10-19 12:34:56 UTC",
        "main",
        "something broke",
        "crates/x.rs:1:1",
        "   0: frame\n",
    );
    ac_io::crash::save(&crashes, &text).unwrap();
    let mut s = settings_at(&dir);
    s.set_update_checks(UpdateChecks::Off);
    let (url, _) = feed(OLDER);
    let mut h = editor(s, &url, Some(crashes.clone()));
    h.get_by_label("Audio Constructor stopped unexpectedly");
    h.get_by_label_contains("A report was saved from audioc");
    click(&mut h, "Show Report");
    h.get_by_label_contains("message:  something broke");
    click(&mut h, "Report…");
    let opened = h
        .state()
        .product()
        .and_then(|p| p.opened())
        .unwrap()
        .to_string();
    assert!(
        opened.starts_with("https://github.com/Andorbal/audio-constructor/issues/new?title=Crash%3A%20something%20broke"),
        "{opened}"
    );
    assert!(
        h.query_by_label("Audio Constructor stopped unexpectedly")
            .is_none()
    );
    assert!(
        ac_io::crash::pending(&crashes).is_empty(),
        "marked as offered"
    );
    // Not offered again.
    let h = editor(settings_at(&dir), &url, Some(crashes.clone()));
    assert!(
        h.query_by_label("Audio Constructor stopped unexpectedly")
            .is_none()
    );
    // A new one is, unless the person said not to.
    ac_io::crash::save(&crashes, &text).unwrap();
    let mut h = editor(settings_at(&dir), &url, Some(crashes.clone()));
    click(&mut h, "Don't offer crash reports again");
    click(&mut h, "Dismiss");
    assert!(!settings_at(&dir).prefs.crash_prompt);
    ac_io::crash::save(&crashes, &text).unwrap();
    let h = editor(settings_at(&dir), &url, Some(crashes));
    assert!(
        h.query_by_label("Audio Constructor stopped unexpectedly")
            .is_none()
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn report_a_problem_opens_the_issue_form_with_the_version() {
    let dir = temp("problem");
    let mut s = settings_at(&dir);
    s.set_update_checks(UpdateChecks::Off);
    let (url, _) = feed(OLDER);
    let mut h = editor(s, &url, None);
    open_help(&mut h);
    click(&mut h, "Report a Problem…");
    let opened = h
        .state()
        .product()
        .and_then(|p| p.opened())
        .unwrap()
        .to_string();
    assert!(opened.starts_with("https://github.com/Andorbal/audio-constructor/issues/new?body="));
    assert!(opened.contains(&format!(
        "Audio%20Constructor%20{}",
        env!("CARGO_PKG_VERSION")
    )));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn an_editor_without_the_product_has_no_update_entries() {
    let ctx = egui::Context::default();
    let app = EditorApp::new(&ctx, AppOptions::headless());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .with_accessibility_check(false)
        .build_ui_state(|ui, app: &mut EditorApp| app.ui(ui), app);
    h.run_steps(4);
    open_help(&mut h);
    assert!(h.query_by_label("Check for Updates…").is_none());
    assert!(h.query_by_label("Check for updates?").is_none());
}

#[test]
fn the_update_and_crash_windows_are_answered_from_the_keyboard() {
    let dir = temp("keys");
    let crashes = dir.join("crashes");
    let text = ac_io::crash::format_report(
        "audioc",
        "2026-10-19 12:34:56 UTC",
        "main",
        "something broke",
        "crates/x.rs:1:1",
        "   0: frame\n",
    );
    ac_io::crash::save(&crashes, &text).unwrap();
    let (url, _) = feed(OLDER);
    let mut h = editor(settings_at(&dir), &url, Some(crashes.clone()));
    let key = |h: &mut Harness<'static, EditorApp>, k: egui::Key| {
        h.key_press(k);
        h.run_steps(3);
    };
    // The crash window comes first, with the keyboard on Report….
    h.get_by_label("Audio Constructor stopped unexpectedly");
    assert!(h.get_by_label("Report…").is_focused());
    // Tab to Show Report, then Dismiss, and Enter presses it.
    key(&mut h, egui::Key::Tab);
    assert!(h.get_by_label("Show Report").is_focused());
    key(&mut h, egui::Key::Tab);
    assert!(h.get_by_label("Dismiss").is_focused());
    key(&mut h, egui::Key::Enter);
    assert!(
        h.query_by_label("Audio Constructor stopped unexpectedly")
            .is_none()
    );
    // Then the update question takes the keyboard.
    h.run_steps(3);
    assert!(h.get_by_label("Check Daily").is_focused());
    key(&mut h, egui::Key::Tab);
    assert!(h.get_by_label("Don't Check").is_focused());
    key(&mut h, egui::Key::Space);
    assert!(h.query_by_label("Check for updates?").is_none());
    assert_eq!(settings_at(&dir).prefs.update_checks, UpdateChecks::Off);
    std::fs::remove_dir_all(dir).ok();
}
