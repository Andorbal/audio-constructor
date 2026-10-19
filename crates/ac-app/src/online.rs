//! Blocks from elsewhere and their previews, in the editor (DECISIONS
//! D190–D194): links imported from the File menu or the Library tab, the
//! shared libraries added by link, block exports, and the previews the
//! Library tab plays.
//!
//! Everything slow (downloads, renders, reading files) runs on a thread of
//! its own and reports back through one channel the editor polls each
//! frame, so nothing here ever holds up a frame.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use ac_core::{Block, BlockSource, Registry, SignalType};
use ac_io::index::IndexBlock;
use ac_io::remote::{Fetched, RemoteBlock, RemoteLibrary};

/// Something the Library tab can preview.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PreviewKey {
    /// A block in the library.
    Library(BlockSource),
    /// A block in a shared library: its index's link and the block's id.
    Remote { link: String, id: String },
}

/// What the preview player is doing.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Playing {
    #[default]
    Idle,
    /// Downloading, reading, or rendering it.
    Loading(PreviewKey),
    /// Playing it (the player's clip id).
    Playing(PreviewKey, u64),
}

/// A shared library added by link, and what's known of it so far.
#[derive(Clone, Debug, PartialEq)]
pub struct SharedLibrary {
    /// Its index's link, as stored in the settings.
    pub link: String,
    pub state: LibraryState,
    /// Blocks being installed, by id.
    pub installing: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LibraryState {
    /// Not read yet (read when the Library tab first shows it).
    NotLoaded,
    Loading,
    Ready(Box<RemoteLibrary>),
    Failed(String),
}

impl SharedLibrary {
    pub fn name(&self) -> &str {
        match &self.state {
            LibraryState::Ready(r) => &r.index.name,
            _ => &self.link,
        }
    }

    pub fn ready(&self) -> Option<&RemoteLibrary> {
        match &self.state {
            LibraryState::Ready(r) => Some(r),
            _ => None,
        }
    }
}

/// A finished piece of background work.
pub enum Done {
    /// "Import from Link…".
    Link {
        link: String,
        result: Result<Fetched, String>,
    },
    /// A shared library's index, read or re-read.
    Library {
        link: String,
        result: Result<RemoteLibrary, String>,
    },
    /// A block from a shared library, downloaded and checked.
    Install {
        link: String,
        id: String,
        result: Result<Box<RemoteBlock>, String>,
    },
    /// A preview, decoded and at the rate it was asked for.
    Preview {
        key: PreviewKey,
        rate: u32,
        result: Result<Vec<Vec<f32>>, String>,
    },
    /// A block written to a file (with its preview, if it has one).
    Export {
        name: String,
        result: Result<ac_io::Exported, String>,
    },
    /// The preview of a block just saved to the library, to keep beside it.
    SavedPreview {
        source: BlockSource,
        block: Box<Block>,
        wav: Option<Vec<u8>>,
    },
}

/// A decoded preview: what it is, the rate it's at, and its channels.
type Kept = (PreviewKey, u32, Arc<Vec<Vec<f32>>>);

/// Decoded previews kept for replaying at once.
const CACHED_PREVIEWS: usize = 8;

/// The editor's network and preview state.
pub struct Online {
    pub libraries: Vec<SharedLibrary>,
    pub playing: Playing,
    /// Links being imported ("Import from Link…").
    pub importing: Vec<String>,
    /// Block files being written.
    pub exporting: usize,
    /// The "Import from Link…" dialog's text, while it's open.
    pub link_dialog: Option<LinkDialog>,
    /// Where downloaded previews are kept between sessions.
    pub cache_dir: Option<PathBuf>,
    /// Where packs from links are saved.
    pub download_dir: Option<PathBuf>,
    cached: Vec<Kept>,
    tx: Sender<Done>,
    rx: Receiver<Done>,
}

impl Default for Online {
    fn default() -> Self {
        let (tx, rx) = channel();
        Online {
            libraries: Vec::new(),
            playing: Playing::Idle,
            importing: Vec::new(),
            exporting: 0,
            link_dialog: None,
            cache_dir: None,
            download_dir: None,
            cached: Vec::new(),
            tx,
            rx,
        }
    }
}

/// The text field of "Import from Link…".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LinkDialog {
    pub text: String,
    pub focus: bool,
}

/// Run `job` off the UI thread (inline where there are no threads).
fn spawn(job: impl FnOnce() + Send + 'static) {
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(job);
    #[cfg(target_arch = "wasm32")]
    job();
}

/// Whether a block has anything to preview: an audio output.
pub fn can_preview(block: &Block) -> bool {
    block.def.outputs.iter().any(|o| o.ty == SignalType::Audio)
}

/// `channels` at `rate`, converted from `from` if they differ.
fn at_rate(channels: Vec<Vec<f32>>, from: u32, rate: u32) -> Vec<Vec<f32>> {
    if from == rate {
        return channels;
    }
    match ac_dsp::resample::Resampler::new(f64::from(from), f64::from(rate)) {
        Some(r) => channels.iter().map(|c| r.process(c)).collect(),
        None => channels,
    }
}

