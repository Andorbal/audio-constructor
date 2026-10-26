//! Song-locked time in the editor (ROADMAP M4.3): the ruler moves the
//! playhead, sets the loop and the punch range, and holds markers; the
//! transport bar's position drags; the click and its count-in are a button
//! and its menu.

use std::time::{Duration, Instant};

use ac_app::audio::AudioMode;
use ac_app::{AppOptions, EditorApp};
use ac_core::{Project, TempoMap};
use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn harness() -> Harness<'static, EditorApp> {
    let ctx = egui::Context::default();
    let mut app = EditorApp::new(
        &ctx,
        AppOptions {
            audio: AudioMode::Null,
            ..AppOptions::headless()
        },
    );
    let mut p = Project::default();
    // 10 s at 120 BPM is 20 beats: the ruler shows 8 bars, 32 beats.
    p.settings.length = 10.0;
    app.open_unsaved(p);
    let mut h = Harness::builder()
        .with_size(vec2(1400.0, 900.0))
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

/// Step in real time, so the null stream runs blocks, until `done` holds.
fn wait_for(h: &mut Harness<'static, EditorApp>, mut done: impl FnMut(&EditorApp) -> bool) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(5) {
        if done(h.state()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(15));
        h.run_steps(1);
    }
    done(h.state())
}

fn ruler(h: &Harness<'static, EditorApp>) -> Rect {
    h.get_by_role_and_label(Role::Slider, "Song ruler").rect()
}

/// Where song beat `beat` is on the ruler (32 beats across, 10 px in).
fn at_beat(r: Rect, beat: f32) -> Pos2 {
    Pos2::new(
        r.left() + 10.0 + beat / 32.0 * (r.width() - 20.0),
        r.center().y,
    )
}

fn button(h: &mut Harness<'static, EditorApp>, p: Pos2, button: egui::PointerButton) {
    h.hover_at(p);
    steps(h, 2);
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos: p,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    steps(h, 3);
}

fn click(h: &mut Harness<'static, EditorApp>, p: Pos2) {
    button(h, p, egui::PointerButton::Primary);
}

fn drag(h: &mut Harness<'static, EditorApp>, from: Pos2, to: Pos2, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.hover_at(from);
    steps(h, 2);
    h.drag_at(from);
    steps(h, 2);
    for i in 1..=8 {
        let t = i as f32 / 8.0;
        h.hover_at(from + (to - from) * t);
        steps(h, 1);
    }
    h.drop_at(to);
    steps(h, 3);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    steps(h, 1);
}

fn sample_of(h: &Harness<'static, EditorApp>, beat: f64) -> u64 {
    let app = h.state();
    let rate = f64::from(app.engine.info.sample_rate);
    TempoMap::new(rate, &app.document().project.settings.tempo).sample_at(beat)
}

#[test]
fn a_click_on_the_ruler_moves_the_playhead_to_that_beat() {
    let mut h = harness();
    let r = ruler(&h);
    click(&mut h, at_beat(r, 16.2));
    let want = sample_of(&h, 16.0);
    assert!(
        wait_for(&mut h, |app| app.engine.handle.position() == want),
        "at {} not {want}",
        h.state().engine.handle.position()
    );
}

#[test]
fn dragging_the_ruler_sets_the_loop_and_shift_dragging_the_punch_range() {
    let mut h = harness();
    let r = ruler(&h);
    drag(&mut h, at_beat(r, 4.0), at_beat(r, 12.0), Modifiers::NONE);
    let s = &h.state().document().project.settings;
    assert_eq!(s.loop_region, Some([4.0, 12.0]));
    assert_eq!(h.state().document().undo_label(), Some("Loop"));
    drag(&mut h, at_beat(r, 16.0), at_beat(r, 8.0), Modifiers::SHIFT);
    let s = &h.state().document().project.settings;
    assert_eq!(s.punch, Some([8.0, 16.0]));
    assert_eq!(s.loop_region, Some([4.0, 12.0]), "the loop stays");
    assert_eq!(h.state().document().undo_label(), Some("Punch range"));
}

#[test]
fn markers_go_where_the_playhead_is_and_the_playhead_jumps_to_them() {
    let mut h = harness();
    let r = ruler(&h);
    click(&mut h, at_beat(r, 8.0));
    let eight = sample_of(&h, 8.0);
    assert!(wait_for(&mut h, |app| app.engine.handle.position() == eight));
    h.key_press_modifiers(Modifiers::ALT, Key::M);
    steps(&mut h, 3);
    let markers = &h.state().document().project.settings.markers;
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].beat, 8.0);
    assert_eq!(markers[0].name, "Marker 1");
    click(&mut h, at_beat(r, 2.0));
    let two = sample_of(&h, 2.0);
    assert!(wait_for(&mut h, |app| app.engine.handle.position() == two));
    h.key_press_modifiers(Modifiers::ALT, Key::ArrowRight);
    steps(&mut h, 3);
    assert!(wait_for(&mut h, |app| app.engine.handle.position() == eight));
    // Dragging a marker moves it.
    drag(&mut h, at_beat(r, 8.0), at_beat(r, 20.0), Modifiers::NONE);
    assert_eq!(h.state().document().project.settings.markers[0].beat, 20.0);
}

#[test]
fn the_click_is_a_button_and_its_menu_sets_the_count_in() {
    let mut h = harness();
    h.get_by_label("Metronome").click();
    steps(&mut h, 3);
    assert!(h.state().document().project.settings.metronome);
    assert_eq!(h.state().document().undo_label(), Some("Metronome"));
    let b = h.get_by_label("Metronome").rect().center();
    button(&mut h, b, egui::PointerButton::Secondary);
    h.get_by_label("2 bars").click();
    steps(&mut h, 3);
    assert_eq!(h.state().document().project.settings.count_in, 2);
    // Shift+C turns the click off again.
    h.key_press_modifiers(Modifiers::SHIFT, Key::C);
    steps(&mut h, 3);
    assert!(!h.state().document().project.settings.metronome);
}

#[test]
fn dragging_the_position_moves_the_playhead_by_beats() {
    let mut h = harness();
    let p = h.get_by_label_contains("Position: bar").rect().center();
    drag(&mut h, p, p + vec2(8.0 * 6.0, 0.0), Modifiers::NONE);
    let six = sample_of(&h, 6.0);
    assert!(
        wait_for(&mut h, |app| app.engine.handle.position() == six),
        "at {}",
        h.state().engine.handle.position()
    );
}
