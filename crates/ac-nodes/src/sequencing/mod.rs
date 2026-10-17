//! sequencing nodes.

use ac_core::Registry;

pub mod midi_note;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<midi_note::MidiNote>();
}
