//! Block previews and shared libraries from the command line
//! (DECISIONS D190–D192): every shipped block's preview against its golden
//! hash, `audioc block-export`, `audioc library-index`, and
//! `audioc block-import` from a server on this machine.
//!
//! Regenerate the preview hashes with
//! `GOLDEN_UPDATE=1 cargo test -p ac-cli --test previews`.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
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
    // Every shipped block makes a sound worth previewing, except one with
    // no audio output (a Clip Slot's logic) or whose sound comes in on a
    // Buffer input (a Drum Pad's), which can't make one alone (D222).
    for b in &blocks {
        let def = &b.block.def;
        let can = def
            .outputs
            .iter()
            .any(|o| o.ty == ac_core::SignalType::Audio)
            && !def
                .inputs
                .iter()
                .any(|i| i.ty == ac_core::SignalType::Buffer);
        let g = &got[&b.source.id];
        assert_eq!(g.get("none").is_none(), can, "{}: {g}", b.source.id);
    }
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

/// Serve the files in `dir` under `/lib/` on this machine; returns the base.
fn serve(dir: &Path) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let dir = dir.to_path_buf();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            let _ = reader.read_line(&mut first);
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
                line.clear();
            }
            let path = first.split(' ').nth(1).unwrap_or("/");
            let body = path
                .strip_prefix("/lib/")
                .filter(|f| !f.contains(['/', '\\']) && !f.starts_with('.'))
                .and_then(|f| std::fs::read(dir.join(f)).ok());
            let response = match body {
                Some(b) => {
                    let mut r = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        b.len()
                    )
                    .into_bytes();
                    r.extend_from_slice(&b);
                    r
                }
                None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_vec(),
            };
            let _ = stream.write_all(&response);
        }
    });
    base
}

#[test]
fn a_library_is_indexed_shared_and_imported() {
    let shared = temp("shared");
    // Someone's own takes on two shipped blocks (the shipped ones are in
    // every library already).
    let work = temp("work");
    for (id, name) in [("chorus", "Wide Chorus"), ("limiter", "Hard Limiter")] {
        let text_in = std::fs::read_to_string(root().join(format!("blocks/{id}.acblock"))).unwrap();
        let mut b = Block::from_json(&text_in).unwrap();
        b.def.name = name.into();
        let src = work.join(format!("{id}.acblock"));
        std::fs::write(&src, b.to_json()).unwrap();
        let o = audioc(&[
            "block-export",
            p(&src),
            "-o",
            p(&shared.join(format!("{id}.acblock"))),
        ]);
        assert!(o.status.success(), "{}", text(&o));
    }
    std::fs::remove_dir_all(&work).ok();
    std::fs::write(shared.join("broken.acblock"), "{\"name\": 3}").unwrap();

    // A broken block is reported and left out; the rest is indexed.
    let o = audioc(&[
        "library-index",
        p(&shared),
        "--name",
        "Shared Things",
        "--description",
        "For the test.",
    ]);
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    let t = text(&o);
    assert!(
        t.contains("broken.acblock") && t.contains("indexed 2 blocks (2 with a preview)"),
        "{t}"
    );
    let index: Value =
        serde_json::from_str(&std::fs::read_to_string(shared.join("index.json")).unwrap()).unwrap();
    assert_eq!(index["format"], "audio-constructor/library");
    assert_eq!(index["version"], 1);
    assert_eq!(index["name"], "Shared Things");
    let ids: Vec<&str> = index["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["chorus", "limiter"]);
    assert_eq!(index["blocks"][0]["preview"]["file"], "chorus.preview.wav");
    // Once it's fixed, --check says whether the index is current.
    std::fs::remove_file(shared.join("broken.acblock")).unwrap();
    let o = audioc(&["library-index", p(&shared), "--check"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("up to date"), "{}", text(&o));

    // Listed and imported from a link.
    let base = serve(&shared);
    let lib = temp("lib");
    let o = audioc(&["block-import", &format!("{base}/lib/index.json"), "--list"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(
        text(&o).contains("Shared Things (2 blocks)"),
        "{}",
        text(&o)
    );
    let o = audioc(&[
        "block-import",
        &format!("{base}/lib/"),
        "--library",
        p(&lib),
        "--only",
        "chorus",
    ]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(
        text(&o).contains("imported Wide Chorus → imported:chorus"),
        "{}",
        text(&o)
    );
    assert!(lib.join("imported/chorus.acblock").exists());
    assert_eq!(
        std::fs::read(lib.join("imported/chorus.preview.wav")).unwrap(),
        std::fs::read(shared.join("chorus.preview.wav")).unwrap()
    );
    assert!(!lib.join("imported/limiter.acblock").exists());
    // The same block again changes nothing (D83's file-import rule).
    let o = audioc(&[
        "block-import",
        &format!("{base}/lib/chorus.acblock"),
        "--library",
        p(&lib),
    ]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("already in the library"), "{}", text(&o));
    // A changed block no longer matches the index and isn't kept.
    let mut bytes = std::fs::read(shared.join("limiter.acblock")).unwrap();
    let at = bytes.windows(4).position(|w| w == b"Hard").unwrap();
    bytes[at] = b'W';
    std::fs::write(shared.join("limiter.acblock"), bytes).unwrap();
    let o = audioc(&[
        "block-import",
        &format!("{base}/lib/"),
        "--library",
        p(&lib),
    ]);
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(
        text(&o).contains("doesn't match the library's index"),
        "{}",
        text(&o)
    );
    assert!(!lib.join("imported/limiter.acblock").exists());
    // A pack's link is saved where -o says and unpacked beside itself.
    let work = temp("pack-work");
    let song = work.join("song.acproj");
    std::fs::copy(root().join("examples/start.acproj"), &song).unwrap();
    let o = audioc(&["pack", p(&song), "-o", p(&shared.join("song.acpack"))]);
    assert!(o.status.success(), "{}", text(&o));
    let out = temp("pack-out");
    let o = audioc(&[
        "block-import",
        &format!("{base}/lib/song.acpack"),
        "-o",
        p(&out),
        "--library",
        p(&lib),
    ]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(out.join("song.acpack").is_file());
    assert!(
        out.join("song").join("song.acproj").is_file(),
        "{}",
        text(&o)
    );
    std::fs::remove_dir_all(&work).ok();
    std::fs::remove_dir_all(&out).ok();
    // Plain http elsewhere is refused before anything is fetched.
    let o = audioc(&[
        "block-import",
        "http://example.org/x.acblock",
        "--library",
        p(&lib),
    ]);
    assert!(!o.status.success());
    assert!(text(&o).contains("https"), "{}", text(&o));
    std::fs::remove_dir_all(&shared).ok();
    std::fs::remove_dir_all(&lib).ok();
}
