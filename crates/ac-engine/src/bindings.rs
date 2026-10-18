//! Bindings on the audio thread (ARCHITECTURE §8 "Parameters in"): the
//! project's bindings compiled to parameter slots and transport actions,
//! and matched against each block's MIDI and control events.
//!
//! The compiler resolves each binding's target through the same
//! (node path, socket) → param slot map the editor's parameter changes
//! use, so a bound input smooths like one moved by hand. Bindings whose
//! target is missing, wired, or of a type that can't be bound are reported
//! and left out. The table travels inside the compiled graph, so it swaps
//! in with the slots it points at; what a binding last set carries over
//! to the next graph (see [`carry_over`]), so editing the patch doesn't
//! snap bound inputs back to their stored values.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ac_core::binding::bindable;
use ac_core::util::Fnv64;
use ac_core::{
    ControlEvent, Effect, Mapping, MidiEvent, Project, Reading, Registry, SourceKey, Target,
    TransportAction, VOICES_KIND,
};

use crate::compile::ParamMap;
use crate::graph::CompiledGraph;
use crate::report::{CompileReport, Severity};

/// What each binding last set on its input, published by the audio thread
/// with relaxed atomics (like telemetry) so the editor can draw a bound
/// widget at its live value. Indexed like `Project::bindings`.
#[derive(Debug, Default)]
pub struct BindingValues {
    values: Box<[AtomicU32]>,
}

/// Stored for "nothing set since the engine compiled".
const UNSET: u32 = 0x7fc0_0000;

impl BindingValues {
    fn new(n: usize) -> BindingValues {
        BindingValues {
            values: (0..n).map(|_| AtomicU32::new(UNSET)).collect(),
        }
    }

    /// Bindings in the graph these values belong to.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The value binding `index` last set on its input, if it set one since
    /// the patch was opened and nothing has moved the input by hand since.
    /// Trigger and transport bindings never report one.
    pub fn get(&self, index: usize) -> Option<f32> {
        let v = f32::from_bits(self.values.get(index)?.load(Ordering::Relaxed));
        v.is_finite().then_some(v)
    }

    fn set(&self, index: u32, v: f32) {
        if let Some(a) = self.values.get(index as usize) {
            a.store(v.to_bits(), Ordering::Relaxed);
        }
    }

    fn clear(&self, index: u32) {
        if let Some(a) = self.values.get(index as usize) {
            a.store(UNSET, Ordering::Relaxed);
        }
    }
}

/// What a compiled binding drives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Action {
    /// The param slots `slots[start..end]` of the table.
    Params(u32, u32),
    Transport(TransportAction),
}

/// One compiled binding.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Entry {
    /// Index in `Project::bindings`, where its value is published.
    pub index: u32,
    /// Identity across recompiles (its source and target).
    pub key: u64,
    pub source: SourceKey,
    pub mapping: Mapping,
    pub action: Action,
    /// The control is held (presses and toggles act on the rising edge).
    pub on: bool,
    /// The value last set on the target; NaN when none.
    pub last: f32,
}

/// A bound event in this block: where it lands, which entry, and how it
/// reads.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bound {
    pub offset: u32,
    pub entry: u32,
    pub reading: Reading,
}

/// Bound events one host block can carry (every MIDI and control event
/// the engine accepts, each matching one binding). Past it, events wait
/// for nothing: they're dropped.
pub(crate) const BOUND_CAPACITY: usize = 2048;

/// Most extra sub-blocks bound events may split a host block into. Past
/// it, events act at the start of the next sub-block instead of on their
/// own sample (a flood of controller messages can't multiply the work).
pub(crate) const MAX_SPLITS: usize = 64;

/// The bindings of a compiled graph.
#[derive(Debug, Default)]
pub(crate) struct BindingTable {
    pub entries: Vec<Entry>,
    pub slots: Vec<u32>,
    pub values: Arc<BindingValues>,
}

