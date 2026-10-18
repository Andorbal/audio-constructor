//! time nodes.

use ac_core::Registry;

pub mod delay;
pub mod diffuser;
pub mod freeze;
pub mod pitch_shift;
pub mod reverse;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<delay::Delay>();
    r.register::<reverse::Reverse>();
    r.register::<freeze::Freeze>();
    r.register::<pitch_shift::PitchShift>();
    r.register::<diffuser::Diffuser>();
}
