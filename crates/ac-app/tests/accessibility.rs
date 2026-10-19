//! The editor for screen readers and the keyboard (egui_kittest, docs/UX.md
//! "Accessibility and scaling"): every interactive thing in the main window
//! has a name, the canvas's cards and sockets can be found by what they're
//! called, and a patch can be built, set, and played without the pointer.

use ac_app::audio::AudioMode;
use ac_app::panels::Tab;
use ac_app::{AppOptions, EditorApp};
use ac_core::{Endpoint, NodeId, NodeInstance, Project, Value};
use egui::accesskit::{Action, Role, Toggled};
use egui::{Key, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

fn harness_with(opts: AppOptions, project: Option<Project>) -> Harness<'static, EditorApp> {
    let ctx = egui::Context::default();
    let mut app = EditorApp::new(&ctx, opts);
    if let Some(p) = project {
        app.open_unsaved(p);
    }
    let mut h = Harness::builder()
        .with_size(vec2(1400.0, 900.0))
        .with_max_steps(64)
        .with_step_dt(1.0 / 30.0)
        .with_accessibility_check(false)
        .build_ui_state(|ui, app: &mut EditorApp| app.ui(ui), app);
    h.run_steps(4);
    h
}

/// The editor on the Start example, as it opens for someone new.
fn start() -> Harness<'static, EditorApp> {
    let h = harness_with(
        AppOptions {
            audio: AudioMode::Null,
            start_example: true,
            ..AppOptions::headless()
        },
        None,
    );
    assert!(
        h.state().document().graph().nodes.len() > 4,
        "the Start example opened"
    );
    h
}

fn press(h: &mut Harness<'static, EditorApp>, modifiers: Modifiers, key: Key) {
    h.key_press_modifiers(modifiers, key);
    h.run_steps(3);
}

fn key(h: &mut Harness<'static, EditorApp>, key: Key) {
    press(h, Modifiers::NONE, key);
}

/// Icons are text in the icon font (Private Use Area): a name made only of
/// them says nothing.
fn says_something(label: &str) -> bool {
    label
        .chars()
        .filter(|c| !is_icon(*c))
        .any(char::is_alphanumeric)
}

fn is_icon(c: char) -> bool {
    ('\u{E000}'..='\u{F8FF}').contains(&c)
}

/// Everything in the window a person can press, type in, or move to that
/// has no name a screen reader could say, with its role and where it is.
fn unlabelled(h: &Harness<'static, EditorApp>) -> Vec<String> {
    let mut out = Vec::new();
    for n in h.root().children_recursive() {
        let a = n.accesskit_node();
        if a.is_hidden() {
            continue;
        }
        let interactive = a.data().supports_action(Action::Focus)
            || a.data().supports_action(Action::Click)
            || a.is_text_input()
            || matches!(
                a.role(),
                Role::Slider | Role::SpinButton | Role::CheckBox | Role::ComboBox
            );
        // egui's scroll bars say their role and nothing else; the area
        // they scroll also scrolls with the wheel and a screen reader's
        // own scroll commands.
        if !interactive || a.role() == Role::ScrollBar {
            continue;
        }
        let label = a.label().unwrap_or_default();
        let value = a.value().unwrap_or_default();
        // A label says its text, and a text field may be named by what's
        // in it (a name field shows the name).
        // Text says itself (a bullet is read as one); a text field may be
        // named by what's in it.
        let speaks = if a.label_comes_from_value() {
            value.chars().any(|c| !c.is_whitespace() && !is_icon(c))
        } else {
            says_something(&label) || (a.is_text_input() && says_something(&value))
        };
        // An icon in a name is read out as its code point, or as nothing.
        if speaks && !label.chars().any(is_icon) {
            continue;
        }
        let at = a
            .bounding_box()
            .map(|r| format!("{:.0},{:.0} {:.0}x{:.0}", r.x0, r.y0, r.width(), r.height()))
            .unwrap_or_default();
        out.push(format!("{:?} {label:?} value {value:?} at {at}", a.role()));
    }
    out
}

