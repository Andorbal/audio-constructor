//! The menu bar: File, Edit, View, Add, Transport, Help. Every entry is an
//! [`Action`] and shows its shortcut.

use std::path::PathBuf;

use ac_core::{Category, Registry};
use egui::RichText;
use egui::accesskit::{Role, Toggled};

use crate::actions::Action;
use crate::files::Recent;
use crate::help::Example;
use crate::i18n::{self, Arg, LanguageChoice, tr, tr_args};
use crate::icons;
use crate::palette::Candidate;
use crate::settings::UI_SCALES;
use crate::theme::{self, ThemeChoice};
use crate::user_themes::UserThemes;

/// What a menu click asks for.
pub enum MenuCommand {
    Run(Action),
    OpenRecent(PathBuf),
    ForgetRecent(PathBuf),
    ClearRecent,
    /// File ▸ Examples: open a shipped example as an untitled copy.
    OpenExample(PathBuf),
    Add(Candidate),
    /// View ▸ Theme.
    SetTheme(ThemeChoice),
    /// View ▸ Theme ▸ Reload Themes: read the themes folder again.
    ReloadThemes,
    /// View ▸ Theme ▸ Open Themes Folder.
    OpenThemesFolder,
    /// View ▸ Language.
    SetLanguage(LanguageChoice),
    /// View ▸ UI Scale (1 is 100 %).
    SetUiScale(f32),
    /// Help ▸ Check for Updates…: ask now.
    CheckForUpdates,
    /// Help ▸ Check for Updates Daily.
    SetUpdateChecks(bool),
    /// Help ▸ Report a Problem…: the issue form in the browser.
    ReportProblem,
}

/// State the menus show (checkmarks, enabled entries, labels).
pub struct MenuState<'a> {
    pub undo: Option<&'a str>,
    pub redo: Option<&'a str>,
    pub has_selection: bool,
    pub side_panel: bool,
    pub timing: bool,
    pub minimap: bool,
    /// View ▸ Reduce Motion is on.
    pub reduce_motion: bool,
    /// The panels are shown instead of the canvas.
    pub panels: bool,
    pub playing: bool,
    pub looping: bool,
    pub play_keys: bool,
    pub theme: &'a ThemeChoice,
    /// The person's own themes, listed after the built-in ones.
    pub themes: &'a UserThemes,
    pub language: LanguageChoice,
    pub ui_scale: f32,
    pub recent: &'a Recent,
    /// The shipped examples, by name.
    pub examples: &'a [Example],
    pub registry: &'a Registry,
    /// File ▸ Quit is offered (not in a plugin, whose host closes it).
    pub can_quit: bool,
    /// Help's update entries, with whether daily checks are on; `None` where
    /// the editor doesn't check for updates (a plugin, the browser).
    pub updates: Option<bool>,
    /// File ▸ Open Recent is offered (not where the host keeps the files).
    pub show_recent: bool,
}

fn entry(
    ui: &mut egui::Ui,
    out: &mut Vec<MenuCommand>,
    action: Action,
    icon: &str,
    enabled: bool,
) -> egui::Response {
    entry_labeled(ui, out, action, icon, &action.label(), enabled)
}

/// A submenu whose label may start with an icon, which screen readers
/// leave out.
fn submenu(ui: &mut egui::Ui, label: String, add: impl FnOnce(&mut egui::Ui)) {
    icons::spoken(ui.menu_button(label, add).response);
}

/// An entry with a check mark when on: checked or not to screen readers
/// (one of several choices for `radio`), rather than a check mark's code
/// point.
fn checked(r: &egui::Response, on: bool, radio: bool) {
    r.ctx.accesskit_node_builder(r.id, |n| {
        n.set_role(if radio {
            Role::MenuItemRadio
        } else {
            Role::MenuItemCheckBox
        });
        n.set_toggled(if on { Toggled::True } else { Toggled::False });
    });
}

fn entry_labeled(
    ui: &mut egui::Ui,
    out: &mut Vec<MenuCommand>,
    action: Action,
    icon: &str,
    label: &str,
    enabled: bool,
) -> egui::Response {
    let mut b = egui::Button::new(format!("{icon}  {label}"));
    if let Some(sc) = action.shortcut() {
        b = b
            .shortcut_text(RichText::new(ui.ctx().format_shortcut(&sc)).color(theme::text_muted()));
    }
    let r = icons::spoken(ui.add_enabled(enabled, b));
    if r.clicked() {
        out.push(MenuCommand::Run(action));
        ui.close();
    }
    r
}

