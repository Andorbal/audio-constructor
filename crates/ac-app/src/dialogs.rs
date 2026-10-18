//! Modal dialogs: unsaved-changes confirmation, typing a file path (when
//! the system has no file dialog), keyboard shortcuts, and About.

use std::path::PathBuf;
use std::time::Instant;

use egui::{Key, RichText};

use crate::actions::Action;
use crate::icons;
use crate::theme;

/// What a file dialog is for.
#[derive(Clone, Debug, PartialEq)]
pub enum FilePurpose {
    Open,
    SaveAs,
    Export,
    ChooseClip(ac_core::NodeId),
    Locate(String),
    /// Where to write a pack of the saved project.
    ExportPack,
    /// A block file to add to the library.
    ImportBlock,
}

impl FilePurpose {
    fn title(&self) -> &'static str {
        match self {
            FilePurpose::Open => "Open project",
            FilePurpose::SaveAs => "Save project as",
            FilePurpose::Export => "Export WAV",
            FilePurpose::ChooseClip(_) => "Choose an audio file",
            FilePurpose::Locate(_) => "Locate missing file",
            FilePurpose::ExportPack => "Export pack",
            FilePurpose::ImportBlock => "Import block file",
        }
    }

    fn saving(&self) -> bool {
        matches!(
            self,
            FilePurpose::SaveAs | FilePurpose::Export | FilePurpose::ExportPack
        )
    }
}

/// No system file dialog could be shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoDialog;

/// Ask for a file with the system dialog. Returns `Err(NoDialog)` when no
/// system dialog is available (it returned at once), so the caller can fall
/// back to typing a path.
pub fn native(
    purpose: &FilePurpose,
    suggested: Option<&std::path::Path>,
) -> Result<Option<PathBuf>, NoDialog> {
    let mut d = rfd::FileDialog::new().set_title(purpose.title());
    d = match purpose {
        FilePurpose::Open => d
            .add_filter(
                "Projects, packs, and blocks",
                &["acproj", "acpack", "acblock"],
            )
            .add_filter("Audio Constructor project", &["acproj"])
            .add_filter("Audio Constructor pack", &["acpack"])
            .add_filter("Audio Constructor block", &["acblock"]),
        FilePurpose::SaveAs => d.add_filter("Audio Constructor project", &["acproj"]),
        FilePurpose::Export => d.add_filter("WAV audio", &["wav"]),
        FilePurpose::ExportPack => d.add_filter("Audio Constructor pack", &["acpack"]),
        FilePurpose::ImportBlock => d.add_filter("Audio Constructor block", &["acblock"]),
        FilePurpose::ChooseClip(_) | FilePurpose::Locate(_) => {
            d.add_filter("WAV audio", &["wav", "wave"])
        }
    };
    if let Some(s) = suggested {
        if let Some(dir) = s.parent().filter(|p| p.is_dir()) {
            d = d.set_directory(dir);
        }
        if let Some(name) = s.file_name() {
            d = d.set_file_name(name.to_string_lossy());
        }
    }
    let t0 = Instant::now();
    let r = if purpose.saving() {
        d.save_file()
    } else {
        d.pick_file()
    };
    // A person needs longer than this to dismiss a dialog; an instant
    // `None` means there was no dialog to show.
    if r.is_none() && t0.elapsed().as_millis() < 60 {
        return Err(NoDialog);
    }
    Ok(r)
}

/// Typing a path instead of a system dialog.
pub struct PathPrompt {
    pub purpose: FilePurpose,
    pub text: String,
    focus: bool,
}

impl PathPrompt {
    pub fn new(purpose: FilePurpose, suggested: Option<PathBuf>) -> PathPrompt {
        PathPrompt {
            purpose,
            text: suggested
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            focus: true,
        }
    }

