//! The Node tab: the selected node's label, every parameter, its config,
//! and its documentation.

use ac_core::{ConfigExt, NodeId, SignalType, SocketSpec, Value};
use egui::RichText;

use super::{PanelIo, PanelRequest, markdown, section};
use crate::canvas::widgets;
use crate::edit::{self, GROUP_IN, GROUP_OUT, NodeClass};
use crate::editors;
use crate::icons;
use crate::params;
use crate::theme;

/// A config key the Node tab knows how to edit.
struct ConfigField {
    key: &'static str,
    label: &'static str,
    kind: Kind,
}

enum Kind {
    Count {
        min: i64,
        max: i64,
        default: i64,
    },
    Number {
        default: f64,
    },
    Choice {
        choices: &'static [&'static str],
        default: &'static str,
    },
    Text {
        multiline: bool,
    },
    /// An array of strings, edited one per line.
    Lines,
    /// An array of numbers, edited as a comma-separated list.
    Numbers,
    /// A whole number that may be negative (an index, where −1 means none).
    Index {
        min: i64,
        max: i64,
        default: i64,
    },
    /// A number that may be absent (no limit); a checkbox adds or removes it.
    Optional {
        default: f64,
    },
    File,
}

/// Signal types `Select` and `Switch` route.
const ROUTE_TYPES: &[&str] = &["float", "audio", "int", "bool", "trigger", "midi"];

const UNITS: &[&str] = &[
    "generic",
    "hz",
    "db",
    "seconds",
    "beats",
    "ratio",
    "semitones",
    "bpm",
    "samples",
];