fn toggle(ui: &mut egui::Ui, out: &mut Vec<MenuCommand>, action: Action, on: bool) {
    let icon = if on { icons::CHECK } else { "    " };
    let r = entry(ui, out, action, icon, true);
    checked(&r, on, false);
}

/// View ▸ Theme: the built-in themes, the person's own (warning about
/// any whose text is hard to read, or whose file had problems), files that
/// couldn't be used, and the folder they live in.
fn theme_menu(ui: &mut egui::Ui, out: &mut Vec<MenuCommand>, s: &MenuState) {
    let mut item = |ui: &mut egui::Ui, choice: &ThemeChoice, note: Option<String>| {
        let icon = if s.theme == choice {
            icons::CHECK
        } else {
            "    "
        };
        let mut r = icons::spoken(ui.button(format!("{icon}  {}", choice.label())));
        checked(&r, s.theme == choice, true);
        if let Some(note) = note {
            r = r.on_hover_text(note);
        }
        if r.clicked() {
            out.push(MenuCommand::SetTheme(choice.clone()));
            ui.close();
        }
    };
    for t in &ThemeChoice::BUILT_IN {
        item(ui, t, None);
    }
    if !s.themes.themes.is_empty() {
        ui.separator();
    }
    for t in &s.themes.themes {
        let choice = ThemeChoice::User(t.name.clone());
        let mut notes = Vec::new();
        if t.low_contrast() {
            notes.push(tr_args(
                "menu-theme-low-contrast-tip",
                &[
                    ("ratio", format!("{:.1}", t.text_contrast.0).into()),
                    ("where", Arg::from(&t.text_contrast.1)),
                ],
            ));
        }
        notes.extend(t.notes.iter().cloned());
        let note = (!notes.is_empty()).then(|| format!("{}\n{}", t.file, notes.join("\n")));
        item(ui, &choice, note);
        if t.low_contrast() {
            ui.label(
                RichText::new(format!(
                    "{}  {}",
                    icons::WARNING,
                    tr_args(
                        "menu-theme-low-contrast",
                        &[("ratio", format!("{:.1}", t.text_contrast.0).into())]
                    )
                ))
                .small()
                .color(theme::warning()),
            );
        }
    }
    for p in &s.themes.problems {
        ui.label(
            RichText::new(format!("{}  {p}", icons::WARNING))
                .small()
                .color(theme::warning()),
        );
    }
    ui.separator();
    if s.themes.dir.is_some() {
        if ui
            .button(format!(
                "{}  {}",
                icons::FOLDER_OPEN,
                tr("menu-open-themes-folder")
            ))
            .clicked()
        {
            out.push(MenuCommand::OpenThemesFolder);
            ui.close();
        }
        if ui
            .button(format!(
                "{}  {}",
                icons::ARROWS_CLOCKWISE,
                tr("menu-reload-themes")
            ))
            .clicked()
        {
            out.push(MenuCommand::ReloadThemes);
            ui.close();
        }
    }
}

/// File ▸ Open Recent.
fn recent_menu(ui: &mut egui::Ui, s: &MenuState, out: &mut Vec<MenuCommand>) {
    submenu(
        ui,
        format!(
            "{}  {}",
            icons::CLOCK_COUNTER_CLOCKWISE,
            tr("menu-open-recent")
        ),
        |ui| {
            if s.recent.entries.is_empty() {
                ui.label(RichText::new(tr("menu-no-recent")).color(theme::text_muted()));
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
                    tr_args("menu-recent-missing", &[("name", name.into())])
                };
                let r = ui
                    .add_enabled(exists, egui::Button::new(label))
                    .on_hover_text(p.display().to_string());
                if r.clicked() {
                    out.push(MenuCommand::OpenRecent(p.clone()));
                    ui.close();
                }
                if !exists
                    && icons::spoken(ui.small_button(format!(
                        "{}  {}",
                        icons::X,
                        tr("menu-remove-from-list")
                    )))
                    .clicked()
                {
                    out.push(MenuCommand::ForgetRecent(p.clone()));
                }
            }
            if !s.recent.entries.is_empty() {
                ui.separator();
                if ui.button(tr("menu-clear-list")).clicked() {
                    out.push(MenuCommand::ClearRecent);
                    ui.close();
                }
            }
        },
    );
}

