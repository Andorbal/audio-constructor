//! Embeds the shipped blocks (`blocks/*.acblock` at the repository root) so
//! the library has them without files on disk.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let dir = manifest.join("../../blocks");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "acblock"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let mut out = String::from(
        "/// `(id, file contents)` of every shipped block.\npub static SHIPPED: &[(&str, &str)] = &[\n",
    );
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        let id = f.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        let path = f.canonicalize().unwrap_or_else(|_| f.clone());
        let _ = writeln!(
            out,
            "    ({id:?}, include_str!({:?})),",
            path.display().to_string()
        );
    }
    out.push_str("];\n");
    let dest =
        PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default()).join("shipped_blocks.rs");
    if let Err(e) = std::fs::write(&dest, out) {
        panic!("writing {}: {e}", dest.display());
    }
}
