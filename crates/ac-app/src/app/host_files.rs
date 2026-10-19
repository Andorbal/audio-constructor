//! Files a host keeps for the editor (DECISIONS D153): the web build has
//! no file system the editor can reach, so with
//! [`AppOptions::host_files`](super::AppOptions::host_files) the editor
//! asks its host instead. Opening, saving, exporting, choosing a clip's
//! file, and opening an example become [`FileRequest`]s the host takes
//! with [`EditorApp::take_file_requests`] and answers later, when the
//! person has picked a file or a download has finished.
//!
//! Audio files live in memory then, by the path the project's nodes use:
//! those a pack or an example brought, and those the person picked. They
//! play, pack, and count as present like files on disk; a project opened
//! on its own shows its audio files as missing until they're located.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ac_core::{NodeId, Project};
use ac_io::AssetStore;

use super::{AfterConfirm, EditorApp, plural};
use crate::dialogs::FilePurpose;
use crate::files::ImportedAsset;
use crate::help;

/// What a [`FileRequest::Save`] holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveKind {
    /// The project (`.acproj`).
    Project,
    /// An exported render (`.wav`).
    Wav,
    /// The project with its audio files (`.acpack`).
    Pack,
}

impl SaveKind {
    /// The file extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            SaveKind::Project => "acproj",
            SaveKind::Wav => "wav",
            SaveKind::Pack => ac_io::PACK_EXTENSION,
        }
    }

    /// The file's media type.
    pub fn mime(self) -> &'static str {
        match self {
            SaveKind::Project => "application/json",
            SaveKind::Wav => "audio/wav",
            SaveKind::Pack => "application/zip",
        }
    }
}

/// What an audio file the host is asked for is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioPurpose {
    /// The file a Clip node plays.
    Clip(NodeId),
    /// A file to use where the project's nodes refer to this missing one.
    Locate(String),
}

/// Something the editor needs its host to do with files.
#[derive(Clone, Debug, PartialEq)]
pub enum FileRequest {
    /// Let the person pick a project, pack, or block file; answer with
    /// [`EditorApp::open_file`].
    Open,
    /// Let the person pick a block file for the library; answer with
    /// [`EditorApp::open_file`].
    ImportBlock,
    /// Let the person pick a WAV file; answer with
    /// [`EditorApp::audio_file`].
    Audio(AudioPurpose),
    /// Fetch the example at `path` and the audio files it plays; answer
    /// with [`EditorApp::open_example_files`].
    Example(PathBuf),
    /// Save `bytes` as a file called `name`. With `same_place` the host
    /// may write where this document was saved last (File ▸ Save) instead
    /// of asking. Answer with [`EditorApp::file_saved`] (passing
    /// `revision` back) once it's written.
    Save {
        name: String,
        bytes: Vec<u8>,
        kind: SaveKind,
        same_place: bool,
        /// The document revision the bytes are (projects).
        revision: u64,
    },
}

impl EditorApp {
    /// What the editor asked of its host since the last call
    /// ([`AppOptions::host_files`](super::AppOptions::host_files)).
    pub fn take_file_requests(&mut self) -> Vec<FileRequest> {
        std::mem::take(&mut self.file_requests)
    }

    /// Whether the host keeps the editor's files.
    pub fn host_files(&self) -> bool {
        self.host_files
    }

    /// File dialogs, when the host keeps the files.
    pub(super) fn host_file_dialog(&mut self, purpose: FilePurpose, now: f64) {
        match purpose {
            FilePurpose::Open => self.file_requests.push(FileRequest::Open),
            FilePurpose::ImportBlock => self.file_requests.push(FileRequest::ImportBlock),
            FilePurpose::ChooseClip(node) => self
                .file_requests
                .push(FileRequest::Audio(AudioPurpose::Clip(node))),
            FilePurpose::Locate(old) => self
                .file_requests
                .push(FileRequest::Audio(AudioPurpose::Locate(old))),
            FilePurpose::SaveAs => self.host_save(true, now),
            FilePurpose::ExportPack => self.host_export_pack(now),
            FilePurpose::Export => self.host_export_wav(now),
        }
    }

