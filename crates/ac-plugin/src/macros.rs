//! What each macro parameter drives, for the host's parameter readouts.
//!
//! Hosts list the macros by fixed names (`Macro 1` …), since a plugin's
//! parameter list can't change under the host; what a macro does lives in
//! the project as bindings from [`Source::Macro`]. This table follows the
//! project so a host shows a macro's position as the value it sets
//! ("−6 dB", "1.20 kHz") rather than a bare 0–1, and the editor can name
//! its target.

use std::sync::RwLock;

use ac_core::binding::map_range;
use ac_core::{MACRO_COUNT, Mapping, Project, Registry, SignalType, Source, Target, Unit};

/// One macro's first binding to an input, as the readout needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct MacroTarget {
    /// Who it drives, for people: "Level › gain".
    pub name: String,
    mapping: Mapping,
    unit: Unit,
    choices: Vec<String>,
}

/// The table, shared by the parameter callbacks (host threads) and the
/// session that updates it.
#[derive(Debug)]
pub struct MacroLabels {
    slots: RwLock<Vec<Option<MacroTarget>>>,
}

impl Default for MacroLabels {
    fn default() -> Self {
        MacroLabels {
            slots: RwLock::new(vec![None; MACRO_COUNT]),
        }
    }
}

impl MacroLabels {
    /// Rebuild from a project's bindings (the first input binding of each
    /// macro; transport bindings have no value to show).
    pub fn update(&self, project: &Project, registry: &Registry) {
        let mut slots = vec![None; MACRO_COUNT];
        for b in &project.bindings {
            let Source::Macro { .. } = b.source else {
                continue;
            };
            let ac_core::SourceKey::Macro { index } = b.source.key() else {
                continue;
            };
            let Some(slot) = slots.get_mut(usize::from(index)) else {
                continue;
            };
            if slot.is_some() {
                continue;
            }
            let Target::Input { node, socket } = &b.target else {
                continue;
            };
            let Some(inst) = project.node_at(node) else {
                continue;
            };
            let Some(spec) = registry
                .signature_of(inst, &project.group_defs)
                .and_then(|sig| sig.inputs.into_iter().find(|s| s.name == socket.as_str()))
            else {
                continue;
            };
            let Some(mapping) = Mapping::for_input(b, &spec) else {
                continue;
            };
            let node_name = inst
                .ui
                .label
                .clone()
                .filter(|l| !l.is_empty())
                .or_else(|| registry.get(&inst.kind).map(|e| e.meta.name.to_string()))
                .unwrap_or_else(|| inst.kind.clone());
            *slot = Some(MacroTarget {
                name: format!("{node_name} › {socket}"),
                mapping,
                unit: spec.unit,
                choices: if spec.ty == SignalType::Int {
                    spec.choices.iter().map(|c| c.to_string()).collect()
                } else {
                    Vec::new()
                },
            });
        }
        if let Ok(mut s) = self.slots.write() {
            *s = slots;
        }
    }

    /// What macro `index` (0-based) drives, if anything.
    pub fn target(&self, index: usize) -> Option<MacroTarget> {
        self.slots.try_read().ok()?.get(index).cloned().flatten()
    }

    /// Macro `index` at position `v` (0–1) as the host shows it: the value
    /// it sets on its target, or a percentage when it drives nothing.
    /// Never blocks: a table being rebuilt reads as unmapped.
    pub fn format(&self, index: usize, v: f32) -> String {
        let percent = || format!("{:.0} %", v.clamp(0.0, 1.0) * 100.0);
        let Ok(slots) = self.slots.try_read() else {
            return percent();
        };
        let Some(Some(t)) = slots.get(index) else {
            return percent();
        };
        match t.mapping {
            Mapping::Range {
                min,
                max,
                log,
                round,
                invert,
            } => {
                let pos = if invert { 1.0 - v } else { v };
                let x = map_range(pos, min, max, log, round);
                if let Some(c) = (x >= 0.0).then(|| t.choices.get(x as usize)).flatten() {
                    return c.clone();
                }
                format_value(f64::from(x), t.unit, round)
            }
            Mapping::Gate { invert, toggle } => {
                if toggle {
                    percent()
                } else if (v >= 0.5) != invert {
                    "On".into()
                } else {
                    "Off".into()
                }
            }
            Mapping::Press => percent(),
        }
    }

