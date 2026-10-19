//! Blocks, packs, and libraries from a link (DECISIONS D192): what
//! "Import from Link…" and `audioc block-import` download, checked before
//! anything is kept.
//!
//! - a link to an `.acblock` is a block (its preview comes along when the
//!   block records one beside it and it matches);
//! - a link to an `.acpack` is a pack, opened as packs from files are;
//! - anything else is read as a library index (`index.json`, or a folder
//!   link ending in `/`, which means its `index.json`), and failing that,
//!   sniffed: a ZIP is a pack, a block file a block.
//!
//! Blocks listed in an index are checked against the index's size and
//! SHA-256 before they're read, and so are their previews. Every download
//! goes through `crate::net`, so only `https` (or `http` on this
//! machine) and only up to a size limit. The browser build has no `net`
//! (the page fetches for itself): there these types exist and every
//! download says so.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ac_core::{BLOCK_EXTENSION, Block, slug};

use crate::hash::{matches_sha256, sha256_hex};
use crate::index::{INDEX_FILE, IndexBlock, IndexFile, LibraryIndex, resolve_url, url_unescape};
#[cfg(not(target_arch = "wasm32"))]
use crate::net::{FetchError, Limits, fetch};
use crate::pack::PACK_EXTENSION;
use crate::wav::read_wav_bytes;
#[cfg(target_arch = "wasm32")]
use offline::{FetchError, Limits, fetch};

/// Downloads in the browser build, which fetches with the page instead.
#[cfg(target_arch = "wasm32")]
mod offline {
    use std::fmt;
    use std::time::Duration;

    #[derive(Clone, Copy, Debug)]
    pub struct Limits {
        pub max_bytes: u64,
        pub timeout: Duration,
    }

    impl Limits {
        pub const DOCUMENT: Limits = Limits {
            max_bytes: 4 << 20,
            timeout: Duration::from_secs(20),
        };
        pub const PACK: Limits = Limits {
            max_bytes: 256 << 20,
            timeout: Duration::from_secs(300),
        };
    }

    #[derive(Debug)]
    pub enum FetchError {
        // Matched like the native one's; the browser never makes it.
        #[allow(dead_code)]
        TooLarge {
            limit: u64,
        },
        Unsupported,
    }

    impl fmt::Display for FetchError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                FetchError::TooLarge { limit } => write!(f, "larger than {limit} bytes"),
                FetchError::Unsupported => f.write_str("the browser build downloads links itself"),
            }
        }
    }

    pub fn fetch(_url: &str, _limits: Limits) -> Result<Vec<u8>, FetchError> {
        Err(FetchError::Unsupported)
    }
}

/// What a link turned out to hold.
#[derive(Clone, Debug, PartialEq)]
pub enum Fetched {
    /// A block, with its preview's bytes if it has one that checked out.
    Block(Box<RemoteBlock>),
    /// A pack's bytes, and a name for its file.
    Pack { bytes: Vec<u8>, name: String },
    /// A library: its index and where it is.
    Library(RemoteLibrary),
}

/// A downloaded block.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteBlock {
    pub block: Block,
    /// What to call it in the library (from its file name).
    pub id_hint: String,
    pub preview: Option<Vec<u8>>,
}

/// A library index and the link it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteLibrary {
    /// The index's own link, which its `file`s are relative to.
    pub url: String,
    pub index: LibraryIndex,
    /// Entries left out, with why.
    pub skipped: Vec<String>,
}

/// A link as typed: trimmed, and a folder link (ending in `/`) pointing at
/// the folder's `index.json`.
pub fn normalize_link(link: &str) -> String {
    let link = link.trim();
    if link.ends_with('/') && link.contains("://") {
        format!("{link}{INDEX_FILE}")
    } else {
        link.to_string()
    }
}

