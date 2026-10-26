//! Node editors outside the Node tab (UX.md "Node editors"): any number in
//! windows of their own, and one docked under the canvas. Each shows the
//! node's editor live, with its playhead, and edits it like the Node tab.
//!
//! An editor belongs to the graph it was opened in: inside another group it
//! waits, hidden, until that graph is shown again.

use ac_core::{NodeId, Registry};
use egui::{RichText, vec2};

use crate::canvas::live::Live;
use crate::document::Document;
use crate::editors::{self, Editor};
use crate::i18n::tr;
use crate::icons;
use crate::theme;

/// A node whose editor is open, and the graph it's in.
#[derive(Clone, Debug, PartialEq)]
pub struct Opened {
    pub scope: Vec<NodeId>,
    pub node: NodeId,
}

#[derive(Default)]
pub struct EditorWindows {
    /// Editors in windows, in the order they were opened.
    pub open: Vec<Opened>,
    /// The editor docked under the canvas.
    pub docked: Option<Opened>,
}

/// What the person asked of an editor's frame this frame.
enum Frame {
    Close,
    Dock,
    Undock,
}

impl EditorWindows {
    fn here(doc: &Document, node: NodeId) -> Opened {
        Opened {
            scope: doc.scope_prefix(),
            node,
        }
    }

    /// Whether `node` (in the graph shown) has a window open.
    pub fn is_open(&self, doc: &Document, node: NodeId) -> bool {
        self.open.contains(&Self::here(doc, node))
    }

    /// Open a window for `node`'s editor, or close it.
    pub fn toggle(&mut self, doc: &Document, node: NodeId) {
        let o = Self::here(doc, node);
        match self.open.iter().position(|x| *x == o) {
            Some(i) => {
                self.open.remove(i);
            }
            None => self.open.push(o),
        }
    }

    /// Dock `node`'s editor under the canvas (closing its window); the
    /// editor docked before goes back to a window.
    pub fn dock(&mut self, doc: &Document, node: NodeId) {
        let o = Self::here(doc, node);
        self.open.retain(|x| *x != o);
        if let Some(old) = self.docked.replace(o)
            && !self.open.contains(&old)
        {
            self.open.push(old);
        }
    }

    /// The node docked, when it's in the graph shown.
    pub fn docked_here(&self, doc: &Document) -> Option<NodeId> {
        self.docked
            .as_ref()
            .filter(|o| o.scope == doc.scope_prefix())
            .map(|o| o.node)
    }

    /// Forget editors whose nodes are gone or no longer have one.
    fn prune(&mut self, doc: &Document) {
        let scope = doc.scope_prefix();
        let gone = |o: &Opened| {
            o.scope == scope
                && doc
                    .graph()
                    .node(o.node)
                    .is_none_or(|n| editors::editor_for(&n.kind, &n.config).is_none())
        };
        self.open.retain(|o| !gone(o));
        if self.docked.as_ref().is_some_and(gone) {
            self.docked = None;
        }
    }

    /// Draw every window of the graph shown.
    pub fn windows(
        &mut self,
        ctx: &egui::Context,
        doc: &mut Document,
        registry: &Registry,
        live: &Live,
        now: f64,
    ) {
        self.prune(doc);
        let scope = doc.scope_prefix();
        let mut asked = Vec::new();
        for o in self.open.iter().filter(|o| o.scope == scope) {
            let node = o.node;
            let mut open = true;
            let mut frame = None;
            egui::Window::new(title(doc, registry, node))
                .id(egui::Id::unique(("editor-window", &o.scope, node)))
                .open(&mut open)
                .default_size(vec2(640.0, 380.0))
                .resizable(true)
                .collapsible(true)
                .show(ctx, |ui| {
                    if icons::spoken(ui.small_button(format!(
                        "{}  {}",
                        icons::TRAY_ARROW_DOWN,
                        tr("editor-dock")
                    )))
                    .on_hover_text(tr("editor-dock-tip"))
                    .clicked()
                    {
                        frame = Some(Frame::Dock);
                    }
                    let size = ui.available_size();
                    body(ui, doc, live, node, ("window", node), size, now);
                });
            if !open {
                frame = Some(Frame::Close);
            }
            if let Some(f) = frame {
                asked.push((node, f));
            }
        }
        for (node, f) in asked {
            match f {
                Frame::Close => self.toggle(doc, node),
                Frame::Dock => self.dock(doc, node),
                Frame::Undock => {}
            }
        }
    }

    /// The docked editor, in a panel under the canvas (call before the
    /// canvas's central panel).
    pub fn dock_panel(
        &mut self,
        ui: &mut egui::Ui,
        doc: &mut Document,
        registry: &Registry,
        live: &Live,
        now: f64,
    ) {
        self.prune(doc);
        let Some(node) = self.docked_here(doc) else {
            return;
        };
        let mut frame = None;
        egui::Panel::bottom("docked-editor")
            .resizable(true)
            .default_size(240.0)
            .size_range(120.0..=600.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::panel_bg())
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(title(doc, registry, node))
                            .strong()
                            .color(theme::text_strong()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::spoken(ui.small_button(icons::X))
                            .on_hover_text(tr("editor-close-dock"))
                            .accessible_name(tr("editor-close-dock"))
                            .clicked()
                        {
                            frame = Some(Frame::Close);
                        }
                        if icons::spoken(ui.small_button(format!(
                            "{}  {}",
                            icons::ARROW_SQUARE_OUT,
                            tr("editor-undock")
                        )))
                        .clicked()
                        {
                            frame = Some(Frame::Undock);
                        }
                    });
                });
                let size = ui.available_size();
                body(ui, doc, live, node, ("dock", node), size, now);
            });
        match frame {
            Some(Frame::Close) => self.docked = None,
            Some(Frame::Undock) => {
                if let Some(o) = self.docked.take()
                    && !self.open.contains(&o)
                {
                    self.open.push(o);
                }
            }
            _ => {}
        }
    }
}

/// The node's label, or its kind's name.
fn title(doc: &Document, registry: &Registry, node: NodeId) -> String {
    doc.graph()
        .node(node)
        .map(|n| {
            n.ui.label.clone().unwrap_or_else(|| {
                registry
                    .get(&n.kind)
                    .map_or_else(|| n.kind.clone(), |e| e.meta.name.to_string())
            })
        })
        .unwrap_or_default()
}

/// A node's editor filling `size`, live, its edits stored as undo steps.
fn body(
    ui: &mut egui::Ui,
    doc: &mut Document,
    live: &Live,
    node: NodeId,
    salt: (&str, NodeId),
    size: egui::Vec2,
    now: f64,
) {
    let Some(n) = doc.graph().node(node).cloned() else {
        return;
    };
    let Some(editor) = editors::editor_for(&n.kind, &n.config) else {
        return;
    };
    let view = editors::live_view(&n.kind, |k| live.node_value(node, k));
    let w = size.x.max(200.0);
    let h = (size.y - 4.0).max(editor.height(w).min(120.0));
    let edit = editors::show(ui, editor, &n.config, &view, salt, w, h);
    apply(doc, node, editor, edit, now);
}

/// Store an editor's change on `node` (one undo step per gesture).
pub fn apply(
    doc: &mut Document,
    node: NodeId,
    editor: Editor,
    edit: Option<editors::Edit>,
    now: f64,
) {
    if let Some(e) = edit {
        doc.set_config(node, editor.key(), e.value, now);
        if e.done {
            doc.end_coalescing();
        }
    }
}
