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
