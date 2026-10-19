//! The Node tab: the selected node's label, every parameter, its config,
//! and its documentation.

use ac_core::{
    BlockSource, ConfigExt, GroupDef, GroupId, NodeId, Origin, SignalType, SocketSpec, Value,
};
use egui::RichText;

use super::{PanelIo, PanelRequest, markdown, section, set_config};
use crate::canvas::widgets;
use crate::edit::{self, GROUP_IN, GROUP_OUT, NodeClass};
use crate::editors;
use crate::i18n::{self, tr, tr_args};
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
    /// One line of text with a default the node uses while the key is
    /// absent (shown in the field, so what's seen is what runs). A list of
    /// names stored as an array shows comma-separated.
    Line {
        default: &'static str,
    },
    /// An array of strings, edited one per line.
    Lines,
    /// An array of numbers, edited as a comma-separated list.
    Numbers,
    /// A number picked from a list (stored as a number, shown as text).
    NumberChoice {
        choices: &'static [&'static str],
        default: &'static str,
    },
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
    /// On or off, as a checkbox.
    Flag {
        default: bool,
    },
}

/// FFT sizes the spectral nodes offer (others are rounded to a power of two).
const FFT_SIZES: &[&str] = &["256", "512", "1024", "2048", "4096", "8192", "16384"];
const WINDOWS: &[&str] = &["hann", "hamming", "blackman", "rectangular"];

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
        "merge" => vec![
            f(
                "inputs",
                "Inputs",
                Kind::Count {
                    min: 2,
                    max: 16,
                    default: 2,
                },
            ),
            f(
                "type",
                "Type",
                Kind::Choice {
                    choices: &["midi", "trigger"],
                    default: "midi",
                },
            ),
        ],
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
        "noise" => vec![f(
            "seed",
            "Seed",
            Kind::Count {
                min: 0,
                max: 1_000_000,
                default: 0,
            },
        )],
        "graph" => vec![f(
            "unit",
            "Duration in",
            Kind::Choice {
                choices: &["seconds", "beats"],
                default: "seconds",
            },
        )],
        "sampler" => vec![
            f("file", "File (when `buffer` is unwired)", Kind::File),
            f(
                "channels",
                "Channels",
                Kind::Count {
                    min: 1,
                    max: 64,
                    default: 2,
                },
            ),
            f(
                "root",
                "Root note (plays at its own pitch)",
                Kind::Number { default: 60.0 },
            ),
        ],
        "wavetable" => vec![
            f(
                "table",
                "Table",
                Kind::Choice {
                    choices: &["basic", "buffer"],
                    default: "basic",
                },
            ),
            f(
                "frame_size",
                "Frame size (buffer)",
                Kind::Count {
                    min: 32,
                    max: 8192,
                    default: 2048,
                },
            ),
            f(
                "max_frames",
                "Most frames (buffer)",
                Kind::Count {
                    min: 1,
                    max: 256,
                    default: 64,
                },
            ),
        ],
        "lookahead" => vec![f("time", "Look ahead (s)", Kind::Number { default: 0.005 })],
        "normalize" => vec![f(
            "mode",
            "Measure",
            Kind::Choice {
                choices: &["peak", "lufs"],
                default: "peak",
            },
        )],
        "comb" | "allpass" => vec![f(
            "max_delay",
            "Longest delay (s)",
            Kind::Number { default: 0.1 },
        )],
        "freeze" => vec![f("max_time", "Memory (s)", Kind::Number { default: 4.0 })],
        "pitch_shift" => vec![f(
            "max_grain",
            "Longest grain (s, sets latency)",
            Kind::Number { default: 0.1 },
        )],
        "mid_side" => vec![f(
            "mode",
            "Mode",
            Kind::Choice {
                choices: &["encode", "decode"],
                default: "encode",
            },
        )],
        "fft" => vec![
            f(
                "size",
                "Size",
                Kind::NumberChoice {
                    choices: FFT_SIZES,
                    default: "2048",
                },
            ),
            f(
                "overlap",
                "Overlap",
                Kind::NumberChoice {
                    choices: &["1", "2", "4", "8"],
                    default: "4",
                },
            ),
            f(
                "window",
                "Window",
                Kind::Choice {
                    choices: WINDOWS,
                    default: "hann",
                },
            ),
        ],
        "ifft" => vec![f(
            "window",
            "Window (match the FFT)",
            Kind::Choice {
                choices: WINDOWS,
                default: "hann",
            },
        )],
        "band_split" => vec![
            f(
                "mode",
                "Split with",
                Kind::Choice {
                    choices: &["crossover", "fft"],
                    default: "crossover",
                },
            ),
            f(
                "size",
                "FFT size (fft mode)",
                Kind::NumberChoice {
                    choices: FFT_SIZES,
                    default: "2048",
                },
            ),
        ],
        "pitch_detect" => vec![
            f(
                "window",
                "Window (samples)",
                Kind::NumberChoice {
                    choices: FFT_SIZES,
                    default: "2048",
                },
            ),
            f(
                "hop",
                "Hop (samples)",
                Kind::Count {
                    min: 16,
                    max: 16_384,
                    default: 256,
                },
            ),
        ],
        "analyzer" => vec![f(
            "size",
            "FFT size",
            Kind::NumberChoice {
                choices: FFT_SIZES,
                default: "4096",
            },
        )],
        "frequency_trigger" => vec![f(
            "mode",
            "Listen for",
            Kind::Choice {
                choices: &["band", "pitch"],
                default: "band",
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
        // The address, host, and port are inputs (text fields on the node).
        "osc_in" | "osc_out" => vec![f(
            "args",
            "Arguments",
            Kind::Count {
                min: 1,
                max: ac_core::control::OSC_ARGS_MAX as i64,
                default: 1,
            },
        )],
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
        "oscillator" => vec![f(
            "pure",
            "Pure (no anti-aliasing, for LFOs)",
            Kind::Flag { default: false },
        )],
        "delay" => vec![
            f(
                "unit",
                "Time in",
                Kind::Choice {
                    choices: &["seconds", "beats"],
                    default: "seconds",
                },
            ),
            f(
                "max_time",
                "Longest delay (s)",
                Kind::Number { default: 4.0 },
            ),
        ],
        "gamepad" => vec![
            f(
                "pad",
                "Gamepad (1 is the first)",
                Kind::Count {
                    min: 1,
                    max: 16,
                    default: 1,
                },
            ),
            f(
                "releases",
                "Release triggers",
                Kind::Flag { default: false },
            ),
        ],
        "keyboard_in" => vec![
            f(
                "keys",
                "Keys (names, comma-separated)",
                Kind::Line {
                    default: "1, 2, 3, 4",
                },
            ),
            f(
                "low",
                "Note of the A key",
                Kind::Count {
                    min: 0,
                    max: 127,
                    default: 60,
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
                RichText::new(tr_args("node-tab-selected", &[("count", sel.len().into())]))
                    .color(theme::text_muted()),
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
                .map_or_else(|| tr("node-tab-group"), |d| d.name.clone()),
            theme::group_header(),
        ),
        NodeClass::Missing(_) => (node.kind.clone(), theme::missing_header()),
        _ => (
            entry.map_or(node.kind.clone(), |e| e.meta.name.to_string()),
            entry.map_or(theme::missing_header(), |e| {
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
                .color(theme::text_strong()),
        );
        if let Some(e) = entry {
            ui.label(
                RichText::new(i18n::category(e.meta.category))
                    .small()
                    .color(theme::text_muted()),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::spoken(ui.small_button(format!(
                    "{}  {}",
                    icons::BOOK_OPEN,
                    tr("node-tab-help")
                )))
                .on_hover_text(tr("node-tab-help-tip"))
                .clicked()
                {
                    io.requests
                        .push(PanelRequest::Help(e.meta.kind.to_string()));
                }
            });
        }
    });
    ui.label(
        RichText::new(format!("{} · id {}", node.kind, node.id))
            .small()
            .color(theme::text_faint()),
    );
    if let NodeClass::Missing(m) = &class {
        ui.colored_label(theme::warning(), format!("{} {m}", icons::WARNING));
        ui.label(tr("node-tab-missing"));
    }

    ui.add_space(4.0);
    // A portal tag's label is the portal's name, shared by all its tags.
    let portal = edit::portal::tag(&node);
    if let Some(t) = &portal {
        let role = if t.send {
            tr("node-tab-portal-sender")
        } else {
            tr("node-tab-portal-receiver")
        };
        ui.label(RichText::new(role).small().color(theme::text_muted()));
    }
    let current = match &portal {
        Some(t) => t.name.clone(),
        None => node.ui.label.clone().unwrap_or_default(),
    };
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
        let hint = if portal.is_some() {
            tr("node-tab-portal-name")
        } else {
            tr_args("node-tab-label-hint", &[("name", (&name).into())])
        };
        let r = ui
            .add(
                egui::TextEdit::singleline(text)
                    .id(field_id)
                    .hint_text(&hint)
                    .desired_width(f32::INFINITY),
            )
            // The hint is all that says what the field is for.
            .accessible_name(hint);
        if r.lost_focus() && text.trim() != current {
            let label = text.trim().to_string();
            if portal.is_some() {
                io.doc.edit(io.registry, &tr("undo-rename-portal"), |g, _| {
                    edit::portal::rename(g, id, &label);
                });
            } else {
                io.doc.edit(io.registry, &tr("undo-rename"), |g, _| {
                    if let Some(n) = g.node_mut(id) {
                        n.ui.label = (!label.is_empty()).then_some(label);
                    }
                });
            }
        }
    }

    ui.horizontal(|ui| {
        let mut muted = node.muted;
        if ui.checkbox(&mut muted, tr("node-tab-muted")).changed() {
            io.doc.edit(io.registry, &tr("undo-mute"), |g, _| {
                if let Some(n) = g.node_mut(id) {
                    n.muted = muted;
                }
            });
        }
        if let NodeClass::Group(_) = class
            && icons::spoken(ui.button(format!(
                "{}  {}",
                icons::ARROW_SQUARE_IN,
                tr("node-tab-enter-group")
            )))
            .clicked()
        {
            io.doc.enter_group(id);
        }
    });
    timing_line(ui, io, id);

    // A node drawn rather than typed shows its editor first.
    node_editor(ui, io, id, &node);

    // Parameters.
    let sockets = edit::sockets_of(
        &io.doc.display_graph(),
        id,
        io.registry,
        &io.doc.project.group_defs,
        io.doc.interface().as_ref(),
    );
    if !sockets.inputs.is_empty() {
        section(ui, &tr("section-inputs"));
        let graph = io.doc.display_graph();
        egui::Grid::new(("node-params", id))
            .num_columns(2)
            .spacing(egui::vec2(10.0, 6.0))
            .show(ui, |ui| {
                for s in &sockets.inputs {
                    let wired = graph.wire_into(&ac_core::Endpoint::new(id, s.name.clone()));
                    let label_id = ui
                        .horizontal(|ui| {
                            let (r, _) =
                                ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                            ui.painter()
                                .circle_filled(r.center(), 3.5, theme::type_color(s.ty));
                            let label = ui.label(crate::canvas::paint_pretty(&s.name));
                            let id = label.id;
                            if let Some(doc) =
                                s.spec.as_ref().map(|s| s.doc).filter(|d| !d.is_empty())
                            {
                                label.on_hover_text(doc);
                            }
                            id
                        })
                        .inner;
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
                            tr("node-tab-group-input")
                        } else {
                            src
                        };
                        ui.label(
                            RichText::new(format!("← {from}.{}", w.from.socket))
                                .color(theme::text_muted()),
                        );
                    } else if let Some(spec) = &s.spec {
                        param_widget(ui, io, id, &node, spec, label_id);
                    } else {
                        ui.label(RichText::new("—").color(theme::text_faint()));
                    }
                    ui.end_row();
                }
            });
    }
    if sockets
        .inputs
        .iter()
        .any(|s| s.spec.as_ref().is_some_and(ac_core::binding::bindable))
    {
        super::bindings_ui::node_section(ui, io, id);
    }
    if !sockets.outputs.is_empty() {
        section(ui, &tr("section-outputs"));
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
                            tr("canvas-tip-silent")
                        } else {
                            tr_args(
                                "node-tab-dbfs",
                                &[("db", format!("{:.1}", 20.0 * l.log10()).into())],
                            )
                        }
                    }),
                    SignalType::Trigger | SignalType::Midi => tele
                        .socket_events(&path, &s.name)
                        .map(|c| tr_args("canvas-tip-events", &[("count", c.into())])),
                    ty => tele
                        .socket_value(&path, &s.name)
                        .map(|v| widgets::live_text(v, ty, s.spec.as_ref())),
                };
                if let Some(r) = reading {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(r)
                                .font(theme::mono_font(theme::FONT_VALUE))
                                .color(theme::text_muted()),
                        );
                    });
                }
            });
        }
    }

    config_fields(ui, io, id, &node);

    if let NodeClass::Group(d) = &class
        && let Some(def) = io.doc.project.group_defs.get(d).cloned()
    {
        section(ui, &tr("section-group-definition"));
        let mut name = def.name.clone();
        ui.horizontal(|ui| {
            let label = ui.label(tr("node-tab-name"));
            if ui
                .text_edit_singleline(&mut name)
                .labelled_by(label.id)
                .lost_focus()
                && name.trim() != def.name
                && !name.trim().is_empty()
            {
                let (d, name) = (d.clone(), name.trim().to_string());
                io.doc.edit_project(&tr("undo-rename-group"), |p| {
                    if let Some(def) = p.group_defs.get_mut(&d) {
                        def.name = name;
                    }
                });
            }
        });
        let instances = edit::group::instance_paths(&io.doc.project, d).len();
        ui.label(
            RichText::new(tr_args(
                "node-tab-group-summary",
                &[
                    ("inputs", def.inputs.len().into()),
                    ("outputs", def.outputs.len().into()),
                    ("nodes", def.graph.nodes.len().into()),
                    ("count", instances.into()),
                ],
            ))
            .small()
            .color(theme::text_muted()),
        );
        let mut poly = node.kind == ac_core::VOICES_KIND;
        if ui
            .checkbox(&mut poly, tr("node-tab-polyphonic"))
            .on_hover_text(tr("node-tab-polyphonic-tip"))
            .changed()
        {
            io.doc.edit(io.registry, &tr("undo-polyphonic"), |g, _| {
                edit::group::set_polyphonic(g, id, poly);
            });
        }
        library_buttons(ui, io, d, &def);
        if !def.description.is_empty() {
            markdown::show(ui, &def.description);
        }
    }

    // Documentation, with the tip highlighted; the Help tab has the rest.
    if let Some(e) = entry {
        section(ui, &tr("section-help"));
        markdown::show(ui, &e.meta.description());
        if let Some(tip) = e.meta.tip() {
            super::help_tab::tip_box(ui, &tip);
        }
        if icons::spoken(ui.link(format!(
            "{}  {}",
            icons::BOOK_OPEN,
            tr("node-tab-help-link")
        )))
        .clicked()
        {
            io.requests
                .push(PanelRequest::Help(e.meta.kind.to_string()));
        }
    }
}