/// Config keys of the built-in nodes. Other keys found in a node's config
/// are shown generically by their JSON type.
fn schema(kind: &str) -> Vec<ConfigField> {
    fn f(key: &'static str, label: &'static str, kind: Kind) -> ConfigField {
        ConfigField { key, label, kind }
    }
    match kind {
        "constant" => vec![f(
            "type",
            "Type",
            Kind::Choice {
                choices: &["float", "int", "bool"],
                default: "float",
            },
        )],
        "slider" | "knob" => vec![
            f("min", "Minimum", Kind::Number { default: 0.0 }),
            f("max", "Maximum", Kind::Number { default: 1.0 }),
            f(
                "unit",
                "Unit",
                Kind::Choice {
                    choices: UNITS,
                    default: "generic",
                },
            ),
            f(
                "scale",
                "Scale",
                Kind::Choice {
                    choices: &["linear", "log"],
                    default: "linear",
                },
            ),
        ],
        "comment" => vec![f("text", "Text", Kind::Text { multiline: true })],
        "xy_pad" => vec![
            f("x_min", "X minimum", Kind::Number { default: 0.0 }),
            f("x_max", "X maximum", Kind::Number { default: 1.0 }),
            f("y_min", "Y minimum", Kind::Number { default: 0.0 }),
            f("y_max", "Y maximum", Kind::Number { default: 1.0 }),
        ],
        "keyboard" => vec![
            f(
                "low",
                "Lowest note",
                Kind::Count {
                    min: 0,
                    max: 127,
                    default: 48,
                },
            ),
            f(
                "keys",
                "Keys",
                Kind::Count {
                    min: 1,
                    max: 88,
                    default: 25,
                },
            ),
            f(
                "channel",
                "MIDI channel",
                Kind::Count {
                    min: 1,
                    max: 16,
                    default: 1,
                },
            ),
        ],
        "number" => vec![
            f(
                "type",
                "Type",
                Kind::Choice {
                    choices: &["float", "int"],
                    default: "float",
                },
            ),
            f("min", "Minimum", Kind::Optional { default: 0.0 }),
            f("max", "Maximum", Kind::Optional { default: 1.0 }),
        ],
        "dropdown" => vec![f("choices", "Choices (one per line)", Kind::Lines)],
        "select" => vec![
            f(
                "inputs",
                "Inputs",
                Kind::Count {
                    min: 1,
                    max: 64,
                    default: 4,
                },
            ),
            f(
                "type",
                "Type",
                Kind::Choice {
                    choices: ROUTE_TYPES,
                    default: "float",
                },
            ),
        ],
        "switch" => vec![f(
            "type",
            "Type",
            Kind::Choice {
                choices: ROUTE_TYPES,
                default: "float",
            },
        )],
        "lfo" => vec![f(
            "unit",
            "Rate in",
            Kind::Choice {
                choices: &["hz", "beats"],
                default: "hz",
            },
        )],
        "envelope" => vec![
            f(
                "sustain",
                "Sustain point (−1 none)",
                Kind::Index {
                    min: -1,
                    max: 63,
                    default: ac_nodes::modulation::envelope::DEFAULT_SUSTAIN,
                },
            ),
            f(
                "loop_start",
                "Loop from point",
                Kind::Index {
                    min: -1,
                    max: 63,
                    default: -1,
                },
            ),
            f(
                "loop_end",
                "Loop to point",
                Kind::Index {
                    min: -1,
                    max: 63,
                    default: -1,
                },
            ),
        ],
        "step_sequencer" => vec![f(
            "steps",
            "Steps",
            Kind::Count {
                min: 1,
                max: 64,
                default: 8,
            },
        )],
        "midi_cc" => vec![f(
            "initial",
            "Start at (0–1 of the range)",
            Kind::Number { default: 0.0 },
        )],
        "scale_quantize" => vec![f(
            "type",
            "Quantize",
            Kind::Choice {
                choices: &["float", "midi"],
                default: "float",
            },
        )],
        "chord" => vec![f(
            "intervals",
            "Custom intervals (semitones)",
            Kind::Numbers,
        )],
        "pattern" => vec![
            f(
                "steps",
                "Steps",
                Kind::Count {
                    min: 1,
                    max: 64,
                    default: 16,
                },
            ),
            f(
                "rows",
                "Rows",
                Kind::Count {
                    min: 1,
                    max: 16,
                    default: 4,
                },
            ),
        ],
        "midi_pattern" => vec![
            f("length", "Length (beats)", Kind::Number { default: 4.0 }),
            f(
                "channel",
                "MIDI channel",
                Kind::Count {
                    min: 1,
                    max: 16,
                    default: 1,
                },
            ),
        ],
        "random_walk" => vec![f(
            "seed",
            "Seed",
            Kind::Count {
                min: 0,
                max: 1_000_000,
                default: 0,
            },
        )],
        "trigger_delay" | "trigger_to_gate" => vec![f(
            "unit",
            "Time in",
            Kind::Choice {
                choices: &["seconds", "beats"],
                default: "seconds",
            },
        )],
        "audio_out" => vec![
            f(
                "channel",
                "First output",
                Kind::Count {
                    min: 0,
                    max: 63,
                    default: 0,
                },
            ),
            f(
                "channels",
                "Channels",
                Kind::Count {
                    min: 1,
                    max: 2,
                    default: 2,
                },
            ),
        ],
        "audio_in" => vec![
            f(
                "channel",
                "First input",
                Kind::Count {
                    min: 0,
                    max: 63,
                    default: 0,
                },
            ),
            f(
                "channels",
                "Channels",
                Kind::Count {
                    min: 1,
                    max: 2,
                    default: 1,
                },
            ),
        ],
        "record" => vec![f(
            "channels",
            "Channels",
            Kind::Count {
                min: 1,
                max: 32,
                default: 2,
            },
        )],
        "file_out" => vec![
            f(
                "path",
                "File (offline renders)",
                Kind::Text { multiline: false },
            ),
            f(
                "channels",
                "Channels",
                Kind::Count {
                    min: 1,
                    max: 2,
                    default: 2,
                },
            ),
        ],
        "split_channels" | "merge_channels" => vec![f(
            "channels",
            "Channels",
            Kind::Count {
                min: 1,
                max: 32,
                default: 2,
            },
        )],
        "mix" => vec![f(
            "inputs",
            "Inputs",
            Kind::Count {
                min: 1,
                max: 64,
                default: 2,
            },
        )],
        "clock" => vec![f(
            "unit",
            "Interval in",
            Kind::Choice {
                choices: &["beats", "seconds"],
                default: "beats",
            },
        )],
        "clip" => vec![f("file", "File", Kind::File)],
        "voices" => vec![
            f(
                "voices",
                "Voices",
                Kind::Count {
                    min: 1,
                    max: 64,
                    default: 8,
                },
            ),
            f(
                "steal",
                "When all are busy",
                Kind::Choice {
                    choices: ac_nodes::sequencing::voices::STEAL_CHOICES,
                    default: "oldest",
                },
            ),
        ],
        "reverse" => vec![f(
            "segment",
            "Slice (s, 0 = all)",
            Kind::Number { default: 0.0 },
        )],
        "latency" => vec![
            f(
                "samples",
                "Samples",
                Kind::Count {
                    min: 0,
                    max: 960_000,
                    default: 0,
                },
            ),
            f(
                "channels",
                "Channels",
                Kind::Count {
                    min: 1,
                    max: 32,
                    default: 2,
                },
            ),
        ],
        _ => vec![],
    }
}

