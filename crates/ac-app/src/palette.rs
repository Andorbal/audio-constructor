//! The add-node palette (`Shift+A`, double-click, or dropping a wire on
//! empty canvas): fuzzy search with synonyms, keyboard driven, filtered to
//! nodes that can complete a dragged wire. It offers nodes, the groups in
//! the project, and the blocks in the library.

use ac_core::{
    Block, BlockSource, Category, Config, GROUP_KIND, GroupId, Registry, SignalType, VOICES_KIND,
};
use ac_io::{Library, LibraryBlock};
use egui::{Align2, Key, Pos2, RichText, Sense, vec2};

use crate::canvas::PendingWire;
use crate::document::Document;
use crate::edit;
use crate::i18n::{self, tr, tr_args};
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
    /// A library block: adding it pastes an instance of its definition.
    pub block: Option<BlockSource>,
}

impl Candidate {
    /// A node of `kind` with an empty config.
    pub fn node(name: &str, kind: &str, category: Option<Category>, summary: String) -> Candidate {
        Candidate {
            name: name.to_string(),
            kind: kind.to_string(),
            category,
            summary,
            config: ac_core::empty_config(),
            socket: None,
            block: None,
        }
    }
}

/// Whether a block is meant for polyphony: a Voice node inside reads each
/// copy's note, so it's added as a Voices instance.
pub fn block_is_polyphonic(block: &Block) -> bool {
    block.is_polyphonic()
}

/// The palette's entry for a library block.
pub fn block_candidate(b: &LibraryBlock, socket: Option<String>) -> Candidate {
    let kind = if block_is_polyphonic(&b.block) {
        VOICES_KIND
    } else {
        GROUP_KIND
    };
    Candidate {
        name: b.name().to_string(),
        kind: kind.to_string(),
        category: None,
        summary: first_paragraph(&b.block.def.description).to_string(),
        config: ac_core::empty_config(),
        socket,
        block: Some(b.source.clone()),
    }
}

/// Text up to the first blank line.
pub fn first_paragraph(text: &str) -> &str {
    text.split("\n\n").next().unwrap_or("").trim()
}

/// How well a name matches a query: 0 exactly, 1 as a prefix, 2 otherwise.
fn tier(name: &str, q: &str) -> u8 {
    let name = name.to_lowercase();
    if name == q {
        0
    } else if name.starts_with(q) {
        1
    } else {
        2
    }
}

/// What the user picked, with the wire it completes and where it goes.
#[derive(Clone, Debug, PartialEq)]
pub struct Picked {
    pub candidate: Candidate,
    pub pending: Option<PendingWire>,
    /// Canvas position for the new node.
    pub at: Pos2,
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
    /// A line above the search saying what happens to the node added
    /// (the selected nodes it will be wired from).
    pub note: Option<String>,
    focus: bool,
    /// Frames since it opened (the opening click mustn't close it).
    age: u32,
}

/// What the palette offers for `query`, best first: nodes in the
/// registry's order, with groups and blocks whose names match the query
/// outright moved up among them.
pub fn candidates(
    registry: &Registry,
    library: &Library,
    doc: &Document,
    query: &str,
    pending: Option<&PendingWire>,
) -> Vec<Candidate> {
    let q = query.trim().to_lowercase();
    let mut out: Vec<(u8, Candidate)> = Vec::new();
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
        let t = tier(e.meta.name, &q).min(tier(e.meta.kind, &q));
        out.push((
            t,
            Candidate {
                name: e.meta.name.to_string(),
                kind: e.meta.kind.to_string(),
                category: Some(e.meta.category),
                summary: e.meta.summary(),
                config,
                socket,
                block: None,
            },
        ));
    }
    let mut extra = Vec::new();
    // Group definitions in this project, except ones that would contain
    // the group being edited.
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
        extra.push(Candidate {
            name: def.name.clone(),
            kind: GROUP_KIND.to_string(),
            category: None,
            summary: if def.description.is_empty() {
                tr_args("palette-group-instance", &[("name", (&def.name).into())])
            } else {
                def.description.clone()
            },
            config: serde_json::json!({ "def": GroupId::clone(id).0 }),
            socket,
            block: None,
        });
    }
    // Library blocks, by name, tags, or description.
    for b in library.search(query) {
        let sig = if block_is_polyphonic(&b.block) {
            ac_core::voices_signature(&b.block.def)
        } else {
            ac_core::group_signature(&b.block.def)
        };
        let socket = match pending {
            Some(p) => match pick_socket(&sig, p) {
                Some(s) => Some(s),
                None => continue,
            },
            None => None,
        };
        extra.push(block_candidate(b, socket));
    }
    for c in extra {
        let t = if q.is_empty() { 2 } else { tier(&c.name, &q) };
        let at = out.iter().position(|(u, _)| *u > t).unwrap_or(out.len());
        out.insert(at, (t, c));
    }
    out.into_iter().map(|(_, c)| c).collect()
}