/// The selected node's processing time per block (the same reading as its
/// header with `Ctrl+T`), or a button that starts measuring.
fn timing_line(ui: &mut egui::Ui, io: &mut PanelIo, id: NodeId) {
    use crate::actions::Action;
    use crate::timing;
    let shortcut = Action::ShowTiming
        .shortcut()
        .map(|s| ui.ctx().format_shortcut(&s))
        .unwrap_or_default();
    let voices = io
        .doc
        .graph()
        .node(id)
        .is_some_and(|n| n.kind == ac_core::VOICES_KIND);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{}  {}", icons::TIMER, tr("node-tab-time")))
                .color(theme::text_muted()),
        );
        let Some(t) = io.timing else {
            let r = ui
                .small_button(tr("node-tab-measure"))
                .on_hover_text(tr_args(
                    "node-tab-measure-tip",
                    &[("shortcut", shortcut.into())],
                ));
            if r.clicked() {
                io.requests.push(PanelRequest::Run(Action::ShowTiming));
            }
            return;
        };
        let Some(r) = t.node(id) else {
            ui.label(
                RichText::new(tr("node-tab-not-timed"))
                    .small()
                    .color(theme::text_faint()),
            );
            return;
        };
        let color = theme::heat(r.heat());
        let (bar, _) = ui.allocate_exact_size(egui::vec2(56.0, 6.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(bar, theme::radius(3.0), theme::meter_bg());
        let k = r.share.unwrap_or(0.0).clamp(0.0, 1.0);
        if k > 0.0 {
            let fill = egui::Rect::from_min_size(
                bar.min,
                egui::vec2((bar.width() * k).max(2.0), bar.height()),
            );
            ui.painter().rect_filled(fill, theme::radius(3.0), color);
        }
        let text = match r.share {
            Some(s) => tr_args(
                "node-tab-time-share",
                &[
                    ("time", timing::format_ns(r.ns).into()),
                    ("share", timing::format_share(s).into()),
                ],
            ),
            None => timing::format_ns(r.ns),
        };
        ui.label(
            RichText::new(text)
                .font(theme::mono_font(theme::FONT_VALUE))
                .color(color),
        )
        .on_hover_text(timing::describe(&r, t.budget_ns(), voices));
    });
}