fn assert_all_labelled(h: &Harness<'static, EditorApp>, where_: &str) {
    let bad = unlabelled(h);
    assert!(
        bad.is_empty(),
        "unlabelled in {where_}:\n{}",
        bad.join("\n")
    );
}

#[test]
fn every_interactive_thing_in_the_main_window_has_a_name() {
    let mut h = start();
    assert!(h.state().side.open, "the side panel shows");
    // A node selected, so the Node tab has its editors.
    let osc = h
        .state()
        .document()
        .graph()
        .nodes
        .iter()
        .find(|n| n.kind == "oscillator")
        .map(|n| n.id)
        .unwrap();
    h.state_mut().doc.selection = [osc].into();
    h.run_steps(3);
    for tab in [
        Tab::Node,
        Tab::Library,
        Tab::Project,
        Tab::Devices,
        Tab::Problems,
        Tab::Help,
    ] {
        h.state_mut().side.tab = tab;
        h.run_steps(3);
        assert_all_labelled(&h, &format!("the canvas with the {tab:?} tab"));
    }
    // The panels (the performance view).
    press(&mut h, Modifiers::COMMAND, Key::Tab);
    assert_all_labelled(&h, "the panels");
}

/// The editor on a project of its own, with the null audio stream.
fn harness(project: Project) -> Harness<'static, EditorApp> {
    harness_with(
        AppOptions {
            audio: AudioMode::Null,
            ..AppOptions::headless()
        },
        Some(project),
    )
}

/// An example from `examples/`, opened as it would be from the menu.
fn example(name: &str) -> Harness<'static, EditorApp> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name);
    harness_with(
        AppOptions {
            audio: AudioMode::Null,
            project: Some(path),
            ..AppOptions::headless()
        },
        None,
    )
}

#[test]
fn the_panels_and_performance_mode_have_names_too() {
    let mut h = example("build-a-keyboard.acproj");
    assert!(!h.state().document().project.panels.is_empty());
    press(&mut h, Modifiers::COMMAND, Key::Tab);
    assert!(h.state().perform.shown);
    assert!(!h.state().perform.cells.is_empty(), "the panel has cards");
    assert_all_labelled(&h, "a panel");
    // Every card and its header buttons are there by name.
    let _ = h.get_by_role_and_label(Role::Slider, "Cutoff (Hz)");
    let _ = h.get_by_role_and_label(Role::Switch, "Demo");
    let _ = h.get_by_role_and_label(Role::Button, "Unpin Cutoff (Hz)");
    press(&mut h, Modifiers::NONE, Key::F11);
    assert!(h.state().perform.fullscreen);
    assert_all_labelled(&h, "performance mode");
}

/// A gain called Level after an oscillator, wired.
fn osc_and_level() -> Project {
    let mut p = Project::default();
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(1), "oscillator").at(0.0, 0.0));
    let mut gain = NodeInstance::new(NodeId(2), "gain").at(320.0, 0.0);
    gain.ui.label = Some("Level".into());
    gain.params.insert("gain".into(), Value::Float(-6.0));
    p.graph.nodes.push(gain);
    p.graph.wires.push(ac_core::Wire::new(
        Endpoint::new(NodeId(1), "out"),
        Endpoint::new(NodeId(2), "in"),
    ));
    p
}

