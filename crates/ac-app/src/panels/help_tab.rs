//! The Help tab (UX.md: "`F1` on a node opens its reference in the side
//! panel"): a node kind's name, category, docs, its "Try this" tip, its
//! sockets with their units and ranges, and the blocks and example
//! projects that use it. With no node chosen it lists every node, and its
//! search finds them the way the palette does.

use ac_core::{Category, NodeEntry, SignalType, SocketSpec, Value, empty_config};
use egui::{RichText, Stroke};

use super::{PanelIo, PanelRequest, markdown, section};
use crate::files;
use crate::help::{self, Example};
use crate::i18n::{self, tr, tr_args};
use crate::icons;
use crate::palette;
use crate::theme;

/// What the Help tab shows.
#[derive(Debug, Default)]
pub struct HelpView {
    /// The node kind shown; `None` shows the list of every node.
    pub kind: Option<String>,
    /// The search field: while it holds words, matching nodes show.
    pub query: String,
    /// The shipped examples, read when help first needs them.
    examples: Option<Vec<Example>>,
}

impl HelpView {
    /// Show `kind`'s reference.
    pub fn show_kind(&mut self, kind: &str) {
        self.kind = Some(kind.to_string());
        self.query.clear();
    }

    /// The shipped example projects (read once).
    pub fn examples(&mut self) -> &[Example] {
        self.examples.get_or_insert_with(|| {
            files::find_examples_dir()
                .map(|d| help::load_examples(&d))
                .unwrap_or_default()
        })
    }
}

pub(super) fn show(ui: &mut egui::Ui, io: &mut PanelIo, view: &mut HelpView) {
    ui.add(
        egui::TextEdit::singleline(&mut view.query)
            .hint_text(format!(
                "{}  {}",
                icons::MAGNIFYING_GLASS,
                tr("help-search")
            ))
            .desired_width(f32::INFINITY)
            .margin(egui::Margin::symmetric(8, 5)),
    )
    .accessible_name(tr("help-search"));
    ui.add_space(4.0);
    if !view.query.trim().is_empty() {
        results(ui, io, view);
        return;
    }
    let entry = view.kind.as_deref().and_then(|k| io.registry.get(k));
    match (entry, view.kind.clone()) {
        (Some(e), Some(kind)) => page(ui, io, view, &kind, e),
        (None, Some(kind)) => {
            ui.label(
                RichText::new(format!(
                    "{}  {}",
                    icons::WARNING,
                    tr_args("help-unknown-kind", &[("kind", kind.as_str().into())])
                ))
                .color(theme::warning()),
            );
            index(ui, io, view);
        }
        _ => index(ui, io, view),
    }
}

/// A node as a link: its category's color, its name, and its summary on
/// hover. Clicking shows its reference.
fn node_link(ui: &mut egui::Ui, view: &mut HelpView, e: &NodeEntry) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(3.0, 16.0), egui::Sense::hover());
        ui.painter().rect_filled(
            r,
            theme::radius(1.5),
            theme::category_color(e.meta.category),
        );
        let r = ui
            .selectable_label(false, RichText::new(e.meta.name).color(theme::text()))
            .on_hover_text(e.meta.summary());
        if r.clicked() {
            view.show_kind(e.meta.kind);
        }
    });
}

fn results(ui: &mut egui::Ui, io: &PanelIo, view: &mut HelpView) {
    let found = io.registry.search(&view.query);
    if found.is_empty() {
        ui.label(RichText::new(tr("help-no-match")).color(theme::text_muted()));
    }
    for e in found {
        node_link(ui, view, e);
    }
}