/// A group definition and the library: where it came from, saving it as
/// one of the person's blocks, and taking the library's newer version.
fn library_buttons(ui: &mut egui::Ui, io: &mut PanelIo, d: &GroupId, def: &GroupDef) {
    if let Some(source) = def.source.as_deref().and_then(BlockSource::parse) {
        let text = match io.library.get(&source) {
            Some(b) => {
                let origin = match source.origin {
                    Origin::Shipped => tr("node-tab-origin-shipped"),
                    Origin::User => tr("node-tab-origin-user"),
                    Origin::Imported => tr("node-tab-origin-imported"),
                };
                tr_args(
                    "node-tab-block-from",
                    &[
                        ("origin", origin.into()),
                        ("name", b.name().into()),
                        ("version", def.version.to_string().into()),
                    ],
                )
            }
            None => tr_args(
                "node-tab-block-unknown",
                &[
                    ("source", source.to_string().into()),
                    ("version", def.version.to_string().into()),
                ],
            ),
        };
        ui.label(
            RichText::new(format!("{}  {text}", icons::BOOKS))
                .small()
                .color(theme::text_muted()),
        );
    }
    let update = io
        .library
        .updates(&io.doc.project)
        .into_iter()
        .find(|u| &u.def == d);
    ui.horizontal_wrapped(|ui| {
        if icons::spoken(ui.button(format!(
            "{}  {}",
            icons::TRAY_ARROW_DOWN,
            tr("node-tab-save-to-library")
        )))
        .on_hover_text(tr("node-tab-save-to-library-tip"))
        .clicked()
        {
            io.requests.push(PanelRequest::SaveBlock(d.clone()));
        }
        if icons::spoken(ui.button(format!("{}  {}", icons::EXPORT, tr("library-export"))))
            .on_hover_text(tr("node-tab-export-block-tip"))
            .clicked()
        {
            io.requests.push(PanelRequest::ExportBlock(
                crate::dialogs::BlockToExport::Def(d.clone()),
            ));
        }
        if let Some(u) = update
            && icons::spoken(ui.button(format!(
                "{}  {}",
                icons::ARROWS_CLOCKWISE,
                tr_args(
                    "node-tab-update-from-library",
                    &[
                        ("from", u.from.to_string().into()),
                        ("to", u.to.to_string().into()),
                    ],
                )
            )))
            .on_hover_text(tr("node-tab-update-from-library-tip"))
            .clicked()
        {
            io.requests.push(PanelRequest::UpdateBlock(d.clone()));
        }
    });
}