pub(super) fn show(ui: &mut egui::Ui, io: &mut PanelIo, label_edit: &mut Option<(NodeId, String)>) {
    let sel: Vec<NodeId> = io.doc.selection.iter().copied().collect();
    let [id] = sel.as_slice() else {
        if sel.is_empty() {
            empty_state(ui, io);
        } else {
            ui.label(
                RichText::new(format!("{} nodes selected", sel.len())).color(theme::TEXT_MUTED),
            );
        }
        return;
    };
    let id = *id;
    if id == GROUP_IN || id == GROUP_OUT {
        interface(ui, io, id == GROUP_IN);
        return;
    }
    let Some(node) = io.doc.graph().node(id).cloned() else {
        return;
    };
    let class = edit::classify(&node, io.registry, &io.doc.project.group_defs);
    let entry = io.registry.get(&node.kind);
    let (name, color) = match &class {
        NodeClass::Group(d) => (
            io.doc
                .project
                .group_defs
                .get(d)
                .map_or("Group".into(), |d| d.name.clone()),
            theme::GROUP_HEADER,
        ),
        NodeClass::Missing(_) => (node.kind.clone(), theme::MISSING_HEADER),
        _ => (
            entry.map_or(node.kind.clone(), |e| e.meta.name.to_string()),
            entry.map_or(theme::MISSING_HEADER, |e| {
                theme::category_color(e.meta.category)
            }),
        ),
    };

    // Title: the kind, its category, and the editable label.
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(egui::vec2(4.0, 22.0), egui::Sense::hover());
        ui.painter().rect_filled(r, theme::radius(2.0), color);
        ui.label(
            RichText::new(&name)
                .size(theme::FONT_HEADING)
                .color(theme::TEXT_STRONG),
        );
        if let Some(e) = entry {
            ui.label(
                RichText::new(e.meta.category.name())
                    .small()
                    .color(theme::TEXT_MUTED),
            );
        }
    });
    ui.label(
        RichText::new(format!("{} · id {}", node.kind, node.id))
            .small()
            .color(theme::TEXT_FAINT),
    );
    if let NodeClass::Missing(m) = &class {
        ui.colored_label(theme::WARNING, format!("{} {m}", icons::WARNING));
        ui.label("The node is kept in the project with its settings and wires, but does nothing.");
    }

    ui.add_space(4.0);
    let current = node.ui.label.clone().unwrap_or_default();
    let field_id = egui::Id::unique(("node-label", id));
    let focused = ui.memory(|m| m.has_focus(field_id));
    // Follow the document unless the user is typing here.
    if !focused
        && label_edit
            .as_ref()
            .is_none_or(|(n, t)| *n != id || *t != current)
    {
        *label_edit = Some((id, current.clone()));
    }
    if let Some((_, text)) = label_edit.as_mut() {
        let r = ui.add(
            egui::TextEdit::singleline(text)
                .id(field_id)
                .hint_text(format!("Label (default: {name})"))
                .desired_width(f32::INFINITY),
        );
        if r.lost_focus() && text.trim() != current {
            let label = text.trim().to_string();
            io.doc.edit(io.registry, "Rename", |g, _| {
                if let Some(n) = g.node_mut(id) {
                    n.ui.label = (!label.is_empty()).then_some(label);
                }
            });
        }
    }

    ui.horizontal(|ui| {
        let mut muted = node.muted;
        if ui.checkbox(&mut muted, "Muted").changed() {
            io.doc.edit(io.registry, "Mute", |g, _| {
                if let Some(n) = g.node_mut(id) {
                    n.muted = muted;
                }
            });
        }
        if let NodeClass::Group(_) = class
            && ui
                .button(format!("{}  Enter group", icons::ARROW_SQUARE_IN))
                .clicked()
        {
            io.doc.enter_group(id);
        }
    });

    // Parameters.
    let sockets = edit::sockets_of(
        &io.doc.display_graph(),
        id,
        io.registry,
        &io.doc.project.group_defs,
        io.doc.interface().as_ref(),
    );
    if !sockets.inputs.is_empty() {
        section(ui, "Inputs");
        let graph = io.doc.display_graph();
        egui::Grid::new(("node-params", id))
            .num_columns(2)
            .spacing(egui::vec2(10.0, 6.0))
            .show(ui, |ui| {
                for s in &sockets.inputs {
                    let wired = graph.wire_into(&ac_core::Endpoint::new(id, s.name.clone()));
                    ui.horizontal(|ui| {
                        let (r, _) =
                            ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                        ui.painter()
                            .circle_filled(r.center(), 3.5, theme::type_color(s.ty));
                        let label = ui.label(crate::canvas::paint_pretty(&s.name));
                        if let Some(doc) = s.spec.as_ref().map(|s| s.doc).filter(|d| !d.is_empty())
                        {
                            label.on_hover_text(doc);
                        }
                    });
                    if let Some(w) = wired {
                        let src = io.doc.graph().node(w.from.node).map_or_else(
                            || w.from.node.to_string(),
                            |n| {
                                n.ui.label.clone().unwrap_or_else(|| {
                                    io.registry
                                        .get(&n.kind)
                                        .map_or(n.kind.clone(), |e| e.meta.name.to_string())
                                })
                            },
                        );
                        let from = if w.from.node == GROUP_IN {
                            "group input".to_string()
                        } else {
                            src
                        };
                        ui.label(
                            RichText::new(format!("← {from}.{}", w.from.socket))
                                .color(theme::TEXT_MUTED),
                        );
                    } else if let Some(spec) = &s.spec {
                        param_widget(ui, io, id, &node, spec);
                    } else {
                        ui.label(RichText::new("—").color(theme::TEXT_FAINT));
                    }
                    ui.end_row();
                }
            });
    }
    if !sockets.outputs.is_empty() {
        section(ui, "Outputs");
        let tele = io.engine.handle.telemetry();
        let path = io.doc.node_path(id);
        for s in &sockets.outputs {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter()
                    .circle_filled(r.center(), 3.5, theme::type_color(s.ty));
                ui.label(crate::canvas::paint_pretty(&s.name));
                ui.label(
                    RichText::new(s.ty.name())
                        .small()
                        .color(theme::type_color(s.ty)),
                );
                let reading = match s.ty {
                    SignalType::Audio => tele.socket_level(&path, &s.name).map(|l| {
                        if l < 1e-6 {
                            "silent".into()
                        } else {
                            format!("{:.1} dBFS", 20.0 * l.log10())
                        }
                    }),
                    SignalType::Trigger | SignalType::Midi => tele
                        .socket_events(&path, &s.name)
                        .map(|c| format!("{c} events")),
                    ty => tele
                        .socket_value(&path, &s.name)
                        .map(|v| widgets::live_text(v, ty, s.spec.as_ref())),
                };
                if let Some(r) = reading {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(r)
                                .font(theme::mono_font(theme::FONT_VALUE))
                                .color(theme::TEXT_MUTED),
                        );
                    });
                }
            });
        }
    }

    config_fields(ui, io, id, &node);
    node_editor(ui, io, id, &node);

    if let NodeClass::Group(d) = &class
        && let Some(def) = io.doc.project.group_defs.get(d).cloned()
    {
        section(ui, "Group definition");
        let mut name = def.name.clone();
        ui.horizontal(|ui| {
            ui.label("Name");
            if ui.text_edit_singleline(&mut name).lost_focus()
                && name.trim() != def.name
                && !name.trim().is_empty()
            {
                let (d, name) = (d.clone(), name.trim().to_string());
                io.doc.edit_project("Rename group", |p| {
                    if let Some(def) = p.group_defs.get_mut(&d) {
                        def.name = name;
                    }
                });
            }
        });
        let instances = edit::group::instance_paths(&io.doc.project, d).len();
        ui.label(
            RichText::new(format!(
                "{} inputs · {} outputs · {} nodes inside · {} instance{}. Editing the inside changes every instance.",
                def.inputs.len(),
                def.outputs.len(),
                def.graph.nodes.len(),
                instances,
                if instances == 1 { "" } else { "s" }
            ))
            .small()
            .color(theme::TEXT_MUTED),
        );
        let mut poly = node.kind == ac_core::VOICES_KIND;
        if ui
            .checkbox(&mut poly, "Polyphonic (one copy per voice)")
            .on_hover_text(
                "Runs the inside once per voice and shares notes from the `midi` input \
                 among the copies. Put a Voice node inside to read each copy's note.",
            )
            .changed()
        {
            io.doc.edit(io.registry, "Polyphonic", |g, _| {
                edit::group::set_polyphonic(g, id, poly);
            });
        }
        if !def.description.is_empty() {
            markdown::show(ui, &def.description);
        }
    }

    // Documentation.
    if let Some(e) = entry {
        section(ui, "Help");
        markdown::show(ui, &e.meta.doc_text());
    }
}