#[test]
fn cards_and_sockets_are_found_by_what_they_are_called() {
    let mut h = harness(osc_and_level());
    let _ = h.get_by_role_and_label(Role::Canvas, "Node canvas, 2 nodes");
    // A card says its name, its kind when it has a name of its own, and
    // how many sockets it has.
    let level = h.get_by_role_and_label(Role::Button, "Level, Gain, 2 inputs, 1 output");
    assert!(!level.accesskit_node().is_selected().unwrap_or(false));
    let osc = h
        .query_all_by_role(Role::Button)
        .filter_map(|n| n.accesskit_node().label())
        .find(|l| l.starts_with("Oscillator, ") && l.contains("inputs"))
        .expect("the oscillator's card");
    assert!(!osc.contains("Oscillator, Oscillator"), "{osc}");
    // A socket says its card, its name, its type and side, and what it's
    // wired to or set to.
    let _ = h.get_by_label("Oscillator out: Float output, to Level in");
    let _ = h.get_by_label("Level in: Audio input, from Oscillator out");
    let _ = h.get_by_label("Level gain: Float input, -6 dB");

    // A screen reader's activate selects a card, and says so.
    h.get_by_role_and_label(Role::Button, "Level, Gain, 2 inputs, 1 output")
        .click_accesskit();
    h.run_steps(3);
    assert_eq!(
        h.state().document().selection,
        [NodeId(2)].into(),
        "selected from AccessKit"
    );
    let level = h.get_by_role_and_label(Role::Button, "Level, Gain, 2 inputs, 1 output, selected");
    assert_eq!(level.accesskit_node().is_selected(), Some(true));
    // A screen reader moving its focus to a socket moves the canvas's.
    h.get_by_label("Level gain: Float input, -6 dB").focus();
    h.run_steps(3);
    assert!(h.state().canvas.socket_focused());
}

#[test]
fn a_patch_is_built_set_and_played_from_the_keyboard_alone() {
    use ac_app::canvas::Focus;
    let mut p = Project::default();
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(1), "oscillator").at(0.0, 0.0));
    p.graph
        .nodes
        .push(NodeInstance::new(NodeId(2), "audio_out").at(640.0, 0.0));
    let mut h = harness(p);
    let focus = |h: &Harness<'static, EditorApp>| h.state().canvas.focus.clone();

    // Tab reaches the first card; the selection follows it.
    key(&mut h, Key::Tab);
    assert_eq!(focus(&h), Some(Focus::Node(NodeId(1))));
    assert_eq!(h.state().document().selection, [NodeId(1)].into());

    // Add a Gain from the palette: nothing selected, so nothing is wired
    // into it, and it takes the focus.
    press(&mut h, Modifiers::ALT, Key::A);
    press(&mut h, Modifiers::SHIFT, Key::A);
    assert!(h.state().palette.open);
    h.event(egui::Event::Text("gain".into()));
    h.run_steps(3);
    key(&mut h, Key::Enter);
    assert!(!h.state().palette.open);
    let gain = h
        .state()
        .document()
        .graph()
        .nodes
        .iter()
        .find(|n| n.kind == "gain")
        .map(|n| n.id)
        .expect("a gain was added");
    assert_eq!(focus(&h), Some(Focus::Node(gain)));
    assert!(h.state().document().graph().wires.is_empty());

    // Wire the oscillator into it: Alt+Down to the gain's input, Enter
    // picks it up, Tab around to the oscillator, Enter connects.
    press(&mut h, Modifiers::ALT, Key::ArrowDown);
    assert!(matches!(focus(&h), Some(Focus::Socket { ref name, input: true, .. }) if name == "in"));
    key(&mut h, Key::Enter);
    assert!(h.state().canvas.keyboard_wire().is_some());
    while focus(&h) != Some(Focus::Node(NodeId(1))) {
        key(&mut h, Key::Tab);
    }
    key(&mut h, Key::Enter);
    let wired = |h: &Harness<'static, EditorApp>, from: NodeId, to: NodeId| {
        h.state()
            .document()
            .graph()
            .wires
            .iter()
            .any(|w| w.from.node == from && w.to.node == to)
    };
    assert!(wired(&h, NodeId(1), gain), "oscillator into gain");

    // And the gain into the output, from the output's side.
    while focus(&h) != Some(Focus::Node(NodeId(2))) {
        key(&mut h, Key::Tab);
    }
    press(&mut h, Modifiers::ALT, Key::ArrowDown);
    key(&mut h, Key::Enter);
    while focus(&h) != Some(Focus::Node(gain)) {
        press(&mut h, Modifiers::SHIFT, Key::Tab);
    }
    key(&mut h, Key::Enter);
    assert!(wired(&h, gain, NodeId(2)), "gain into the output");

    // Set the gain: Alt+Down twice to its gain input, F2, type, Enter.
    press(&mut h, Modifiers::ALT, Key::ArrowDown);
    press(&mut h, Modifiers::ALT, Key::ArrowDown);
    assert!(matches!(focus(&h), Some(Focus::Socket { ref name, .. }) if name == "gain"));
    key(&mut h, Key::F2);
    assert!(h.state().canvas.is_typing());
    h.event(egui::Event::Text("-12".into()));
    h.run_steps(2);
    key(&mut h, Key::Enter);
    assert!(!h.state().canvas.is_typing());
    let set = h
        .state()
        .document()
        .graph()
        .node(gain)
        .and_then(|n| n.params.get("gain").cloned());
    assert_eq!(set, Some(Value::Float(-12.0)));
    // The keyboard is back on the socket, ring and all.
    assert!(h.state().canvas.socket_focused());

    // Space plays.
    key(&mut h, Key::Space);
    std::thread::sleep(std::time::Duration::from_millis(50));
    h.run_steps(2);
    assert!(h.state().engine.handle.is_playing());
}

