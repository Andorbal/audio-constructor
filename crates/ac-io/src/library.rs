//! The block library (ARCHITECTURE §12): blocks that ship with the app
//! (compiled in from `blocks/`), the person's own blocks, and blocks
//! imported from packs or files, each kept as one `.acblock` file.
//!
//! On disk, a library directory holds the person's blocks directly and
//! imported ones under `imported/`. A block's id is its file name; its
//! source (`user:my-bass`) is what project copies remember, so the editor
//! can offer an update when the library's version is newer.

use std::path::{Path, PathBuf};

use ac_core::{
    BLOCK_EXTENSION, Block, BlockPreview, BlockSource, GroupId, Origin, Project, preview_file_name,
    slug,
};

use crate::hash::{matches_sha256, sha256_hex};

mod shipped {
    include!(concat!(env!("OUT_DIR"), "/shipped_blocks.rs"));
}

/// One block in the library.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryBlock {
    pub source: BlockSource,
    pub block: Block,
    /// The file, for user and imported blocks.
    pub path: Option<PathBuf>,
}

impl LibraryBlock {
    pub fn name(&self) -> &str {
        &self.block.def.name
    }

    /// The block's preview file, if it records one and it's there.
    pub fn preview_path(&self) -> Option<PathBuf> {
        let p = self.block.preview.as_ref()?;
        let path = self.path.as_ref()?.parent()?.join(&p.file);
        path.is_file().then_some(path)
    }

    /// Whether `query` (words, any order, case-insensitive) matches the
    /// block's name, tags, or description.
    pub fn matches(&self, query: &str) -> bool {
        let d = &self.block.def;
        let hay = format!("{} {} {}", d.name, d.tags.join(" "), d.description).to_lowercase();
        query
            .split_whitespace()
            .all(|w| hay.contains(&w.to_lowercase()))
    }
}

/// A project definition the library has a newer version of.
#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    pub def: GroupId,
    pub source: BlockSource,
    pub from: u32,
    pub to: u32,
}

/// Shipped, user, and imported blocks.
#[derive(Clone, Debug, Default)]
pub struct Library {
    /// Where user and imported blocks are saved; `None` keeps them in memory.
    pub dir: Option<PathBuf>,
    blocks: Vec<LibraryBlock>,
    /// Files that couldn't be read, with the reason.
    pub issues: Vec<String>,
}

/// The person's block library: `blocks/` in the platform's data folder
/// (`None` where the platform has none). The editor and `audioc` share it.
#[cfg(not(target_arch = "wasm32"))]
pub fn default_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "Audio Constructor").map(|d| d.data_dir().join("blocks"))
}

/// Where downloaded previews are kept between sessions: `previews/` in the
/// platform's cache folder.
#[cfg(not(target_arch = "wasm32"))]
pub fn preview_cache_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "Audio Constructor")
        .map(|d| d.cache_dir().join("previews"))
}

/// The preview beside block file `path`, if its record names one there and
/// the file is what the record says (SHA-256).
pub fn sidecar_preview(path: &Path, block: &Block) -> Option<Vec<u8>> {
    let p = block.preview.as_ref()?;
    let bytes = std::fs::read(path.parent()?.join(&p.file)).ok()?;
    matches_sha256(&bytes, &p.sha256).then_some(bytes)
}

/// The blocks that ship with the app.
pub fn shipped_blocks() -> Vec<LibraryBlock> {
    shipped::SHIPPED
        .iter()
        .filter_map(|(id, text)| {
            Block::from_json(text).ok().map(|block| LibraryBlock {
                source: BlockSource::new(Origin::Shipped, *id),
                block,
                path: None,
            })
        })
        .collect()
}

impl Library {
    /// Just the shipped blocks.
    pub fn shipped_only() -> Library {
        Library {
            dir: None,
            blocks: shipped_blocks(),
            issues: Vec::new(),
        }
    }

    /// Shipped blocks plus the user and imported blocks in `dir` (created
    /// when the first block is saved).
    pub fn open(dir: &Path) -> Library {
        let mut lib = Library::shipped_only();
        lib.dir = Some(dir.to_path_buf());
        lib.scan(dir, Origin::User);
        lib.scan(&dir.join("imported"), Origin::Imported);
        lib
    }

