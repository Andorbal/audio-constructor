//! Generator nodes: sources of signal over time.

use ac_core::Registry;

pub mod clip;
pub mod oscillator;
pub mod transport;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<clip::Clip>();
    r.register::<oscillator::Oscillator>();
    r.register::<transport::Transport>();
}
