//! sequencing nodes.

use ac_core::Registry;

pub mod arpeggiator;
pub mod chord;
pub mod midi_cc;
pub mod midi_filter;
pub mod midi_note;
pub mod midi_pattern;
pub mod midi_transpose;
pub mod note_split;
pub mod note_to_midi;
pub(crate) mod notes;
pub mod pattern;
pub mod scale_quantize;
pub mod velocity;
pub mod voice;
pub mod voices;
pub mod zones;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<midi_note::MidiNote>();
    r.register::<voices::Voices>();
    r.register::<voice::Voice>();
    r.register::<midi_cc::MidiCc>();
    r.register::<midi_filter::MidiFilter>();
    r.register::<midi_transpose::MidiTranspose>();
    r.register::<scale_quantize::ScaleQuantize>();
    r.register::<chord::Chord>();
    r.register::<note_to_midi::NoteToMidi>();
    r.register::<note_split::NoteSplit>();
    r.register::<velocity::Velocity>();
    r.register::<zones::Zones>();
    r.register::<pattern::Pattern>();
    r.register::<midi_pattern::MidiPattern>();
    r.register::<arpeggiator::Arpeggiator>();
}