/// The socket of a new node that completes `p`: for a wire from an output,
/// the first input that takes its type (an exact match first); for a wire
/// from an input, the first output that feeds it.
pub fn pick_socket(sig: &ac_core::Signature, p: &PendingWire) -> Option<String> {
    let ty = p.ty;
    let visible = |list: &[ac_core::SocketSpec]| -> Vec<ac_core::SocketSpec> {
        list.iter().filter(|s| !s.hidden).cloned().collect()
    };
    if p.from.is_some() {
        let ins = visible(&sig.inputs);
        ins.iter()
            .find(|s| s.ty == ty && ty != SignalType::Any)
            .or_else(|| ins.iter().find(|s| edit::compatible(ty, s.ty)))
            .map(|s| s.name.to_string())
    } else {
        let outs = visible(&sig.outputs);
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
        self.note = None;
        self.focus = true;
        self.age = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.pending = None;
        self.note = None;
    }

    /// Show the palette; returns the chosen candidate.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        registry: &Registry,
        library: &Library,
        doc: &Document,
    ) -> Option<Picked> {
        if !self.open {
            return None;
        }
        let list = candidates(registry, library, doc, &self.query, self.pending.as_ref());
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
                                let args = [("type", p.ty.name().into())];
                                let text = if p.from.is_some() {
                                    tr_args("palette-nodes-that-take", &args)
                                } else {
                                    tr_args("palette-nodes-that-feed", &args)
                                };
                                ui.label(RichText::new(text).small().color(theme::text_muted()));
                            });
                        }
                        if let Some(note) = &self.note {
                            ui.label(
                                RichText::new(format!("{}  {note}", icons::PLUGS_CONNECTED))
                                    .small()
                                    .color(theme::text_muted()),
                            );
                        }
                        let te = egui::TextEdit::singleline(&mut self.query)
                            .id(egui::Id::unique("add-node-query"))
                            .hint_text(format!(
                                "{}  {}",
                                icons::MAGNIFYING_GLASS,
                                tr("palette-hint")
                            ))
                            .desired_width(f32::INFINITY)
                            .margin(egui::Margin::symmetric(8, 5));
                        let r = ui.add(te).accessible_name(tr("palette-name"));
                        if self.focus {
                            r.request_focus();
                            self.focus = false;
                        }
                        if r.changed() {
                            self.selected = 0;
                        }
                        ui.add_space(4.0);
                        if list.is_empty() {
                            ui.label(
                                RichText::new(tr("palette-nothing-matches"))
                                    .color(theme::text_muted()),
                            );
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
                            ui.label(RichText::new(&c.summary).small().color(theme::text()));
                        }
                    });
            });
        let clicked_outside = self.age > 0
            && ctx.input(|i| i.pointer.any_pressed())
            && !area.response.contains_pointer();
        self.age += 1;
        let picked = chosen.map(|c| Picked {
            candidate: c,
            pending: self.pending.take(),
            at: self.canvas,
        });
        if picked.is_some() || clicked_outside {
            self.close();
        }
        picked
    }
}