fn empty_state(ui: &mut egui::Ui, io: &PanelIo) {
    ui.add_space(20.0);
    ui.vertical_centered(|ui| {
        let icon = ui.label(
            RichText::new(icons::CUBE)
                .size(34.0)
                .color(theme::text_faint()),
        );
        icons::decoration(&icon);
        ui.label(RichText::new(tr("node-tab-nothing-selected")).color(theme::text_muted()));
        ui.add_space(6.0);
        let hint = if io.doc.graph().nodes.is_empty() {
            tr("node-tab-empty-hint")
        } else {
            tr("node-tab-select-hint")
        };
        ui.label(RichText::new(hint).small().color(theme::text_faint()));
    });
}

/// The same widget the canvas shows, at panel size.
fn param_widget(
    ui: &mut egui::Ui,
    io: &mut PanelIo,
    id: NodeId,
    node: &ac_core::NodeInstance,
    spec: &SocketSpec,
    label: egui::Id,
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
                    ui.add(s).labelled_by(label)
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
                    ui.add(d).labelled_by(label)
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
            let combo = egui::ComboBox::from_id_salt(("param-choice", id, &socket))
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
            combo.response.labelled_by(label);
        }
        widgets::Field::Check => {
            let mut on = widgets::value(node, spec).as_bool().unwrap_or(false);
            if ui.checkbox(&mut on, "").labelled_by(label).changed() {
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
            if icons::spoken(ui.button(format!("{} {}", icons::LIGHTNING, tr("node-tab-fire"))))
                .clicked()
            {
                params::fire(io.doc, &mut io.engine.handle, id, &socket);
            }
        }
        widgets::Field::Text => {
            // Typing stays local until Enter or focus loss, so every
            // keystroke doesn't recompile.
            let edit_id = ui.make_persistent_id(("param-text", id, &socket));
            let current = widgets::text(node, spec);
            let mut draft: String = ui
                .data(|d| d.get_temp(edit_id))
                .unwrap_or_else(|| current.clone());
            let r = ui
                .add(
                    egui::TextEdit::singleline(&mut draft)
                        .font(theme::mono_font(theme::FONT_SMALL))
                        .desired_width(ui.available_width().max(80.0)),
                )
                .labelled_by(label);
            if r.lost_focus() {
                if draft.trim() != current {
                    params::set_text(io.doc, id, &socket, draft.trim().to_string(), io.now);
                    io.doc.end_coalescing();
                }
                ui.data_mut(|d| d.remove::<String>(edit_id));
            } else if r.has_focus() {
                ui.data_mut(|d| d.insert_temp(edit_id, draft));
            }
        }
        widgets::Field::Label => {
            ui.label(RichText::new(tr("node-tab-unwired")).color(theme::text_faint()));
        }
    }
}

