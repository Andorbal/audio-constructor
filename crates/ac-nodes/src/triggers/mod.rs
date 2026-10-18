//! triggers nodes.

use ac_core::Registry;

pub mod clock;
pub mod frequency_trigger;
pub mod onset;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<clock::Clock>();
    r.register::<frequency_trigger::FrequencyTrigger>();
    r.register::<onset::Onset>();
}
