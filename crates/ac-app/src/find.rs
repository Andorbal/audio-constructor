//! Find (`Ctrl+F`): a bar over the canvas that finds nodes by name, kind,
//! or id anywhere in the project, inside groups too, and jumps to them.
//! Names rank the way the add-node palette ranks them
//! ([`ac_core::fuzzy_score`]). A node inside a group used several times is
//! listed once per instance, since each is a different place to jump to
//! (D101).

use std::collections::BTreeMap;

use ac_core::{
    ConfigExt, GROUP_KIND, Graph, GroupDef, GroupId, NodeId, NodePath, Project, Registry,
    VOICES_KIND, fuzzy_score,
};
use egui::{Key, Rect, RichText, Sense, pos2, vec2};

use crate::edit::connect;
use crate::icons;
use crate::theme;

/// The most results listed.
const MAX_RESULTS: usize = 100;
/// Groups nested deeper than this aren't searched (a damaged file could
/// nest them forever).
const MAX_DEPTH: usize = 16;

/// A node that matches.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    /// Where it is: the group instances to go through, then the node.
    pub path: NodePath,
    pub title: String,
    /// Its kind's name ("Oscillator"), or its group's.
    pub kind: String,
    /// The groups it's inside, outermost first ("Strip 1 › Filter"), or
    /// empty at the top level.
    pub place: String,
    score: i32,
}

impl Found {
    pub fn id(&self) -> Option<NodeId> {
        self.path.leaf()
    }
}

/// Every node in the project matching `query`, best first; nodes nearer
/// the top level come first among equals.
pub fn search(project: &Project, registry: &Registry, query: &str) -> Vec<Found> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut at = Walk::default();
    walk(
        &project.graph,
        &project.group_defs,
        registry,
        &q,
        &mut at,
        &mut out,
    );
    out.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(a.path.0.len().cmp(&b.path.0.len()))
            .then(a.place.cmp(&b.place))
            .then(a.title.cmp(&b.title))
            .then(a.path.0.cmp(&b.path.0))
    });
    out.truncate(MAX_RESULTS);
    out
}

/// Where the search is: the instances entered, their names, and the
/// definitions open (so a group can't be searched inside itself).
#[derive(Default)]
struct Walk {
    path: Vec<NodeId>,
    names: Vec<String>,
    open: Vec<GroupId>,
}

fn walk(
    graph: &Graph,
    defs: &BTreeMap<GroupId, GroupDef>,
    registry: &Registry,
    q: &str,
    at: &mut Walk,
    out: &mut Vec<Found>,
) {
    for n in &graph.nodes {
        let title = connect::title(n, registry, defs);
        let def = n
            .group_def()
            .filter(|_| n.kind == GROUP_KIND || n.kind == VOICES_KIND);
        let kind = match def.as_ref().and_then(|d| defs.get(d)) {
            Some(d) => format!("Group {}", d.name),
            None => registry
                .get(&n.kind)
                .map_or_else(|| n.kind.clone(), |e| e.meta.name.to_string()),
        };
        let id = n.id.to_string();
        let mut score = [&title, &kind, &n.kind]
            .iter()
            .filter_map(|f| fuzzy_score(q, f))
            .max();
        // Ids match from their start only: a few hex digits are in most ids.
        if id.starts_with(q) {
            score = score.max(Some(if id == q { 1000 } else { 800 }));
        }
        // A note's text, anywhere in it.
        if n.kind == "comment" && n.config.get_str_or("text", "").to_lowercase().contains(q) {
            score = score.max(Some(300));
        }
        let mut path = at.path.clone();
        path.push(n.id);
        if let Some(score) = score {
            out.push(Found {
                path: NodePath(path),
                title: title.clone(),
                kind,
                place: at.names.join(" › "),
                score,
            });
        }
        let Some(d) = def else { continue };
        let Some(inner) = defs.get(&d) else { continue };
        if at.open.contains(&d) || at.open.len() >= MAX_DEPTH {
            continue;
        }
        at.path.push(n.id);
        at.names.push(title);
        at.open.push(d);
        walk(&inner.graph, defs, registry, q, at, out);
        at.path.pop();
        at.names.pop();
        at.open.pop();
    }
}