    /// Show the prompt. `Some(Some(path))` on OK, `Some(None)` on cancel.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Option<PathBuf>> {
        let mut result = None;
        egui::Modal::new(egui::Id::unique("path-prompt")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(
                RichText::new(self.purpose.title())
                    .size(theme::FONT_HEADING)
                    .color(theme::text_strong()),
            );
            ui.label(RichText::new("Type the file's path.").color(theme::text_muted()));
            ui.add_space(6.0);
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.text)
                    .desired_width(f32::INFINITY)
                    .font(theme::mono_font(theme::FONT_VALUE)),
            );
            if self.focus {
                r.request_focus();
                self.focus = false;
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                    result = Some(None);
                }
                let ok = ui.add_enabled(!self.text.trim().is_empty(), egui::Button::new("OK"));
                if (ok.clicked() || enter) && !self.text.trim().is_empty() {
                    let t = self.text.trim();
                    let p = if let Some(rest) = t.strip_prefix("~/") {
                        directories::UserDirs::new()
                            .map_or_else(|| PathBuf::from(t), |d| d.home_dir().join(rest))
                    } else {
                        PathBuf::from(t)
                    };
                    result = Some(Some(p));
                }
            });
        });
        result
    }
}

/// The answer to "save changes?".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    Save,
    Discard,
    Cancel,
}

pub fn confirm_discard(ctx: &egui::Context, title: &str) -> Option<Confirm> {
    let mut out = None;
    let r = egui::Modal::new(egui::Id::unique("confirm-discard")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(icons::WARNING)
                    .size(24.0)
                    .color(theme::warning()),
            );
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(format!("Save changes to “{title}”?"))
                        .size(theme::FONT_HEADING)
                        .color(theme::text_strong()),
                );
                ui.label(
                    RichText::new("Your changes will be lost if you don't save them.")
                        .color(theme::text_muted()),
                );
            });
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("Don't Save").clicked() {
                out = Some(Confirm::Discard);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(RichText::new("Save").color(theme::text_strong()))
                    .clicked()
                {
                    out = Some(Confirm::Save);
                }
                if ui.button("Cancel").clicked() {
                    out = Some(Confirm::Cancel);
                }
            });
        });
    });
    if r.should_close() && out.is_none() {
        out = Some(Confirm::Cancel);
    }
    out
}

/// The keyboard shortcut reference.
pub fn shortcuts(ctx: &egui::Context, open: &mut bool) {
    egui::Window::new(format!("{}  Keyboard shortcuts", icons::KEYBOARD))
        .open(open)
        .collapsible(false)
        .resizable(true)
        .default_width(520.0)
        .default_height(560.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("shortcut-table")
                    .striped(true)
                    .num_columns(2)
                    .show(ui, |ui| {
                        for a in Action::ALL {
                            let Some(sc) = a.shortcut() else { continue };
                            ui.label(a.label());
                            ui.label(
                                RichText::new(ctx.format_shortcut(&sc))
                                    .font(theme::mono_font(theme::FONT_VALUE))
                                    .color(theme::text_strong()),
                            );
                            ui.end_row();
                        }
                        for (what, how) in [
                            ("Pan", "Middle-drag · Space+drag · two-finger scroll"),
                            ("Zoom", "Wheel · pinch"),
                            ("Box select", "Drag on empty canvas (Shift adds)"),
                            ("Snap to grid", "Hold Ctrl while moving"),
                            ("Cut wires", "Ctrl+drag across them"),
                            ("Add reroute", "Shift+right-drag across a wire"),
                            ("Disconnect", "Drag a wire's end off its input"),
                            ("Insert on wire", "Drop an unwired node onto a wire"),
                            (
                                "Fine / coarse values",
                                "Ctrl / Shift while dragging a value",
                            ),
                            ("Type a value", "Click a value"),
                            ("Delete", "X or Delete"),
                            ("Move a panel card", "Drag it by its edge"),
                            ("Leave performance mode", "Esc or F11"),
                        ] {
                            ui.label(what);
                            ui.label(RichText::new(how).color(theme::text_muted()));
                            ui.end_row();
                        }
                    });
            });
        });
}

pub fn about(ctx: &egui::Context, open: &mut bool) {
    egui::Window::new("About Audio Constructor")
        .open(open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(RichText::new(format!("{}  Audio Constructor", icons::WAVEFORM)).size(theme::FONT_TITLE).color(theme::text_strong()));
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.add_space(6.0);
            ui.label("A node-based audio construction suite: build instruments, effects, recorders, and sequencers from small nodes and wires.");
            ui.add_space(6.0);
            ui.label(RichText::new("Fonts: Inter and JetBrains Mono (SIL Open Font License). Icons: Phosphor (MIT).").small().color(theme::text_muted()));
        });
}
