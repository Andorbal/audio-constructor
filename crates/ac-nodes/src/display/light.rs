use ac_core::{
    Config, ConfigExt, Io, Node, NodeMeta, PrepareCtx, ProcessCtx, Signature, SocketSpec, Unit,
    node,
};

node! {
    kind: "light",
    name: "Light",
    category: Display,
    keywords: ["lamp", "led", "indicator", "status", "lit", "on", "playing", "state"],
    /// A lamp that shows a state: on when its input is, dimmed by a level.
    ///
    /// Wire a `Bool` into `in` (a clip's `playing`, a toggle, a gate) and
    /// the light is on while it's true; a level from 0 to 1 sets its
    /// brightness, so a slow envelope glows. A gate shorter than the screen
    /// refreshes still flashes: the light shows the brightest moment of each
    /// block. The `color` config picks the lamp's color: `amber` (the
    /// default), `green`, `red`, `blue`, `white`, or a `#RRGGBB` code.
    ///
    /// **Try this:** wire each clip's `playing` into a Light, pin them all to
    /// a panel, and the panel shows which clips are running; or wire a
    /// `Pattern`'s `gate_0` into one to see the beat.
    pub struct Light {}
}

const IN: usize = 0;

/// Telemetry: the brightest moment of the last block, 0–1.
pub const TELEMETRY_LEVEL: usize = 0;

/// The named colors the `color` config takes; anything else is a
/// `#RRGGBB` code or the default.
pub const COLORS: &[&str] = &["amber", "green", "red", "blue", "white"];

/// The `color` config: a name from [`COLORS`] or `#RRGGBB`, as `[r, g, b]`;
/// anything else is `None` (the editor's amber).
pub fn color(config: &Config) -> Option<[u8; 3]> {
    parse_color(config.get_str_or("color", "amber"))
}

/// A color name or `#RRGGBB` code as `[r, g, b]`.
pub fn parse_color(text: &str) -> Option<[u8; 3]> {
    let t = text.trim();
    let named = match t.to_ascii_lowercase().as_str() {
        "amber" => Some([0xF0, 0xA0, 0x30]),
        "green" => Some([0x4C, 0xD0, 0x6E]),
        "red" => Some([0xE8, 0x4A, 0x4A]),
        "blue" => Some([0x4A, 0x9A, 0xF0]),
        "white" => Some([0xF2, 0xF2, 0xF2]),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    let hex = t.strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

impl Node for Light {
    fn meta() -> NodeMeta {
        Self::base_meta().telemetry(1)
    }

    fn describe(_: &Config) -> Signature {
        Signature::new().input(
            SocketSpec::float("in", Unit::Generic, 0.0)
                .range(0.0, 1.0)
                .no_smooth()
                .doc("On while true; a level 0–1 sets the brightness."),
        )
    }

    fn new(_: &Config, _: &PrepareCtx) -> Self {
        Light {}
    }

    fn process(&mut self, ctx: &ProcessCtx, io: &mut Io) {
        let input = io.inputs.float(IN);
        let n = if input.is_varying() {
            ctx.frames
        } else {
            ctx.frames.min(1)
        };
        let level = (0..n)
            .map(|i| input.at(i))
            .filter(|v| v.is_finite())
            .fold(0.0f32, f32::max)
            .min(1.0);
        io.telemetry.set(TELEMETRY_LEVEL, level);
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_core::testing::Harness;
    use serde_json::json;

    #[test]
    fn shows_the_brightest_moment_of_a_block() {
        let mut h = Harness::new::<Light>(json!({}));
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_LEVEL), 0.0);
        // Frames are counted from the start: this is frame 30 of the second block.
        let mut gate = vec![0.0; 128];
        gate[94] = 1.0;
        h.set_samples("in", gate);
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_LEVEL), 1.0);
        h.set("in", 0.25);
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_LEVEL), 0.25);
        // Above 1 is full; NaN and negative levels are dark.
        h.set("in", 7.0);
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_LEVEL), 1.0);
        h.set_samples("in", vec![f32::NAN; 512]);
        h.run(64);
        assert_eq!(h.telemetry(TELEMETRY_LEVEL), 0.0);
    }

    #[test]
    fn colors_by_name_or_code() {
        assert_eq!(color(&json!({})), Some([0xF0, 0xA0, 0x30]));
        assert_eq!(color(&json!({"color": "Green"})), Some([0x4C, 0xD0, 0x6E]));
        assert_eq!(
            color(&json!({"color": "#102030"})),
            Some([0x10, 0x20, 0x30])
        );
        assert_eq!(color(&json!({"color": "#1020"})), None);
        assert_eq!(color(&json!({"color": "#gg0000"})), None);
        assert_eq!(color(&json!({"color": "#ééé"})), None);
        for c in COLORS {
            assert!(parse_color(c).is_some());
        }
    }
}