impl BindingTable {
    /// Match a block's events against every binding into `out` (cleared
    /// first, filled up to its capacity), in offset order; at one offset,
    /// MIDI comes before controls and bindings in project order. Real-time
    /// safe.
    pub fn gather(&self, midi: &[MidiEvent], controls: &[ControlEvent], out: &mut Vec<Bound>) {
        out.clear();
        if self.entries.is_empty() {
            return;
        }
        for e in midi {
            for (k, entry) in self.entries.iter().enumerate() {
                if let Some(reading) = entry.source.read_midi(&e.msg)
                    && out.len() < out.capacity()
                {
                    out.push(Bound {
                        offset: e.offset,
                        entry: k as u32,
                        reading,
                    });
                }
            }
        }
        for e in controls {
            for (k, entry) in self.entries.iter().enumerate() {
                if let Some(reading) = entry.source.read_control(e)
                    && out.len() < out.capacity()
                {
                    out.push(Bound {
                        offset: e.offset,
                        entry: k as u32,
                        reading,
                    });
                }
            }
        }
        // Stable insertion sort: each list is already in order, so this is
        // a near-linear merge, and unlike `sort_by_key` it never allocates.
        for i in 1..out.len() {
            let mut j = i;
            while j > 0 && out[j - 1].offset > out[j].offset {
                out.swap(j - 1, j);
                j -= 1;
            }
        }
    }
}

impl CompiledGraph {
    /// Apply one bound event. Returns the transport action it presses, if
    /// any (the processor owns the transport).
    pub(crate) fn apply_binding(&mut self, b: Bound) -> Option<TransportAction> {
        let entry = self.bindings.entries.get_mut(b.entry as usize)?;
        let was = entry.on;
        entry.on = b.reading.on;
        let effect = entry.mapping.effect(b.reading, was);
        let (action, index) = (entry.action, entry.index);
        let (start, end) = match (action, effect) {
            (_, Effect::None) => return None,
            (Action::Transport(a), Effect::Fire) => return Some(a),
            (Action::Transport(_), _) => return None,
            (Action::Params(s, e), _) => (s as usize, e as usize),
        };
        let value = match effect {
            Effect::Set(v) => v,
            Effect::Flip => {
                let first = self.bindings.slots.get(start).copied().unwrap_or(u32::MAX);
                let current = self.params.get(first as usize).map_or(0.0, |p| p.target);
                if current >= 0.5 { 0.0 } else { 1.0 }
            }
            Effect::Fire => {
                for k in start..end {
                    if let Some(&slot) = self.bindings.slots.get(k) {
                        self.fire(slot);
                    }
                }
                return None;
            }
            Effect::None => return None,
        };
        for k in start..end {
            if let Some(&slot) = self.bindings.slots.get(k) {
                self.set_param(slot, value);
            }
        }
        if let Some(entry) = self.bindings.entries.get_mut(b.entry as usize) {
            entry.last = value;
        }
        self.bindings.values.set(index, value);
        None
    }

    /// A parameter message set `slot` by hand: bindings driving it forget
    /// their value, so the editor shows the new one and a recompile keeps
    /// it.
    pub(crate) fn forget_bound_value(&mut self, slot: u32) {
        let BindingTable {
            entries,
            slots,
            values,
        } = &mut self.bindings;
        for e in entries.iter_mut() {
            if let Action::Params(s, end) = e.action
                && slots
                    .get(s as usize..end as usize)
                    .is_some_and(|v| v.contains(&slot))
            {
                e.last = f32::NAN;
                values.clear(e.index);
            }
        }
    }

    /// Set a param at once, without smoothing (a bound value carried into
    /// a new graph).
    fn set_param_now(&mut self, slot: u32, value: f32) {
        if let Some(p) = self.params.get_mut(slot as usize)
            && value.is_finite()
        {
            p.value = value;
            p.target = value;
            p.remaining = 0;
            p.step = 0.0;
        }
    }
}