fn empty_state(ui: &mut egui::Ui, io: &PanelIo) {
    ui.add_space(20.0);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(icons::CUBE)
                .size(34.0)
                .color(theme::TEXT_FAINT),
        );
        ui.label(RichText::new("Nothing selected").color(theme::TEXT_MUTED));
        ui.add_space(6.0);
        let hint = if io.doc.graph().nodes.is_empty() {
            "Press Shift+A or double-click the canvas to add a node."
        } else {
            "Click a node to see its settings and help."
        };
        ui.label(RichText::new(hint).small().color(theme::TEXT_FAINT));
    });
}

/// The same widget the canvas shows, at panel size.
fn param_widget(
    ui: &mut egui::Ui,
    io: &mut PanelIo,
    id: NodeId,
    node: &ac_core::NodeInstance,
    spec: &SocketSpec,
) {
    let socket = spec.name.to_string();
    match widgets::field(spec) {
        widgets::Field::Number => {
            let mut v = widgets::number(node, spec);
            let fmt = spec.clone();
            let changed = match spec.range {
                Some((a, b)) => {
                    let log = spec.scale == ac_core::Scale::Log
                        || (spec.unit == ac_core::Unit::Hz && a > 0.0);
                    let mut s = egui::Slider::new(&mut v, a..=b)
                        .logarithmic(log)
                        .clamping(egui::SliderClamping::Never)
                        .custom_formatter(move |x, _| widgets::format(x, &fmt));
                    if spec.ty == SignalType::Int {
                        s = s.integer();
                    }
                    ui.add(s)
                }
                None => {
                    let speed = 0.01 * (v.abs() + 1.0);
                    let mut d = egui::DragValue::new(&mut v).speed(speed);
                    let fmt2 = spec.clone();
                    d = d.custom_formatter(move |x, _| widgets::format(x, &fmt2));
                    let parse_spec = spec.clone();
                    d = d.custom_parser(move |t| {
                        widgets::parse(t, &parse_spec).and_then(|v| v.as_f64())
                    });
                    ui.add(d)
                }
            };
            if changed.changed() {
                let value = if spec.ty == SignalType::Int {
                    Value::Int(v.round() as i64)
                } else {
                    Value::Float(v)
                };
                params::set(io.doc, &mut io.engine.handle, id, &socket, value, io.now);
            }
            if changed.drag_stopped() || changed.lost_focus() {
                io.doc.end_coalescing();
            }
        }
        widgets::Field::Choice => {
            let current = widgets::choice(node, spec);
            egui::ComboBox::from_id_salt(("param-choice", id, &socket))
                .selected_text(widgets::choice_label(spec, &current))
                .show_ui(ui, |ui| {
                    for c in spec.choices {
                        if ui
                            .selectable_label(current == *c, widgets::choice_label(spec, c))
                            .clicked()
                        {
                            params::set(
                                io.doc,
                                &mut io.engine.handle,
                                id,
                                &socket,
                                Value::Text(c.to_string()),
                                io.now,
                            );
                            io.doc.end_coalescing();
                        }
                    }
                });
        }
        widgets::Field::Check => {
            let mut on = widgets::value(node, spec).as_bool().unwrap_or(false);
            if ui.checkbox(&mut on, "").changed() {
                params::set(
                    io.doc,
                    &mut io.engine.handle,
                    id,
                    &socket,
                    Value::Bool(on),
                    io.now,
                );
                io.doc.end_coalescing();
            }
        }
        widgets::Field::Fire => {
            if ui.button(format!("{} Fire", icons::LIGHTNING)).clicked() {
                params::fire(io.doc, &mut io.engine.handle, id, &socket);
            }
        }
        widgets::Field::Label => {
            ui.label(RichText::new("unwired").color(theme::TEXT_FAINT));
        }
    }
}