/// A WAV file's audio at `rate`.
fn decode(wav: &[u8], rate: u32) -> Result<Vec<Vec<f32>>, String> {
    let audio = ac_io::wav::read_wav_bytes(wav).map_err(|e| format!("can't play it: {e}"))?;
    Ok(at_rate(audio.channels, audio.sample_rate, rate))
}

impl Online {
    /// Libraries from the settings' links, not read yet.
    pub fn with_libraries(links: &[String]) -> Online {
        Online {
            libraries: links
                .iter()
                .map(|l| SharedLibrary {
                    link: l.clone(),
                    state: LibraryState::NotLoaded,
                    installing: BTreeSet::new(),
                })
                .collect(),
            ..Online::default()
        }
    }

    /// Finished work, oldest first.
    pub fn poll(&mut self) -> Vec<Done> {
        self.rx.try_iter().collect()
    }

    /// Whether anything is running in the background (the editor keeps
    /// repainting to pick it up).
    pub fn busy(&self) -> bool {
        !self.importing.is_empty()
            || self.exporting > 0
            || matches!(self.playing, Playing::Loading(_))
            || self
                .libraries
                .iter()
                .any(|l| l.state == LibraryState::Loading || !l.installing.is_empty())
    }

    pub fn library(&self, link: &str) -> Option<&SharedLibrary> {
        self.libraries.iter().find(|l| l.link == link)
    }

    pub fn library_mut(&mut self, link: &str) -> Option<&mut SharedLibrary> {
        self.libraries.iter_mut().find(|l| l.link == link)
    }

    /// Download and identify `link` ("Import from Link…").
    pub fn import_link(&mut self, link: &str) {
        let link = ac_io::remote::normalize_link(link);
        if self.importing.contains(&link) {
            return;
        }
        self.importing.push(link.clone());
        let tx = self.tx.clone();
        spawn(move || {
            let result = ac_io::remote::fetch_link(&link);
            let _ = tx.send(Done::Link { link, result });
        });
    }

    /// Add a shared library (once), or put a fresh copy of its index in.
    pub fn add_library(&mut self, remote: RemoteLibrary) {
        let link = remote.url.clone();
        let state = LibraryState::Ready(Box::new(remote));
        match self.library_mut(&link) {
            Some(l) => l.state = state,
            None => self.libraries.push(SharedLibrary {
                link,
                state,
                installing: BTreeSet::new(),
            }),
        }
    }

    pub fn remove_library(&mut self, link: &str) {
        self.libraries.retain(|l| l.link != link);
        if let Playing::Loading(PreviewKey::Remote { link: l, .. })
        | Playing::Playing(PreviewKey::Remote { link: l, .. }, _) = &self.playing
            && l == link
        {
            self.playing = Playing::Idle;
        }
    }

    /// Read (or re-read) a shared library's index.
    pub fn refresh(&mut self, link: &str) {
        let Some(l) = self.library_mut(link) else {
            return;
        };
        if l.state == LibraryState::Loading {
            return;
        }
        l.state = LibraryState::Loading;
        let tx = self.tx.clone();
        let link = link.to_string();
        spawn(move || {
            let result = ac_io::remote::fetch_library(&link);
            let _ = tx.send(Done::Library { link, result });
        });
    }

    /// Download block `id` of the shared library at `link`.
    pub fn install(&mut self, link: &str, id: &str) {
        let Some(l) = self.library_mut(link) else {
            return;
        };
        let Some(remote) = l.ready().cloned() else {
            return;
        };
        let Some(entry) = remote.index.get(id).cloned() else {
            return;
        };
        if !l.installing.insert(id.to_string()) {
            return;
        }
        let tx = self.tx.clone();
        let (link, id) = (link.to_string(), id.to_string());
        spawn(move || {
            let result = remote.fetch_block(&entry).map(Box::new);
            let _ = tx.send(Done::Install { link, id, result });
        });
    }

    /// The entry a remote preview key names.
    pub fn remote_entry(&self, link: &str, id: &str) -> Option<(&RemoteLibrary, &IndexBlock)> {
        let r = self.library(link)?.ready()?;
        Some((r, r.index.get(id)?))
    }

    /// A decoded preview at `rate`, if one is kept.
    pub fn cached(&self, key: &PreviewKey, rate: u32) -> Option<Arc<Vec<Vec<f32>>>> {
        self.cached
            .iter()
            .find(|(k, r, _)| k == key && *r == rate)
            .map(|(_, _, a)| a.clone())
    }

    pub fn keep(&mut self, key: PreviewKey, rate: u32, audio: Arc<Vec<Vec<f32>>>) {
        self.cached.retain(|(k, r, _)| !(k == &key && *r == rate));
        self.cached.push((key, rate, audio));
        if self.cached.len() > CACHED_PREVIEWS {
            self.cached.remove(0);
        }
    }