fn index(ui: &mut egui::Ui, io: &PanelIo, view: &mut HelpView) {
    ui.label(
        RichText::new(tr("help-index-hint"))
            .small()
            .color(theme::text_muted()),
    );
    for cat in Category::ALL {
        let mut entries: Vec<&NodeEntry> = io
            .registry
            .iter()
            .filter(|e| e.meta.category == cat)
            .collect();
        if entries.is_empty() {
            continue;
        }
        entries.sort_by_key(|e| e.meta.name);
        egui::CollapsingHeader::new(
            RichText::new(format!("{}  ({})", i18n::category(cat), entries.len()))
                .color(theme::text()),
        )
        .id_salt(("help-index", cat.slug()))
        .default_open(false)
        .show(ui, |ui| {
            for e in entries {
                node_link(ui, view, e);
            }
        });
    }
}

/// A node kind's reference.
fn page(ui: &mut egui::Ui, io: &mut PanelIo, view: &mut HelpView, kind: &str, e: &NodeEntry) {
    let m = &e.meta;
    if icons::spoken(ui.small_button(format!("{}  {}", icons::ARROW_LEFT, tr("help-all-nodes"))))
        .clicked()
    {
        view.kind = None;
        return;
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(4.0, 22.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(r, theme::radius(2.0), theme::category_color(m.category));
        ui.label(
            RichText::new(m.name)
                .size(theme::FONT_HEADING)
                .color(theme::text_strong()),
        );
        ui.label(
            RichText::new(i18n::category(m.category))
                .small()
                .color(theme::text_muted()),
        );
    });
    let mut tags = kind.to_string();
    if m.live {
        tags.push_str(" · ");
        tags.push_str(&tr("help-live-source"));
    }
    if m.whole_input {
        tags.push_str(" · ");
        tags.push_str(&tr("help-whole-input"));
    }
    ui.label(
        RichText::new(tags)
            .font(theme::mono_font(theme::FONT_SMALL))
            .color(theme::text_faint()),
    );
    ui.add_space(6.0);
    markdown::show(ui, &m.description());
    if let Some(tip) = m.tip() {
        tip_box(ui, &tip);
    }

    let sig = (e.describe)(&empty_config());
    sockets(ui, &tr("section-inputs"), &sig.inputs, true);
    sockets(ui, &tr("section-outputs"), &sig.outputs, false);
    used_in(ui, io, view, kind);
}

/// The "Try this" tip, highlighted.
pub(super) fn tip_box(ui: &mut egui::Ui, tip: &str) {
    ui.add_space(4.0);
    egui::Frame::new()
        .fill(theme::fade(theme::accent(), 0.10))
        .stroke(Stroke::new(1.0, theme::fade(theme::accent(), 0.5)))
        .corner_radius(theme::radius(6.0))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(format!("{}  {}", icons::LIGHTBULB, tr("help-try-this")))
                    .color(theme::accent())
                    .strong(),
            );
            markdown::show(ui, tip);
        });
    ui.add_space(4.0);
}

/// Sockets with their type, default, range or choices, and docs (as the
/// node has them with its default settings).
fn sockets(ui: &mut egui::Ui, title: &str, list: &[SocketSpec], inputs: bool) {
    let shown: Vec<&SocketSpec> = list.iter().filter(|s| !s.hidden).collect();
    if shown.is_empty() {
        return;
    }
    section(ui, title);
    for s in shown {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(r.center(), 3.5, theme::type_color(s.ty));
            ui.label(
                RichText::new(s.name.as_ref())
                    .font(theme::mono_font(theme::FONT_VALUE))
                    .color(theme::text_strong()),
            );
            ui.label(
                RichText::new(type_label(s))
                    .small()
                    .color(theme::type_color(s.ty)),
            );
        });
        ui.indent(("help-socket", s.name.as_ref()), |ui| {
            if inputs && let Some(detail) = detail(s) {
                ui.label(RichText::new(detail).small().color(theme::text_muted()));
            }
            if !s.doc.is_empty() {
                ui.add(egui::Label::new(RichText::new(s.doc).color(theme::text())).wrap());
            }
        });
        ui.add_space(2.0);
    }
}

fn type_label(s: &SocketSpec) -> String {
    let unit = s.unit.suffix();
    if s.ty == SignalType::Float && !unit.is_empty() {
        format!("Float ({unit})")
    } else {
        s.ty.name().to_string()
    }
}

