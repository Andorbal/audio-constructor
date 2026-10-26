//! Generator nodes: sources of signal over time.

use ac_core::Registry;

pub mod clip;
pub mod granular;
pub mod graph;
pub mod impulse;
pub mod looper;
pub mod noise;
pub mod oscillator;
pub mod sample;
pub mod sampler;
pub mod slicer;
pub mod transport;
pub mod wavetable;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<clip::Clip>();
    r.register::<graph::Graph>();
    r.register::<granular::Granular>();
    r.register::<impulse::Impulse>();
    r.register::<looper::Looper>();
    r.register::<noise::Noise>();
    r.register::<oscillator::Oscillator>();
    r.register::<sample::Sample>();
    r.register::<sampler::Sampler>();
    r.register::<slicer::Slicer>();
    r.register::<transport::Transport>();
    r.register::<wavetable::Wavetable>();
}
