//! Math and logic nodes. The stateless ones are rate-polymorphic: the output
//! runs at the fastest rate among the `Float`/`Bool` inputs. Stateful ones
//! that react to triggers (Random, SampleHold, Accumulate, ...) run at
//! sample rate so their output changes on the trigger's exact sample.

use ac_core::Registry;

pub mod accumulate;
pub mod clamp;
pub mod compare;
pub mod constant;
pub mod convert;
pub mod counter;
pub mod differentiate;
pub mod integrate;
pub mod logic;
pub mod map;
// Node files are named after their kind, and this kind is `math`.
#[allow(clippy::module_inception)]
pub mod math;
pub mod number_to_text;
pub mod quantize;
pub mod random;
mod route;
pub mod sample_hold;
pub mod select;
pub mod smooth;
pub mod switch;
pub mod text_to_number;
pub mod unary;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<constant::Constant>();
    r.register::<math::Math>();
    r.register::<unary::Unary>();
    r.register::<compare::Compare>();
    r.register::<map::Map>();
    r.register::<clamp::Clamp>();
    r.register::<logic::Logic>();
    r.register::<convert::Convert>();
    r.register::<smooth::Smooth>();
    r.register::<integrate::Integrate>();
    r.register::<differentiate::Differentiate>();
    r.register::<random::Random>();
    r.register::<sample_hold::SampleHold>();
    r.register::<quantize::Quantize>();
    r.register::<counter::Counter>();
    r.register::<accumulate::Accumulate>();
    r.register::<select::Select>();
    r.register::<switch::Switch>();
    r.register::<number_to_text::NumberToText>();
    r.register::<text_to_number::TextToNumber>();
}

/// `x` if it is finite, else 0. Math nodes promise finite outputs, so NaN and
/// infinity never travel down a wire from here.
#[inline]
pub(crate) fn finite_or_zero(x: f32) -> f32 {
    if x.is_finite() { x } else { 0.0 }
}

/// [`finite_or_zero`] for `f64` state, narrowed to `f32` (values beyond the
/// `f32` range are 0 too).
#[inline]
pub(crate) fn finite_f32(x: f64) -> f32 {
    finite_or_zero(x as f32)
}
