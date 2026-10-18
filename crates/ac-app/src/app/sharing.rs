//! Blocks and packs: adding a library block, saving a group to the
//! library, updating a project's copies when the library has a newer
//! version, importing block files, and exporting packs.

use std::path::Path;

use ac_core::{Block, BlockSource, GroupId, update_def};
use ac_io::Update;

use super::{EditorApp, plural};
use crate::dialogs::FilePurpose;
use crate::files;
use crate::palette;
use crate::panels::Tab;

impl EditorApp {
    /// Clipboard text holding one instance of library block `source`, its
    /// definition tagged with where it came from. A block that reads a
    /// Voice node inside comes as a Voices instance.
    pub(super) fn block_instance(&self, source: &BlockSource) -> Option<String> {
        let b = self.library.get(source)?;
        let id = GroupId(ac_core::slug(&b.block.def.name));
        let poly = palette::block_is_polyphonic(&b.block);
        Some(b.block.instance(&id, source, poly).to_json())
    }

    /// Add a block file someone shared to the library.
    pub fn import_block_file(&mut self, path: &Path, now: f64) {
        let block = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|t| Block::from_json(&t).map_err(|e| e.to_string()));
        let block = match block {
            Ok(b) => b,
            Err(e) => return self.warn(format!("Couldn't read {}: {e}", path.display()), now),
        };
        let name = block.def.name.clone();
        let known = self.library.blocks().iter().any(|b| b.block == block);
        let hint = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&name)
            .to_string();
        match self.library.import(block, &hint) {
            Ok(_) if known => self.say(format!("{name} is already in your library"), now),
            Ok(_) => {
                self.side.open = true;
                self.side.tab = Tab::Library;
                self.say(format!("Imported {name} into your library"), now);
            }
            Err(e) => self.warn(format!("Couldn't import {name}: {e}"), now),
        }
    }

    /// Save group definition `def` to the library as one of the person's
    /// own blocks. The project's copy then remembers where it went and at
    /// which version, so other projects using it can be offered updates.
    pub fn save_block(&mut self, def: &GroupId, now: f64) {
        let Some(block) = Block::from_project(&self.doc.project, def) else {
            return;
        };
        let name = block.def.name.clone();
        match self.library.save_user(block) {
            Ok(source) => {
                let version = self.library.get(&source).map_or(1, |b| b.block.def.version);
                let tag = source.to_string();
                self.doc.edit_project("Save block", |p| {
                    if let Some(d) = p.group_defs.get_mut(def) {
                        d.source = Some(tag);
                        d.version = version;
                    }
                });
                self.say(format!("Saved {name} to your library (v{version})"), now);
            }
            Err(e) => self.warn(format!("Couldn't save {name} to your library: {e}"), now),
        }
    }

    /// Replace definition `def` with the library's newer version, as one
    /// undo step. Returns whether there was one.
    pub fn update_block(&mut self, def: &GroupId, now: f64) -> bool {
        let Some(u) = self
            .library
            .updates(&self.doc.project)
            .into_iter()
            .find(|u| &u.def == def)
        else {
            self.say("That block is up to date", now);
            return false;
        };
        let dropped = self.apply_updates(std::slice::from_ref(&u), "Update block");
        let name = self
            .doc
            .project
            .group_defs
            .get(def)
            .map_or_else(|| def.0.clone(), |d| d.name.clone());
        self.report_update(format!("Updated {name} to v{}", u.to), dropped, now);
        true
    }

    /// Update every definition the library has a newer version of, as one
    /// undo step. Returns how many.
    pub fn update_all_blocks(&mut self, now: f64) -> usize {
        let updates = self.library.updates(&self.doc.project);
        if updates.is_empty() {
            self.say("Every block is up to date", now);
            return 0;
        }
        let dropped = self.apply_updates(&updates, "Update blocks");
        let n = updates.len();
        self.report_update(format!("Updated {n} block{}", plural(n)), dropped, now);
        n
    }

    /// Apply `updates` as one undo step; returns how many wires went with
    /// sockets the new versions don't have.
    fn apply_updates(&mut self, updates: &[Update], label: &str) -> usize {
        let blocks: Vec<(GroupId, BlockSource, Block)> = updates
            .iter()
            .filter_map(|u| {
                let b = self.library.get(&u.source)?;
                Some((u.def.clone(), u.source.clone(), b.block.clone()))
            })
            .collect();
        self.doc.edit_project(label, |p| {
            blocks
                .iter()
                .map(|(id, source, block)| update_def(p, id, block, source).dropped_wires.len())
                .sum()
        })
    }

    fn report_update(&mut self, text: String, dropped: usize, now: f64) {
        if dropped > 0 {
            self.warn(
                format!(
                    "{text} · removed {dropped} wire{} to sockets the new version doesn't have",
                    plural(dropped)
                ),
                now,
            );
        } else {
            self.say(text, now);
        }
    }

    /// Take a saved or imported block out of the library (its file goes
    /// too). Projects that use it keep their copy.
    pub fn remove_block(&mut self, source: &BlockSource, now: f64) {
        let Some(b) = self.library.get(source) else {
            return;
        };
        let name = b.name().to_string();
        let file = b.path.is_some();
        match self.library.remove(source) {
            Ok(()) => {
                let what = if file { " and deleted its file" } else { "" };
                self.say(
                    format!(
                        "Removed {name} from your library{what}; projects that use it keep their copy"
                    ),
                    now,
                );
            }
            Err(e) => self.warn(format!("Couldn't remove {name}: {e}"), now),
        }
    }

    /// File ▸ Export Pack: a pack is made from the saved project, so an
    /// untitled project is saved first (asking where), and unsaved changes
    /// are saved; then the pack's own Save dialog.
    pub(super) fn export_pack(&mut self, now: f64) {
        let Some(path) = self.doc.path.clone() else {
            self.pack_after_save = true;
            self.say(
                "Save the project first; the pack is made from the saved file",
                now,
            );
            self.file_dialog(FilePurpose::SaveAs, now);
            return;
        };
        if self.doc.is_dirty() {
            self.save_to(&path, now);
            if self.doc.is_dirty() {
                return; // the save failed and said why
            }
        }
        self.file_dialog(FilePurpose::ExportPack, now);
    }

    /// Write the saved project and its audio files to the pack `out`.
    pub fn export_pack_to(&mut self, out: &Path, now: f64) {
        let Some(project) = self.doc.path.clone() else {
            self.warn("Save the project before packing it", now);
            return;
        };
        let out = files::with_extension(out.to_path_buf(), ac_io::PACK_EXTENSION);
        match ac_io::write_pack(&project, &out, &self.registry) {
            Ok(r) => {
                let audio = r.files.saturating_sub(1);
                let mut text = format!(
                    "Packed the project and {audio} audio file{} into {}",
                    plural(audio),
                    out.display()
                );
                if !r.moved.is_empty() {
                    let n = r.moved.len();
                    text.push_str(&format!(
                        " · copied {n} file{} from outside the project folder into the pack's assets/",
                        plural(n)
                    ));
                }
                if r.missing.is_empty() {
                    self.say(text, now);
                } else {
                    let n = r.missing.len();
                    text.push_str(&format!(
                        " · left out {n} missing file{}: {}",
                        plural(n),
                        r.missing.join(", ")
                    ));
                    self.warn(text, now);
                }
            }
            Err(e) => self.warn(format!("Couldn't write the pack: {e}"), now),
        }
    }
}