/// The last part of a link's path, unescaped: `…/My%20Echo.acblock?x` →
/// `My Echo.acblock`.
fn file_part(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let path = path.split_once("://").map_or(path, |(_, rest)| rest);
    let last = path.rsplit('/').next().unwrap_or("");
    // A bare host has no file part.
    if !path.contains('/') {
        return String::new();
    }
    url_unescape(last)
}

/// `name` without `.ext` (any case), if it has it.
fn strip_ext<'a>(name: &'a str, ext: &str) -> Option<&'a str> {
    let (stem, e) = name.rsplit_once('.')?;
    e.eq_ignore_ascii_case(ext).then_some(stem)
}

fn fetch_err(what: &str, e: FetchError) -> String {
    format!("couldn't get {what}: {e}")
}

/// Download and identify whatever `link` holds.
pub fn fetch_link(link: &str) -> Result<Fetched, String> {
    let url = normalize_link(link);
    let file = file_part(&url);
    if let Some(stem) = strip_ext(&file, BLOCK_EXTENSION) {
        let bytes = fetch(&url, Limits::DOCUMENT).map_err(|e| fetch_err(&file, e))?;
        return block_from(&url, &bytes, stem).map(|b| Fetched::Block(Box::new(b)));
    }
    if let Some(stem) = strip_ext(&file, PACK_EXTENSION) {
        let bytes = fetch(&url, Limits::PACK).map_err(|e| fetch_err(&file, e))?;
        return Ok(Fetched::Pack {
            bytes,
            name: slug(stem),
        });
    }
    let bytes = fetch(&url, Limits::DOCUMENT).map_err(|e| match e {
        FetchError::TooLarge { .. } => format!(
            "{} is too large for a library index or a block; a pack's link should end in .{PACK_EXTENSION}",
            if file.is_empty() { &url } else { &file }
        ),
        e => fetch_err(if file.is_empty() { "the link" } else { &file }, e),
    })?;
    if bytes.starts_with(b"PK\x03\x04") {
        let stem = file.rsplit_once('.').map_or(file.as_str(), |(s, _)| s);
        return Ok(Fetched::Pack {
            bytes,
            name: slug(if stem.is_empty() { "pack" } else { stem }),
        });
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "the link holds neither a block, a pack, nor a library index".to_string())?;
    match LibraryIndex::parse(text) {
        Ok((index, skipped)) => Ok(Fetched::Library(RemoteLibrary {
            url,
            index,
            skipped,
        })),
        // Not an index: a block file served under another name?
        Err(index_error) => match Block::from_json(text) {
            Ok(_) => {
                let stem = file.rsplit_once('.').map_or(file.as_str(), |(s, _)| s);
                block_from(&url, &bytes, if stem.is_empty() { "block" } else { stem })
                    .map(|b| Fetched::Block(Box::new(b)))
            }
            Err(_) => Err(index_error),
        },
    }
}

/// A block file's bytes, from `url`, with its preview if it records one
/// and it downloads and checks out.
fn block_from(url: &str, bytes: &[u8], id_hint: &str) -> Result<RemoteBlock, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "the block file isn't text".to_string())?;
    let mut block = Block::from_json(text).map_err(|e| format!("not a readable block: {e}"))?;
    let preview = block.preview.as_ref().and_then(|p| {
        let link = resolve_url(url, &crate::index::url_escape(&p.file)).ok()?;
        let wav = fetch(&link, Limits::PACK).ok()?;
        (matches_sha256(&wav, &p.sha256) && read_wav_bytes(&wav).is_ok()).then_some(wav)
    });
    if preview.is_none() {
        block.preview = None;
    }
    Ok(RemoteBlock {
        block,
        id_hint: id_hint.to_string(),
        preview,
    })
}

/// Download the library index at `link`.
pub fn fetch_library(link: &str) -> Result<RemoteLibrary, String> {
    let url = normalize_link(link);
    let bytes = fetch(&url, Limits::DOCUMENT).map_err(|e| fetch_err("the library index", e))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "not a library index".to_string())?;
    let (index, skipped) = LibraryIndex::parse(text)?;
    Ok(RemoteLibrary {
        url,
        index,
        skipped,
    })
}