/// Carry each binding's state from the running graph into the one being
/// installed: whether its control is held, and the value it last set,
/// which the new graph takes at once. Real-time safe.
pub(crate) fn carry_over(new: &mut CompiledGraph, old: &CompiledGraph) {
    for i in 0..new.bindings.entries.len() {
        let Some(e) = new.bindings.entries.get(i).copied() else {
            continue;
        };
        let Some(prev) = old.bindings.entries.iter().find(|o| o.key == e.key) else {
            continue;
        };
        let (on, last) = (prev.on, prev.last);
        if let Some(n) = new.bindings.entries.get_mut(i) {
            n.on = on;
        }
        let Action::Params(s, end) = e.action else {
            continue;
        };
        if !last.is_finite() || matches!(e.mapping, Mapping::Press) {
            continue;
        }
        for k in s as usize..end as usize {
            if let Some(&slot) = new.bindings.slots.get(k) {
                new.set_param_now(slot, last);
            }
        }
        if let Some(n) = new.bindings.entries.get_mut(i) {
            n.last = last;
        }
        new.bindings.values.set(e.index, last);
    }
}

/// Compile the project's bindings against the param slots of the graph
/// being built. Problems are warnings: the binding is left out.
pub(crate) fn compile_bindings(
    project: &Project,
    registry: &Registry,
    params: &ParamMap,
    report: &mut CompileReport,
) -> BindingTable {
    let mut entries = Vec::new();
    let mut slots = Vec::new();
    for (index, b) in project.bindings.iter().enumerate() {
        let mut h = Fnv64::new();
        h.write(
            serde_json::to_string(&b.source)
                .unwrap_or_default()
                .as_bytes(),
        );
        h.write(b"->");
        h.write(
            serde_json::to_string(&b.target)
                .unwrap_or_default()
                .as_bytes(),
        );
        let key = h.finish();
        let (mapping, action) = match &b.target {
            Target::Transport { action } => (Mapping::Press, Action::Transport(*action)),
            Target::Input { node, socket } => {
                let Some(targets) = params
                    .get(&(node.clone(), socket.clone()))
                    .filter(|t| !t.is_empty())
                else {
                    let why = unbound_reason(project, registry, node, socket);
                    report.push(
                        Severity::Warning,
                        Some(node),
                        Some(socket),
                        format!("binding ({}) does nothing: {why}", b.source),
                    );
                    continue;
                };
                let spec = &targets[0].spec;
                let Some(mapping) = Mapping::for_input(b, spec) else {
                    report.push(
                        Severity::Warning,
                        Some(node),
                        Some(socket),
                        format!(
                            "binding ({}) does nothing: {} inputs can't be bound",
                            b.source, spec.ty
                        ),
                    );
                    continue;
                };
                let start = slots.len() as u32;
                slots.extend(targets.iter().map(|t| t.slot));
                (mapping, Action::Params(start, slots.len() as u32))
            }
        };
        entries.push(Entry {
            index: index as u32,
            key,
            source: b.source.key(),
            mapping,
            action,
            on: false,
            last: f32::NAN,
        });
    }
    BindingTable {
        entries,
        slots,
        values: Arc::new(BindingValues::new(project.bindings.len())),
    }
}

/// Why an input target has no param slot, for the report.
fn unbound_reason(
    project: &Project,
    registry: &Registry,
    node: &ac_core::NodePath,
    socket: &str,
) -> String {
    let Some(inst) = project.node_at(node) else {
        return "its node isn't in the project".into();
    };
    let spec = registry
        .signature_of(inst, &project.group_defs)
        .and_then(|sig| sig.inputs.into_iter().find(|s| s.name == socket));
    let Some(spec) = spec else {
        return format!("the node has no input `{socket}`");
    };
    if !bindable(&spec) {
        return format!("{} inputs can't be bound", spec.ty);
    }
    // The graph holding the node: is the input wired there?
    let mut graph = &project.graph;
    let mut polyphonic = false;
    if let Some((last, parents)) = node.0.split_last() {
        for id in parents {
            let Some(inst) = graph.node(*id) else { break };
            polyphonic |= inst.kind == VOICES_KIND;
            let Some(def) = inst.group_def().and_then(|d| project.group_defs.get(&d)) else {
                break;
            };
            graph = &def.graph;
        }
        if graph
            .wire_into(&ac_core::Endpoint::new(*last, socket))
            .is_some()
        {
            return "the input is wired".into();
        }
    }
    if polyphonic {
        return "nodes inside a polyphonic group can't be bound yet".into();
    }
    "the input isn't part of the running patch".into()
}