    /// Parse what someone typed into the host's field for macro `index`:
    /// a value of its target ("-6", "-6 dB", "1.2 kHz", a choice name), or a
    /// position ("40 %", or a bare number when it drives nothing).
    pub fn parse(&self, index: usize, text: &str) -> Option<f32> {
        let text = text.trim();
        if let Some(p) = text.strip_suffix('%') {
            return p
                .trim()
                .parse::<f32>()
                .ok()
                .map(|p| (p / 100.0).clamp(0.0, 1.0));
        }
        let target = self.target(index);
        let Some(t) = target else {
            return text.parse::<f32>().ok().map(|v| v.clamp(0.0, 1.0));
        };
        match t.mapping {
            Mapping::Range {
                min,
                max,
                log,
                invert,
                ..
            } => {
                let x = if let Some(i) = t.choices.iter().position(|c| c.eq_ignore_ascii_case(text))
                {
                    i as f32
                } else {
                    parse_value(text, t.unit)?
                };
                let pos = if log && min > 0.0 && max > 0.0 && x > 0.0 {
                    (x / min).ln() / (max / min).ln()
                } else if max != min {
                    (x - min) / (max - min)
                } else {
                    0.0
                };
                let pos = if invert { 1.0 - pos } else { pos };
                pos.is_finite().then_some(pos.clamp(0.0, 1.0))
            }
            Mapping::Gate { invert, .. } => {
                let on = match text.to_ascii_lowercase().as_str() {
                    "on" | "1" | "true" => true,
                    "off" | "0" | "false" => false,
                    _ => return None,
                };
                Some(if on != invert { 1.0 } else { 0.0 })
            }
            Mapping::Press => text.parse::<f32>().ok().map(|v| v.clamp(0.0, 1.0)),
        }
    }
}

/// A value with its unit, the way the editor's inline controls read.
fn format_value(v: f64, unit: Unit, integer: bool) -> String {
    if integer {
        return format!("{}", v.round() as i64);
    }
    match unit {
        Unit::Hz if v.abs() >= 1000.0 => format!("{:.2} kHz", v / 1000.0),
        Unit::Hz => format!("{} Hz", trim(v, if v.abs() < 10.0 { 2 } else { 1 })),
        Unit::Db if v <= -120.0 => "-∞ dB".into(),
        Unit::Db => format!("{} dB", trim(v, 1)),
        Unit::Seconds if v.abs() < 1.0 => format!("{} ms", trim(v * 1000.0, 1)),
        Unit::Seconds => format!("{} s", trim(v, 2)),
        Unit::Generic => trim(v, 3),
        other => format!("{} {}", trim(v, 2), other.suffix()),
    }
}

/// A number typed with or without its unit ("1.2 kHz", "250 ms", "-6").
fn parse_value(text: &str, unit: Unit) -> Option<f32> {
    let lower = text.to_ascii_lowercase();
    let split = lower
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e')))
        .unwrap_or(lower.len());
    let (num, suffix) = lower.split_at(split);
    let v: f32 = num.trim().parse().ok()?;
    let scale = match (unit, suffix.trim()) {
        (Unit::Hz, "khz" | "k") => 1000.0,
        (Unit::Seconds, "ms") => 0.001,
        _ => 1.0,
    };
    Some(v * scale)
}

fn trim(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        let t = s.trim_end_matches('0').trim_end_matches('.');
        if t == "-0" { "0".into() } else { t.to_string() }
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::{Binding, NodeId, NodeInstance, NodePath};

    fn project() -> Project {
        let mut p = Project::default();
        p.graph
            .nodes
            .push(NodeInstance::new(NodeId(1), "gain").with_label("Level"));
        p.graph.nodes.push(NodeInstance::new(NodeId(2), "biquad"));
        p.bindings.push(Binding {
            min: Some(-24.0),
            max: Some(24.0),
            ..Binding::input(
                Source::Macro { number: 1 },
                NodePath::root(NodeId(1)),
                "gain",
            )
        });
        p.bindings.push(Binding::input(
            Source::Macro { number: 2 },
            NodePath::root(NodeId(2)),
            "cutoff",
        ));
        p.bindings.push(Binding::input(
            Source::Macro { number: 3 },
            NodePath::root(NodeId(2)),
            "mode",
        ));
        p
    }

    #[test]
    fn readouts_show_the_value_a_macro_sets() {
        let labels = MacroLabels::default();
        let registry = ac_nodes::registry();
        labels.update(&project(), &registry);
        assert_eq!(labels.target(0).unwrap().name, "Level › gain");
        assert_eq!(labels.format(0, 0.5), "0 dB");
        assert_eq!(labels.format(0, 0.25), "-12 dB");
        assert_eq!(labels.parse(0, "-12 dB"), Some(0.25));
        assert_eq!(labels.parse(0, "12"), Some(0.75));
        // Hz on a log scale.
        let top = labels.format(1, 1.0);
        assert!(top.ends_with("kHz"), "{top}");
        let back = labels.parse(1, &labels.format(1, 0.3)).unwrap();
        assert!((back - 0.3).abs() < 0.01, "{back}");
        // An enumerated input shows its choices.
        let first = labels.format(2, 0.0);
        assert!(
            !first.is_empty() && first.parse::<f32>().is_err(),
            "{first}"
        );
        assert_eq!(labels.parse(2, &first), Some(0.0));
        // Unmapped macros read as a position.
        assert_eq!(labels.format(5, 0.4), "40 %");
        assert_eq!(labels.parse(5, "40 %"), Some(0.4));
        assert_eq!(labels.parse(5, "0.25"), Some(0.25));
        assert_eq!(labels.parse(0, "loud"), None);
        assert!(labels.target(5).is_none());
    }
}