/// "440 Hz by default · 20 … 20000 Hz", or the choices.
fn detail(s: &SocketSpec) -> Option<String> {
    let unit = s.unit.suffix();
    let with_unit = |v: f64| {
        if unit.is_empty() {
            format!("{v}")
        } else {
            format!("{v} {unit}")
        }
    };
    let default = match (&s.default, s.ty) {
        (_, SignalType::Audio | SignalType::Midi | SignalType::Spectrum | SignalType::Buffer) => {
            None
        }
        (_, SignalType::Trigger) if s.choices.is_empty() => None,
        (Value::Text(t), _) => Some(t.clone()),
        (Value::Bool(b), _) => Some(if *b {
            tr("canvas-tip-on")
        } else {
            tr("canvas-tip-off")
        }),
        (Value::Int(i), _) => Some(with_unit(*i as f64)),
        (Value::Float(f), _) => Some(with_unit(*f)),
    };
    let range = if !s.choices.is_empty() {
        Some(s.choices.join(", "))
    } else {
        s.range.map(|(a, b)| format!("{a} … {}", with_unit(b)))
    };
    let parts: Vec<String> = [
        default.map(|d| tr_args("help-default", &[("value", d.into())])),
        range,
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// The blocks (in the library) and the examples that use `kind`: clicking
/// a block adds it to the canvas, clicking an example opens it.
fn used_in(ui: &mut egui::Ui, io: &mut PanelIo, view: &mut HelpView, kind: &str) {
    section(ui, &tr("section-used-in"));
    let blocks = help::blocks_using(io.library, kind);
    let examples: Vec<Example> = help::examples_using(view.examples(), kind)
        .into_iter()
        .cloned()
        .collect();
    if blocks.is_empty() && examples.is_empty() {
        ui.label(
            RichText::new(tr("help-unused"))
                .small()
                .color(theme::text_muted()),
        );
        return;
    }
    if !blocks.is_empty() {
        ui.label(
            RichText::new(tr("help-blocks"))
                .small()
                .color(theme::text_muted()),
        );
        for b in blocks {
            let d = &b.block.def;
            let r = ui
                .selectable_label(
                    false,
                    RichText::new(format!("{}  {}", icons::STACK, d.name)).color(theme::accent()),
                )
                // The icon tells a block from an example on screen.
                .accessible_name(tr_args("help-block-link", &[("name", (&d.name).into())]))
                .on_hover_ui(|ui| {
                    ui.set_max_width(340.0);
                    if !d.description.is_empty() {
                        markdown::show(ui, &d.description);
                    }
                    ui.label(
                        RichText::new(tr("help-click-to-add"))
                            .small()
                            .color(theme::text_faint()),
                    );
                });
            if r.clicked() {
                io.requests
                    .push(PanelRequest::Add(palette::block_candidate(b, None)));
            }
        }
    }
    if !examples.is_empty() {
        ui.add_space(4.0);
        ui.label(
            RichText::new(tr("help-examples"))
                .small()
                .color(theme::text_muted()),
        );
        for ex in examples {
            let r = ui
                .selectable_label(
                    false,
                    RichText::new(format!("{}  {}", icons::FOLDER_OPEN, ex.name))
                        .color(theme::accent()),
                )
                .accessible_name(tr_args("help-example-link", &[("name", (&ex.name).into())]))
                .on_hover_text(tr("help-open-example"));
            if r.clicked() {
                io.requests.push(PanelRequest::OpenExample(ex.path.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{SocketSpec, Unit};

    #[test]
    fn socket_details_read_like_the_reference() {
        let s = SocketSpec::float("freq", Unit::Hz, 440.0).range(20.0, 20000.0);
        assert_eq!(
            detail(&s).as_deref(),
            Some("440 Hz by default · 20 … 20000 Hz")
        );
        assert_eq!(type_label(&s), "Float (Hz)");
        assert_eq!(detail(&SocketSpec::audio("in")), None);
    }
}