    fn scan(&mut self, dir: &Path, origin: Origin) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut files: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == BLOCK_EXTENSION))
            .collect();
        files.sort();
        for path in files {
            let Some(id) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            match std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| Block::from_json(&t).map_err(|e| e.to_string()))
            {
                Ok(block) => self.blocks.push(LibraryBlock {
                    source: BlockSource::new(origin, id),
                    block,
                    path: Some(path),
                }),
                Err(e) => self.issues.push(format!("{}: {e}", path.display())),
            }
        }
    }

    pub fn blocks(&self) -> &[LibraryBlock] {
        &self.blocks
    }

    pub fn get(&self, source: &BlockSource) -> Option<&LibraryBlock> {
        self.blocks.iter().find(|b| &b.source == source)
    }

    /// Blocks matching `query` (all of them for an empty one), by origin
    /// and then name.
    pub fn search(&self, query: &str) -> Vec<&LibraryBlock> {
        let mut out: Vec<&LibraryBlock> = self.blocks.iter().filter(|b| b.matches(query)).collect();
        out.sort_by(|a, b| {
            (a.source.origin, a.name().to_lowercase())
                .cmp(&(b.source.origin, b.name().to_lowercase()))
        });
        out
    }

    /// Every tag used, sorted.
    pub fn tags(&self) -> Vec<String> {
        let mut t: Vec<String> = self
            .blocks
            .iter()
            .flat_map(|b| b.block.def.tags.iter().map(|t| t.to_lowercase()))
            .collect();
        t.sort();
        t.dedup();
        t
    }

    /// Save `block` as the person's own. A block of the same name is
    /// replaced and its version raised past the old one, so projects using
    /// it are offered the update. Returns its source.
    pub fn save_user(&mut self, mut block: Block) -> Result<BlockSource, String> {
        let id = slug(&block.def.name);
        let source = BlockSource::new(Origin::User, &id);
        if let Some(old) = self.get(&source) {
            block.def.version = block.def.version.max(old.block.def.version + 1);
        }
        self.store(source, block)
    }

    /// Add someone else's block (from a pack or a file). The same block
    /// again changes nothing; a different one under the same name gets a
    /// new id. Returns its source.
    pub fn import(&mut self, block: Block, id_hint: &str) -> Result<BlockSource, String> {
        self.import_with_preview(block, id_hint, None)
    }

    /// [`Library::import`], keeping `preview` (a WAV file's bytes) beside
    /// the imported block, if it's given and the library has a folder.
    pub fn import_with_preview(
        &mut self,
        mut block: Block,
        id_hint: &str,
        preview: Option<&[u8]>,
    ) -> Result<BlockSource, String> {
        if let Some(same) = self.blocks.iter().find(|b| b.block.same_definition(&block)) {
            return Ok(same.source.clone());
        }
        let base = slug(id_hint);
        let mut id = base.clone();
        let mut n = 2;
        while self.get(&BlockSource::new(Origin::Imported, &id)).is_some() {
            id = format!("{base}-{n}");
            n += 1;
        }
        let source = BlockSource::new(Origin::Imported, id);
        block.preview = None;
        self.store_with_preview(source, block, preview)
    }

    fn store(&mut self, source: BlockSource, mut block: Block) -> Result<BlockSource, String> {
        block.preview = None;
        self.store_with_preview(source, block, None)
    }

    /// Write `block` (and `preview` beside it as `<id>.preview.wav`, which
    /// the block's file then records). Without a preview, an old one of
    /// that id goes: it was of an earlier version.
    fn store_with_preview(
        &mut self,
        source: BlockSource,
        mut block: Block,
        preview: Option<&[u8]>,
    ) -> Result<BlockSource, String> {
        let path = match &self.dir {
            Some(dir) => {
                let dir = match source.origin {
                    Origin::Imported => dir.join("imported"),
                    _ => dir.clone(),
                };
                std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                let file = preview_file_name(&source.id);
                let wav = dir.join(&file);
                match preview {
                    Some(bytes) => {
                        std::fs::write(&wav, bytes)
                            .map_err(|e| format!("{}: {e}", wav.display()))?;
                        block.preview = Some(BlockPreview {
                            file,
                            sha256: sha256_hex(bytes),
                        });
                    }
                    None => {
                        let _ = std::fs::remove_file(&wav);
                    }
                }
                let path = dir.join(format!("{}.{BLOCK_EXTENSION}", source.id));
                write_atomic(&path, &block.to_json())?;
                Some(path)
            }
            None => None,
        };
        self.blocks.retain(|b| b.source != source);
        self.blocks.push(LibraryBlock {
            source: source.clone(),
            block,
            path,
        });
        Ok(source)
    }

    /// Remove a user or imported block (shipped ones can't be removed).
    pub fn remove(&mut self, source: &BlockSource) -> Result<(), String> {
        if source.origin == Origin::Shipped {
            return Err("shipped blocks can't be removed".into());
        }
        if let Some(b) = self.get(source)
            && let Some(path) = b.path.clone()
        {
            std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if let Some(preview) = b.preview_path() {
                let _ = std::fs::remove_file(preview);
            }
        }
        self.blocks.retain(|b| &b.source != source);
        Ok(())
    }

    /// Definitions in `project` that came from a library block the library
    /// has a newer version of.
    pub fn updates(&self, project: &Project) -> Vec<Update> {
        project
            .group_defs
            .iter()
            .filter_map(|(id, def)| {
                let source = BlockSource::parse(def.source.as_deref()?)?;
                let lib = self.get(&source)?;
                (lib.block.def.version > def.version).then(|| Update {
                    def: id.clone(),
                    source,
                    from: def.version,
                    to: lib.block.def.version,
                })
            })
            .collect()
    }

    /// Import the definitions in `project` that came from a library this
    /// one doesn't have (someone else's blocks, arriving in a pack), so
    /// they show up under Imported. Returns how many were added.
    pub fn import_unknown(&mut self, project: &Project) -> usize {
        let mut added = 0;
        for id in project.group_defs.keys() {
            let def = &project.group_defs[id];
            let Some(source) = def.source.as_deref().and_then(BlockSource::parse) else {
                continue;
            };
            if self.get(&source).is_some() {
                continue;
            }
            let Some(mut block) = Block::from_project(project, id) else {
                continue;
            };
            block.def.source = None;
            if self.blocks.iter().any(|b| b.block.same_definition(&block)) {
                continue;
            }
            if self.import(block, &source.id).is_ok() {
                added += 1;
            }
        }
        added
    }

    /// Take in the blocks of a project that came from someone else (a
    /// pack): every copy of a person's own or imported block that isn't
    /// exactly one of ours is imported and its copy re-pointed at the
    /// imported block, so `user:x` from them can't be mistaken for our
    /// `user:x` (and offered "updates" from it). Shipped blocks are the same
    /// everywhere and stay as they are. Returns how many copies were
    /// re-pointed; the project changes only then.
    pub fn adopt(&mut self, project: &mut Project) -> usize {
        let mut retagged = 0;
        let ids: Vec<GroupId> = project.group_defs.keys().cloned().collect();
        for id in ids {
            let Some(source) = project.group_defs[&id]
                .source
                .as_deref()
                .and_then(BlockSource::parse)
            else {
                continue;
            };
            if source.origin == Origin::Shipped {
                continue;
            }
            let Some(mut block) = Block::from_project(project, &id) else {
                continue;
            };
            block.def.source = None;
            // Ours, exactly: nothing to do.
            if self
                .get(&source)
                .is_some_and(|b| b.block.same_definition(&block))
            {
                continue;
            }
            let Ok(new) = self.import(block, &source.id) else {
                continue;
            };
            if let Some(def) = project.group_defs.get_mut(&id) {
                def.source = Some(new.to_string());
                retagged += 1;
            }
        }
        retagged
    }
}

