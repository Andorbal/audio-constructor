//! Blocks and packs: adding a library block, saving a group to the
//! library, updating a project's copies when the library has a newer
//! version, importing block files, exporting blocks and packs, previews,
//! and blocks, packs, and shared libraries from links (`online.rs` does
//! the background work).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ac_core::{Block, BlockSource, GroupId, Origin, update_def};
use ac_io::Update;
use ac_io::remote::{Fetched, RemoteBlock};
use egui::{Key, RichText};

use super::{AfterConfirm, EditorApp};
use crate::dialogs::{BlockToExport, FilePurpose};
use crate::files;
use crate::i18n::{tr, tr_args};
use crate::icons;
use crate::online::{Done, LibraryState, LinkDialog, Playing, PreviewKey, can_preview};
use crate::palette;
use crate::panels::{BlockFilter, Tab};
use crate::theme;

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

    /// Add a block file someone shared to the library, with the preview
    /// beside it if it has one that matches its record.
    pub fn import_block_file(&mut self, path: &Path, now: f64) {
        let block = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|t| Block::from_json(&t).map_err(|e| e.to_string()));
        let block = match block {
            Ok(b) => b,
            Err(e) => {
                return self.warn(
                    tr_args(
                        "status-couldnt-read",
                        &[
                            ("path", path.display().to_string().into()),
                            ("error", e.into()),
                        ],
                    ),
                    now,
                );
            }
        };
        let preview = ac_io::library::sidecar_preview(path, &block);
        let hint = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&block.def.name)
            .to_string();
        self.import_block(block, &hint, preview.as_deref(), now);
    }

    /// Add a block someone shared, given the text of its file (called
    /// `what` in messages), under an id made from `hint` or its name: a host
    /// that keeps the files hands over their contents (the web build).
    pub(super) fn import_block_text(&mut self, text: &str, hint: &str, what: &str, now: f64) {
        let block = match Block::from_json(text) {
            Ok(b) => b,
            Err(e) => {
                return self.warn(
                    tr_args(
                        "status-couldnt-read",
                        &[("path", what.into()), ("error", e.to_string().into())],
                    ),
                    now,
                );
            }
        };
        let hint = if hint.is_empty() {
            block.def.name.clone()
        } else {
            hint.to_string()
        };
        self.import_block(block, &hint, None, now);
    }

    /// Import someone else's block (D83's file rules, wherever it came
    /// from) and show where it landed.
    fn import_block(&mut self, block: Block, hint: &str, preview: Option<&[u8]>, now: f64) {
        let name = block.def.name.clone();
        let known = self
            .library
            .blocks()
            .iter()
            .any(|b| b.block.same_definition(&block));
        match self.library.import_with_preview(block, hint, preview) {
            Ok(_) if known => self.say(
                tr_args("status-block-already-there", &[("name", (&name).into())]),
                now,
            ),
            Ok(source) => {
                // Show it where it landed.
                self.side.open = true;
                self.side.tab = Tab::Library;
                self.side.block_filter = BlockFilter {
                    origin: Some(Origin::Imported),
                    tag: None,
                };
                let with_preview = self
                    .library
                    .get(&source)
                    .and_then(|b| b.preview_path())
                    .is_some();
                self.say(
                    if with_preview {
                        tr_args(
                            "status-block-imported-with-preview",
                            &[("name", (&name).into())],
                        )
                    } else {
                        tr_args("status-block-imported", &[("name", (&name).into())])
                    },
                    now,
                );
            }
            Err(e) => self.warn(
                tr_args(
                    "status-couldnt-import",
                    &[("name", (&name).into()), ("error", e.to_string().into())],
                ),
                now,
            ),
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
                // Its preview follows, rendered in the background and kept
                // beside it, so the folder can be shared as a library.
                self.online.forget(&source);
                if let Some(b) = self.library.get(&source)
                    && b.path.is_some()
                    && can_preview(&b.block)
                {
                    let block = b.block.clone();
                    self.online
                        .render_saved(source.clone(), block, self.registry.clone());
                }
                let tag = source.to_string();
                self.doc.edit_project(&tr("undo-save-block"), |p| {
                    if let Some(d) = p.group_defs.get_mut(def) {
                        d.source = Some(tag);
                        d.version = version;
                    }
                });
                self.say(
                    tr_args(
                        "status-block-saved",
                        &[
                            ("name", (&name).into()),
                            ("version", version.to_string().into()),
                        ],
                    ),
                    now,
                );
            }
            Err(e) => self.warn(
                tr_args(
                    "status-couldnt-save-block",
                    &[("name", (&name).into()), ("error", e.to_string().into())],
                ),
                now,
            ),
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
            self.say(tr("status-block-up-to-date"), now);
            return false;
        };
        let dropped = self.apply_updates(std::slice::from_ref(&u), &tr("undo-update-block"));
        let name = self
            .doc
            .project
            .group_defs
            .get(def)
            .map_or_else(|| def.0.clone(), |d| d.name.clone());
        self.report_update(
            tr_args(
                "status-block-updated",
                &[("name", name.into()), ("version", u.to.to_string().into())],
            ),
            dropped,
            now,
        );
        true
    }

    /// Update every definition the library has a newer version of, as one
    /// undo step. Returns how many.
    pub fn update_all_blocks(&mut self, now: f64) -> usize {
        let updates = self.library.updates(&self.doc.project);
        if updates.is_empty() {
            self.say(tr("status-blocks-up-to-date"), now);
            return 0;
        }
        let dropped = self.apply_updates(&updates, &tr("undo-update-blocks"));
        let n = updates.len();
        self.report_update(
            tr_args("status-blocks-updated", &[("count", n.into())]),
            dropped,
            now,
        );
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
                    "{text} · {}",
                    tr_args("status-update-dropped-wires", &[("count", dropped.into())])
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
                let text = if file {
                    tr_args("status-block-removed-file", &[("name", (&name).into())])
                } else {
                    tr_args("status-block-removed", &[("name", (&name).into())])
                };
                self.say(text, now);
            }
            Err(e) => self.warn(
                tr_args(
                    "status-couldnt-remove-block",
                    &[("name", (&name).into()), ("error", e.to_string().into())],
                ),
                now,
            ),
        }
    }

    /// File ▸ Export Pack: a pack is made from the saved project, so an
    /// untitled project is saved first (asking where), and unsaved changes
    /// are saved; then the pack's own Save dialog.
    pub(super) fn export_pack(&mut self, now: f64) {
        if self.host_files {
            return self.host_export_pack(now);
        }
        let Some(path) = self.doc.path.clone() else {
            self.pack_after_save = true;
            self.say(tr("status-save-before-pack"), now);
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
            self.warn(tr("status-save-before-packing"), now);
            return;
        };
        let out = files::with_extension(out.to_path_buf(), ac_io::PACK_EXTENSION);
        match ac_io::write_pack(&project, &out, &self.registry) {
            Ok(r) => {
                let audio = r.files.saturating_sub(1);
                let mut text = tr_args(
                    "status-packed",
                    &[
                        ("count", audio.into()),
                        ("path", out.display().to_string().into()),
                    ],
                );
                if !r.moved.is_empty() {
                    let n = r.moved.len();
                    text.push_str(" · ");
                    text.push_str(&tr_args("status-pack-copied", &[("count", n.into())]));
                }
                if r.missing.is_empty() {
                    self.say(text, now);
                } else {
                    let n = r.missing.len();
                    text.push_str(" · ");
                    text.push_str(&tr_args(
                        "status-pack-left-out",
                        &[("count", n.into()), ("files", r.missing.join(", ").into())],
                    ));
                    self.warn(text, now);
                }
            }
            Err(e) => self.warn(
                tr_args("status-couldnt-pack", &[("error", e.to_string().into())]),
                now,
            ),
        }
    }

    // ------------------------------------------------------------ export

    /// What an export of `what` is called.
    pub(super) fn export_name(&self, what: &BlockToExport) -> String {
        match what {
            BlockToExport::Def(id) => self
                .doc
                .project
                .group_defs
                .get(id)
                .map_or_else(|| id.0.clone(), |d| d.name.clone()),
            BlockToExport::Library(source) => self
                .library
                .get(source)
                .map_or_else(|| source.id.clone(), |b| b.name().to_string()),
        }
    }

    /// Write `what` to `path` as a block file with its preview beside it
    /// (rendered in the background).
    /// The block `what` names, as it would be exported.
    pub(super) fn block_to_export(&self, what: &BlockToExport) -> Option<Block> {
        match what {
            BlockToExport::Def(id) => Block::from_project(&self.doc.project, id),
            BlockToExport::Library(source) => self.library.get(source).map(|b| b.block.clone()),
        }
    }

    pub fn export_block_to(&mut self, what: &BlockToExport, path: PathBuf, now: f64) {
        let Some(block) = self.block_to_export(what) else {
            return self.warn(tr("status-export-block-gone"), now);
        };
        let path = files::with_extension(path, ac_core::BLOCK_EXTENSION);
        let name = block.def.name.clone();
        self.online.export(block, path, self.registry.clone());
        self.say(
            tr_args("status-exporting-name", &[("name", name.into())]),
            now,
        );
    }

    // ------------------------------------------------------------ links

    /// File ▸ Import from Link…: ask for the link.
    pub(super) fn open_link_dialog(&mut self) {
        self.online.link_dialog = Some(LinkDialog {
            text: String::new(),
            focus: true,
        });
    }

    /// Import whatever `link` holds: a block, a pack, or a shared library
    /// (the download runs in the background; tests call this directly).
    pub fn import_link(&mut self, link: &str, now: f64) {
        let link = link.trim();
        if link.is_empty() {
            return;
        }
        self.online.import_link(link);
        self.say(tr_args("status-downloading", &[("link", link.into())]), now);
    }

    /// The "Import from Link…" dialog, while it's open.
    pub(super) fn link_dialog(&mut self, ctx: &egui::Context, now: f64) {
        let Some(d) = self.online.link_dialog.as_mut() else {
            return;
        };
        let mut result: Option<Option<String>> = None;
        let r = egui::Modal::new(egui::Id::unique("import-from-link")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.label(
                RichText::new(format!("{}  {}", icons::LINK, tr("dialog-link-title")))
                    .size(theme::FONT_HEADING)
                    .color(theme::text_strong()),
            );
            ui.label(RichText::new(tr("dialog-link-text")).color(theme::text_muted()));
            ui.add_space(6.0);
            let field = ui
                .add(
                    egui::TextEdit::singleline(&mut d.text)
                        .hint_text("https://…")
                        .desired_width(f32::INFINITY)
                        .font(theme::mono_font(theme::FONT_VALUE)),
                )
                .accessible_name(tr("dialog-link-field"));
            if d.focus {
                field.request_focus();
                d.focus = false;
            }
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tr("dialog-cancel")).clicked()
                    || ui.input(|i| i.key_pressed(Key::Escape))
                {
                    result = Some(None);
                }
                let ready = !d.text.trim().is_empty();
                let import = ui.add_enabled(ready, egui::Button::new(tr("dialog-link-import")));
                if ready && (import.clicked() || enter) {
                    result = Some(Some(d.text.trim().to_string()));
                }
            });
        });
        if r.should_close() && result.is_none() {
            result = Some(None);
        }
        if let Some(r) = result {
            self.online.link_dialog = None;
            if let Some(link) = r {
                self.import_link(&link, now);
            }
        }
    }

    /// Stop listing a shared library (its blocks you installed stay).
    pub(super) fn remove_shared_library(&mut self, link: &str, now: f64) {
        let name = self
            .online
            .library(link)
            .map_or_else(|| link.to_string(), |l| l.name().to_string());
        self.online.remove_library(link);
        self.settings.remove_library(link);
        self.say(
            tr_args("status-library-removed", &[("name", name.into())]),
            now,
        );
    }

    // ------------------------------------------------------------ previews

    /// Play `key`'s preview over the patch, or stop it if it's the one
    /// playing (or loading).
    pub fn toggle_preview(&mut self, key: PreviewKey, now: f64) {
        let current = match &self.online.playing {
            Playing::Idle => None,
            Playing::Loading(k) | Playing::Playing(k, _) => Some(k.clone()),
        };
        if let Some((a, _)) = self.engine.audition() {
            a.stop();
        }
        self.online.playing = Playing::Idle;
        if current.as_ref() == Some(&key) {
            return;
        }
        let Some((_, rate)) = self.engine.audition() else {
            return self.warn(tr("status-preview-needs-output"), now);
        };
        if let Some(audio) = self.online.cached(&key, rate) {
            return self.play_preview(key, &audio);
        }
        match &key {
            PreviewKey::Library(source) => {
                let Some(b) = self.library.get(source) else {
                    return;
                };
                if !can_preview(&b.block) {
                    return self.warn(
                        tr_args("status-preview-no-output", &[("name", b.name().into())]),
                        now,
                    );
                }
                let (block, file) = (b.block.clone(), b.preview_path());
                self.online.load_library_preview(
                    key.clone(),
                    block,
                    file,
                    self.registry.clone(),
                    rate,
                );
            }
            PreviewKey::Remote { link, id } => {
                let (link, id) = (link.clone(), id.clone());
                self.online.load_remote_preview(&link, &id, rate);
            }
        }
    }

    fn play_preview(&mut self, key: PreviewKey, audio: &[Vec<f32>]) {
        let Some((a, _)) = self.engine.audition() else {
            return;
        };
        if let Some(id) = a.play(audio.to_vec(), 1.0) {
            self.online.playing = Playing::Playing(key, id);
        }
    }

    /// The preview playing, or being loaded, if any (tests read this).
    pub fn preview_state(&self) -> &Playing {
        &self.online.playing
    }

    // ------------------------------------------------------------ results

    /// Take in whatever background work finished, and keep frames coming
    /// while some is still running or a preview plays.
    pub(super) fn poll_online(&mut self, ctx: &egui::Context, now: f64) {
        for done in self.online.poll() {
            self.online_done(done, now);
        }
        if let Playing::Playing(_, id) = &self.online.playing {
            let done = self.engine.audition().is_none_or(|(a, _)| a.is_done(*id));
            if done {
                self.online.playing = Playing::Idle;
            }
        }
        if self.online.busy() || self.online.playing != Playing::Idle {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn online_done(&mut self, done: Done, now: f64) {
        match done {
            Done::Link { link, result } => {
                self.online.importing.retain(|l| l != &link);
                match result {
                    Ok(Fetched::Block(b)) => self.import_remote(*b, now),
                    Ok(Fetched::Pack { bytes, name }) => {
                        self.open_downloaded_pack(&bytes, &name, now)
                    }
                    Ok(Fetched::Library(remote)) => {
                        let (name, n) = (remote.index.name.clone(), remote.index.blocks.len());
                        let skipped = remote.skipped.len();
                        self.settings.add_library(&remote.url);
                        self.online.add_library(remote);
                        self.side.open = true;
                        self.side.tab = Tab::Library;
                        let mut text = tr_args(
                            "status-library-added",
                            &[("name", name.into()), ("count", n.into())],
                        );
                        if skipped > 0 {
                            text.push_str(" · ");
                            text.push_str(&tr_args(
                                "status-library-skipped",
                                &[("count", skipped.into())],
                            ));
                            self.warn(text, now);
                        } else {
                            self.say(text, now);
                        }
                    }
                    Err(e) => self.warn(
                        tr_args(
                            "status-couldnt-import-link",
                            &[("error", e.to_string().into())],
                        ),
                        now,
                    ),
                }
            }
            Done::Library { link, result } => {
                let Some(l) = self.online.library_mut(&link) else {
                    return;
                };
                l.state = match result {
                    Ok(r) => LibraryState::Ready(Box::new(r)),
                    Err(e) => LibraryState::Failed(e),
                };
            }
            Done::Install { link, id, result } => {
                if let Some(l) = self.online.library_mut(&link) {
                    l.installing.remove(&id);
                }
                match result {
                    Ok(b) => self.import_remote(*b, now),
                    Err(e) => self.warn(
                        tr_args("status-couldnt-install", &[("error", e.to_string().into())]),
                        now,
                    ),
                }
            }
            Done::Preview { key, rate, result } => {
                let wanted = self.online.playing == Playing::Loading(key.clone());
                match result {
                    Ok(audio) => {
                        let audio = Arc::new(audio);
                        self.online.keep(key.clone(), rate, audio.clone());
                        if wanted {
                            self.online.playing = Playing::Idle;
                            self.play_preview(key, &audio);
                        }
                    }
                    Err(e) if wanted => {
                        self.online.playing = Playing::Idle;
                        self.warn(
                            tr_args("status-couldnt-preview", &[("error", e.to_string().into())]),
                            now,
                        );
                    }
                    Err(_) => {}
                }
            }
            Done::Export { name, result } => {
                self.online.exporting = self.online.exporting.saturating_sub(1);
                match result {
                    Ok(e) => {
                        let path = e.block.display().to_string();
                        let args = [("name", (&name).into()), ("path", (&path).into())];
                        match (&e.preview, &e.no_preview) {
                            (Some(_), _) => {
                                self.say(tr_args("status-exported-block-preview", &args), now)
                            }
                            (None, Some(why)) => self.warn(
                                tr_args(
                                    "status-exported-block-no-preview",
                                    &[
                                        ("name", (&name).into()),
                                        ("path", path.into()),
                                        ("why", why.to_string().into()),
                                    ],
                                ),
                                now,
                            ),
                            (None, None) => self.say(tr_args("status-exported-block", &args), now),
                        }
                    }
                    Err(e) => self.warn(
                        tr_args(
                            "status-couldnt-export-block",
                            &[("name", name.into()), ("error", e.to_string().into())],
                        ),
                        now,
                    ),
                }
            }
            Done::SavedPreview { source, block, wav } => {
                let current = self
                    .library
                    .get(&source)
                    .is_some_and(|b| b.block.same_definition(&block));
                if let (true, Some(wav)) = (current, wav) {
                    // Failing to keep a preview loses nothing: Play renders one.
                    let _ = self.library.set_preview(&source, &wav);
                }
            }
        }
    }

    fn import_remote(&mut self, b: RemoteBlock, now: f64) {
        let RemoteBlock {
            block,
            id_hint,
            preview,
        } = b;
        self.import_block(block, &id_hint, preview.as_deref(), now);
    }

    /// Keep a pack from a link in the download folder and open it as a
    /// pack file opens (asking about unsaved changes first).
    fn open_downloaded_pack(&mut self, bytes: &[u8], name: &str, now: f64) {
        let path = self.online.pack_path(name);
        let saved = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, bytes));
        match saved {
            Ok(()) => {
                self.say(
                    tr_args(
                        "status-pack-downloaded",
                        &[("path", path.display().to_string().into())],
                    ),
                    now,
                );
                self.request(AfterConfirm::Open(path), now);
            }
            Err(e) => self.warn(
                tr_args(
                    "status-couldnt-save-pack",
                    &[
                        ("path", path.display().to_string().into()),
                        ("error", e.to_string().into()),
                    ],
                ),
                now,
            ),
        }
    }
}
