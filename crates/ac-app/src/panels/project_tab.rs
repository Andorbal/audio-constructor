//! The Project tab: name, rates, length, seed, the tempo map, the loop
//! region, and the project's audio files (with missing ones to locate).

use ac_core::TempoSegment;
use ac_io::assets::{AssetIssue, referenced_assets};
use egui::RichText;

use super::{PanelIo, PanelRequest, section};
use crate::icons;
use crate::theme;

/// Blocks the project uses that the library has newer versions of.
fn blocks(ui: &mut egui::Ui, io: &mut PanelIo) {
    section(ui, "Blocks");
    let from_library = io
        .doc
        .project
        .group_defs
        .values()
        .filter(|d| d.source.is_some())
        .count();
    let updates = io.library.updates(&io.doc.project);
    if updates.is_empty() {
        let text = if from_library == 0 {
            "This project uses no library blocks."
        } else {
            "The library blocks this project uses are up to date."
        };
        ui.label(RichText::new(text).color(theme::TEXT_MUTED));
        return;
    }
    egui::Grid::new("project-block-updates")
        .num_columns(3)
        .spacing(egui::vec2(10.0, 6.0))
        .show(ui, |ui| {
            for u in &updates {
                let name = io
                    .doc
                    .project
                    .group_defs
                    .get(&u.def)
                    .map_or_else(|| u.def.0.clone(), |d| d.name.clone());
                ui.label(RichText::new(name).color(theme::TEXT));
                ui.label(
                    RichText::new(format!("v{} → v{}", u.from, u.to))
                        .font(theme::mono_font(theme::FONT_VALUE))
                        .color(theme::TEXT_MUTED),
                );
                if ui
                    .small_button(format!("{}  Update", icons::ARROWS_CLOCKWISE))
                    .on_hover_text(format!("Take the library's version ({})", u.source))
                    .clicked()
                {
                    io.requests.push(PanelRequest::UpdateBlock(u.def.clone()));
                }
                ui.end_row();
            }
        });
    if ui
        .button(format!("{}  Update all", icons::ARROWS_CLOCKWISE))
        .on_hover_text("Update every block above, as one undo step.")
        .clicked()
    {
        io.requests.push(PanelRequest::UpdateAllBlocks);
    }
}

