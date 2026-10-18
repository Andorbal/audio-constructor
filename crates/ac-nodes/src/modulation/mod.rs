//! Modulation nodes: envelopes, LFOs, sequencers, and random movement.

use ac_core::Registry;

pub mod adsr;
pub mod lfo;
pub mod random_walk;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<adsr::Adsr>();
    r.register::<lfo::Lfo>();
    r.register::<random_walk::RandomWalk>();
}
