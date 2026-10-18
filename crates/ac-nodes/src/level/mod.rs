//! Level, dynamics, and shaping nodes.

use ac_core::Registry;

pub mod crossfade;
pub mod envelope_follower;
pub mod gain;
pub mod gain_computer;
pub mod lookahead;
pub mod mix;
pub mod normalize;
pub mod pan;
pub mod shaper;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<crossfade::Crossfade>();
    r.register::<envelope_follower::EnvelopeFollower>();
    r.register::<gain::Gain>();
    r.register::<gain_computer::GainComputer>();
    r.register::<lookahead::Lookahead>();
    r.register::<mix::Mix>();
    r.register::<normalize::Normalize>();
    r.register::<pan::Pan>();
    r.register::<shaper::Shaper>();
}
