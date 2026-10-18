//! The menu bar: File, Edit, View, Add, Transport, Help. Every entry is an
//! [`Action`] and shows its shortcut.

use std::path::PathBuf;

use ac_core::{Category, Registry};
use egui::RichText;

use crate::actions::Action;
use crate::files::Recent;
use crate::icons;
use crate::palette::Candidate;
use crate::theme;

/// What a menu click asks for.
pub enum MenuCommand {
    Run(Action),
    OpenRecent(PathBuf),
    ForgetRecent(PathBuf),
    ClearRecent,
    Add(Candidate),
}

/// State the menus show (checkmarks, enabled entries, labels).
pub struct MenuState<'a> {
    pub undo: Option<&'a str>,
    pub redo: Option<&'a str>,
    pub has_selection: bool,
    pub side_panel: bool,
    pub timing: bool,
    /// The panels are shown instead of the canvas.
    pub panels: bool,
    pub playing: bool,
    pub looping: bool,
    pub play_keys: bool,
    pub recent: &'a Recent,
    pub registry: &'a Registry,
}

fn entry(ui: &mut egui::Ui, out: &mut Vec<MenuCommand>, action: Action, icon: &str, enabled: bool) {
    entry_labeled(ui, out, action, icon, action.label(), enabled);
}

fn entry_labeled(
    ui: &mut egui::Ui,
    out: &mut Vec<MenuCommand>,
    action: Action,
    icon: &str,
    label: &str,
    enabled: bool,
) {
    let mut b = egui::Button::new(format!("{icon}  {label}"));
    if let Some(sc) = action.shortcut() {
        b = b
            .shortcut_text(RichText::new(ui.ctx().format_shortcut(&sc)).color(theme::text_muted()));
    }
    if ui.add_enabled(enabled, b).clicked() {
        out.push(MenuCommand::Run(action));
        ui.close();
    }
}

fn toggle(ui: &mut egui::Ui, out: &mut Vec<MenuCommand>, action: Action, on: bool) {
    let icon = if on { icons::CHECK } else { "    " };
    entry(ui, out, action, icon, true);
}