/// The node's own editor (curve, grid, piano roll, bands, steps), and a
/// larger copy in a window.
fn node_editor(ui: &mut egui::Ui, io: &mut PanelIo, id: NodeId, node: &ac_core::NodeInstance) {
    let Some(editor) = editors::editor_for(&node.kind, &node.config) else {
        return;
    };
    section(ui, &tr("section-editor"));
    if node.kind == "shaper" {
        ui.label(
            RichText::new(tr("node-tab-shaper-hint"))
                .small()
                .color(theme::text_muted()),
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
        if icons::spoken(ui.button(format!(
            "{}  {}",
            icons::ARROW_SQUARE_OUT,
            tr("node-tab-larger-editor")
        )))
        .clicked()
        {
            open = !open;
        }
        ui.add(
            egui::Label::new(
                RichText::new(editor.hint())
                    .small()
                    .color(theme::text_faint()),
            )
            .wrap(),
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

/// A text field whose draft lives in egui's memory while it has focus, so
/// the document (and the engine behind it) isn't rewritten on every
/// keystroke and the text survives the next frame's redraw from the
/// document. Returns the text when focus leaves it changed.
fn draft_text(
    ui: &mut egui::Ui,
    field_id: egui::Id,
    stored: &str,
    multiline: bool,
    label: egui::Id,
) -> Option<String> {
    let focused = ui.memory(|m| m.has_focus(field_id));
    let mut text = if focused {
        ui.data(|d| d.get_temp::<String>(field_id))
            .unwrap_or_else(|| stored.to_string())
    } else {
        stored.to_string()
    };
    let r = if multiline {
        ui.add(
            egui::TextEdit::multiline(&mut text)
                .id(field_id)
                .desired_rows(4)
                .desired_width(f32::INFINITY),
        )
    } else {
        ui.add(
            egui::TextEdit::singleline(&mut text)
                .id(field_id)
                .desired_width(f32::INFINITY),
        )
    }
    .labelled_by(label);
    if r.lost_focus() {
        ui.data_mut(|d| d.remove_temp::<String>(field_id));
        return (text != stored).then_some(text);
    }
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(field_id, text));
    }
    None
}

/// What a [`Kind::Line`] field shows: the stored text, a stored list of
/// names comma-separated, or the default when nothing is stored.
fn line_text(stored: Option<&serde_json::Value>, default: &str) -> String {
    match stored {
        None => default.to_string(),
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join(", "),
        Some(other) => other.to_string(),
    }
}

fn remove_config(io: &mut PanelIo, id: NodeId, key: &str) {
    io.doc
        .edit(io.registry, &tr("undo-change-setting"), |g, _| {
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
                        && !(node.kind == "sampler" && k.as_str() == "hash")
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
    section(ui, &tr("section-settings"));
    egui::Grid::new(("node-config", id))
        .num_columns(2)
        .spacing(egui::vec2(10.0, 6.0))
        .show(ui, |ui| {
            for f in &known {
                let label = ui.label(f.label).id;
                match &f.kind {
                    Kind::Count { min, max, default } => {
                        let mut v = node.config.get_u64_or(f.key, *default as u64) as i64;
                        if ui
                            .add(egui::DragValue::new(&mut v).range(*min..=*max).speed(0.05))
                            .labelled_by(label)
                            .changed()
                        {
                            set_config(io, id, f.key, v.into());
                        }
                    }
                    Kind::Number { default } => {
                        let mut v = node.config.get_f64_or(f.key, *default);
                        let speed = 0.01 * (v.abs() + 1.0);
                        let r = ui
                            .add(egui::DragValue::new(&mut v).speed(speed))
                            .labelled_by(label);
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
                            })
                            .response
                            .labelled_by(label);
                    }
                    Kind::Text { multiline } => {
                        let stored = node.config.get_str_or(f.key, "");
                        let field = egui::Id::unique(("cfg", id, f.key));
                        if let Some(t) = draft_text(ui, field, stored, *multiline, label) {
                            set_config(io, id, f.key, t.into());
                        }
                    }
                    Kind::Line { default } => {
                        let stored = line_text(node.config.get(f.key), default);
                        let field = egui::Id::unique(("cfg", id, f.key));
                        if let Some(t) = draft_text(ui, field, &stored, false, label)
                            && t.trim() != stored
                        {
                            set_config(io, id, f.key, t.trim().into());
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
                            .labelled_by(label)
                            .changed()
                        {
                            set_config(io, id, f.key, v.into());
                        }
                    }
                    Kind::NumberChoice { choices, default } => {
                        let cur = node
                            .config
                            .get(f.key)
                            .and_then(serde_json::Value::as_f64)
                            .map_or_else(|| (*default).to_string(), |v| format!("{v}"));
                        egui::ComboBox::from_id_salt(("cfg", id, f.key))
                            .selected_text(&cur)
                            .show_ui(ui, |ui| {
                                for c in *choices {
                                    if ui.selectable_label(cur == *c, *c).clicked()
                                        && let Ok(v) = c.parse::<u64>()
                                    {
                                        set_config(io, id, f.key, v.into());
                                    }
                                }
                            })
                            .response
                            .labelled_by(label);
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
                        let r = ui
                            .add(egui::TextEdit::singleline(&mut t).hint_text("0, 4, 7"))
                            .labelled_by(label);
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
                        let r = ui
                            .add(
                                egui::TextEdit::multiline(&mut t)
                                    .desired_rows(3)
                                    .desired_width(f32::INFINITY),
                            )
                            .labelled_by(label);
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
                            if ui.checkbox(&mut on, "").labelled_by(label).changed() {
                                if on {
                                    set_config(io, id, f.key, (*default).into());
                                } else {
                                    remove_config(io, id, f.key);
                                }
                            }
                            match current {
                                Some(mut v) => {
                                    let speed = 0.01 * (v.abs() + 1.0);
                                    if ui
                                        .add(egui::DragValue::new(&mut v).speed(speed))
                                        .labelled_by(label)
                                        .changed()
                                    {
                                        set_config(io, id, f.key, v.into());
                                    }
                                }
                                None => {
                                    ui.label(
                                        RichText::new(tr("node-tab-none"))
                                            .color(theme::text_faint()),
                                    );
                                }
                            }
                        });
                    }
                    Kind::Flag { default } => {
                        let mut on = node.config.get_bool_or(f.key, *default);
                        if ui.checkbox(&mut on, "").labelled_by(label).changed() {
                            set_config(io, id, f.key, on.into());
                        }
                    }
                    Kind::File => {
                        let file = node.config.get_str_or(f.key, "").to_string();
                        ui.vertical(|ui| {
                            let missing = !file.is_empty() && io.missing.contains(&file);
                            if file.is_empty() {
                                ui.label(
                                    RichText::new(tr("node-tab-no-file"))
                                        .color(theme::text_faint()),
                                );
                            } else if missing {
                                ui.colored_label(
                                    theme::error(),
                                    format!(
                                        "{} {}",
                                        icons::FILE_X,
                                        tr_args(
                                            "node-tab-file-missing",
                                            &[("file", (&file).into())]
                                        )
                                    ),
                                );
                            } else {
                                ui.label(format!("{} {file}", icons::FILE_AUDIO));
                            }
                            ui.horizontal(|ui| {
                                if ui.button(tr("node-tab-choose")).clicked() {
                                    io.requests.push(PanelRequest::ChooseClipFile(id));
                                }
                                if missing && ui.button(tr("node-tab-locate")).clicked() {
                                    io.requests.push(PanelRequest::Locate(file.clone()));
                                }
                            });
                            let ch = node.config.get_u64_or("channels", 0);
                            if ch > 0 {
                                ui.label(
                                    RichText::new(tr_args(
                                        "node-tab-channels",
                                        &[("count", ch.into())],
                                    ))
                                    .small()
                                    .color(theme::text_muted()),
                                );
                            }
                        });
                    }
                }
                ui.end_row();
            }
            for (k, v) in extra {
                let label = ui.label(&k).id;
                match v {
                    serde_json::Value::Bool(mut b) => {
                        if ui.checkbox(&mut b, "").labelled_by(label).changed() {
                            set_config(io, id, &k, b.into());
                        }
                    }
                    serde_json::Value::Number(n) => {
                        let mut x = n.as_f64().unwrap_or(0.0);
                        let int = n.is_i64() || n.is_u64();
                        let r = ui
                            .add(egui::DragValue::new(&mut x).speed(if int { 0.05 } else { 0.01 }))
                            .labelled_by(label);
                        if r.changed() {
                            let v: serde_json::Value = if int {
                                (x.round() as i64).into()
                            } else {
                                x.into()
                            };
                            set_config(io, id, &k, v);
                        }
                    }
                    serde_json::Value::String(s) => {
                        let field = egui::Id::unique(("cfg", id, k.as_str()));
                        if let Some(t) = draft_text(ui, field, &s, false, label) {
                            set_config(io, id, &k, t.into());
                        }
                    }
                    other => {
                        ui.label(
                            RichText::new(other.to_string())
                                .font(theme::mono_font(theme::FONT_SMALL))
                                .color(theme::text_muted()),
                        );
                    }
                }
                ui.end_row();
            }
        });
    // The OSC nodes can't send or receive with a bad address, host, or
    // port; say so while those are typed in (a wired one is checked live).
    if let Some(m) = osc_problem(io, id, node) {
        ui.colored_label(theme::warning(), format!("{} {m}", icons::WARNING));
    }
}

/// What keeps an OSC node's typed inputs from working, if anything.
fn osc_problem(io: &PanelIo, id: NodeId, node: &ac_core::NodeInstance) -> Option<&'static str> {
    if node.kind != "osc_in" && node.kind != "osc_out" {
        return None;
    }
    let sig = io.registry.signature_of(node, &io.doc.project.group_defs)?;
    let graph = io.doc.display_graph();
    let typed = |name: &str| -> Option<SocketSpec> {
        let wired = graph
            .wire_into(&ac_core::Endpoint::new(id, name.to_string()))
            .is_some();
        (!wired).then(|| sig.inputs.iter().find(|s| s.name == name).cloned())?
    };
    let address = typed("address").map(|s| widgets::text(node, &s));
    if node.kind == "osc_in" {
        return ac_nodes::io::osc_in::problem(&address?);
    }
    let host = typed("host").map(|s| widgets::text(node, &s));
    let port = typed("port").map(|s| widgets::number(node, &s).round() as i32);
    ac_nodes::io::osc_out::problem(
        address
            .as_deref()
            .unwrap_or(ac_nodes::io::osc_out::DEFAULT_ADDRESS),
        host.as_deref()
            .unwrap_or(ac_nodes::io::osc_out::DEFAULT_HOST),
        port.unwrap_or(i32::from(ac_nodes::io::osc_out::DEFAULT_PORT)),
    )
}