    /// Forget kept previews of library block `source` (it changed).
    pub fn forget(&mut self, source: &BlockSource) {
        self.cached
            .retain(|(k, _, _)| k != &PreviewKey::Library(source.clone()));
    }

    /// Get a library block's preview at `rate`: its preview file if it has
    /// one, or a render (DECISIONS D190, so it's what an export writes).
    pub fn load_library_preview(
        &mut self,
        key: PreviewKey,
        block: Block,
        file: Option<PathBuf>,
        registry: Arc<Registry>,
        rate: u32,
    ) {
        self.playing = Playing::Loading(key.clone());
        let tx = self.tx.clone();
        spawn(move || {
            let result = match file.and_then(|f| std::fs::read(f).ok()) {
                Some(wav) => decode(&wav, rate),
                None => ac_io::render_preview(&block, registry)
                    .map(|p| at_rate(p.channels, p.sample_rate, rate)),
            };
            let _ = tx.send(Done::Preview { key, rate, result });
        });
    }

    /// Get a shared library block's preview at `rate`, from the cache
    /// folder or downloaded (and checked) into it.
    pub fn load_remote_preview(&mut self, link: &str, id: &str, rate: u32) {
        let Some((remote, entry)) = self.remote_entry(link, id) else {
            return;
        };
        let (remote, entry) = (remote.clone(), entry.clone());
        let key = PreviewKey::Remote {
            link: link.to_string(),
            id: id.to_string(),
        };
        self.playing = Playing::Loading(key.clone());
        let cache = self.cache_dir.clone();
        let tx = self.tx.clone();
        spawn(move || {
            let result = remote
                .fetch_preview(&entry, cache.as_deref())
                .and_then(|wav| decode(&wav, rate));
            let _ = tx.send(Done::Preview { key, rate, result });
        });
    }

    /// Write `block` to `out` with a preview beside it, in the background.
    pub fn export(&mut self, block: Block, out: PathBuf, registry: Arc<Registry>) {
        self.exporting += 1;
        let tx = self.tx.clone();
        let name = block.def.name.clone();
        spawn(move || {
            let result = ac_io::export_block(block, &out, Some(registry));
            let _ = tx.send(Done::Export { name, result });
        });
    }

    /// Render the preview of a block just saved to the library as
    /// `source`, to keep beside it.
    pub fn render_saved(&mut self, source: BlockSource, block: Block, registry: Arc<Registry>) {
        let tx = self.tx.clone();
        spawn(move || {
            let wav = ac_io::render_preview(&block, registry)
                .ok()
                .map(|p| p.wav());
            let _ = tx.send(Done::SavedPreview {
                source,
                block: Box::new(block),
                wav,
            });
        });
    }

    /// Where a pack from a link is saved: `name.acpack` in the download
    /// folder (`name-2.acpack`… if that's taken).
    pub fn pack_path(&self, name: &str) -> PathBuf {
        let dir = self
            .download_dir
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("audio-constructor-downloads"));
        free_path(&dir, name, ac_io::PACK_EXTENSION)
    }
}

/// `dir/name.ext`, or `dir/name-2.ext`… if that's taken.
pub fn free_path(dir: &Path, name: &str, ext: &str) -> PathBuf {
    let name = if name.is_empty() { "download" } else { name };
    let mut path = dir.join(format!("{name}.{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{name}-{n}.{ext}"));
        n += 1;
    }
    path
}

/// The platform's folder for downloads (the person's Downloads, else their
/// home), where packs from links are saved.
pub fn default_download_dir() -> Option<PathBuf> {
    let dirs = directories::UserDirs::new()?;
    Some(
        dirs.download_dir()
            .map_or_else(|| dirs.home_dir().to_path_buf(), Path::to_path_buf),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_convert_to_the_stream_rate() {
        let one_second = vec![vec![0.5f32; 48_000]];
        let same = at_rate(one_second.clone(), 48_000, 48_000);
        assert_eq!(same, one_second);
        let down = at_rate(one_second, 48_000, 44_100);
        assert_eq!(down[0].len(), 44_100);
        // The middle of a constant stays the constant.
        assert!((down[0][22_050] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn kept_previews_are_few_and_by_rate() {
        let mut o = Online::default();
        let key = |i: u32| PreviewKey::Remote {
            link: "l".into(),
            id: i.to_string(),
        };
        for i in 0..10 {
            o.keep(key(i), 48_000, Arc::new(vec![vec![i as f32]]));
        }
        assert_eq!(o.cached.len(), CACHED_PREVIEWS);
        assert!(o.cached(&key(0), 48_000).is_none(), "the oldest went");
        assert!(o.cached(&key(9), 48_000).is_some());
        assert!(o.cached(&key(9), 44_100).is_none());
    }

    #[test]
    fn packs_from_links_never_overwrite() {
        let dir = std::env::temp_dir().join(format!("ac-online-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("song.acpack"), b"x").unwrap();
        assert_eq!(free_path(&dir, "song", "acpack"), dir.join("song-2.acpack"));
        assert_eq!(free_path(&dir, "", "acpack"), dir.join("download.acpack"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