/// Limits for a file whose size the index gives: exactly that much, with
/// time to match.
fn limits_for(size: u64, cap: Limits) -> Result<Limits, String> {
    if size > cap.max_bytes {
        return Err(format!(
            "it's {} MB, more than the {} MB this accepts",
            size.div_ceil(1 << 20),
            cap.max_bytes.div_ceil(1 << 20)
        ));
    }
    Ok(Limits {
        max_bytes: size,
        timeout: cap.timeout.max(Duration::from_secs(20)),
    })
}

/// Download `file` from beside the index at `base`, refusing it unless it
/// is the size and hash the index says.
fn fetch_checked(base: &str, file: &IndexFile, cap: Limits, what: &str) -> Result<Vec<u8>, String> {
    let url = resolve_url(base, &file.file)?;
    let limits = limits_for(file.size, cap).map_err(|e| format!("{what}: {e}"))?;
    let bytes = fetch(&url, limits).map_err(|e| match e {
        FetchError::TooLarge { .. } => {
            format!("{what} is larger than the library's index says; it wasn't kept")
        }
        e => fetch_err(what, e),
    })?;
    if bytes.len() as u64 != file.size || !matches_sha256(&bytes, &file.sha256) {
        return Err(format!(
            "{what} doesn't match the library's index (it changed after the index was made, \
             or something altered it on the way); it wasn't kept"
        ));
    }
    Ok(bytes)
}

impl RemoteLibrary {
    /// Download block `entry`, checked against the index, and its preview
    /// (also checked; a preview that fails is left out, not the block).
    pub fn fetch_block(&self, entry: &IndexBlock) -> Result<RemoteBlock, String> {
        let file = IndexFile {
            file: entry.file.clone(),
            sha256: entry.sha256.clone(),
            size: entry.size,
        };
        let what = format!("{} ({})", entry.name, entry.id);
        let bytes = fetch_checked(&self.url, &file, Limits::DOCUMENT, &what)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| format!("{what} isn't text"))?;
        let mut block =
            Block::from_json(text).map_err(|e| format!("{what} isn't a readable block: {e}"))?;
        let preview = entry
            .preview
            .as_ref()
            .and_then(|p| fetch_checked(&self.url, p, Limits::PACK, "the preview").ok());
        match (&preview, block.preview.as_mut()) {
            (Some(wav), Some(rec)) => rec.sha256 = sha256_hex(wav),
            (Some(wav), None) => {
                block.preview = Some(ac_core::BlockPreview {
                    file: ac_core::preview_file_name(&slug(&entry.id)),
                    sha256: sha256_hex(wav),
                });
            }
            (None, _) => block.preview = None,
        }
        Ok(RemoteBlock {
            block,
            id_hint: entry.id.clone(),
            preview,
        })
    }

    /// Entry `entry`'s preview, from `cache` if a copy is there (kept by its
    /// hash, so a changed preview is a new file) and downloaded, checked,
    /// and stored there otherwise.
    pub fn fetch_preview(
        &self,
        entry: &IndexBlock,
        cache: Option<&Path>,
    ) -> Result<Vec<u8>, String> {
        let p = entry
            .preview
            .as_ref()
            .ok_or_else(|| format!("{} has no preview", entry.name))?;
        let cached = cache.map(|dir| cache_path(dir, &p.sha256));
        if let Some(path) = &cached
            && let Ok(bytes) = std::fs::read(path)
            && matches_sha256(&bytes, &p.sha256)
        {
            return Ok(bytes);
        }
        let bytes = fetch_checked(&self.url, p, Limits::PACK, "the preview")?;
        if read_wav_bytes(&bytes).is_err() {
            return Err("the preview isn't a WAV file this can play".into());
        }
        if let Some(path) = cached {
            // A cache is a convenience: failing to keep a copy changes nothing.
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, &bytes);
        }
        Ok(bytes)
    }
}