pub(super) fn show(ui: &mut egui::Ui, io: &mut PanelIo) {
    let now = io.now;
    let s = io.doc.project.settings.clone();
    egui::Grid::new("project-settings")
        .num_columns(2)
        .spacing(egui::vec2(10.0, 6.0))
        .show(ui, |ui| {
            ui.label("Name");
            let mut name = io.doc.project.name.clone();
            if ui
                .add(egui::TextEdit::singleline(&mut name).hint_text(io.doc.title()))
                .changed()
            {
                io.doc.edit_settings("name", now, |p| p.name = name);
            }
            ui.end_row();

            ui.label("Sample rate");
            egui::ComboBox::from_id_salt("rate")
                .selected_text(format!("{} Hz", s.sample_rate))
                .show_ui(ui, |ui| {
                    for r in [44_100u32, 48_000, 88_200, 96_000, 192_000] {
                        if ui
                            .selectable_label(s.sample_rate == r, format!("{r} Hz"))
                            .clicked()
                        {
                            io.doc
                                .edit_settings("rate", now, |p| p.settings.sample_rate = r);
                            io.doc.end_coalescing();
                        }
                    }
                });
            ui.end_row();

            ui.label("Length");
            let mut len = s.length;
            if ui
                .add(
                    egui::DragValue::new(&mut len)
                        .range(0.1..=36_000.0)
                        .speed(0.1)
                        .suffix(" s"),
                )
                .changed()
            {
                io.doc
                    .edit_settings("length", now, |p| p.settings.length = len);
            }
            ui.end_row();

            ui.label("Render block");
            egui::ComboBox::from_id_salt("block")
                .selected_text(format!("{} frames", s.block_size))
                .show_ui(ui, |ui| {
                    for b in [32u32, 64, 128, 256, 512, 1024, 2048, 4096] {
                        if ui
                            .selectable_label(s.block_size == b, format!("{b} frames"))
                            .clicked()
                        {
                            io.doc
                                .edit_settings("block", now, |p| p.settings.block_size = b);
                            io.doc.end_coalescing();
                        }
                    }
                });
            ui.end_row();

            ui.label("Seed");
            let mut seed = s.seed;
            if ui.add(egui::DragValue::new(&mut seed).speed(1.0)).changed() {
                io.doc
                    .edit_settings("seed", now, |p| p.settings.seed = seed);
            }
            ui.end_row();
        });
    ui.label(
        RichText::new("Offline renders use the sample rate and block size; the live engine runs at the device's rate.")
            .small()
            .color(theme::TEXT_FAINT),
    );

    section(ui, "Tempo map");
    let mut tempo = s.tempo.clone();
    let mut changed = false;
    let mut remove = None;
    egui::Grid::new("tempo-map")
        .num_columns(4)
        .spacing(egui::vec2(8.0, 4.0))
        .show(ui, |ui| {
            ui.label(RichText::new("at beat").small().color(theme::TEXT_MUTED));
            ui.label(RichText::new("BPM").small().color(theme::TEXT_MUTED));
            ui.label(RichText::new("time sig").small().color(theme::TEXT_MUTED));
            ui.label("");
            ui.end_row();
            for (i, seg) in tempo.iter_mut().enumerate() {
                if i == 0 {
                    ui.label(RichText::new("0").font(theme::mono_font(theme::FONT_VALUE)));
                } else {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut seg.beat)
                                .range(0.0..=1.0e6)
                                .speed(0.25),
                        )
                        .changed();
                }
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut seg.bpm)
                            .range(1.0..=999.0)
                            .speed(0.1)
                            .fixed_decimals(1),
                    )
                    .changed();
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    changed |= ui
                        .add(egui::DragValue::new(&mut seg.time_sig[0]).range(1..=32))
                        .changed();
                    ui.label("/");
                    let mut den = seg.time_sig[1];
                    egui::ComboBox::from_id_salt(("den", i))
                        .width(44.0)
                        .selected_text(den.to_string())
                        .show_ui(ui, |ui| {
                            for d in [1u8, 2, 4, 8, 16, 32] {
                                if ui.selectable_value(&mut den, d, d.to_string()).clicked() {
                                    changed = true;
                                }
                            }
                        });
                    seg.time_sig[1] = den;
                });
                if i > 0
                    && ui
                        .small_button(icons::X)
                        .on_hover_text("Remove this tempo change")
                        .clicked()
                {
                    remove = Some(i);
                } else if i == 0 {
                    ui.label("");
                }
                ui.end_row();
            }
        });
    if let Some(i) = remove {
        tempo.remove(i);
        changed = true;
    }
    if ui
        .button(format!("{}  Add tempo change", icons::PLUS))
        .clicked()
    {
        let last = tempo.last().cloned().unwrap_or_default();
        tempo.push(TempoSegment {
            beat: last.beat + 16.0,
            ..last
        });
        changed = true;
    }
    if changed {
        io.doc
            .edit_settings("tempo", now, |p| p.settings.tempo = tempo);
    }

    section(ui, "Loop");
    let mut on = s.loop_region.is_some();
    let [mut a, mut b] = s.loop_region.unwrap_or([0.0, 16.0]);
    ui.horizontal(|ui| {
        let mut changed = ui.checkbox(&mut on, "Loop between beats").changed();
        ui.add_enabled_ui(on, |ui| {
            changed |= ui
                .add(egui::DragValue::new(&mut a).range(0.0..=1.0e6).speed(0.25))
                .changed();
            ui.label("–");
            changed |= ui
                .add(egui::DragValue::new(&mut b).range(0.0..=1.0e6).speed(0.25))
                .changed();
        });
        if changed {
            let region = on.then_some([a.min(b), a.max(b).max(a.min(b) + 0.25)]);
            io.doc
                .edit_settings("loop", now, |p| p.settings.loop_region = region);
        }
    });

    blocks(ui, io);

    section(ui, "Engine");
    let stats = io.engine.report.stats.clone();
    let rate = io.engine.info.sample_rate.max(1);
    egui::Grid::new("project-engine")
        .num_columns(2)
        .spacing(egui::vec2(10.0, 6.0))
        .show(ui, |ui| {
            ui.label("Output latency");
            ui.label(format!(
                "{} samples ({:.1} ms)",
                stats.latency,
                f64::from(stats.latency) * 1000.0 / f64::from(rate)
            ))
            .on_hover_text(
                "Delay that latency-reporting nodes add on the way to the outputs. Parallel \
                 paths are delayed to match where they meet.",
            );
            ui.end_row();
            if !stats.whole_input.is_empty() {
                let baked = stats
                    .whole_input
                    .iter()
                    .filter(|(_, s)| *s == ac_engine::WholeState::Baked)
                    .count();
                let disabled = stats
                    .whole_input
                    .iter()
                    .filter(|(_, s)| matches!(s, ac_engine::WholeState::Disabled { .. }))
                    .count();
                ui.label("Whole-input nodes");
                ui.label(format!(
                    "{baked} baked · {} baking · {disabled} disabled",
                    stats.whole_input.len() - baked - disabled
                ));
                ui.end_row();
            }
            ui.label("Schedule");
            ui.label(format!(
                "{} steps in {} waves · {} audio buffers",
                stats.steps, stats.waves, stats.sample_buffers
            ))
            .on_hover_text("Steps in one wave can run on different cores (Devices tab).");
            ui.end_row();
        });

    section(ui, "Audio files");
    let refs = referenced_assets(&io.doc.project, io.registry);
    if refs.is_empty() {
        ui.label(RichText::new("No audio files. Clip nodes add them.").color(theme::TEXT_MUTED));
    }
    for (path, _) in refs {
        let issue = io.assets.issues.iter().find(|i| match i {
            AssetIssue::Missing { path: p, .. } | AssetIssue::Changed { path: p } => *p == path,
        });
        ui.horizontal(|ui| {
            match issue {
                Some(AssetIssue::Missing { reason, .. }) => {
                    ui.colored_label(theme::ERROR, icons::FILE_X)
                        .on_hover_text(reason);
                }
                Some(AssetIssue::Changed { .. }) => {
                    ui.colored_label(theme::WARNING, icons::WARNING)
                        .on_hover_text(
                            "The file changed since it was added (its content hash differs).",
                        );
                }
                None => {
                    ui.colored_label(theme::OK, icons::FILE_AUDIO);
                }
            }
            ui.add(
                egui::Label::new(RichText::new(&path).font(theme::mono_font(theme::FONT_SMALL)))
                    .truncate(),
            );
            if issue.is_some() && ui.small_button("Locate…").clicked() {
                io.requests.push(PanelRequest::Locate(path.clone()));
            }
        });
    }
    if let Some(dir) = io.doc.path.as_ref().and_then(|p| p.parent()) {
        ui.label(
            RichText::new(format!("Relative paths resolve against {}", dir.display()))
                .small()
                .color(theme::TEXT_FAINT),
        );
    } else {
        ui.label(
            RichText::new("Save the project to copy its audio files into its assets folder.")
                .small()
                .color(theme::TEXT_FAINT),
        );
    }
}
