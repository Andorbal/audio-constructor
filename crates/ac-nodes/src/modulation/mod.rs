//! Modulation nodes: envelopes, LFOs, sequencers, and random movement.

use ac_core::Registry;

pub mod adsr;
pub mod chaos;
pub mod envelope;
pub mod lfo;
pub mod random_walk;
pub mod step_sequencer;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<adsr::Adsr>();
    r.register::<lfo::Lfo>();
    r.register::<envelope::Envelope>();
    r.register::<step_sequencer::StepSequencer>();
    r.register::<random_walk::RandomWalk>();
    r.register::<chaos::Chaos>();
}
