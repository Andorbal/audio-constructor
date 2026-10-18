//! io nodes.

use ac_core::Registry;

pub mod audio_in;
pub mod audio_out;
pub mod file_out;
pub mod midi_in;
pub mod midi_out;
pub mod record;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<audio_in::AudioIn>();
    r.register::<audio_out::AudioOut>();
    r.register::<midi_in::MidiIn>();
    r.register::<midi_out::MidiOut>();
    r.register::<record::Record>();
    r.register::<file_out::FileOut>();
}
