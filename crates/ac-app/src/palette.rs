//! The add-node palette (`Shift+A`, double-click, or dropping a wire on
//! empty canvas): fuzzy search with synonyms, keyboard driven, filtered to
//! nodes that can complete a dragged wire.

use ac_core::{Category, Config, GROUP_KIND, GroupId, Registry, SignalType};
use egui::{Align2, Color32, Key, Pos2, RichText, Sense, vec2};

use crate::canvas::PendingWire;
use crate::document::Document;
use crate::edit;
use crate::icons;
use crate::theme;

/// One thing the palette can add.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub name: String,
    pub kind: String,
    pub category: Option<Category>,
    pub summary: String,
    pub config: Config,
    /// The socket to auto-wire when completing a pending wire.
    pub socket: Option<String>,
}

#[derive(Default)]
pub struct Palette {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    /// Where the palette appears (screen) and where the node goes (canvas).
    pub screen: Pos2,
    pub canvas: Pos2,
    pub pending: Option<PendingWire>,
    focus: bool,
}

/// What the palette offers for `query`, best first.
pub fn candidates(
    registry: &Registry,
    doc: &Document,
    query: &str,
    pending: Option<&PendingWire>,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    for e in registry.search(query) {
        if e.meta.kind == GROUP_KIND {
            continue;
        }
        let config = ac_core::empty_config();
        let sig = (e.describe)(&config);
        let socket = match pending {
            Some(p) => match pick_socket(&sig, p) {
                Some(s) => Some(s),
                None => continue,
            },
            None => None,
        };
        out.push(Candidate {
            name: e.meta.name.to_string(),
            kind: e.meta.kind.to_string(),
            category: Some(e.meta.category),
            summary: e.meta.summary(),
            config,
            socket,
        });
    }
    // Group definitions in this project, except ones that would contain
    // the group being edited.
    let q = query.trim().to_lowercase();
    for (id, def) in &doc.project.group_defs {
        if let Some(scope) = doc.current_def()
            && (id == scope || edit::group::def_contains(&doc.project.group_defs, id, scope))
        {
            continue;
        }
        let name = def.name.to_lowercase();
        if !q.is_empty() && !name.contains(&q) && !"group".starts_with(&q) {
            continue;
        }
        let sig = ac_core::group_signature(def);
        let socket = match pending {
            Some(p) => match pick_socket(&sig, p) {
                Some(s) => Some(s),
                None => continue,
            },
            None => None,
        };
        out.push(Candidate {
            name: def.name.clone(),
            kind: GROUP_KIND.to_string(),
            category: None,
            summary: if def.description.is_empty() {
                format!("An instance of the group “{}”.", def.name)
            } else {
                def.description.clone()
            },
            config: serde_json::json!({ "def": GroupId::clone(id).0 }),
            socket,
        });
    }
    out
}

/// The socket of a new node that completes `p`: for a wire from an output,
/// the first input that takes its type (an exact match first); for a wire
/// from an input, the first output that feeds it.
pub fn pick_socket(sig: &ac_core::Signature, p: &PendingWire) -> Option<String> {
    let ty = p.ty;
    if p.from.is_some() {
        let ins = &sig.inputs;
        ins.iter()
            .find(|s| s.ty == ty && ty != SignalType::Any)
            .or_else(|| ins.iter().find(|s| edit::compatible(ty, s.ty)))
            .map(|s| s.name.to_string())
    } else {
        let outs = &sig.outputs;
        outs.iter()
            .find(|s| s.ty == ty && ty != SignalType::Any)
            .or_else(|| outs.iter().find(|s| edit::compatible(s.ty, ty)))
            .map(|s| s.name.to_string())
    }
}