#[test]
fn panel_controls_play_from_the_keyboard() {
    let mut h = example("build-a-keyboard.acproj");
    press(&mut h, Modifiers::COMMAND, Key::Tab);
    let node = |h: &Harness<'static, EditorApp>, label: &str| {
        h.state()
            .document()
            .graph()
            .nodes
            .iter()
            .find(|n| n.ui.label.as_deref() == Some(label))
            .map(|n| n.id)
            .unwrap()
    };
    let cutoff = node(&h, "Cutoff (Hz)");
    let value = |h: &Harness<'static, EditorApp>| match h
        .state()
        .document()
        .graph()
        .node(cutoff)
        .and_then(|n| n.params.get("value").cloned())
    {
        Some(Value::Float(v)) => v,
        other => panic!("{other:?}"),
    };
    let before = value(&h);
    // A screen reader (or Tab) puts the keyboard on the knob.
    h.get_by_role_and_label(Role::Slider, "Cutoff (Hz)").focus();
    h.run_steps(2);
    key(&mut h, Key::ArrowUp);
    assert!(value(&h) > before, "up turns it up");
    press(&mut h, Modifiers::SHIFT, Key::ArrowDown);
    assert!(value(&h) < before, "Shift+Down turns it down by more");
    // Enter types a value.
    key(&mut h, Key::Enter);
    assert!(h.state().perform.is_typing());
    h.event(egui::Event::Text("800".into()));
    h.run_steps(2);
    key(&mut h, Key::Enter);
    assert!((value(&h) - 800.0).abs() < 1e-6, "{}", value(&h));
    // A screen reader's increment and set value work as well.
    h.get_by_role_and_label(Role::Slider, "Cutoff (Hz)")
        .set_value_accesskit(1200.0);
    h.run_steps(2);
    assert!((value(&h) - 1200.0).abs() < 1e-6);
}

#[test]
fn panel_buttons_hold_with_space_and_toggles_flip() {
    let mut p = Project::default();
    for (i, kind) in [(1, "button"), (2, "toggle")] {
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(i), kind).at(260.0 * i as f32, 0.0));
    }
    let mut panel = ac_core::Panel::new("Live");
    panel.items = vec![
        ac_core::PanelItem::new(NodeId(1)),
        ac_core::PanelItem::new(NodeId(2)),
    ];
    p.panels.push(panel);
    let mut h = harness(p);
    press(&mut h, Modifiers::COMMAND, Key::Tab);
    let _ = h.get_by_role_and_label(Role::Group, "Panel Live, 2 controls");
    // Tab from the tab strip reaches the cards in order.
    h.get_by_role_and_label(Role::Button, "Button").focus();
    h.run_steps(2);
    let held = |h: &Harness<'static, EditorApp>| h.state().perform.controls.held_inputs(NodeId(1));
    h.key_down(Key::Space);
    h.run_steps(2);
    assert_eq!(held(&h), ["pressed"], "Space holds the button down");
    assert!(!h.state().engine.handle.is_playing(), "and doesn't play");
    h.key_up(Key::Space);
    h.run_steps(3);
    assert!(held(&h).is_empty(), "and lets go with it");
    // The toggle says whether it's on, and Space flips it.
    key(&mut h, Key::Tab);
    let switch = |h: &Harness<'static, EditorApp>| {
        h.get_by_role_and_label(Role::Switch, "Toggle")
            .accesskit_node()
            .toggled()
    };
    assert_eq!(switch(&h), Some(egui::accesskit::Toggled::False));
    key(&mut h, Key::Space);
    let mut on = false;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
        if switch(&h) == Some(egui::accesskit::Toggled::True) {
            on = true;
            break;
        }
    }
    assert!(on, "the toggle flipped");
}