/// The node's own editor (curve, grid, piano roll, bands, steps), and a
/// larger copy in a window.
fn node_editor(ui: &mut egui::Ui, io: &mut PanelIo, id: NodeId, node: &ac_core::NodeInstance) {
    let Some(editor) = editors::editor_for(&node.kind, &node.config) else {
        return;
    };
    section(ui, "Editor");
    if node.kind == "shaper" {
        ui.label(
            RichText::new("The curve shapes the signal when `shape` is custom.")
                .small()
                .color(theme::TEXT_MUTED),
        );
    }
    let width = ui.available_width().max(120.0);
    let edit = editors::show(
        ui,
        editor,
        &node.config,
        ("panel", id),
        width,
        editor.height(width),
    );
    apply_editor(io, id, editor, edit);
    let open_id = egui::Id::unique(("node-editor-window", id));
    let mut open = ui.data(|d| d.get_temp::<bool>(open_id).unwrap_or(false));
    ui.horizontal(|ui| {
        if ui
            .button(format!("{}  Larger editor", icons::ARROW_SQUARE_OUT))
            .clicked()
        {
            open = !open;
        }
        ui.label(
            RichText::new("Double-click to add, right-click to change or remove.")
                .small()
                .color(theme::TEXT_FAINT),
        );
    });
    if open {
        let title = node
            .ui
            .label
            .clone()
            .or_else(|| io.registry.get(&node.kind).map(|e| e.meta.name.to_string()))
            .unwrap_or_else(|| node.kind.clone());
        let mut edit = None;
        egui::Window::new(title)
            .id(egui::Id::unique(("node-editor-window-frame", id)))
            .open(&mut open)
            .default_size(egui::vec2(720.0, 420.0))
            .resizable(true)
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                let size = ui.available_size();
                let w = size.x.max(200.0);
                let h = (size.y - 30.0).max(editor.height(w).min(200.0));
                edit = editors::show(ui, editor, &node.config, ("window", id), w, h);
            });
        apply_editor(io, id, editor, edit);
    }
    ui.data_mut(|d| d.insert_temp(open_id, open));
}