/// The bar itself.
#[derive(Default)]
pub struct FindBar {
    pub open: bool,
    pub query: String,
    /// The highlighted result.
    pub selected: usize,
    focus: bool,
    /// Frames since it opened (the opening click mustn't close it).
    age: u32,
}

impl FindBar {
    /// Open with an empty query and the typing focus.
    pub fn show_bar(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.focus = true;
        self.age = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Draw the bar over the top of the `canvas` (screen). Returns the
    /// result to jump to when one is chosen (`Enter` or a click); the bar
    /// stays open so the next one is a key away. `Esc`, or a click
    /// elsewhere, closes it.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        canvas: Rect,
        project: &Project,
        registry: &Registry,
    ) -> Option<Found> {
        if !self.open {
            return None;
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
        let list = search(project, registry, &self.query);
        if self.selected >= list.len() {
            self.selected = list.len().saturating_sub(1);
        }
        if down && !list.is_empty() {
            self.selected = (self.selected + 1) % list.len();
        }
        if up && !list.is_empty() {
            self.selected = (self.selected + list.len() - 1) % list.len();
        }
        let mut chosen = None;
        if enter && let Some(f) = list.get(self.selected) {
            chosen = Some(f.clone());
        }
        let width = 400.0;
        let at = pos2(canvas.center().x - width / 2.0 - 8.0, canvas.min.y + 10.0);
        let area = egui::Area::new(egui::Id::unique("find-bar"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin::same(8))
                    .corner_radius(theme::radius(10.0))
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.horizontal(|ui| {
                            let te = egui::TextEdit::singleline(&mut self.query)
                                .id(egui::Id::unique("find-query"))
                                .hint_text(format!(
                                    "{}  Find nodes by name, kind, or id",
                                    icons::MAGNIFYING_GLASS
                                ))
                                .desired_width(width - 80.0)
                                // Enter jumps and keeps the typing here.
                                .return_key(None)
                                .margin(egui::Margin::symmetric(8, 5));
                            let r = ui.add(te);
                            if self.focus {
                                r.request_focus();
                                self.focus = false;
                            }
                            if r.changed() {
                                self.selected = 0;
                            }
                            let count = if self.query.trim().is_empty() {
                                String::new()
                            } else if list.is_empty() {
                                "none".to_string()
                            } else {
                                format!("{} of {}", self.selected + 1, list.len())
                            };
                            ui.label(RichText::new(count).small().color(theme::text_muted()));
                        });
                        if !list.is_empty() {
                            ui.add_space(4.0);
                            egui::ScrollArea::vertical()
                                .max_height(260.0)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    for (i, f) in list.iter().enumerate() {
                                        let selected = i == self.selected;
                                        let resp = row(ui, f, selected, width);
                                        if selected && (up || down) {
                                            resp.scroll_to_me(None);
                                        }
                                        if resp.clicked() {
                                            self.selected = i;
                                            chosen = Some(f.clone());
                                        }
                                    }
                                });
                        }
                        ui.label(
                            RichText::new("↑ ↓ choose · Enter jumps to it · Esc closes")
                                .small()
                                .color(theme::text_faint()),
                        );
                    });
            });
        let clicked_outside = self.age > 0
            && ctx.input(|i| i.pointer.any_pressed())
            && !area.response.contains_pointer();
        self.age += 1;
        if clicked_outside {
            self.close();
        }
        chosen
    }
}

