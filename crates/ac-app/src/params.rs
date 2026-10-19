//! Changing parameter values: the document records the value (as a
//! coalesced undo step) and the engine gets it through its parameter queue,
//! smoothed, without a recompile. Inside a group, every instance of the
//! definition gets the change.

use ac_core::{NodeId, Value};
use ac_engine::EngineHandle;

use crate::document::Document;

/// Set an unwired input's value in the document and the running engine.
pub fn set(
    doc: &mut Document,
    engine: &mut EngineHandle,
    id: NodeId,
    socket: &str,
    value: Value,
    now: f64,
) {
    doc.set_param(id, socket, value.clone(), now);
    for path in doc.all_node_paths(id) {
        engine.set_param(&path, socket, value.clone());
    }
}

/// Set several unwired inputs of one node that one gesture moves together
/// (an XY Pad's `x` and `y`): one undo step for all of them. Nothing is
/// sent when none of them changed.
pub fn set_many(
    doc: &mut Document,
    engine: &mut EngineHandle,
    id: NodeId,
    values: &[(&str, Value)],
    now: f64,
) {
    if !doc.set_params(id, values, now) {
        return;
    }
    for path in doc.all_node_paths(id) {
        for (socket, value) in values {
            engine.set_param(&path, socket, value.clone());
        }
    }
}

/// Set an unwired `Text` input. The engine's parameter queue carries
/// numbers only, so the document asks for a recompile instead; the node's
/// state carries over, and the new text is read on the next block.
pub fn set_text(doc: &mut Document, id: NodeId, socket: &str, text: String, now: f64) {
    doc.set_text_param(id, socket, text, now);
}

/// Press an unwired trigger input's button.
pub fn fire(doc: &Document, engine: &mut EngineHandle, id: NodeId, socket: &str) {
    for path in doc.all_node_paths(id) {
        engine.fire(&path, socket);
    }
}

/// Write straight to the engine without touching the document (momentary
/// controls such as a held Button).
pub fn momentary(
    doc: &Document,
    engine: &mut EngineHandle,
    id: NodeId,
    socket: &str,
    value: Value,
) {
    for path in doc.all_node_paths(id) {
        engine.set_param(&path, socket, value.clone());
    }
}
