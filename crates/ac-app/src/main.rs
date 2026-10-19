//! `audio-constructor`: the desktop editor.
//!
//! `audio-constructor [project.acproj]` opens a project (or a pack, or
//! imports a block file); with no argument it opens the Start example (or a
//! fresh Oscillator → Audio Out patch). The person's blocks live in the
//! platform data folder, under `blocks/`.

use ac_app::audio::AudioMode;
use ac_app::files::Recent;
use ac_app::{AppOptions, EditorApp};

fn main() -> eframe::Result {
    let project = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let opts = AppOptions {
        audio: AudioMode::Device(ac_io::AudioConfig::default()),
        project,
        start_example: true,
        recent: Recent::load_default(),
        native_dialogs: std::env::var_os("AC_NO_NATIVE_DIALOGS").is_none(),
        seed: ac_app::clock_seed(),
        settings: ac_app::settings::AppSettings::load_default(),
        library: ac_app::files::default_library_dir(),
    };
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Audio Constructor")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([720.0, 480.0]),
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        "Audio Constructor",
        native,
        Box::new(move |cc| Ok(Box::new(EditorApp::new(&cc.egui_ctx, opts)))),
    )
}
