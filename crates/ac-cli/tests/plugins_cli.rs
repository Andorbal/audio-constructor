//! `audioc plugins` and rendering a project that hosts a plugin, through
//! the `audioc` binary: our own CLAP build in a folder `CLAP_PATH` names,
//! scanned in child processes, inserted into a project, and rendered.
//!
//! The plugin library is the `ac-plugin` workspace member's, which
//! `cargo test --workspace` builds before any test runs; run on its own,
//! this test says so and passes without it (the hosting itself is tested
//! in `ac-host`, which builds the library as a dev-dependency).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

fn audioc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_audioc"))
}

/// The plugin library cargo built in this target folder, if any.
fn plugin_library() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("AC_PLUGIN_LIBRARY") {
        return Some(PathBuf::from(p));
    }
    let name = format!(
        "{}ac_plugin{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_audioc"));
    let profile = exe.parent()?;
    [profile.join("deps").join(&name), profile.join(&name)]
        .into_iter()
        .find(|p| p.is_file())
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ac-cli-plugins-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn link_or_copy(src: &Path, dst: &Path) {
    #[cfg(unix)]
    {
        if std::os::unix::fs::symlink(src, dst).is_ok() {
            return;
        }
    }
    std::fs::copy(src, dst).unwrap();
}

/// A folder with `Audio Constructor.clap`, and the environment that makes
/// `audioc` look there (and keep its cache in `dir`).
fn setup(dir: &Path, lib: &Path) -> Vec<(&'static str, PathBuf)> {
    let clap = dir.join("clap");
    std::fs::create_dir_all(&clap).unwrap();
    link_or_copy(lib, &clap.join("Audio Constructor.clap"));
    vec![
        ("CLAP_PATH", clap),
        ("AC_PLUGIN_CACHE", dir.join("plugins.json")),
    ]
}

fn run(cmd: &mut Command, env: &[(&str, PathBuf)]) -> std::process::Output {
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// The strongest frequency between 50 Hz and 2 kHz, to the nearest Hz.
fn pitch(x: &[f32], rate: f64) -> f64 {
    let mag = |f: f64| {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, v) in x.iter().enumerate() {
            let a = std::f64::consts::TAU * f * i as f64 / rate;
            re += f64::from(*v) * a.cos();
            im += f64::from(*v) * a.sin();
        }
        re.hypot(im)
    };
    (50..2000)
        .map(|f| (f, mag(f64::from(f))))
        .fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a })
        .0
        .into()
}

#[test]
fn audioc_lists_and_renders_a_hosted_plugin() {
    let Some(lib) = plugin_library() else {
        eprintln!(
            "skipped: the ac-plugin library isn't built (`cargo test --workspace` builds it)"
        );
        return;
    };
    let dir = temp("render");
    let env = setup(&dir, &lib);
    // The list, as JSON: both plugins, described in child processes.
    let out = run(audioc().args(["plugins", "--json"]), &env);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let catalog: Value = serde_json::from_slice(&out.stdout).unwrap();
    let find = |id: &str| {
        catalog["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("{id} not listed: {catalog}"))
    };
    let instrument = find("io.github.andorbal.audio-constructor.instrument");
    find("io.github.andorbal.audio-constructor.effect");
    assert!(dir.join("plugins.json").is_file(), "the cache is written");
    // The human-readable list names it and its folder.
    let out = run(audioc().arg("plugins"), &env);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Audio Constructor Instrument"), "{text}");
    assert!(text.contains("CLAP folders:"), "{text}");
    // A project playing A4 on it for one second.
    let project = json!({
        "schema_version": 2,
        "name": "Hosted synth",
        "settings": { "sample_rate": 48000, "length": 1.5 },
        "graph": {
            "nodes": [
                { "id": "1", "kind": "midi_pattern", "config": { "notes": [[0, 2, 69, 100]], "length": 4 } },
                { "id": "2", "kind": "plugin", "config": { "plugin": instrument } },
                { "id": "3", "kind": "audio_out" }
            ],
            "wires": [
                { "from": "1:midi", "to": "2:midi" },
                { "from": "2:out", "to": "3:in" }
            ]
        }
    });
    let file = dir.join("synth.acproj");
    std::fs::write(&file, serde_json::to_vec_pretty(&project).unwrap()).unwrap();
    let wav = dir.join("synth.wav");
    let out = run(
        audioc()
            .arg("render")
            .arg(&file)
            .arg("-o")
            .arg(&wav)
            .args(["--bit-depth", "32f"]),
        &env,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let audio = ac_io::read_wav(&wav).unwrap();
    let l = &audio.channels[0];
    let sustain = &l[4_800..43_200];
    assert!(rms(sustain) > 0.01, "silent");
    let f = pitch(sustain, 48_000.0);
    assert!((f - 440.0).abs() <= 3.0, "strongest at {f} Hz");
    // Validate is clean; without the plugin it warns and still exits 0.
    let out = run(audioc().arg("validate").arg(&file), &env);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success() && !text.contains("warning"), "{text}");
    let gone = project.to_string().replace(
        "audio-constructor.instrument\"",
        "audio-constructor.not-here\"",
    );
    let gone_file = dir.join("gone.acproj");
    std::fs::write(&gone_file, gone).unwrap();
    let out = run(audioc().arg("validate").arg(&gone_file), &env);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    assert!(text.contains("isn't installed on this computer"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_bundle_that_isnt_a_plugin_is_listed_as_failed() {
    let dir = temp("broken");
    let clap = dir.join("clap");
    std::fs::create_dir_all(&clap).unwrap();
    std::fs::write(clap.join("Broken.clap"), b"not a library").unwrap();
    let env = vec![
        ("CLAP_PATH", clap.clone()),
        ("AC_PLUGIN_CACHE", dir.join("plugins.json")),
    ];
    let out = run(audioc().args(["plugins", "--json"]), &env);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let catalog: Value = serde_json::from_slice(&out.stdout).unwrap();
    let failed = catalog["failed"].as_array().unwrap();
    assert!(
        failed.iter().any(|f| f["path"]
            .as_str()
            .is_some_and(|p| p.ends_with("Broken.clap"))),
        "{catalog}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