/// Write via a temporary file so a crash never leaves half a block.
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("acblock.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{GroupDef, NodeId, NodeInstance};

    fn block(name: &str, tags: &[&str]) -> Block {
        let mut def = GroupDef::new(name);
        def.tags = tags.iter().map(|t| t.to_string()).collect();
        def.graph.nodes.push(NodeInstance::new(NodeId(1), "gain"));
        Block {
            def,
            uses: Default::default(),
            preview: None,
        }
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ac-library-{tag}-{}-{}",
            std::process::id(),
            crate::clock::now_ns()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn every_shipped_block_loads() {
        assert_eq!(shipped_blocks().len(), shipped::SHIPPED.len());
        for b in shipped_blocks() {
            assert!(!b.block.def.name.is_empty(), "{}", b.source);
            assert!(
                !b.block.def.description.is_empty(),
                "{} has no description",
                b.source
            );
        }
    }

    #[test]
    fn user_blocks_save_bump_and_reload() {
        let dir = temp("user");
        let mut lib = Library::open(&dir);
        let s = lib.save_user(block("My Bass", &["synth"])).unwrap();
        assert_eq!(s.to_string(), "user:my-bass");
        assert!(dir.join("my-bass.acblock").exists());
        // Saving again raises the version.
        lib.save_user(block("My Bass", &["synth"])).unwrap();
        assert_eq!(lib.get(&s).unwrap().block.def.version, 2);
        let again = Library::open(&dir);
        assert_eq!(again.get(&s).unwrap().block.def.version, 2);
        assert!(again.issues.is_empty());
        lib.remove(&s).unwrap();
        assert!(!dir.join("my-bass.acblock").exists());
        assert!(lib.remove(&BlockSource::new(Origin::Shipped, "x")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn imports_dedupe_and_rename() {
        let dir = temp("import");
        let mut lib = Library::open(&dir);
        let a = lib.import(block("Echo", &[]), "echo").unwrap();
        assert_eq!(a.to_string(), "imported:echo");
        assert_eq!(
            lib.import(block("Echo", &[]), "echo").unwrap(),
            a,
            "same block"
        );
        let b = lib.import(block("Echo", &["other"]), "echo").unwrap();
        assert_eq!(b.to_string(), "imported:echo-2");
        assert!(dir.join("imported/echo-2.acblock").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn imported_previews_live_beside_their_blocks() {
        let dir = temp("preview");
        let mut lib = Library::open(&dir);
        let wav = crate::wav::wav_bytes(&[vec![0.25; 64]], 48_000, crate::BitDepth::Pcm16).unwrap();
        let mut b = block("Echo", &[]);
        // Whatever the file said its preview was called, it's kept by id.
        b.preview = Some(BlockPreview {
            file: "elsewhere.wav".into(),
            sha256: "0".repeat(64),
        });
        let s = lib.import_with_preview(b, "echo", Some(&wav)).unwrap();
        let file = dir.join("imported/echo.preview.wav");
        assert_eq!(std::fs::read(&file).unwrap(), wav);
        let again = Library::open(&dir);
        let got = again.get(&s).unwrap();
        assert_eq!(got.preview_path().as_deref(), Some(file.as_path()));
        assert_eq!(got.block.preview.as_ref().unwrap().sha256, sha256_hex(&wav));
        let path = got.path.clone().unwrap();
        assert_eq!(sidecar_preview(&path, &got.block), Some(wav.clone()));
        // A block without one has none, even if its file claims one.
        let mut claims = block("Claims", &[]);
        claims.preview = Some(BlockPreview {
            file: "claims.preview.wav".into(),
            sha256: "0".repeat(64),
        });
        let c = lib.import(claims, "claims").unwrap();
        assert_eq!(lib.get(&c).unwrap().block.preview, None);
        // Removing the block takes its preview too.
        lib.remove(&s).unwrap();
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_matches_names_tags_and_descriptions() {
        let mut lib = Library::default();
        lib.save_user(block("Plate Reverb", &["space"])).unwrap();
        lib.save_user(block("Ducker", &["dynamics", "sidechain"]))
            .unwrap();
        assert_eq!(lib.search("reverb").len(), 1);
        assert_eq!(lib.search("SIDECHAIN").len(), 1);
        assert_eq!(lib.search("dynamics duck").len(), 1);
        assert_eq!(lib.search("").len(), lib.blocks().len());
        assert!(lib.tags().contains(&"space".to_string()));
    }

    #[test]
    fn updates_and_unknown_blocks_from_a_project() {
        let mut lib = Library::default();
        let src = lib.save_user(block("Comp", &[])).unwrap();
        lib.save_user(block("Comp", &[])).unwrap(); // version 2
        let mut p = Project::default();
        let mut old = block("Comp", &[]).def;
        old.source = Some(src.to_string());
        p.group_defs.insert(GroupId("comp".into()), old);
        let mut theirs = block("Their Thing", &[]).def;
        theirs.source = Some("user:their-thing".into());
        p.group_defs.insert(GroupId("their".into()), theirs);
        let u = lib.updates(&p);
        assert_eq!(u.len(), 1);
        assert_eq!((u[0].from, u[0].to), (1, 2));
        assert_eq!(lib.import_unknown(&p), 1);
        assert!(
            lib.get(&BlockSource::new(Origin::Imported, "their-thing"))
                .is_some()
        );
        assert_eq!(lib.import_unknown(&p), 0, "only once");
    }

    #[test]
    fn someone_elses_block_of_the_same_name_is_adopted_not_confused() {
        let mut lib = Library::default();
        let mine = lib.save_user(block("Comp", &["mine"])).unwrap();
        lib.save_user(block("Comp", &["mine"])).unwrap(); // our v2
        // Their `user:comp` is a different block at v1.
        let mut p = Project::default();
        let mut theirs = block("Comp", &["theirs"]).def;
        theirs.source = Some(mine.to_string());
        p.group_defs.insert(GroupId("comp".into()), theirs);
        // An exact copy of a shipped-style or our own current block stays.
        let mut ours = lib.get(&mine).unwrap().block.def.clone();
        ours.source = Some(mine.to_string());
        p.group_defs.insert(GroupId("ours".into()), ours);
        assert_eq!(lib.adopt(&mut p), 1);
        let src = p.group_defs[&GroupId("comp".into())]
            .source
            .clone()
            .unwrap();
        assert_eq!(src, "imported:comp");
        assert_eq!(
            p.group_defs[&GroupId("ours".into())].source.as_deref(),
            Some("user:comp")
        );
        // No update is offered from our Comp to their block any more.
        assert!(
            lib.updates(&p)
                .iter()
                .all(|u| u.def != GroupId("comp".into()))
        );
        assert_eq!(lib.adopt(&mut p), 0, "already adopted");
    }
}
