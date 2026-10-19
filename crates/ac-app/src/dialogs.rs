//! Modal dialogs: unsaved-changes confirmation, typing a file path (when
//! the system has no file dialog), keyboard shortcuts, and About.

use std::path::PathBuf;

use egui::{Key, RichText};

use crate::actions::Action;
use crate::i18n::{tr, tr_args};
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
    fn title(&self) -> String {
        match self {
            FilePurpose::Open => tr("dialog-open-title"),
            FilePurpose::SaveAs => tr("dialog-save-as-title"),
            FilePurpose::Export => tr("dialog-export-title"),
            FilePurpose::ChooseClip(_) => tr("dialog-choose-clip-title"),
            FilePurpose::Locate(_) => tr("dialog-locate-title"),
            FilePurpose::ExportPack => tr("dialog-export-pack-title"),
            FilePurpose::ImportBlock => tr("dialog-import-block-title"),
        }
    }

    #[cfg(feature = "native-dialogs")]
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
#[cfg(feature = "native-dialogs")]
pub fn native(
    purpose: &FilePurpose,
    suggested: Option<&std::path::Path>,
) -> Result<Option<PathBuf>, NoDialog> {
    let mut d = rfd::FileDialog::new().set_title(purpose.title());
    d = match purpose {
        FilePurpose::Open => d
            .add_filter(
                tr("dialog-filter-everything"),
                &["acproj", "acpack", "acblock"],
            )
            .add_filter(tr("dialog-filter-project"), &["acproj"])
            .add_filter(tr("dialog-filter-pack"), &["acpack"])
            .add_filter(tr("dialog-filter-block"), &["acblock"]),
        FilePurpose::SaveAs => d.add_filter(tr("dialog-filter-project"), &["acproj"]),
        FilePurpose::Export => d.add_filter(tr("dialog-filter-wav"), &["wav"]),
        FilePurpose::ExportPack => d.add_filter(tr("dialog-filter-pack"), &["acpack"]),
        FilePurpose::ImportBlock => d.add_filter(tr("dialog-filter-block"), &["acblock"]),
        FilePurpose::ChooseClip(_) | FilePurpose::Locate(_) => {
            d.add_filter(tr("dialog-filter-wav"), &["wav", "wave"])
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
    let t0 = std::time::Instant::now();
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

/// A build without the system's file dialogs (`native-dialogs` off) has
/// none to show: the caller asks for a typed path.
#[cfg(not(feature = "native-dialogs"))]
pub fn native(
    _purpose: &FilePurpose,
    _suggested: Option<&std::path::Path>,
) -> Result<Option<PathBuf>, NoDialog> {
    Err(NoDialog)
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
            let hint = ui.label(RichText::new(tr("dialog-type-path")).color(theme::text_muted()));
            ui.add_space(6.0);
            let r = ui
                .add(
                    egui::TextEdit::singleline(&mut self.text)
                        .desired_width(f32::INFINITY)
                        .font(theme::mono_font(theme::FONT_VALUE)),
                )
                .labelled_by(hint.id);
            if self.focus {
                r.request_focus();
                self.focus = false;
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tr("dialog-cancel")).clicked()
                    || ui.input(|i| i.key_pressed(Key::Escape))
                {
                    result = Some(None);
                }
                let ok = ui.add_enabled(
                    !self.text.trim().is_empty(),
                    egui::Button::new(tr("dialog-ok")),
                );
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
            let icon = ui.label(
                RichText::new(icons::WARNING)
                    .size(24.0)
                    .color(theme::warning()),
            );
            icons::decoration(&icon);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(tr_args("dialog-save-changes", &[("project", title.into())]))
                        .size(theme::FONT_HEADING)
                        .color(theme::text_strong()),
                );
                ui.label(RichText::new(tr("dialog-changes-lost")).color(theme::text_muted()));
            });
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button(tr("dialog-dont-save")).clicked() {
                out = Some(Confirm::Discard);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(RichText::new(tr("dialog-save")).color(theme::text_strong()))
                    .clicked()
                {
                    out = Some(Confirm::Save);
                }
                if ui.button(tr("dialog-cancel")).clicked() {
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
    egui::Window::new(format!(
        "{}  {}",
        icons::KEYBOARD,
        tr("dialog-shortcuts-title")
    ))
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
                        (tr("gesture-pan"), tr("gesture-pan-how")),
                        (tr("gesture-zoom"), tr("gesture-zoom-how")),
                        (tr("gesture-box-select"), tr("gesture-box-select-how")),
                        (tr("gesture-snap"), tr("gesture-snap-how")),
                        (tr("gesture-cut"), tr("gesture-cut-how")),
                        (tr("gesture-reroute"), tr("gesture-reroute-how")),
                        (tr("gesture-disconnect"), tr("gesture-disconnect-how")),
                        (tr("gesture-insert"), tr("gesture-insert-how")),
                        (tr("gesture-wire-into-new"), tr("gesture-wire-into-new-how")),
                        (tr("gesture-fine-coarse"), tr("gesture-fine-coarse-how")),
                        (tr("gesture-type-value"), tr("gesture-type-value-how")),
                        (tr("gesture-delete"), tr("gesture-delete-how")),
                        (tr("gesture-move-card"), tr("gesture-move-card-how")),
                        (
                            tr("gesture-leave-performance"),
                            tr("gesture-leave-performance-how"),
                        ),
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
    egui::Window::new(Action::About.label())
        .open(open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(
                RichText::new(format!("{}  Audio Constructor", icons::WAVEFORM))
                    .size(theme::FONT_TITLE)
                    .color(theme::text_strong()),
            );
            ui.label(tr_args(
                "dialog-about-version",
                &[("version", env!("CARGO_PKG_VERSION").into())],
            ));
            ui.add_space(6.0);
            ui.label(tr("dialog-about-text"));
            ui.add_space(6.0);
            ui.label(
                RichText::new(tr("dialog-about-credits"))
                    .small()
                    .color(theme::text_muted()),
            );
        });
}
