//! time nodes.

use ac_core::Registry;

pub mod delay;
pub mod diffuser;
pub mod reverse;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<delay::Delay>();
    r.register::<reverse::Reverse>();
    r.register::<diffuser::Diffuser>();
}
