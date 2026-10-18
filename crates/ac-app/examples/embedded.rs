//! Try the plugin window without a plugin host. This program stands in for
//! the host: it runs the engine's processor on the default audio device, as
//! a host's audio thread would (silently, on the null stream, if there is no
//! device), and opens the editor through the window adapter
//! (`ac_app::embed`). Each time the window closes it says whether the
//! project changed and opens it again, so the editor's state (document,
//! undo history, view) can be seen surviving.
//!
//! ```text
//! cargo run -p ac-app --no-default-features --features embed --example embedded
//! cargo run -p ac-app --no-default-features --features embed --example embedded -- --parented --opens 3
//! ```
//!
//! `--parented` puts the editor inside a plain parent window, the way a
//! host embeds it; without it the editor gets a top-level window of its
//! own. `--opens N` opens it N times (2 by default).

use std::error::Error;

use ac_app::embed::baseview::dpi::LogicalSize;
use ac_app::embed::baseview::{
    self, Event, EventStatus, HandlerError, WindowContext, WindowHandler, WindowSettings,
    WindowSize,
};
use ac_app::embed::{self, EditorWindow, SharedEditor, WindowOptions};
use ac_app::{AppOptions, EditorApp, HostAudio};
use ac_io::{AudioConfig, AudioHost, CpalAudio, EventPorts, NullAudio};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let parented = args.iter().any(|a| a == "--parented");
    let opens = args
        .windows(2)
        .find(|w| w[0] == "--opens")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(2usize);

    // The stand-in host's audio: the default device, or the null stream.
    let device: Box<dyn AudioHost> = match CpalAudio::open(&AudioConfig::default()) {
        Ok(h) => Box::new(h),
        Err(e) => {
            println!("No audio device ({e}): running silently.");
            Box::new(NullAudio::new(48_000, 256, 0, 2))
        }
    };
    let info = device.info().clone();
    let audio = HostAudio {
        host: format!("example host on {}", info.host),
        sample_rate: info.sample_rate,
        max_block: info.max_block(),
        in_channels: info.in_channels,
        out_channels: info.out_channels,
    };

    // A plugin makes its editor once and keeps it.
    let mut editor = EditorApp::embedded(AppOptions::hosted(audio));
    let processor = editor
        .take_processor()
        .ok_or("the hosted editor has no processor to hand out")?;
    let ports = EventPorts {
        controls: editor.take_control_receiver(),
        ..EventPorts::default()
    };
    let stream = device
        .start_with(processor, ports)
        .map_err(|e| e.error.to_string())?;
    let editor = embed::share(editor);

    for n in 1..=opens {
        println!("Opening the editor ({n} of {opens}). Close its window to go on.");
        if parented {
            run_parented(&editor)?;
        } else {
            EditorWindow::open_standalone(&editor, &WindowOptions::default())?
                .run_until_closed()?;
        }
        let mut e = embed::lock(&editor);
        match e.take_changes() {
            Some(p) => println!(
                "The project changed: {} nodes, {} wires; undo would {}.",
                p.graph.nodes.len(),
                p.graph.wires.len(),
                e.document().undo_label().unwrap_or("do nothing")
            ),
            None => println!("The project didn't change."),
        }
        // While no window shows the editor, the host keeps the engine going.
        e.upkeep();
    }
    drop(stream.stop());
    Ok(())
}

/// A plain window holding the editor as its child, as a host's would.
struct StandInHost {
    _editor: EditorWindow,
}

impl WindowHandler for StandInHost {
    fn draw(&self) -> Result<(), HandlerError> {
        Ok(())
    }

    fn resized(&self, _size: WindowSize) -> Result<(), HandlerError> {
        Ok(())
    }

    fn on_event(&self, _event: Event) -> EventStatus {
        EventStatus::Ignored
    }
}

fn run_parented(editor: &SharedEditor) -> Result<(), Box<dyn Error>> {
    let opts = WindowOptions {
        resizable: false,
        ..WindowOptions::default()
    };
    let scale = f64::from(embed::lock(editor).settings().prefs.ui_scale);
    let settings = WindowSettings::new()
        .with_title("A stand-in plugin host")
        .with_size(LogicalSize::new(opts.size[0] * scale, opts.size[1] * scale))
        .with_resizable(false);
    let editor = editor.clone();
    let host = baseview::Window::create(settings, move |cx: WindowContext| {
        // The host hands its window to the plugin, which opens the editor
        // inside it; closing the host's window closes the editor's.
        let child = EditorWindow::open_parented(&cx, &editor, &opts)?;
        Ok(StandInHost { _editor: child })
    })?;
    host.run_until_closed()?;
    Ok(())
}
