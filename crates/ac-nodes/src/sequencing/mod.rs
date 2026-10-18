//! sequencing nodes.

use ac_core::Registry;

pub mod midi_note;
pub mod voice;
pub mod voices;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<midi_note::MidiNote>();
    r.register::<voices::Voices>();
    r.register::<voice::Voice>();
}
