//! filters nodes.

use ac_core::Registry;

pub mod biquad;
pub mod ladder;
pub mod svf;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<biquad::Biquad>();
    r.register::<svf::Svf>();
    r.register::<ladder::Ladder>();
}