/// egui's animation time, in both theme slots.
fn animation(h: &Harness<'static, EditorApp>) -> f32 {
    let t = [egui::Theme::Dark, egui::Theme::Light].map(|t| h.ctx.style_of(t).animation_time);
    assert_eq!(t[0], t[1]);
    t[0]
}

#[test]
fn reduce_motion_turns_off_animation_and_is_remembered() {
    let dir = std::env::temp_dir().join(format!(
        "ac-app-motion-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let file = dir.join("settings.json");
    let opts = || AppOptions {
        audio: AudioMode::Null,
        settings: ac_app::settings::AppSettings::load_from(Some(file.clone())),
        ..AppOptions::headless()
    };
    let mut h = harness_with(opts(), Some(osc_and_level()));
    assert!(animation(&h) > 0.0);
    h.get_by_role_and_label(Role::Button, "View").click();
    h.run_steps(3);
    h.get_by_label_contains("Reduce Motion").click();
    h.run_steps(3);
    assert!(h.state().settings().prefs.reduce_motion);
    assert_eq!(animation(&h), 0.0, "egui's own animations");
    // A fresh node appears at once rather than fading in.
    let h = harness_with(opts(), Some(osc_and_level()));
    assert!(h.state().settings().prefs.reduce_motion, "remembered");
    assert_eq!(animation(&h), 0.0);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn every_menu_entry_has_a_name_and_says_whether_it_is_checked() {
    let mut h = start();
    let osc = h.state().document().graph().nodes[1].id;
    h.state_mut().doc.selection = [osc].into();
    h.run_steps(2);
    let open = |h: &mut Harness<'static, EditorApp>, path: &[&str]| {
        for _ in 0..3 {
            h.key_press(Key::Escape);
            h.run_steps(2);
        }
        for (i, label) in path.iter().enumerate() {
            if i == 0 {
                // The menu bar's, not the Node tab's Help button.
                h.query_all_by_role_and_label(Role::Button, label)
                    .next()
                    .unwrap()
                    .click();
            } else {
                h.get_by_label(label).click();
            }
            h.run_steps(3);
        }
    };
    for path in [
        &["File"][..],
        &["File", "Open Recent"],
        &["File", "Examples"],
        &["Edit"],
        &["Edit", "Align / Distribute…"],
        &["View"],
        &["View", "Theme"],
        &["View", "Language"],
        &["View", "UI Scale"],
        &["Add"],
        &["Add", "Filters"],
        &["Transport"],
        &["Help"],
    ] {
        open(&mut h, path);
        assert_all_labelled(&h, &path.join(" ▸ "));
    }
    // A check mark is a checked state, and a theme is one of several.
    open(&mut h, &["View", "Theme"]);
    let dark = h.get_by_role_and_label(Role::MenuItemRadio, "Dark");
    assert_eq!(dark.accesskit_node().toggled(), Some(Toggled::True));
    let light = h.get_by_role_and_label(Role::MenuItemRadio, "Light");
    assert_eq!(light.accesskit_node().toggled(), Some(Toggled::False));
    open(&mut h, &["View"]);
    let side = h
        .query_all_by_role(Role::MenuItemCheckBox)
        .find(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with("Side Panel"))
        })
        .expect("View ▸ Side Panel");
    assert_eq!(side.accesskit_node().toggled(), Some(Toggled::True));
}
