//! Plugins in the editor (egui_kittest, null audio stream): installed
//! plugins are offered by name in the palette and the Library tab, adding
//! one makes a Plugin node with its sockets, the Node tab says whether it
//! runs, a missing one warns, and saving stores a running plugin's state
//! in its node.
//!
//! The plugin that runs is our own Audio Constructor Instrument, from the
//! `ac-plugin` library `cargo test --workspace` builds; without it, only
//! the missing-plugin half runs (the hosting is tested in `ac-host`).

use std::path::{Path, PathBuf};

use ac_app::audio::AudioMode;
use ac_app::palette;
use ac_app::plugins::PluginsConfig;
use ac_app::{AppOptions, EditorApp};
use ac_core::{PLUGIN_KIND, PluginDesc, PluginFormat, PluginParam, PluginPort, Project};
use ac_host::Catalog;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn plugin_library() -> Option<PathBuf> {
    let name = format!(
        "{}ac_plugin{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let exe = std::env::current_exe().ok()?;
    let deps = exe.parent()?;
    [Some(deps), deps.parent()]
        .into_iter()
        .flatten()
        .map(|d| d.join(&name))
        .find(|p| p.is_file())
}

/// Our instrument, described from a `.clap` link to the library.
fn our_instrument() -> Option<PluginDesc> {
    let lib = plugin_library()?;
    let dir = std::env::temp_dir().join(format!("ac-app-plugins-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let clap = dir.join("Audio Constructor.clap");
    let _ = std::fs::remove_file(&clap);
    link_or_copy(&lib, &clap);
    ac_host::describe_bundle(&clap)
        .ok()?
        .into_iter()
        .find(|d| d.id.ends_with(".instrument"))
}

fn link_or_copy(src: &Path, dst: &Path) {
    #[cfg(unix)]
    {
        if std::os::unix::fs::symlink(src, dst).is_ok() {
            return;
        }
    }
    let _ = std::fs::copy(src, dst);
}

/// A plugin that isn't installed anywhere.
fn absent() -> PluginDesc {
    PluginDesc {
        format: PluginFormat::Clap,
        id: "org.example.absent".into(),
        name: "Absent Synth".into(),
        vendor: "Example Audio".into(),
        path: "/nowhere/Absent Synth.clap".into(),
        instrument: true,
        audio_out: vec![PluginPort {
            name: "Out".into(),
            channels: 2,
        }],
        midi_in: true,
        params: vec![PluginParam {
            id: 3,
            name: "Brightness".into(),
            socket: "brightness".into(),
            min: 0.0,
            max: 1.0,
            default: 0.5,
            stepped: false,
        }],
        ..PluginDesc::default()
    }
}

fn harness(plugins: Vec<PluginDesc>) -> Harness<'static, EditorApp> {
    let ctx = egui::Context::default();
    let mut app = EditorApp::new(
        &ctx,
        AppOptions {
            audio: AudioMode::Null,
            plugins: Some(PluginsConfig::with_catalog(Catalog {
                plugins,
                ..Catalog::default()
            })),
            ..AppOptions::headless()
        },
    );
    app.open_unsaved(Project::default());
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .with_max_steps(64)
        .with_step_dt(1.0 / 30.0)
        .with_accessibility_check(false)
        .build_ui_state(|ui, app: &mut EditorApp| app.ui(ui), app);
    h.run_steps(4);
    h
}

fn add(h: &mut Harness<'static, EditorApp>, query: &str) -> ac_core::NodeId {
    let plugins = h.state().plugins.as_ref().unwrap().list.clone();
    let c = palette::candidates(
        &h.state().registry,
        &h.state().library,
        h.state().document(),
        query,
        None,
        &plugins,
    )
    .into_iter()
    .find(|c| c.kind == PLUGIN_KIND)
    .unwrap_or_else(|| panic!("no plugin for {query:?}"));
    let id = h
        .state_mut()
        .add_candidate(&c, egui::pos2(0.0, 0.0), None, 0.0)
        .expect("added");
    h.run_steps(4);
    id
}

#[test]
fn installed_plugins_are_offered_by_name_and_the_bare_node_isnt() {
    let h = harness(vec![absent()]);
    let plugins = h.state().plugins.as_ref().unwrap().list.clone();
    let all = palette::candidates(
        &h.state().registry,
        &h.state().library,
        h.state().document(),
        "",
        None,
        &plugins,
    );
    let names: Vec<&str> = all
        .iter()
        .filter(|c| c.kind == PLUGIN_KIND)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(names, ["Absent Synth"]);
    let synth = palette::candidates(
        &h.state().registry,
        &h.state().library,
        h.state().document(),
        "absent",
        None,
        &plugins,
    );
    assert_eq!(synth[0].name, "Absent Synth");
}

#[test]
fn a_missing_plugin_keeps_its_sockets_and_says_so() {
    let mut h = harness(vec![absent()]);
    let id = add(&mut h, "Absent Synth");
    let node = h
        .state()
        .document()
        .project
        .graph
        .node(id)
        .cloned()
        .unwrap();
    assert_eq!(node.kind, PLUGIN_KIND);
    assert_eq!(PluginDesc::from_config(&node.config), Some(absent()));
    let report = h.state().engine.report.to_string();
    assert!(
        report.contains("Absent Synth (Example Audio, CLAP): isn't installed"),
        "{report}"
    );
    // The Node tab shows who it is and what's wrong.
    h.state_mut().doc.selection = [id].into();
    h.run_steps(4);
    h.get_by_label_contains("isn't installed on this computer");
}

#[test]
fn a_running_plugin_saves_its_state_into_its_node() {
    let Some(inst) = our_instrument() else {
        eprintln!(
            "skipped: the ac-plugin library isn't built (`cargo test --workspace` builds it)"
        );
        return;
    };
    let mut h = harness(vec![inst.clone()]);
    let id = add(&mut h, "Audio Constructor Instrument");
    let report = h.state().engine.report.to_string();
    assert!(!report.contains("warning"), "{report}");
    assert!(ac_host::running().contains(&id.to_string()));
    h.state_mut().doc.selection = [id].into();
    h.run_steps(4);
    h.get_by_label("Running");
    let dir = std::env::temp_dir().join(format!("ac-app-plugin-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("hosted.acproj");
    h.state_mut().save_to(&file, 1.0);
    h.run_steps(2);
    let saved = ac_io::load_project(&file).unwrap();
    let node = saved.graph.node(id).unwrap();
    let state = node
        .config
        .get("state")
        .and_then(|s| s.as_str())
        .unwrap_or("");
    assert!(!state.is_empty(), "the plugin's state is saved");
    // Saving didn't rebuild the running plugin (still exactly one).
    assert_eq!(ac_host::running(), vec![id.to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}