fn row(ui: &mut egui::Ui, c: &Candidate, selected: bool, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, theme::radius(5.0), theme::accent_dim());
    } else if resp.hovered() {
        p.rect_filled(rect, theme::radius(5.0), theme::row_hover());
    }
    let color = c
        .category
        .map_or(theme::group_header(), theme::category_color);
    let strip =
        egui::Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(4.0, rect.height() - 12.0));
    p.rect_filled(strip, theme::radius(2.0), color);
    let icon = if c.category.is_none() {
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
            theme::text_strong()
        } else {
            theme::text()
        },
    );
    let cat = match (c.category, &c.block) {
        (Some(cat), _) => i18n::category(cat),
        (None, Some(_)) => tr("palette-block"),
        (None, None) => tr("palette-group"),
    };
    // One option to screen readers: the node, its category, and what it
    // does.
    let spoken = format!("{}, {cat}: {}", c.name, c.summary);
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::ListBoxOption, true, selected, &spoken)
    });
    p.text(
        rect.right_center() - vec2(8.0, 0.0),
        Align2::RIGHT_CENTER,
        &cat,
        theme::ui_font(theme::FONT_SMALL),
        theme::text_muted(),
    );
    if let Some(s) = &c.socket {
        let x = rect.right_center().x
            - 8.0
            - ui.fonts_mut(|f| {
                f.layout_no_wrap(
                    cat.to_string(),
                    theme::ui_font(theme::FONT_SMALL),
                    theme::text_muted(),
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
            theme::text_faint(),
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
        let c = candidates(&r, &Library::default(), &doc(), "volume", None);
        assert_eq!(c[0].kind, "gain");
        let none = Library::default();
        assert!(candidates(&r, &none, &doc(), "", None).len() >= 20);
        assert!(
            !candidates(&r, &none, &doc(), "", None)
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
        let c = candidates(&r, &Library::default(), &doc(), "", Some(&p));
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
        let c = candidates(&r, &Library::default(), &doc(), "", Some(&p));
        assert!(
            c.iter()
                .any(|c| c.kind == "button" && c.socket.as_deref() == Some("trigger"))
        );
        assert!(!c.iter().any(|c| c.kind == "gain"));
    }

    #[test]
    fn blocks_named_outright_come_first() {
        let r = ac_nodes::registry();
        let lib = Library::shipped_only();
        let c = candidates(&r, &lib, &doc(), "tremolo", None);
        // The LFO lists "tremolo" as a synonym; the block of that name leads.
        assert_eq!(
            c[0].block,
            Some(BlockSource::new(ac_core::Origin::Shipped, "tremolo"))
        );
        assert_eq!(c[0].kind, GROUP_KIND);
        assert_eq!(c[0].summary, first_paragraph(&c[0].summary));
        assert!(c.iter().any(|c| c.kind == "lfo"));
        // Tags and descriptions match too.
        let c = candidates(&r, &lib, &doc(), "amplitude", None);
        assert!(c.iter().any(|c| c.block.is_some()));
        // With no query, blocks follow the nodes.
        let all = candidates(&r, &lib, &doc(), "", None);
        assert!(all.last().unwrap().block.is_some());
        assert!(all[0].block.is_none());
    }

    #[test]
    fn pending_wires_filter_blocks_by_their_interface() {
        let r = ac_nodes::registry();
        let lib = Library::shipped_only();
        let from_audio = PendingWire {
            from: Some(Endpoint::new(NodeId(1), "out")),
            to: None,
            ty: SignalType::Audio,
        };
        let c = candidates(&r, &lib, &doc(), "tremolo", Some(&from_audio));
        let t = c
            .iter()
            .find(|c| c.block.is_some())
            .expect("Tremolo takes audio");
        assert_eq!(t.socket.as_deref(), Some("in"));
        let into_midi = PendingWire {
            from: None,
            to: Some(Endpoint::new(NodeId(1), "midi")),
            ty: SignalType::Midi,
        };
        let c = candidates(&r, &lib, &doc(), "tremolo", Some(&into_midi));
        assert!(
            !c.iter().any(|c| c.block.is_some()),
            "Tremolo has no MIDI output"
        );
    }

    #[test]
    fn first_paragraphs() {
        assert_eq!(first_paragraph("One. Two.\n\nThree."), "One. Two.");
        assert_eq!(first_paragraph("  Only  "), "Only");
        assert_eq!(first_paragraph(""), "");
    }
}