fn row(ui: &mut egui::Ui, f: &Found, selected: bool, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 36.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, theme::radius(5.0), theme::accent_dim());
    } else if resp.hovered() {
        p.rect_filled(rect, theme::radius(5.0), theme::widget_bg_hover());
    }
    p.text(
        rect.left_top() + vec2(10.0, 4.0),
        egui::Align2::LEFT_TOP,
        &f.title,
        theme::ui_font(theme::FONT_UI),
        if selected {
            theme::text_strong()
        } else {
            theme::text()
        },
    );
    let place = if f.place.is_empty() {
        f.kind.clone()
    } else {
        format!("{} · in {}", f.kind, f.place)
    };
    let label = format!("{}, {place}", f.title);
    p.text(
        rect.left_bottom() + vec2(10.0, -4.0),
        egui::Align2::LEFT_BOTTOM,
        place,
        theme::ui_font(theme::FONT_SMALL),
        theme::text_muted(),
    );
    // Screen readers (and tests) read the row as one button.
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::NodeInstance;

    /// A root graph with an oscillator and two instances of a group
    /// holding a labeled gain.
    fn project() -> Project {
        let mut p = Project::default();
        let mut osc = NodeInstance::new(NodeId(1), "oscillator");
        osc.ui.label = Some("Bass".into());
        p.graph.nodes.push(osc);
        let mut def = GroupDef::new("Strip");
        let mut g = NodeInstance::new(NodeId(10), "gain");
        g.ui.label = Some("Fader".into());
        def.graph.nodes.push(g);
        p.group_defs.insert(GroupId("strip".into()), def);
        for (id, label) in [(2u64, "Strip A"), (3, "Strip B")] {
            let mut inst = NodeInstance::new(NodeId(id), GROUP_KIND)
                .with_config(serde_json::json!({"def": "strip"}));
            inst.ui.label = Some(label.into());
            p.graph.nodes.push(inst);
        }
        let mut note = NodeInstance::new(NodeId(4), "comment");
        note.config = serde_json::json!({"text": "Turn the fader down first"});
        p.graph.nodes.push(note);
        p
    }

    #[test]
    fn finds_by_label_kind_and_id_inside_groups() {
        let r = ac_nodes::registry();
        let p = project();
        let f = search(&p, &r, "fader");
        // Both instances of the gain, then the note that mentions it.
        assert_eq!(f.len(), 3);
        assert_eq!(f[0].path, NodePath(vec![NodeId(2), NodeId(10)]));
        assert_eq!(f[0].place, "Strip A");
        assert_eq!(f[1].path, NodePath(vec![NodeId(3), NodeId(10)]));
        assert_eq!(f[2].id(), Some(NodeId(4)));
        // By kind name, and by kind.
        let f = search(&p, &r, "oscil");
        assert_eq!(f[0].title, "Bass");
        assert_eq!(f[0].kind, "Oscillator");
        assert!(search(&p, &r, "gain").iter().any(|f| f.title == "Fader"));
        // By id, from its start.
        let f = search(&p, &r, "a");
        assert_eq!(f.first().map(|f| f.title.as_str()), Some("Fader"), "{f:?}");
        let id = search(&p, &r, &NodeId(0xbeef).to_string());
        assert!(id.is_empty());
        // Group instances are found by their group's name too.
        let f = search(&p, &r, "strip");
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].kind, "Group Strip");
        assert!(search(&p, &r, "   ").is_empty());
        assert!(search(&p, &r, "qqq").is_empty());
    }

    #[test]
    fn a_group_inside_itself_is_searched_once() {
        let r = ac_nodes::registry();
        let mut p = project();
        // A damaged file: the strip contains an instance of itself.
        if let Some(def) = p.group_defs.get_mut(&GroupId("strip".into())) {
            def.graph.nodes.push(
                NodeInstance::new(NodeId(11), GROUP_KIND)
                    .with_config(serde_json::json!({"def": "strip"})),
            );
        }
        let f = search(&p, &r, "fader");
        assert_eq!(f.iter().filter(|f| f.title == "Fader").count(), 2);
    }
}
