//! Generator nodes: sources of signal over time.

use ac_core::Registry;

pub mod clip;
pub mod graph;
pub mod impulse;
pub mod noise;
pub mod oscillator;
pub mod sampler;
pub mod transport;
pub mod wavetable;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<clip::Clip>();
    r.register::<graph::Graph>();
    r.register::<impulse::Impulse>();
    r.register::<noise::Noise>();
    r.register::<oscillator::Oscillator>();
    r.register::<sampler::Sampler>();
    r.register::<transport::Transport>();
    r.register::<wavetable::Wavetable>();
}
