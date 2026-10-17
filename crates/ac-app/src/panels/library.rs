//! The Library tab: every node by category, and the groups in this
//! project, searchable with the palette's engine. Click to add at the
//! center of the view, or drag onto the canvas.

use ac_core::{Category, GROUP_KIND};
use egui::{Align2, RichText, Sense, vec2};

use super::{PanelIo, PanelRequest, section};
use crate::icons;
use crate::palette::{self, Candidate};
use crate::theme;

/// The drag-and-drop payload of a library row.
pub type LibraryItem = Candidate;

pub(super) fn show(ui: &mut egui::Ui, io: &mut PanelIo, query: &mut String) {
    ui.add(
        egui::TextEdit::singleline(query)
            .hint_text(format!("{}  Search the library", icons::MAGNIFYING_GLASS))
            .desired_width(f32::INFINITY)
            .margin(egui::Margin::symmetric(8, 5)),
    );
    ui.add_space(4.0);
    let all = palette::candidates(io.registry, io.doc, query, None);
    if !query.trim().is_empty() {
        if all.is_empty() {
            ui.label(RichText::new("Nothing matches.").color(theme::TEXT_MUTED));
        }
        for c in &all {
            item(ui, io, c);
        }
        return;
    }
    for cat in Category::ALL {
        let items: Vec<&Candidate> = all.iter().filter(|c| c.category == Some(cat)).collect();
        if items.is_empty() {
            continue;
        }
        let header = RichText::new(format!("{}  ({})", cat.name(), items.len())).color(theme::TEXT);
        egui::CollapsingHeader::new(header)
            .id_salt(("library", cat.slug()))
            .default_open(false)
            .show(ui, |ui| {
                for c in items {
                    item(ui, io, c);
                }
            });
    }
    let groups: Vec<&Candidate> = all.iter().filter(|c| c.kind == GROUP_KIND).collect();
    if !groups.is_empty() {
        section(ui, "Groups in this project");
        for c in groups {
            item(ui, io, c);
        }
    }
    ui.add_space(8.0);
    ui.label(
        RichText::new("Drag a node onto the canvas, or click to add it in the middle.")
            .small()
            .color(theme::TEXT_FAINT),
    );
}

fn item(ui: &mut egui::Ui, io: &mut PanelIo, c: &Candidate) {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 38.0), Sense::click_and_drag());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(
            rect,
            theme::radius(5.0),
            egui::Color32::from_rgb(0x2B, 0x2F, 0x37),
        );
    }
    let color = c
        .category
        .map_or(theme::GROUP_HEADER, theme::category_color);
    p.rect_filled(
        egui::Rect::from_min_size(rect.min + vec2(4.0, 7.0), vec2(3.0, rect.height() - 14.0)),
        theme::radius(1.5),
        color,
    );
    let icon = if c.kind == GROUP_KIND {
        format!("{}  ", icons::STACK)
    } else {
        String::new()
    };
    p.text(
        rect.min + vec2(14.0, 5.0),
        Align2::LEFT_TOP,
        format!("{icon}{}", c.name),
        theme::ui_font(theme::FONT_UI),
        theme::TEXT_STRONG,
    );
    let mut job = egui::text::LayoutJob::single_section(
        c.summary.clone(),
        egui::TextFormat::simple(theme::ui_font(theme::FONT_SMALL), theme::TEXT_MUTED),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width - 20.0);
    let g = p.layout_job(job);
    p.galley(rect.min + vec2(14.0, 21.0), g, theme::TEXT_MUTED);
    resp.dnd_set_drag_payload(c.clone());
    if resp.clicked() {
        io.requests.push(PanelRequest::Add(c.clone()));
    }
    resp.on_hover_text(&c.summary);
}
