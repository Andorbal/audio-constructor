//! The Project tab: name, rates, length, seed, the tempo map, the loop
//! region, and the project's audio files (with missing ones to locate).

use ac_core::TempoSegment;
use ac_io::assets::{AssetIssue, referenced_assets};
use egui::RichText;

use super::{PanelIo, PanelRequest, section};
use crate::i18n::{tr, tr_args};
use crate::icons;
use crate::theme;

/// Blocks the project uses that the library has newer versions of.
fn blocks(ui: &mut egui::Ui, io: &mut PanelIo) {
    section(ui, &tr("section-blocks"));
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
            tr("project-no-blocks")
        } else {
            tr("project-blocks-current")
        };
        ui.label(RichText::new(text).color(theme::text_muted()));
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
                ui.label(RichText::new(name).color(theme::text()));
                ui.label(
                    RichText::new(format!("v{} → v{}", u.from, u.to))
                        .font(theme::mono_font(theme::FONT_VALUE))
                        .color(theme::text_muted()),
                );
                if ui
                    .small_button(format!(
                        "{}  {}",
                        icons::ARROWS_CLOCKWISE,
                        tr("project-update")
                    ))
                    .on_hover_text(tr_args(
                        "project-update-tip",
                        &[("source", u.source.to_string().into())],
                    ))
                    .clicked()
                {
                    io.requests.push(PanelRequest::UpdateBlock(u.def.clone()));
                }
                ui.end_row();
            }
        });
    if ui
        .button(format!(
            "{}  {}",
            icons::ARROWS_CLOCKWISE,
            tr("project-update-all")
        ))
        .on_hover_text(tr("project-update-all-tip"))
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
            let label = ui.label(tr("project-name"));
            let mut name = io.doc.project.name.clone();
            if ui
                .add(egui::TextEdit::singleline(&mut name).hint_text(io.doc.title()))
                .labelled_by(label.id)
                .changed()
            {
                io.doc.edit_settings("name", now, |p| p.name = name);
            }
            ui.end_row();

            let label = ui.label(tr("devices-sample-rate"));
            let hz = |r: u32| tr_args("devices-hz", &[("rate", r.to_string().into())]);
            egui::ComboBox::from_id_salt("rate")
                .selected_text(hz(s.sample_rate))
                .show_ui(ui, |ui| {
                    for r in [44_100u32, 48_000, 88_200, 96_000, 192_000] {
                        if ui.selectable_label(s.sample_rate == r, hz(r)).clicked() {
                            io.doc
                                .edit_settings("rate", now, |p| p.settings.sample_rate = r);
                            io.doc.end_coalescing();
                        }
                    }
                })
                .response
                .labelled_by(label.id);
            ui.end_row();

            let label = ui.label(tr("project-length"));
            let mut len = s.length;
            if ui
                .add(
                    egui::DragValue::new(&mut len)
                        .range(0.1..=36_000.0)
                        .speed(0.1)
                        .suffix(tr("project-seconds-suffix")),
                )
                .labelled_by(label.id)
                .changed()
            {
                io.doc
                    .edit_settings("length", now, |p| p.settings.length = len);
            }
            ui.end_row();

            let label = ui.label(tr("project-render-block"));
            let frames = |b: u32| tr_args("devices-frames", &[("frames", b.to_string().into())]);
            egui::ComboBox::from_id_salt("block")
                .selected_text(frames(s.block_size))
                .show_ui(ui, |ui| {
                    for b in [32u32, 64, 128, 256, 512, 1024, 2048, 4096] {
                        if ui.selectable_label(s.block_size == b, frames(b)).clicked() {
                            io.doc
                                .edit_settings("block", now, |p| p.settings.block_size = b);
                            io.doc.end_coalescing();
                        }
                    }
                })
                .response
                .labelled_by(label.id);
            ui.end_row();

            let label = ui.label(tr("project-seed"));
            let mut seed = s.seed;
            if ui
                .add(egui::DragValue::new(&mut seed).speed(1.0))
                .labelled_by(label.id)
                .changed()
            {
                io.doc
                    .edit_settings("seed", now, |p| p.settings.seed = seed);
            }
            ui.end_row();
        });
    ui.label(
        RichText::new(tr("project-rates-note"))
            .small()
            .color(theme::text_faint()),
    );

    section(ui, &tr("section-tempo-map"));
    let mut tempo = s.tempo.clone();
    let mut changed = false;
    let mut remove = None;
    egui::Grid::new("tempo-map")
        .num_columns(4)
        .spacing(egui::vec2(8.0, 4.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(tr("project-at-beat"))
                    .small()
                    .color(theme::text_muted()),
            );
            ui.label(
                RichText::new(tr("project-bpm"))
                    .small()
                    .color(theme::text_muted()),
            );
            ui.label(
                RichText::new(tr("project-time-sig"))
                    .small()
                    .color(theme::text_muted()),
            );
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
                        .accessible_name(tr_args(
                            "project-change-beat",
                            &[("number", (i + 1).into())],
                        ))
                        .changed();
                }
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut seg.bpm)
                            .range(1.0..=999.0)
                            .speed(0.1)
                            .fixed_decimals(1),
                    )
                    .accessible_name(tr_args("project-change-bpm", &[("number", (i + 1).into())]))
                    .changed();
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    changed |= ui
                        .add(egui::DragValue::new(&mut seg.time_sig[0]).range(1..=32))
                        .accessible_name(tr_args(
                            "project-change-beats",
                            &[("number", (i + 1).into())],
                        ))
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
                        })
                        .response
                        .accessible_name(tr_args(
                            "project-change-unit",
                            &[("number", (i + 1).into())],
                        ));
                    seg.time_sig[1] = den;
                });
                if i > 0
                    && ui
                        .small_button(icons::X)
                        .on_hover_text(tr("project-remove-tempo-change"))
                        .accessible_name(tr("project-remove-tempo-change"))
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
        .button(format!(
            "{}  {}",
            icons::PLUS,
            tr("project-add-tempo-change")
        ))
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

    section(ui, &tr("section-loop"));
    let mut on = s.loop_region.is_some();
    let [mut a, mut b] = s.loop_region.unwrap_or([0.0, 16.0]);
    ui.horizontal(|ui| {
        let mut changed = ui.checkbox(&mut on, tr("project-loop-between")).changed();
        ui.add_enabled_ui(on, |ui| {
            changed |= ui
                .add(egui::DragValue::new(&mut a).range(0.0..=1.0e6).speed(0.25))
                .accessible_name(tr("project-loop-start"))
                .changed();
            ui.label("–");
            changed |= ui
                .add(egui::DragValue::new(&mut b).range(0.0..=1.0e6).speed(0.25))
                .accessible_name(tr("project-loop-end"))
                .changed();
        });
        if changed {
            let region = on.then_some([a.min(b), a.max(b).max(a.min(b) + 0.25)]);
            io.doc
                .edit_settings("loop", now, |p| p.settings.loop_region = region);
        }
    });

    super::bindings_ui::project_section(ui, io);

    blocks(ui, io);

    section(ui, &tr("section-engine"));
    let stats = io.engine.report.stats.clone();
    let rate = io.engine.info.sample_rate.max(1);
    egui::Grid::new("project-engine")
        .num_columns(2)
        .spacing(egui::vec2(10.0, 6.0))
        .show(ui, |ui| {
            ui.label(tr("project-output-latency"));
            ui.label(tr_args(
                "project-latency",
                &[
                    ("samples", stats.latency.to_string().into()),
                    (
                        "ms",
                        format!("{:.1}", f64::from(stats.latency) * 1000.0 / f64::from(rate))
                            .into(),
                    ),
                ],
            ))
            .on_hover_text(tr("project-latency-tip"));
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
                ui.label(tr("project-whole-input"));
                ui.label(tr_args(
                    "project-whole-input-states",
                    &[
                        ("baked", baked.into()),
                        (
                            "baking",
                            (stats.whole_input.len() - baked - disabled).into(),
                        ),
                        ("disabled", disabled.into()),
                    ],
                ));
                ui.end_row();
            }
            ui.label(tr("project-schedule"));
            ui.label(tr_args(
                "project-schedule-stats",
                &[
                    ("steps", stats.steps.to_string().into()),
                    ("chains", stats.chains.to_string().into()),
                    ("waves", stats.waves.to_string().into()),
                    ("buffers", stats.sample_buffers.to_string().into()),
                ],
            ))
            .on_hover_text(tr("project-schedule-tip"));
            ui.end_row();
        });

    section(ui, &tr("section-audio-files"));
    let refs = referenced_assets(&io.doc.project, io.registry);
    if refs.is_empty() {
        ui.label(RichText::new(tr("project-no-audio-files")).color(theme::text_muted()));
    }
    for (path, _) in refs {
        let issue = io.assets.issues.iter().find(|i| match i {
            AssetIssue::Missing { path: p, .. } | AssetIssue::Changed { path: p } => *p == path,
        });
        ui.horizontal(|ui| {
            match issue {
                Some(AssetIssue::Missing { reason, .. }) => {
                    ui.colored_label(theme::error(), icons::FILE_X)
                        .on_hover_text(reason);
                }
                Some(AssetIssue::Changed { .. }) => {
                    ui.colored_label(theme::warning(), icons::WARNING)
                        .on_hover_text(tr("project-file-changed"));
                }
                None => {
                    ui.colored_label(theme::ok(), icons::FILE_AUDIO);
                }
            }
            ui.add(
                egui::Label::new(RichText::new(&path).font(theme::mono_font(theme::FONT_SMALL)))
                    .truncate(),
            );
            if issue.is_some() && ui.small_button(tr("node-tab-locate")).clicked() {
                io.requests.push(PanelRequest::Locate(path.clone()));
            }
        });
    }
    if let Some(dir) = io.doc.path.as_ref().and_then(|p| p.parent()) {
        ui.label(
            RichText::new(tr_args(
                "project-relative-paths",
                &[("folder", dir.display().to_string().into())],
            ))
            .small()
            .color(theme::text_faint()),
        );
    } else {
        ui.label(
            RichText::new(tr("project-save-to-copy"))
                .small()
                .color(theme::text_faint()),
        );
    }
}