/// Where the preview with hash `sha256` is cached in `dir`.
pub fn cache_path(dir: &Path, sha256: &str) -> PathBuf {
    let name: String = sha256
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(64)
        .collect::<String>()
        .to_ascii_lowercase();
    dir.join(format!("{name}.wav"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{build_index, write_index};
    use crate::library::shipped_blocks;
    use crate::preview::export_block;
    use std::collections::BTreeMap;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// A tiny static file server on this machine: serves `files` by path
    /// (`/index.json`) for as long as the test runs, and counts requests.
    struct Server {
        base: String,
        files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        hits: Arc<Mutex<Vec<String>>>,
    }

    fn serve() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let files: Arc<Mutex<BTreeMap<String, Vec<u8>>>> = Arc::default();
        let hits: Arc<Mutex<Vec<String>>> = Arc::default();
        let (f, h) = (files.clone(), hits.clone());
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                if reader.read_line(&mut first).is_err() {
                    continue;
                }
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
                    line.clear();
                }
                let path = first.split(' ').nth(1).unwrap_or("/").to_string();
                h.lock().unwrap().push(path.clone());
                let body = f.lock().unwrap().get(&path).cloned();
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
                    None => {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    }
                };
                let _ = stream.write_all(&response);
            }
        });
        Server { base, files, hits }
    }

    impl Server {
        fn put(&self, path: &str, bytes: Vec<u8>) {
            self.files.lock().unwrap().insert(path.to_string(), bytes);
        }

        /// Publish every file in `dir` under `/lib/`.
        fn publish(&self, dir: &Path) {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                let name = p.file_name().unwrap().to_string_lossy().to_string();
                self.put(
                    &format!("/lib/{}", crate::index::url_escape(&name)),
                    std::fs::read(&p).unwrap(),
                );
            }
        }
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ac-remote-{tag}-{}-{}",
            std::process::id(),
            crate::clock::now_ns()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn shipped(id: &str) -> Block {
        shipped_blocks()
            .into_iter()
            .find(|b| b.source.id == id)
            .unwrap()
            .block
    }

    /// A published library of two blocks, one with a preview.
    fn library(server: &Server) -> PathBuf {
        let dir = temp("lib");
        let registry = Arc::new(ac_nodes::registry());
        export_block(
            shipped("tremolo"),
            &dir.join("tremolo.acblock"),
            Some(registry.clone()),
        )
        .unwrap();
        export_block(shipped("limiter"), &dir.join("limiter.acblock"), None).unwrap();
        let b = build_index(&dir, &registry, Some("Shared"), Some("Two blocks")).unwrap();
        write_index(&dir, &b.index).unwrap();
        server.publish(&dir);
        dir
    }

    #[test]
    fn links_name_their_files() {
        assert_eq!(
            file_part("https://x.org/a/My%20Echo.acblock?dl=1"),
            "My Echo.acblock"
        );
        assert_eq!(file_part("https://x.org"), "");
        assert_eq!(file_part("https://x.org/"), "");
        assert_eq!(
            normalize_link(" https://x.org/lib/ "),
            "https://x.org/lib/index.json"
        );
        assert_eq!(strip_ext("a.ACBLOCK", "acblock"), Some("a"));
        let d = Path::new("/c");
        assert_eq!(
            cache_path(d, &"Ab".repeat(32)),
            d.join(format!("{}.wav", "ab".repeat(32)))
        );
        assert_eq!(cache_path(d, "../../x"), d.join(".wav"));
    }

    #[test]
    fn a_block_link_brings_its_preview() {
        let server = serve();
        let dir = library(&server);
        let Fetched::Block(b) =
            fetch_link(&format!("{}/lib/tremolo.acblock", server.base)).unwrap()
        else {
            panic!("not a block")
        };
        assert_eq!(b.id_hint, "tremolo");
        assert!(b.block.same_definition(&shipped("tremolo")));
        let wav = std::fs::read(dir.join("tremolo.preview.wav")).unwrap();
        assert_eq!(b.preview.as_deref(), Some(wav.as_slice()));
        // Without its preview on the server, the block still comes.
        server
            .files
            .lock()
            .unwrap()
            .remove("/lib/tremolo.preview.wav");
        let Fetched::Block(b) =
            fetch_link(&format!("{}/lib/tremolo.acblock", server.base)).unwrap()
        else {
            panic!("not a block")
        };
        assert!(b.preview.is_none() && b.block.preview.is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn an_index_link_lists_and_installs_checked_blocks() {
        let server = serve();
        let dir = library(&server);
        for link in [
            format!("{}/lib/index.json", server.base),
            format!("{}/lib/", server.base),
        ] {
            let Fetched::Library(lib) = fetch_link(&link).unwrap() else {
                panic!("not a library")
            };
            assert_eq!(lib.index.name, "Shared");
            assert_eq!(lib.index.blocks.len(), 2);
        }
        let lib = fetch_library(&format!("{}/lib/", server.base)).unwrap();
        let trem = lib.index.get("tremolo").unwrap().clone();
        let got = lib.fetch_block(&trem).unwrap();
        assert!(got.preview.is_some());
        assert_eq!(
            got.block.preview.as_ref().unwrap().sha256,
            trem.preview.as_ref().unwrap().sha256
        );
        // Previews are cached by hash: the second play doesn't download.
        let cache = temp("cache");
        let a = lib.fetch_preview(&trem, Some(&cache)).unwrap();
        let before = server.hits.lock().unwrap().len();
        let b = lib.fetch_preview(&trem, Some(&cache)).unwrap();
        assert_eq!(a, b);
        assert_eq!(server.hits.lock().unwrap().len(), before);
        // A block changed after the index was made is refused.
        let mut altered = std::fs::read(dir.join("limiter.acblock")).unwrap();
        let at = altered.iter().position(|&c| c == b'L').unwrap();
        altered[at] = b'M';
        server.put("/lib/limiter.acblock", altered);
        let e = lib
            .fetch_block(lib.index.get("limiter").unwrap())
            .unwrap_err();
        assert!(e.contains("doesn't match"), "{e}");
        // So is one larger than it says.
        let mut longer = std::fs::read(dir.join("limiter.acblock")).unwrap();
        longer.extend_from_slice(b"  ");
        server.put("/lib/limiter.acblock", longer);
        let e = lib
            .fetch_block(lib.index.get("limiter").unwrap())
            .unwrap_err();
        assert!(e.contains("larger") || e.contains("doesn't match"), "{e}");
        std::fs::remove_dir_all(dir).ok();
        std::fs::remove_dir_all(cache).ok();
    }

    #[test]
    fn packs_and_bad_links_are_told_apart() {
        let server = serve();
        server.put("/song.acpack", b"PK\x03\x04rest".to_vec());
        server.put("/download?id=7", b"PK\x03\x04rest".to_vec());
        server.put("/junk", b"\xff\xfe".to_vec());
        server.put("/other.json", b"{\"hello\": 1}".to_vec());
        assert!(matches!(
            fetch_link(&format!("{}/song.acpack", server.base)).unwrap(),
            Fetched::Pack { ref name, .. } if name == "song"
        ));
        assert!(matches!(
            fetch_link(&format!("{}/download?id=7", server.base)).unwrap(),
            Fetched::Pack { .. }
        ));
        assert!(fetch_link(&format!("{}/junk", server.base)).is_err());
        let e = fetch_link(&format!("{}/other.json", server.base)).unwrap_err();
        assert!(e.contains("not a library index"), "{e}");
        let e = fetch_link(&format!("{}/missing.acblock", server.base)).unwrap_err();
        assert!(e.contains("404"), "{e}");
        let e = fetch_link("http://example.org/a.acblock").unwrap_err();
        assert!(e.contains("https"), "{e}");
        assert!(fetch_link("not a link").is_err());
    }
}
