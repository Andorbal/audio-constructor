//! Block previews from the command line (DECISIONS D190): every shipped
//! block's preview against its golden hash, and `audioc block-export`.
//!
//! Regenerate the preview hashes with
//! `GOLDEN_UPDATE=1 cargo test -p ac-cli --test previews`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use ac_core::Block;
use ac_io::hash::sha256_hex;
use ac_io::{render_preview, shipped_blocks};
use serde_json::{Value, json};

fn audioc(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_audioc"))
        .args(args)
        .output()
        .expect("audioc runs")
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn temp(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let d = std::env::temp_dir().join(format!("ac-cli-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn p(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn shipped_block_previews_match_their_golden_hashes() {
    let update = std::env::var_os("GOLDEN_UPDATE").is_some();
    let reference = cfg!(all(target_os = "linux", target_arch = "x86_64"));
    let registry = Arc::new(ac_nodes::registry());
    let blocks = shipped_blocks();
    // Rendered on a few threads: each is independent.
    let mut got: BTreeMap<String, Value> = BTreeMap::new();
    std::thread::scope(|s| {
        let handles: Vec<_> = blocks
            .chunks(blocks.len().div_ceil(4).max(1))
            .map(|chunk| {
                let registry = registry.clone();
                s.spawn(move || {
                    chunk
                        .iter()
                        .map(|b| {
                            let summary = match render_preview(&b.block, registry.clone()) {
                                Ok(p) => {
                                    let wav = p.wav();
                                    let levels: Vec<Value> = p
                                        .channels
                                        .iter()
                                        .map(|c| {
                                            let peak = c.iter().fold(0.0f32, |m, s| m.max(s.abs()));
                                            let rms = (c.iter().map(|s| s * s).sum::<f32>()
                                                / c.len().max(1) as f32)
                                                .sqrt();
                                            json!({
                                                "peak": (f64::from(peak) * 1e6).round() / 1e6,
                                                "rms": (f64::from(rms) * 1e6).round() / 1e6,
                                            })
                                        })
                                        .collect();
                                    json!({
                                        "sha256": sha256_hex(&wav),
                                        "frames": p.channels[0].len(),
                                        "channels": levels,
                                        "feeds": p.feeds,
                                    })
                                }
                                Err(e) => json!({ "none": e }),
                            };
                            (b.source.id.clone(), summary)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for h in handles {
            got.extend(h.join().unwrap());
        }
    });
    let path = root().join("tests/golden/block-previews.json");
    if update {
        let text = serde_json::to_string_pretty(&got).unwrap() + "\n";
        std::fs::write(&path, text).unwrap();
        return;
    }
    let want: BTreeMap<String, Value> =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("block-previews.json"))
            .unwrap();
    let mut failures = Vec::new();
    for (id, g) in &got {
        let Some(w) = want.get(id) else {
            failures.push(format!("{id}: no expectation (GOLDEN_UPDATE=1 adds it)"));
            continue;
        };
        if g["frames"] != w["frames"] || g["feeds"] != w["feeds"] || g["none"] != w["none"] {
            failures.push(format!("{id}: got {g}\n  want {w}"));
        } else if reference {
            if g["sha256"] != w["sha256"] {
                failures.push(format!("{id}: sha256 {} != {}", g["sha256"], w["sha256"]));
            }
        } else {
            let close = |a: &Value, b: &Value| {
                let (a, b) = (a.as_f64().unwrap_or(0.0), b.as_f64().unwrap_or(0.0));
                (a - b).abs() <= 1e-4 + 1e-3 * b.abs()
            };
            let (gc, wc) = (g["channels"].as_array(), w["channels"].as_array());
            let ok = gc.zip(wc).is_some_and(|(gc, wc)| {
                gc.len() == wc.len()
                    && gc
                        .iter()
                        .zip(wc)
                        .all(|(a, b)| close(&a["peak"], &b["peak"]) && close(&a["rms"], &b["rms"]))
            });
            if !ok {
                failures.push(format!("{id}: levels differ beyond tolerance"));
            }
        }
    }
    for id in want.keys().filter(|k| !got.contains_key(*k)) {
        failures.push(format!("{id}: expected, but no such shipped block"));
    }
    assert!(
        failures.is_empty(),
        "preview mismatches:\n{}",
        failures.join("\n")
    );
    // Every shipped block makes a sound worth previewing.
    assert!(got.values().all(|g| g.get("none").is_none()), "{got:?}");
}

#[test]
fn block_export_writes_the_block_and_the_preview_the_library_renders() {
    let dir = temp("export");
    // From a block file: the same preview the library would render.
    let out = dir.join("trem.acblock");
    let o = audioc(&[
        "block-export",
        p(&root().join("blocks/tremolo.acblock")),
        "-o",
        p(&out),
    ]);
    assert!(o.status.success(), "{}", text(&o));
    let t = text(&o);
    assert!(
        t.contains("preview →") && t.contains("in ← the preview phrase"),
        "{t}"
    );
    let wav = std::fs::read(dir.join("trem.preview.wav")).unwrap();
    let block = Block::from_json(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(block.preview.as_ref().unwrap().sha256, sha256_hex(&wav));
    let lib = shipped_blocks()
        .into_iter()
        .find(|b| b.source.id == "tremolo")
        .unwrap();
    let direct = render_preview(&lib.block, Arc::new(ac_nodes::registry())).unwrap();
    assert_eq!(direct.wav(), wav, "bit for bit");

    // From a project, by the group's name; without a preview.
    let project = root().join("examples/vocoder.acproj");
    let out2 = dir.join("voc.acblock");
    let o = audioc(&[
        "block-export",
        p(&project),
        "vocoder",
        "-o",
        p(&out2),
        "--no-preview",
    ]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(out2.exists() && !dir.join("voc.preview.wav").exists());
    // A project with several groups needs to be told which.
    let o = audioc(&["block-export", p(&project), "-o", p(&out2)]);
    assert!(!o.status.success());
    assert!(text(&o).contains("say which group"), "{}", text(&o));
    let o = audioc(&["block-export", p(&project), "nothing-like-it"]);
    assert!(
        text(&o).contains("no group `nothing-like-it`"),
        "{}",
        text(&o)
    );
    std::fs::remove_dir_all(&dir).ok();
}
