//! controls nodes.

use ac_core::{BoolIn, Config, ConfigExt, Io, Registry, Signature, SocketSpec, Unit};

pub mod button;
pub mod dropdown;
pub mod fader;
pub mod keyboard;
pub mod knob;
pub mod number;
pub mod pad;
pub mod pads;
pub mod slider;
pub mod toggle;
pub mod xy_pad;

/// Register every node in this category.
pub fn register(r: &mut Registry) {
    r.register::<button::Button>();
    r.register::<slider::Slider>();
    r.register::<toggle::Toggle>();
    r.register::<knob::Knob>();
    r.register::<xy_pad::XyPad>();
    r.register::<keyboard::Keyboard>();
    r.register::<number::Number>();
    r.register::<dropdown::Dropdown>();
    r.register::<fader::Fader>();
    r.register::<pad::Pad>();
    r.register::<pads::Pads>();
}

/// 1 if a `lit` input was high anywhere in the block, so a light catches a
/// gate shorter than a frame of the screen.
pub(crate) fn lit_level(lit: BoolIn, frames: usize) -> f32 {
    let n = if lit.is_varying() {
        frames
    } else {
        frames.min(1)
    };
    if (0..n).any(|i| lit.at(i)) { 1.0 } else { 0.0 }
}

/// A range from two config keys, repaired (DECISIONS D29): missing or
/// non-numeric bounds take their defaults, bounds are kept within `f32`, and
/// a reversed range is swapped.
pub(crate) fn bounds(
    config: &Config,
    min_key: &str,
    max_key: &str,
    default: (f64, f64),
) -> (f64, f64) {
    let limit = f64::from(f32::MAX);
    let a = config.get_f64_or(min_key, default.0).clamp(-limit, limit);
    let b = config.get_f64_or(max_key, default.1).clamp(-limit, limit);
    if a <= b { (a, b) } else { (b, a) }
}

/// The setup of a ranged control (Slider, Knob) from its `min`, `max`,
/// `unit`, and `scale` config, repaired: unknown units become generic, and
/// a log scale needs a positive range.
pub(crate) struct Ranged {
    pub min: f64,
    pub max: f64,
    pub unit: Unit,
    pub log: bool,
}

impl Ranged {
    pub fn from_config(config: &Config) -> Ranged {
        let (min, max) = bounds(config, "min", "max", (0.0, 1.0));
        let unit_name = config.get_str_or("unit", "generic");
        let unit = serde_json::from_value(serde_json::Value::from(unit_name)).unwrap_or_default();
        let log = config.get_str_or("scale", "linear") == "log" && min > 0.0;
        Ranged {
            min,
            max,
            unit,
            log,
        }
    }

    /// The sockets: a `value` input with this range, unit, and scale, and an
    /// `out` that follows the input's rate.
    pub fn signature(&self, value_doc: &'static str) -> Signature {
        let mut value = SocketSpec::float("value", self.unit, self.min)
            .range(self.min, self.max)
            .doc(value_doc);
        if self.log {
            value = value.log();
        }
        Signature::new().input(value).output(
            SocketSpec::float("out", self.unit, self.min)
                .poly()
                .doc("`value`, kept within the range."),
        )
    }
}

/// Writes float input `input` to output `output`, kept within `min..=max`.
/// NaN reads as `min`, so the output is always finite.
pub(crate) fn clamp_through(io: &mut Io, input: usize, output: usize, min: f32, max: f32) {
    let value = io.inputs.float(input);
    // `max`/`min` return the other operand for NaN, so NaN reads as `min`.
    io.outputs
        .float(output)
        .fill_with(|i| value.at(i).max(min).min(max));
}