pub fn show(ui: &mut egui::Ui, s: &MenuState) -> Vec<MenuCommand> {
    let mut out = Vec::new();
    egui::MenuBar::new().ui(ui, |ui| {
        ui.label(
            RichText::new(format!("{} ", icons::WAVEFORM))
                .size(17.0)
                .color(theme::type_color(ac_core::SignalType::Audio)),
        );
        ui.menu_button("File", |ui| {
            entry(ui, &mut out, Action::New, icons::FILE_PLUS, true);
            entry(ui, &mut out, Action::Open, icons::FOLDER_OPEN, true);
            ui.menu_button(
                format!("{}  Open Recent", icons::CLOCK_COUNTER_CLOCKWISE),
                |ui| {
                    if s.recent.entries.is_empty() {
                        ui.label(RichText::new("No recent projects").color(theme::text_muted()));
                    }
                    for p in &s.recent.entries {
                        let exists = p.is_file();
                        let name = p.file_name().map_or_else(
                            || p.display().to_string(),
                            |n| n.to_string_lossy().to_string(),
                        );
                        let label = if exists {
                            name
                        } else {
                            format!("{name} (missing)")
                        };
                        let r = ui
                            .add_enabled(exists, egui::Button::new(label))
                            .on_hover_text(p.display().to_string());
                        if r.clicked() {
                            out.push(MenuCommand::OpenRecent(p.clone()));
                            ui.close();
                        }
                        if !exists
                            && ui
                                .small_button(format!("{}  Remove from list", icons::X))
                                .clicked()
                        {
                            out.push(MenuCommand::ForgetRecent(p.clone()));
                        }
                    }
                    if !s.recent.entries.is_empty() {
                        ui.separator();
                        if ui.button("Clear list").clicked() {
                            out.push(MenuCommand::ClearRecent);
                            ui.close();
                        }
                    }
                },
            );
            ui.separator();
            entry(ui, &mut out, Action::Save, icons::FLOPPY_DISK, true);
            entry(ui, &mut out, Action::SaveAs, icons::FLOPPY_DISK, true);
            entry(ui, &mut out, Action::ExportWav, icons::EXPORT, true);
            entry(ui, &mut out, Action::ExportPack, icons::PACKAGE, true);
            ui.separator();
            entry(ui, &mut out, Action::Quit, icons::SIGN_OUT, true);
        });
        ui.menu_button("Edit", |ui| {
            let undo = s.undo.map_or("Undo".to_string(), |l| format!("Undo {l}"));
            let redo = s.redo.map_or("Redo".to_string(), |l| format!("Redo {l}"));
            entry_labeled(
                ui,
                &mut out,
                Action::Undo,
                icons::UNDO,
                &undo,
                s.undo.is_some(),
            );
            entry_labeled(
                ui,
                &mut out,
                Action::Redo,
                icons::REDO,
                &redo,
                s.redo.is_some(),
            );
            ui.separator();
            let sel = s.has_selection;
            entry(ui, &mut out, Action::Copy, icons::COPY, sel);
            entry(ui, &mut out, Action::Paste, icons::CLIPBOARD, true);
            entry(ui, &mut out, Action::Duplicate, icons::COPY, sel);
            entry(ui, &mut out, Action::DuplicateWithInputs, icons::COPY, sel);
            entry(ui, &mut out, Action::Delete, icons::TRASH, sel);
            entry(ui, &mut out, Action::DeleteReconnect, icons::SCISSORS, sel);
            ui.separator();
            entry(ui, &mut out, Action::SelectAll, icons::SELECTION, true);
            entry(ui, &mut out, Action::SelectNone, icons::SELECTION, sel);
            entry(ui, &mut out, Action::SelectLinked, icons::SELECTION, sel);
            ui.separator();
            entry(ui, &mut out, Action::Grab, icons::CROSSHAIR, sel);
            entry(ui, &mut out, Action::Rename, icons::PENCIL, sel);
            entry(ui, &mut out, Action::Mute, icons::SPEAKER_SLASH, sel);
            entry(ui, &mut out, Action::Collapse, icons::CARET_DOWN, sel);
            entry(ui, &mut out, Action::HideUnwired, icons::SELECTION, sel);
            ui.separator();
            entry(ui, &mut out, Action::Pin, icons::PUSH_PIN, sel && !s.panels);
        });
        ui.menu_button("View", |ui| {
            toggle(ui, &mut out, Action::ToggleSidePanel, s.side_panel);
            toggle(ui, &mut out, Action::ShowTiming, s.timing);
            ui.separator();
            toggle(ui, &mut out, Action::TogglePanels, s.panels);
            entry(
                ui,
                &mut out,
                Action::PerformanceMode,
                icons::CORNERS_OUT,
                true,
            );
            ui.separator();
            entry(ui, &mut out, Action::FitAll, icons::CORNERS_OUT, true);
            entry(ui, &mut out, Action::FitSelection, icons::CROSSHAIR, true);
            entry(ui, &mut out, Action::ZoomIn, icons::PLUS, true);
            entry(ui, &mut out, Action::ZoomOut, icons::MINUS, true);
            ui.separator();
            entry(
                ui,
                &mut out,
                Action::EnterLeaveGroup,
                icons::ARROW_SQUARE_IN,
                true,
            );
        });
        ui.menu_button("Add", |ui| {
            entry(ui, &mut out, Action::AddNode, icons::MAGNIFYING_GLASS, true);
            ui.separator();
            for cat in Category::ALL {
                let entries: Vec<_> = s
                    .registry
                    .iter()
                    .filter(|e| e.meta.category == cat && e.meta.kind != ac_core::GROUP_KIND)
                    .collect();
                if entries.is_empty() {
                    continue;
                }
                ui.menu_button(cat.name(), |ui| {
                    for e in entries {
                        if ui
                            .button(e.meta.name)
                            .on_hover_text(e.meta.summary())
                            .clicked()
                        {
                            out.push(MenuCommand::Add(Candidate::node(
                                e.meta.name,
                                e.meta.kind,
                                Some(e.meta.category),
                                e.meta.summary(),
                            )));
                            ui.close();
                        }
                    }
                });
            }
            ui.separator();
            entry(
                ui,
                &mut out,
                Action::Frame,
                icons::FRAME_CORNERS,
                s.has_selection,
            );
            entry(ui, &mut out, Action::Group, icons::STACK, s.has_selection);
            entry(ui, &mut out, Action::Ungroup, icons::STACK, s.has_selection);
            ui.label(
                RichText::new("Reroute: Shift+right-drag across a wire")
                    .small()
                    .color(theme::text_faint()),
            );
        });
        ui.menu_button("Transport", |ui| {
            let play = if s.playing { "Stop" } else { "Play" };
            entry_labeled(ui, &mut out, Action::PlayStop, icons::PLAY, play, true);
            entry(ui, &mut out, Action::PlayFromStart, icons::PLAY, true);
            entry(ui, &mut out, Action::Record, icons::RECORD, true);
            entry(ui, &mut out, Action::Rewind, icons::SKIP_BACK, true);
            toggle(ui, &mut out, Action::ToggleLoop, s.looping);
            ui.separator();
            toggle(ui, &mut out, Action::PlayKeys, s.play_keys);
        });
        ui.menu_button("Help", |ui| {
            entry(ui, &mut out, Action::Shortcuts, icons::KEYBOARD, true);
            entry(ui, &mut out, Action::NodeHelp, icons::BOOK_OPEN, true);
            ui.separator();
            entry(ui, &mut out, Action::About, icons::INFO, true);
        });
    });
    out
}