    /// The name a file of this document gets: its file's, or its title's.
    fn file_name_for(&self, kind: SaveKind) -> String {
        let stem = self
            .doc
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| super::slug(&self.doc.title()));
        format!("{stem}.{}", kind.extension())
    }

    /// The project as saved: with the canvas's view, so it reopens there.
    fn project_to_save(&self) -> Project {
        let mut project = self.doc.project.clone();
        project.view = Some(ac_core::View {
            center: [self.canvas.view.center.x, self.canvas.view.center.y],
            zoom: self.canvas.view.zoom,
        });
        project
    }

    /// File ▸ Save and Save As, when the host keeps the files.
    pub(super) fn host_save(&mut self, save_as: bool, now: f64) {
        let project = self.project_to_save();
        let name = self.file_name_for(SaveKind::Project);
        self.file_requests.push(FileRequest::Save {
            name,
            bytes: project.to_json().into_bytes(),
            kind: SaveKind::Project,
            same_place: !save_as && self.doc.path.is_some(),
            revision: self.doc.revision,
        });
        let audio = ac_io::assets::referenced_assets(&project, &self.registry).len();
        if audio > 0 {
            self.say(
                format!(
                    "Saving… The project's {audio} audio file{} stay{} in this browser tab: \
                     Export Pack saves them with the project",
                    plural(audio),
                    if audio == 1 { "s" } else { "" }
                ),
                now,
            );
        }
    }

    /// File ▸ Export Pack, when the host keeps the files: the project and
    /// the audio files held in memory.
    pub(super) fn host_export_pack(&mut self, now: f64) {
        let project = self.project_to_save();
        let stem = self
            .file_name_for(SaveKind::Pack)
            .trim_end_matches(".acpack")
            .to_string();
        let bank = &self.asset_bank;
        match ac_io::pack_project(project, &stem, &self.registry, |path| {
            bank.get(path).map(|b| b.as_ref().clone())
        }) {
            Ok((bytes, report)) => {
                if !report.missing.is_empty() {
                    let n = report.missing.len();
                    self.warn(
                        format!(
                            "Left out {n} missing audio file{}: {}",
                            plural(n),
                            report.missing.join(", ")
                        ),
                        now,
                    );
                }
                self.file_requests.push(FileRequest::Save {
                    name: format!("{stem}.acpack"),
                    bytes,
                    kind: SaveKind::Pack,
                    same_place: false,
                    revision: self.doc.revision,
                });
            }
            Err(e) => self.warn(format!("Couldn't make the pack: {e}"), now),
        }
    }

    /// File ▸ Export WAV, when the host keeps the files: rendered here and
    /// now (the editor waits), then handed over.
    pub(super) fn host_export_wav(&mut self, now: f64) {
        let resources: Arc<dyn ac_core::Resources> = Arc::new(ac_io::FileResources {
            assets: self.assets.clone(),
            recorder: None,
        });
        match crate::export::export_wav_bytes(&self.doc.project, self.registry.clone(), resources) {
            Ok((bytes, s)) => {
                let secs = s.frames as f64 / f64::from(s.sample_rate.max(1));
                self.say(
                    format!(
                        "Rendered {secs:.2} s × {} ch at {} Hz; saving…",
                        s.channels, s.sample_rate
                    ),
                    now,
                );
                self.file_requests.push(FileRequest::Save {
                    name: self.file_name_for(SaveKind::Wav),
                    bytes,
                    kind: SaveKind::Wav,
                    same_place: false,
                    revision: self.doc.revision,
                });
            }
            Err(e) => self.warn(format!("Export failed: {e}"), now),
        }
    }

    /// The host wrote a file the editor asked to save. A project is saved
    /// as `name` then (as of `revision`: edits made since stay unsaved).
    pub fn file_saved(&mut self, kind: SaveKind, name: &str, revision: u64) {
        let now = self.clock.last;
        if kind == SaveKind::Project {
            self.doc.path = Some(PathBuf::from(name));
            self.doc.mark_saved_at(revision);
            self.epoch += 1;
        }
        self.say(format!("Saved {name}"), now);
    }

    /// The host couldn't save (or the person cancelled: `error` is `None`).
    pub fn file_not_saved(&mut self, error: Option<&str>) {
        let now = self.clock.last;
        self.pack_after_save = false;
        if let Some(e) = error {
            self.warn(format!("Couldn't save: {e}"), now);
        }
    }

    /// Audio files held in memory, as the project's nodes refer to them.
    pub(super) fn bank_assets_for(&self, project: &Project) -> AssetStore {
        AssetStore::load_with(project, &self.registry, |path| {
            self.asset_bank
                .get(path)
                .map(|b| b.as_ref().clone())
                .ok_or_else(|| "not opened in this browser tab: Locate it".to_string())
        })
    }

    /// Reload the project's audio files from memory.
    pub(super) fn reload_bank_assets(&mut self) -> AssetStore {
        self.bank_assets_for(&self.doc.project.clone())
    }

    /// Open a file the person picked: a project, a pack (with its audio
    /// files), or a block file (into the library). `name` is the file's
    /// name, which says which it is.
    pub fn open_file(&mut self, name: &str, bytes: &[u8]) {
        let now = self.clock.last;
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".acblock") {
            let text = String::from_utf8_lossy(bytes);
            let hint = name.trim_end_matches(".acblock").to_string();
            self.import_block_text(&text, &hint, name, now);
            return;
        }
        if lower.ends_with(&format!(".{}", ac_io::PACK_EXTENSION)) {
            match ac_io::unpack_in_memory(bytes) {
                Ok(mut u) => {
                    let adopted = self.library.adopt(&mut u.project);
                    self.asset_bank = u.files.into_iter().map(|(p, b)| (p, Arc::new(b))).collect();
                    let assets = self.bank_assets_for(&u.project);
                    let missing = assets.issues.len();
                    self.replace_document(u.project, None, PathBuf::from("."), assets);
                    let mut text = format!("Opened {name} as a new project");
                    if adopted > 0 {
                        text.push_str(&format!(
                            " · added {adopted} block{} from it to your library",
                            plural(adopted)
                        ));
                    }
                    if missing > 0 {
                        text.push_str(&format!(
                            " — {missing} audio file problem(s); see the Project tab"
                        ));
                        self.warn(text, now);
                    } else {
                        self.say(text, now);
                    }
                }
                Err(e) => self.warn(format!("Couldn't open {name}: {e}"), now),
            }
            return;
        }
        let text = String::from_utf8_lossy(bytes);
        match Project::from_json(&text) {
            Ok(project) => {
                self.asset_bank.clear();
                let assets = self.bank_assets_for(&project);
                let missing = assets.issues.len();
                self.replace_document(
                    project,
                    Some(PathBuf::from(name)),
                    PathBuf::from("."),
                    assets,
                );
                let blocks = self.library.import_unknown(&self.doc.project);
                let mut text = format!("Opened {name}");
                if blocks > 0 {
                    text.push_str(&format!(
                        " · added {blocks} block{} it uses to your library",
                        plural(blocks)
                    ));
                }
                if missing > 0 {
                    text.push_str(&format!(
                        " — {missing} audio file{} to locate in the Project tab \
                         (or open the project's pack, which carries them)",
                        plural(missing)
                    ));
                    self.warn(text, now);
                } else {
                    self.say(text, now);
                }
            }
            Err(e) => self.warn(format!("Couldn't open {name}: {e}"), now),
        }
    }

    /// An audio file the person picked for `purpose`: kept in memory
    /// under `assets/` and used as a file on disk would be.
    pub fn audio_file(&mut self, purpose: AudioPurpose, name: &str, bytes: Vec<u8>) {
        let now = self.clock.last;
        let data = match ac_io::wav::read_wav_bytes(&bytes) {
            Ok(d) => d,
            Err(e) => return self.warn(format!("Couldn't use {name}: {e}"), now),
        };
        let hash = ac_core::util::content_hash(&bytes);
        let file_name = Path::new(name).file_name().map_or_else(
            || "audio.wav".to_string(),
            |f| f.to_string_lossy().to_string(),
        );
        // The same name with other contents gets a new name.
        let mut key = format!("assets/{file_name}");
        let mut n = 2;
        while self
            .asset_bank
            .get(&key)
            .is_some_and(|b| ac_core::util::content_hash(b) != hash)
        {
            let (base, ext) = file_name.rsplit_once('.').unwrap_or((&file_name, "wav"));
            key = format!("assets/{base}-{n}.{ext}");
            n += 1;
        }
        self.asset_bank.insert(key.clone(), Arc::new(bytes));
        let asset = ImportedAsset {
            file: key,
            channels: data.channels.len().max(1) as u64,
            hash,
            note: None,
        };
        match purpose {
            AudioPurpose::Clip(node) => self.assign_clip_asset(node, asset, now),
            AudioPurpose::Locate(old) => self.relink(&old, &asset, now),
        }
    }

    /// Fetched example `path` (from a [`FileRequest::Example`]): open
    /// `text` as a new, untitled project, with `assets` (the audio files
    /// it plays, by the paths its nodes use).
    pub fn open_example_files(&mut self, path: &Path, text: &str, assets: Vec<(String, Vec<u8>)>) {
        let now = self.clock.last;
        match Project::from_json(text) {
            Ok(mut project) => {
                project.view = None;
                self.asset_bank = assets.into_iter().map(|(p, b)| (p, Arc::new(b))).collect();
                let store = self.bank_assets_for(&project);
                let name = help::example_of(path.to_path_buf(), &project).name;
                self.replace_document(project, None, PathBuf::from("."), store);
                self.say(
                    format!(
                        "Opened the example “{name}” as a new project: Save As keeps your changes"
                    ),
                    now,
                );
            }
            Err(e) => self.warn(format!("Couldn't open the example: {e}"), now),
        }
    }

    /// The host's shipped examples (File ▸ Examples and the help's "Used
    /// in"), by where the host finds each again.
    pub fn set_examples(&mut self, examples: Vec<(PathBuf, Project)>) {
        let mut list: Vec<help::Example> = examples
            .iter()
            .map(|(path, p)| help::example_of(path.clone(), p))
            .collect();
        list.sort_by_key(|e| e.name.to_lowercase());
        self.side.help.set_examples(list);
    }

    /// Ask for the example at `path` (after the unsaved-changes question).
    pub fn request_example(&mut self, path: PathBuf) {
        let now = self.clock.last;
        self.request(AfterConfirm::OpenExample(path), now);
    }
}
