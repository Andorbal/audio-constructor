//! Every shipped example opens, is in canonical form, compiles without
//! errors, and makes sound when it doesn't depend on a live input.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ac_engine::{RenderOptions, render};
use ac_io::{AssetStore, FileResources};

fn examples() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("examples/ exists")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "acproj"))
        .collect();
    v.sort();
    v
}

fn render_example(path: &Path, seconds: f64) -> ac_engine::Rendered {
    let registry = Arc::new(ac_nodes::registry());
    let project = ac_io::load_project(path).expect("example loads");
    let assets = AssetStore::load_for(&project, path.parent().expect("dir"), &registry);
    assert!(
        assets.issues.is_empty(),
        "{}: {:?}",
        path.display(),
        assets.issues
    );
    render(
        &project,
        registry,
        Arc::new(FileResources {
            assets,
            recorder: None,
        }),
        &RenderOptions {
            to: Some(seconds),
            ..RenderOptions::default()
        },
    )
}

#[test]
fn examples_are_canonical_and_compile_cleanly() {
    let list = examples();
    assert!(list.len() >= 5, "expected the five Phase 1 examples");
    for path in list {
        let text = std::fs::read_to_string(&path).expect("reads");
        let project = ac_core::Project::from_json(&text).expect("parses");
        assert_eq!(
            project.to_json(),
            text,
            "{} is not canonical (run audioc fmt)",
            path.display()
        );
        let out = render_example(&path, 0.5);
        assert!(
            !out.report.has_errors(),
            "{}:\n{}",
            path.display(),
            out.report
        );
    }
}

#[test]
fn examples_without_live_inputs_make_sound() {
    for name in ["start", "subtractive-synth", "four-tracks"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(format!("{name}.acproj"));
        let out = render_example(&path, 2.0);
        let peak = out
            .channels
            .iter()
            .map(|c| ac_core::testing::peak(c))
            .fold(0.0, f32::max);
        assert!(peak > 0.01, "{name} is silent (peak {peak})");
        assert!(peak <= 1.0, "{name} clips (peak {peak})");
    }
}

#[test]
fn subtractive_synth_cli_render_is_bit_identical() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/subtractive-synth.acproj");
    let wav = std::env::temp_dir().join(format!("ac-synth-{}.wav", std::process::id()));
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_audioc"))
        .args(["render", "-q", "--bit-depth", "32f", "--to", "4", "-o"])
        .arg(&wav)
        .arg(&path)
        .status()
        .expect("audioc runs");
    assert!(status.success());
    let cli = ac_io::read_wav(&wav).expect("reads");
    let lib = render_example(&path, 4.0);
    assert_eq!(cli.channels, lib.channels);
    std::fs::remove_file(wav).ok();
}
