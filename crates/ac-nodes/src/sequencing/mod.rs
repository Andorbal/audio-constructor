//! sequencing nodes.

use ac_core::Registry;

pub mod midi_cc;
pub mod midi_filter;
pub mod midi_note;
pub mod midi_transpose;
mod notes;
pub mod voice;
pub mod voices;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<midi_note::MidiNote>();
    r.register::<voices::Voices>();
    r.register::<voice::Voice>();
    r.register::<midi_cc::MidiCc>();
    r.register::<midi_filter::MidiFilter>();
    r.register::<midi_transpose::MidiTranspose>();
}