/// Selected interface card: list and rename its sockets.
fn interface(ui: &mut egui::Ui, io: &mut PanelIo, input_side: bool) {
    let Some(def) = io.doc.current_group().cloned() else {
        return;
    };
    ui.label(
        RichText::new(if input_side {
            tr("node-tab-group-input-title")
        } else {
            tr("node-tab-group-output-title")
        })
        .size(theme::FONT_HEADING)
        .color(theme::text_strong()),
    );
    ui.label(
        RichText::new(tr_args(
            "node-tab-interface-of",
            &[("name", (&def.name).into())],
        ))
        .color(theme::text_muted()),
    );
    section(
        ui,
        &if input_side {
            tr("section-inputs")
        } else {
            tr("section-outputs")
        },
    );
    let names: Vec<(String, SignalType)> = if input_side {
        def.inputs.iter().map(|i| (i.name.clone(), i.ty)).collect()
    } else {
        def.outputs.iter().map(|o| (o.name.clone(), o.ty)).collect()
    };
    if names.is_empty() {
        ui.label(RichText::new(tr("node-tab-interface-empty")).color(theme::text_muted()));
    }
    for (name, ty) in names {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(r.center(), 3.5, theme::type_color(ty));
            let mut text = name.clone();
            if ui
                .add(egui::TextEdit::singleline(&mut text).desired_width(140.0))
                .accessible_name(tr_args("node-tab-socket-name", &[("name", (&name).into())]))
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
            if icons::spoken(ui.small_button(icons::X))
                .on_hover_text(tr("node-tab-remove-socket"))
                .accessible_name(tr_args(
                    "node-tab-remove-named-socket",
                    &[("name", (&name).into())],
                ))
                .clicked()
            {
                let n = name.clone();
                io.doc
                    .edit(io.registry, &tr("undo-remove-group-socket"), |g, ctx| {
                        edit::remove_interface_socket(g, ctx, input_side, &n);
                    });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Settings a freshly added node has no value for yet must still be
    /// reachable: these nodes' settings change their sockets or where they
    /// send, and an empty config would otherwise show no field at all.
    #[test]
    fn new_nodes_can_reach_their_settings() {
        for (kind, keys) in [
            ("oscillator", &["pure"][..]),
            ("delay", &["unit", "max_time"]),
            ("gamepad", &["pad", "releases"]),
            ("keyboard_in", &["keys", "low", "channel"]),
            // The OSC addresses, host, and port are inputs now (D116).
            ("osc_in", &["args"]),
            ("osc_out", &["args"]),
            ("merge", &["inputs", "type"]),
        ] {
            let fields: Vec<&str> = schema(kind).iter().map(|f| f.key).collect();
            for k in keys {
                assert!(fields.contains(k), "{kind} has no `{k}` field");
            }
        }
    }

    #[test]
    fn line_fields_show_text_lists_and_defaults() {
        let v = serde_json::json!(["Space", "Z", 3]);
        assert_eq!(line_text(Some(&v), "1"), "Space, Z, 3");
        assert_eq!(
            line_text(Some(&serde_json::json!("/xy")), "/fader/1"),
            "/xy"
        );
        assert_eq!(line_text(None, "/fader/1"), "/fader/1");
    }
}