fn apply_editor(
    io: &mut PanelIo,
    id: NodeId,
    editor: editors::Editor,
    edit: Option<editors::Edit>,
) {
    if let Some(e) = edit {
        io.doc.set_config(id, editor.key(), e.value, io.now);
        if e.done {
            io.doc.end_coalescing();
        }
    }
}

fn set_config(io: &mut PanelIo, id: NodeId, key: &str, value: serde_json::Value) {
    let key = key.to_string();
    io.doc.edit(io.registry, "Change setting", |g, _| {
        if let Some(n) = g.node_mut(id) {
            if !n.config.is_object() {
                n.config = ac_core::empty_config();
            }
            if let Some(obj) = n.config.as_object_mut() {
                obj.insert(key, value);
            }
        }
    });
}

fn remove_config(io: &mut PanelIo, id: NodeId, key: &str) {
    io.doc.edit(io.registry, "Change setting", |g, _| {
        if let Some(o) = g.node_mut(id).and_then(|n| n.config.as_object_mut()) {
            o.remove(key);
        }
    });
}

fn config_fields(ui: &mut egui::Ui, io: &mut PanelIo, id: NodeId, node: &ac_core::NodeInstance) {
    let known = schema(&node.kind);
    let extra: Vec<(String, serde_json::Value)> = node
        .config
        .as_object()
        .map(|o| {
            o.iter()
                .filter(|(k, _)| !known.iter().any(|f| f.key == k.as_str()))
                .filter(|(k, _)| {
                    !(node.kind == "clip" && (k.as_str() == "channels" || k.as_str() == "hash"))
                })
                .filter(|(k, _)| node.kind != "group" || k.as_str() != "def")
                // The node editor shows its own key.
                .filter(|(k, _)| {
                    editors::editor_for(&node.kind, &node.config)
                        .is_none_or(|e| e.key() != k.as_str())
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();
    if known.is_empty() && extra.is_empty() {
        return;
    }
    section(ui, "Settings");
    egui::Grid::new(("node-config", id))
        .num_columns(2)
        .spacing(egui::vec2(10.0, 6.0))
        .show(ui, |ui| {
            for f in &known {
                ui.label(f.label);
                match &f.kind {
                    Kind::Count { min, max, default } => {
                        let mut v = node.config.get_u64_or(f.key, *default as u64) as i64;
                        if ui
                            .add(egui::DragValue::new(&mut v).range(*min..=*max).speed(0.05))
                            .changed()
                        {
                            set_config(io, id, f.key, v.into());
                        }
                    }
                    Kind::Number { default } => {
                        let mut v = node.config.get_f64_or(f.key, *default);
                        let speed = 0.01 * (v.abs() + 1.0);
                        let r = ui.add(egui::DragValue::new(&mut v).speed(speed));
                        if r.changed() {
                            set_config(io, id, f.key, v.into());
                        }
                    }
                    Kind::Choice { choices, default } => {
                        let cur = node.config.get_str_or(f.key, default).to_string();
                        egui::ComboBox::from_id_salt(("cfg", id, f.key))
                            .selected_text(&cur)
                            .show_ui(ui, |ui| {
                                for c in *choices {
                                    if ui.selectable_label(cur == *c, *c).clicked() {
                                        set_config(io, id, f.key, (*c).into());
                                    }
                                }
                            });
                    }
                    Kind::Text { multiline } => {
                        let mut t = node.config.get_str_or(f.key, "").to_string();
                        let r = if *multiline {
                            ui.add(
                                egui::TextEdit::multiline(&mut t)
                                    .desired_rows(4)
                                    .desired_width(f32::INFINITY),
                            )
                        } else {
                            ui.text_edit_singleline(&mut t)
                        };
                        if r.lost_focus() && t != node.config.get_str_or(f.key, "") {
                            set_config(io, id, f.key, t.into());
                        }
                    }
                    Kind::Index { min, max, default } => {
                        let mut v = node
                            .config
                            .get(f.key)
                            .and_then(serde_json::Value::as_i64)
                            .unwrap_or(*default);
                        if ui
                            .add(egui::DragValue::new(&mut v).range(*min..=*max).speed(0.05))
                            .changed()
                        {
                            set_config(io, id, f.key, v.into());
                        }
                    }
                    Kind::Numbers => {
                        let stored: Vec<f64> = node
                            .config
                            .get(f.key)
                            .and_then(|v| v.as_array())
                            .map(|a| a.iter().filter_map(serde_json::Value::as_f64).collect())
                            .unwrap_or_default();
                        let fmt = |v: &f64| {
                            if v.fract() == 0.0 {
                                format!("{v:.0}")
                            } else {
                                format!("{v}")
                            }
                        };
                        let mut t = stored.iter().map(fmt).collect::<Vec<_>>().join(", ");
                        let r = ui.add(egui::TextEdit::singleline(&mut t).hint_text("0, 4, 7"));
                        if r.lost_focus() {
                            let parsed: Vec<f64> = t
                                .split([',', ' '])
                                .filter_map(|x| x.trim().parse::<f64>().ok())
                                .filter(|v| v.is_finite())
                                .collect();
                            if parsed != stored {
                                let ints = parsed.iter().all(|v| v.fract() == 0.0);
                                let value: serde_json::Value = if ints {
                                    parsed.iter().map(|v| *v as i64).collect::<Vec<_>>().into()
                                } else {
                                    parsed.into()
                                };
                                set_config(io, id, f.key, value);
                            }
                        }
                    }
                    Kind::Lines => {
                        let stored: Vec<String> = node
                            .config
                            .get(f.key)
                            .and_then(|v| v.as_array())
                            .map(|a| {
                                a.iter()
                                    .map(|v| match v {
                                        serde_json::Value::String(s) => s.clone(),
                                        other => other.to_string(),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        let mut t = stored.join("\n");
                        let r = ui.add(
                            egui::TextEdit::multiline(&mut t)
                                .desired_rows(3)
                                .desired_width(f32::INFINITY),
                        );
                        if r.lost_focus() {
                            let lines: Vec<String> = t
                                .lines()
                                .map(str::trim)
                                .filter(|l| !l.is_empty())
                                .map(str::to_string)
                                .collect();
                            if lines != stored {
                                set_config(io, id, f.key, lines.into());
                            }
                        }
                    }
                    Kind::Optional { default } => {
                        let current = node.config.get(f.key).and_then(|v| v.as_f64());
                        ui.horizontal(|ui| {
                            let mut on = current.is_some();
                            if ui.checkbox(&mut on, "").changed() {
                                if on {
                                    set_config(io, id, f.key, (*default).into());
                                } else {
                                    remove_config(io, id, f.key);
                                }
                            }
                            match current {
                                Some(mut v) => {
                                    let speed = 0.01 * (v.abs() + 1.0);
                                    if ui.add(egui::DragValue::new(&mut v).speed(speed)).changed() {
                                        set_config(io, id, f.key, v.into());
                                    }
                                }
                                None => {
                                    ui.label(RichText::new("none").color(theme::TEXT_FAINT));
                                }
                            }
                        });
                    }
                    Kind::File => {
                        let file = node.config.get_str_or(f.key, "").to_string();
                        ui.vertical(|ui| {
                            let missing = !file.is_empty() && io.missing.contains(&file);
                            if file.is_empty() {
                                ui.label(RichText::new("no file").color(theme::TEXT_FAINT));
                            } else if missing {
                                ui.colored_label(
                                    theme::ERROR,
                                    format!("{} {file} (missing)", icons::FILE_X),
                                );
                            } else {
                                ui.label(format!("{} {file}", icons::FILE_AUDIO));
                            }
                            ui.horizontal(|ui| {
                                if ui.button("Choose…").clicked() {
                                    io.requests.push(PanelRequest::ChooseClipFile(id));
                                }
                                if missing && ui.button("Locate…").clicked() {
                                    io.requests.push(PanelRequest::Locate(file.clone()));
                                }
                            });
                            let ch = node.config.get_u64_or("channels", 0);
                            if ch > 0 {
                                ui.label(
                                    RichText::new(format!(
                                        "{ch} channel{}",
                                        if ch == 1 { "" } else { "s" }
                                    ))
                                    .small()
                                    .color(theme::TEXT_MUTED),
                                );
                            }
                        });
                    }
                }
                ui.end_row();
            }
            for (k, v) in extra {
                ui.label(&k);
                match v {
                    serde_json::Value::Bool(mut b) => {
                        if ui.checkbox(&mut b, "").changed() {
                            set_config(io, id, &k, b.into());
                        }
                    }
                    serde_json::Value::Number(n) => {
                        let mut x = n.as_f64().unwrap_or(0.0);
                        let int = n.is_i64() || n.is_u64();
                        let r = ui.add(egui::DragValue::new(&mut x).speed(if int {
                            0.05
                        } else {
                            0.01
                        }));
                        if r.changed() {
                            let v: serde_json::Value = if int {
                                (x.round() as i64).into()
                            } else {
                                x.into()
                            };
                            set_config(io, id, &k, v);
                        }
                    }
                    serde_json::Value::String(mut s) => {
                        let before = s.clone();
                        if ui.text_edit_singleline(&mut s).lost_focus() && s != before {
                            set_config(io, id, &k, s.into());
                        }
                    }
                    other => {
                        ui.label(
                            RichText::new(other.to_string())
                                .font(theme::mono_font(theme::FONT_SMALL))
                                .color(theme::TEXT_MUTED),
                        );
                    }
                }
                ui.end_row();
            }
        });
}

/// Selected interface card: list and rename its sockets.
fn interface(ui: &mut egui::Ui, io: &mut PanelIo, input_side: bool) {
    let Some(def) = io.doc.current_group().cloned() else {
        return;
    };
    ui.label(
        RichText::new(if input_side {
            "Group Input"
        } else {
            "Group Output"
        })
        .size(theme::FONT_HEADING)
        .color(theme::TEXT_STRONG),
    );
    ui.label(RichText::new(format!("Interface of “{}”", def.name)).color(theme::TEXT_MUTED));
    section(ui, if input_side { "Inputs" } else { "Outputs" });
    let names: Vec<(String, SignalType)> = if input_side {
        def.inputs.iter().map(|i| (i.name.clone(), i.ty)).collect()
    } else {
        def.outputs.iter().map(|o| (o.name.clone(), o.ty)).collect()
    };
    if names.is_empty() {
        ui.label(RichText::new("None yet. Drag a wire to the empty socket, or right-click an inner socket and choose Expose.").color(theme::TEXT_MUTED));
    }
    for (name, ty) in names {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(r.center(), 3.5, theme::type_color(ty));
            let mut text = name.clone();
            if ui
                .add(egui::TextEdit::singleline(&mut text).desired_width(140.0))
                .lost_focus()
                && text != name
            {
                io.doc
                    .rename_interface_socket(io.registry, input_side, &name, &text);
            }
            ui.label(
                RichText::new(ty.name())
                    .small()
                    .color(theme::type_color(ty)),
            );
            if ui
                .small_button(icons::X)
                .on_hover_text("Remove from the interface")
                .clicked()
            {
                let n = name.clone();
                io.doc.edit(io.registry, "Remove group socket", |g, ctx| {
                    edit::remove_interface_socket(g, ctx, input_side, &n);
                });
            }
        });
    }
}