pub fn show(ui: &mut egui::Ui, s: &MenuState) -> Vec<MenuCommand> {
    let mut out = Vec::new();
    egui::MenuBar::new().ui(ui, |ui| {
        let logo = ui.label(
            RichText::new(format!("{} ", icons::WAVEFORM))
                .size(17.0)
                .color(theme::type_color(ac_core::SignalType::Audio)),
        );
        icons::picture(&logo, "Audio Constructor");
        submenu(ui, tr("menu-file"), |ui| {
            entry(ui, &mut out, Action::New, icons::FILE_PLUS, true);
            entry(ui, &mut out, Action::Open, icons::FOLDER_OPEN, true);
            entry(ui, &mut out, Action::ImportFromLink, icons::LINK, true);
            if s.show_recent {
                recent_menu(ui, s, &mut out);
            }
            submenu(
                ui,
                format!("{}  {}", icons::BOOK_OPEN, tr("menu-examples")),
                |ui| {
                    if s.examples.is_empty() {
                        ui.label(RichText::new(tr("menu-no-examples")).color(theme::text_muted()));
                    }
                    for ex in s.examples {
                        let r = ui.button(&ex.name).on_hover_text(tr("menu-example-tip"));
                        if r.clicked() {
                            out.push(MenuCommand::OpenExample(ex.path.clone()));
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
            if s.can_quit {
                ui.separator();
                entry(ui, &mut out, Action::Quit, icons::SIGN_OUT, true);
            }
        });
        submenu(ui, tr("menu-edit"), |ui| {
            let undo = s.undo.map_or_else(
                || Action::Undo.label(),
                |l| tr_args("menu-undo-named", &[("what", l.into())]),
            );
            let redo = s.redo.map_or_else(
                || Action::Redo.label(),
                |l| tr_args("menu-redo-named", &[("what", l.into())]),
            );
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
            entry(
                ui,
                &mut out,
                Action::Find,
                icons::MAGNIFYING_GLASS,
                !s.panels,
            );
            ui.separator();
            entry(ui, &mut out, Action::Grab, icons::CROSSHAIR, sel);
            entry(ui, &mut out, Action::Rename, icons::PENCIL, sel);
            entry(ui, &mut out, Action::Mute, icons::SPEAKER_SLASH, sel);
            entry(ui, &mut out, Action::Collapse, icons::CARET_DOWN, sel);
            entry(ui, &mut out, Action::HideUnwired, icons::SELECTION, sel);
            entry(ui, &mut out, Action::Portal, icons::TAG, sel);
            ui.separator();
            entry(
                ui,
                &mut out,
                Action::LazyConnect,
                icons::PLUGS_CONNECTED,
                sel && !s.panels,
            );
            entry(
                ui,
                &mut out,
                Action::AutoLayout,
                icons::FLOW_ARROW,
                !s.panels,
            );
            ui.add_enabled_ui(sel && !s.panels, |ui| {
                submenu(
                    ui,
                    format!("{}  {}", icons::ALIGN_LEFT, Action::ArrangeMenu.label()),
                    |ui| {
                        if let Some(sc) = Action::ArrangeMenu.shortcut() {
                            ui.label(
                                RichText::new(tr_args(
                                    "menu-arrange-at-pointer",
                                    &[("shortcut", ui.ctx().format_shortcut(&sc).into())],
                                ))
                                .small()
                                .color(theme::text_faint()),
                            );
                        }
                        for (i, (icon, a)) in crate::canvas::arrange_items()
                            .into_iter()
                            .enumerate()
                            .skip(1)
                        {
                            if i == 7 {
                                ui.separator();
                            }
                            entry(ui, &mut out, a, icon, true);
                        }
                    },
                );
            });
            ui.separator();
            entry(ui, &mut out, Action::Pin, icons::PUSH_PIN, sel && !s.panels);
        });
        submenu(ui, tr("menu-view"), |ui| {
            toggle(ui, &mut out, Action::ToggleSidePanel, s.side_panel);
            toggle(ui, &mut out, Action::ShowTiming, s.timing);
            toggle(ui, &mut out, Action::Minimap, s.minimap);
            toggle(ui, &mut out, Action::ReduceMotion, s.reduce_motion);
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
            submenu(
                ui,
                format!("{}  {}", icons::CIRCLE_HALF, tr("menu-theme")),
                |ui| {
                    theme_menu(ui, &mut out, s);
                },
            );
            submenu(
                ui,
                format!("{}  {}", icons::GLOBE, tr("menu-language")),
                |ui| {
                    for l in LanguageChoice::ALL {
                        let icon = if s.language == l {
                            icons::CHECK
                        } else {
                            "    "
                        };
                        let r = icons::spoken(ui.button(format!("{icon}  {}", l.label())));
                        checked(&r, s.language == l, true);
                        if r.clicked() {
                            out.push(MenuCommand::SetLanguage(l));
                            ui.close();
                        }
                    }
                },
            );
            submenu(
                ui,
                format!("{}  {}", icons::TEXT_AA, tr("menu-ui-scale")),
                |ui| {
                    for step in UI_SCALES {
                        let on = (s.ui_scale - step).abs() < 1e-3;
                        let icon = if on { icons::CHECK } else { "    " };
                        let r = icons::spoken(ui.button(format!(
                            "{icon}  {}",
                            tr_args(
                                "menu-ui-scale-step",
                                &[("percent", format!("{:.0}", step * 100.0).into())]
                            )
                        )));
                        checked(&r, on, true);
                        if r.clicked() {
                            out.push(MenuCommand::SetUiScale(step));
                            ui.close();
                        }
                    }
                    ui.separator();
                    entry(ui, &mut out, Action::UiScaleUp, icons::PLUS, true);
                    entry(ui, &mut out, Action::UiScaleDown, icons::MINUS, true);
                    entry(ui, &mut out, Action::UiScaleReset, "    ", true);
                },
            );
            ui.separator();
            entry(
                ui,
                &mut out,
                Action::EnterLeaveGroup,
                icons::ARROW_SQUARE_IN,
                true,
            );
        });
        submenu(ui, tr("menu-add"), |ui| {
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
                submenu(ui, i18n::category(cat), |ui| {
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
                RichText::new(tr("menu-reroute-hint"))
                    .small()
                    .color(theme::text_faint()),
            );
        });
        submenu(ui, tr("menu-transport"), |ui| {
            let play = if s.playing {
                tr("menu-stop")
            } else {
                tr("menu-play")
            };
            entry_labeled(ui, &mut out, Action::PlayStop, icons::PLAY, &play, true);
            entry(ui, &mut out, Action::Stop, icons::STOP, true);
            entry(ui, &mut out, Action::PlayFromStart, icons::PLAY, true);
            entry(ui, &mut out, Action::Record, icons::RECORD, true);
            entry(ui, &mut out, Action::Rewind, icons::SKIP_BACK, true);
            toggle(ui, &mut out, Action::ToggleLoop, s.looping);
            ui.separator();
            entry(ui, &mut out, Action::Panic, icons::WARNING_OCTAGON, true);
            ui.separator();
            toggle(ui, &mut out, Action::PlayKeys, s.play_keys);
        });
        submenu(ui, tr("menu-help"), |ui| {
            entry(ui, &mut out, Action::Shortcuts, icons::KEYBOARD, true);
            entry(ui, &mut out, Action::NodeHelp, icons::BOOK_OPEN, true);
            if let Some(daily) = s.updates {
                ui.separator();
                if icons::spoken(ui.button(format!(
                    "{}  {}",
                    icons::ARROWS_CLOCKWISE,
                    tr("menu-check-for-updates")
                )))
                .clicked()
                {
                    out.push(MenuCommand::CheckForUpdates);
                    ui.close();
                }
                let mut on = daily;
                if ui
                    .checkbox(&mut on, tr("menu-check-for-updates-daily"))
                    .clicked()
                {
                    out.push(MenuCommand::SetUpdateChecks(on));
                    ui.close();
                }
                if icons::spoken(ui.button(format!(
                    "{}  {}",
                    icons::ARROW_SQUARE_OUT,
                    tr("menu-report-problem")
                )))
                .clicked()
                {
                    out.push(MenuCommand::ReportProblem);
                    ui.close();
                }
            }
            ui.separator();
            entry(ui, &mut out, Action::About, icons::INFO, true);
        });
    });
    out
}