impl Palette {
    pub fn open_at(&mut self, screen: Pos2, canvas: Pos2, pending: Option<PendingWire>) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.screen = screen;
        self.canvas = canvas;
        self.pending = pending;
        self.focus = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.pending = None;
    }

    /// Show the palette; returns the chosen candidate.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        registry: &Registry,
        doc: &Document,
    ) -> Option<Candidate> {
        if !self.open {
            return None;
        }
        let list = candidates(registry, doc, &self.query, self.pending.as_ref());
        if self.selected >= list.len() {
            self.selected = list.len().saturating_sub(1);
        }
        let (up, down, enter, escape) = ctx.input(|i| {
            (
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Escape),
            )
        });
        if escape {
            self.close();
            return None;
        }
        if down && !list.is_empty() {
            self.selected = (self.selected + 1) % list.len();
        }
        if up && !list.is_empty() {
            self.selected = (self.selected + list.len() - 1) % list.len();
        }
        let mut chosen = None;
        if enter && let Some(c) = list.get(self.selected) {
            chosen = Some(c.clone());
        }

        let width = 360.0;
        let area = egui::Area::new(egui::Id::unique("add-node-palette"))
            .order(egui::Order::Foreground)
            .fixed_pos(self.screen - vec2(24.0, 18.0))
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin::same(8))
                    .corner_radius(theme::radius(10.0))
                    .show(ui, |ui| {
                        ui.set_width(width);
                        if let Some(p) = &self.pending {
                            ui.horizontal(|ui| {
                                let (r, _) =
                                    ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                                ui.painter().circle_filled(
                                    r.center(),
                                    4.5,
                                    theme::type_color(p.ty),
                                );
                                let side = if p.from.is_some() { "takes" } else { "feeds" };
                                ui.label(
                                    RichText::new(format!("Nodes that {side} {}", p.ty.name()))
                                        .small()
                                        .color(theme::TEXT_MUTED),
                                );
                            });
                        }
                        let te = egui::TextEdit::singleline(&mut self.query)
                            .id(egui::Id::unique("add-node-query"))
                            .hint_text(format!(
                                "{}  Search nodes  (volume, lpf, clock…)",
                                icons::MAGNIFYING_GLASS
                            ))
                            .desired_width(f32::INFINITY)
                            .margin(egui::Margin::symmetric(8, 5));
                        let r = ui.add(te);
                        if self.focus {
                            r.request_focus();
                            self.focus = false;
                        }
                        if r.changed() {
                            self.selected = 0;
                        }
                        ui.add_space(4.0);
                        if list.is_empty() {
                            ui.label(RichText::new("Nothing matches.").color(theme::TEXT_MUTED));
                        }
                        egui::ScrollArea::vertical()
                            .max_height(300.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                for (i, c) in list.iter().enumerate() {
                                    let selected = i == self.selected;
                                    let resp = row(ui, c, selected, width);
                                    if selected && (up || down) {
                                        resp.scroll_to_me(None);
                                    }
                                    if resp.hovered() && ui.input(|i| i.pointer.is_moving()) {
                                        self.selected = i;
                                    }
                                    if resp.clicked() {
                                        chosen = Some(c.clone());
                                    }
                                }
                            });
                        if let Some(c) = list.get(self.selected) {
                            ui.separator();
                            ui.label(RichText::new(&c.summary).small().color(theme::TEXT));
                        }
                    });
            });
        let clicked_outside =
            ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer();
        if chosen.is_some() || clicked_outside {
            self.close();
        }
        chosen
    }
}

fn row(ui: &mut egui::Ui, c: &Candidate, selected: bool, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, theme::radius(5.0), theme::ACCENT_DIM);
    } else if resp.hovered() {
        p.rect_filled(
            rect,
            theme::radius(5.0),
            Color32::from_rgb(0x2E, 0x32, 0x3A),
        );
    }
    let color = c
        .category
        .map_or(theme::GROUP_HEADER, theme::category_color);
    let strip =
        egui::Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(4.0, rect.height() - 12.0));
    p.rect_filled(strip, theme::radius(2.0), color);
    let icon = if c.kind == GROUP_KIND {
        format!("{}  ", icons::STACK)
    } else {
        String::new()
    };
    p.text(
        rect.left_center() + vec2(18.0, 0.0),
        Align2::LEFT_CENTER,
        format!("{icon}{}", c.name),
        theme::ui_font(theme::FONT_UI),
        if selected {
            theme::TEXT_STRONG
        } else {
            theme::TEXT
        },
    );
    let cat = c.category.map_or("Group", |c| c.name());
    p.text(
        rect.right_center() - vec2(8.0, 0.0),
        Align2::RIGHT_CENTER,
        cat,
        theme::ui_font(theme::FONT_SMALL),
        theme::TEXT_MUTED,
    );
    if let Some(s) = &c.socket {
        let x = rect.right_center().x
            - 8.0
            - ui.fonts_mut(|f| {
                f.layout_no_wrap(
                    cat.to_string(),
                    theme::ui_font(theme::FONT_SMALL),
                    theme::TEXT_MUTED,
                )
                .size()
                .x
            })
            - 10.0;
        p.text(
            egui::pos2(x, rect.center().y),
            Align2::RIGHT_CENTER,
            format!("→ {s}"),
            theme::mono_font(theme::FONT_SMALL),
            theme::TEXT_FAINT,
        );
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{Endpoint, NodeId, Project};
    use std::path::PathBuf;

    fn doc() -> Document {
        Document::new(Project::default(), None, PathBuf::from("."), 1)
    }

    #[test]
    fn search_uses_synonyms() {
        let r = ac_nodes::registry();
        let c = candidates(&r, &doc(), "volume", None);
        assert_eq!(c[0].kind, "gain");
        assert!(candidates(&r, &doc(), "", None).len() >= 20);
        assert!(
            !candidates(&r, &doc(), "", None)
                .iter()
                .any(|c| c.kind == GROUP_KIND)
        );
    }

    #[test]
    fn pending_wires_filter_to_compatible_nodes() {
        let r = ac_nodes::registry();
        let p = PendingWire {
            from: Some(Endpoint::new(NodeId(1), "out")),
            to: None,
            ty: SignalType::Audio,
        };
        let c = candidates(&r, &doc(), "", Some(&p));
        assert!(
            c.iter()
                .any(|c| c.kind == "gain" && c.socket.as_deref() == Some("in"))
        );
        // A Comment has no sockets; a Button has no audio-capable input.
        assert!(!c.iter().any(|c| c.kind == "comment" || c.kind == "button"));
        let p = PendingWire {
            from: None,
            to: Some(Endpoint::new(NodeId(1), "flip")),
            ty: SignalType::Trigger,
        };
        let c = candidates(&r, &doc(), "", Some(&p));
        assert!(
            c.iter()
                .any(|c| c.kind == "button" && c.socket.as_deref() == Some("trigger"))
        );
        assert!(!c.iter().any(|c| c.kind == "gain"));
    }
}
