//! `audio-constructor`: the desktop editor.
//!
//! `audio-constructor [project.acproj]` opens a project (or a pack, or
//! imports a block file); with no argument it opens the Start example (or a
//! fresh Oscillator → Audio Out patch). The person's blocks live in the
//! platform data folder, under `blocks/`. `audio-constructor --version`
//! prints the version without opening a window.

use ac_app::audio::AudioMode;
use ac_app::files::Recent;
use ac_app::{AppOptions, EditorApp};

/// The window and taskbar icon (scripts/make_icon.py draws it).
const ICON: &[u8] = include_bytes!("../../../packaging/icons/icon-256.png");

fn main() -> eframe::Result {
    // A scanning child: describe one plugin bundle and exit (the editor
    // runs itself this way so a crashing plugin can't take it down).
    let mut args = std::env::args_os().skip(1);
    if let Some(first) = args.next()
        && first == ac_app::plugins::SCAN_FLAG
    {
        let code = args
            .next()
            .map_or(2, |b| ac_host::child_main(std::path::Path::new(&b)));
        std::process::exit(code);
    }
    // A bug that panics leaves a report, offered at the next start.
    if let Some(dir) = ac_io::crash::default_dir() {
        ac_io::crash::install(dir, "audio-constructor");
    }
    let project = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    if project
        .as_deref()
        .is_some_and(|p| p == "--version" || p == "-V")
    {
        println!("audio-constructor {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let opts = AppOptions {
        audio: AudioMode::Device(ac_io::AudioConfig::default()),
        project,
        start_example: true,
        recent: Recent::load_default(),
        native_dialogs: std::env::var_os("AC_NO_NATIVE_DIALOGS").is_none(),
        seed: ac_app::clock_seed(),
        settings: ac_app::settings::AppSettings::load_default(),
        library: ac_app::files::default_library_dir(),
        plugins: Some(ac_app::plugins::PluginsConfig::desktop()),
        host_files: false,
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Audio Constructor")
        // Matches the menu entry's file name, so Wayland desktops show
        // its icon and group the windows under it.
        .with_app_id("audio-constructor")
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([720.0, 480.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(ICON) {
        viewport = viewport.with_icon(icon);
    }
    let native = eframe::NativeOptions {
        viewport,
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        "Audio Constructor",
        native,
        Box::new(move |cc| {
            let mut app = EditorApp::new(&cc.egui_ctx, opts);
            app.enable_product(&cc.egui_ctx, ac_app::product::ProductOptions::desktop());
            Ok(Box::new(app))
        }),
    )
}
