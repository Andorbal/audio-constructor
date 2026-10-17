//! The look, in one place (docs/UX.md "Look"): every color, radius, spacing
//! value, and font size the editor draws with. Tune the look here.

use ac_core::{Category, SignalType};
use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Shadow, Stroke};

// ---------------------------------------------------------------- palette

/// Canvas background.
pub const CANVAS_BG: Color32 = Color32::from_rgb(0x18, 0x1A, 0x1F);
/// Dots of the canvas grid.
pub const GRID_DOT: Color32 = Color32::from_rgb(0x30, 0x34, 0x3C);
/// Every fifth dot, slightly stronger, so the grid reads at a glance.
pub const GRID_DOT_MAJOR: Color32 = Color32::from_rgb(0x3C, 0x41, 0x4A);

/// Side panel, menu bar, transport bar.
pub const PANEL_BG: Color32 = Color32::from_rgb(0x20, 0x22, 0x28);
/// Recessed areas inside panels (text fields, meter wells).
pub const WELL_BG: Color32 = Color32::from_rgb(0x15, 0x16, 0x1A);
/// Separators and subtle borders.
pub const BORDER: Color32 = Color32::from_rgb(0x30, 0x33, 0x3A);

/// Node card body. Slightly translucent so wires behind it read as dimmed.
pub const NODE_BG: Color32 = Color32::from_rgba_premultiplied(0x27, 0x2A, 0x31, 0xF2);
/// Node outline.
pub const NODE_OUTLINE: Color32 = Color32::from_rgb(0x38, 0x3C, 0x45);
/// Outline of a selected node.
pub const NODE_SELECTED: Color32 = Color32::from_rgb(0xE6, 0xEA, 0xF0);
/// Outline of a hovered node.
pub const NODE_HOVER: Color32 = Color32::from_rgb(0x5A, 0x60, 0x6B);
/// Inline widget wells inside nodes.
pub const WIDGET_BG: Color32 = Color32::from_rgb(0x1C, 0x1E, 0x23);
pub const WIDGET_BG_HOVER: Color32 = Color32::from_rgb(0x22, 0x25, 0x2B);
/// Value fill of drag sliders.
pub const WIDGET_FILL: Color32 = Color32::from_rgb(0x34, 0x4A, 0x66);
pub const WIDGET_FILL_HOVER: Color32 = Color32::from_rgb(0x3D, 0x57, 0x78);

pub const TEXT: Color32 = Color32::from_rgb(0xD4, 0xD8, 0xDF);
pub const TEXT_STRONG: Color32 = Color32::from_rgb(0xF2, 0xF4, 0xF7);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x8E, 0x95, 0xA0);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x5E, 0x65, 0x70);

/// Interface accent (focus, selection, active tab).
pub const ACCENT: Color32 = Color32::from_rgb(0x7C, 0xA7, 0xF5);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x2E, 0x3D, 0x59);

pub const ERROR: Color32 = Color32::from_rgb(0xF0, 0x6A, 0x6A);
pub const WARNING: Color32 = Color32::from_rgb(0xE8, 0xB0, 0x4A);
pub const OK: Color32 = Color32::from_rgb(0x5F, 0xC8, 0x8F);
pub const RECORD: Color32 = Color32::from_rgb(0xE8, 0x4A, 0x4A);
pub const PLAY: Color32 = Color32::from_rgb(0x5F, 0xC8, 0x8F);

/// Box selection.
pub const SELECT_FILL: Color32 = Color32::from_rgba_premultiplied(0x18, 0x22, 0x33, 0x40);
pub const SELECT_STROKE: Color32 = Color32::from_rgb(0x7C, 0xA7, 0xF5);
/// Knife line for cutting wires.
pub const CUT_LINE: Color32 = Color32::from_rgb(0xF0, 0x6A, 0x6A);
/// Line drawn while Shift+right-dragging to add reroutes.
pub const REROUTE_LINE: Color32 = Color32::from_rgb(0xA9, 0xB4, 0xC2);

/// Frames: translucent fill, label color.
pub const FRAME_FILL: Color32 = Color32::from_rgba_premultiplied(0x2A, 0x2E, 0x36, 0x70);
pub const FRAME_BORDER: Color32 = Color32::from_rgb(0x3A, 0x3F, 0x48);
pub const FRAME_LABEL: Color32 = Color32::from_rgb(0xA8, 0xAF, 0xBA);

/// Comment sticky notes.
pub const NOTE_BG: Color32 = Color32::from_rgb(0x3B, 0x37, 0x26);
pub const NOTE_HEADER: Color32 = Color32::from_rgb(0x4A, 0x44, 0x2A);
pub const NOTE_TEXT: Color32 = Color32::from_rgb(0xEC, 0xE4, 0xC4);

/// Meters: green → yellow → red as the level approaches full scale.
pub const METER_LOW: Color32 = Color32::from_rgb(0x5F, 0xC8, 0x8F);
pub const METER_MID: Color32 = Color32::from_rgb(0xE8, 0xC2, 0x4A);
pub const METER_HIGH: Color32 = Color32::from_rgb(0xF0, 0x6A, 0x6A);
pub const METER_BG: Color32 = Color32::from_rgb(0x14, 0x15, 0x19);

/// Scope trace and its background.
pub const SCOPE_BG: Color32 = Color32::from_rgb(0x12, 0x14, 0x17);
pub const SCOPE_GRID: Color32 = Color32::from_rgb(0x24, 0x28, 0x2F);

/// Signal type colors (UX.md socket table). `Any` (reroutes before they
/// connect) is a neutral grey.
pub fn type_color(ty: SignalType) -> Color32 {
    match ty {
        SignalType::Audio => Color32::from_rgb(0xF0, 0xA0, 0x30),
        SignalType::Float => Color32::from_rgb(0x6F, 0xB1, 0xE6),
        SignalType::Int => Color32::from_rgb(0x5F, 0xC8, 0x8F),
        SignalType::Bool => Color32::from_rgb(0xE0, 0x6C, 0x9F),
        SignalType::Trigger => Color32::from_rgb(0xF2, 0xD1, 0x5C),
        SignalType::Midi => Color32::from_rgb(0xA9, 0x8B, 0xF0),
        SignalType::Any => Color32::from_rgb(0x9A, 0xA0, 0xA8),
    }
}

/// Socket shape: stream types are circles, events diamonds (the
/// colorblind-safe channel). Spectrum and Buffer (Phase 2) will be squares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketShape {
    Circle,
    Diamond,
    Square,
}

pub fn socket_shape(ty: SignalType) -> SocketShape {
    if ty.is_event() {
        SocketShape::Diamond
    } else {
        SocketShape::Circle
    }
}

/// Header color per library category (UX.md).
pub fn category_color(cat: Category) -> Color32 {
    match cat {
        Category::Io => Color32::from_rgb(0xC8, 0x55, 0x55),
        Category::Generators => Color32::from_rgb(0xDB, 0x7E, 0x3E),
        Category::Level => Color32::from_rgb(0xC9, 0xA4, 0x3E),
        Category::Filters => Color32::from_rgb(0x5C, 0xA6, 0x5E),
        Category::Time => Color32::from_rgb(0x3A, 0xA3, 0x98),
        Category::Spectral => Color32::from_rgb(0x3C, 0xB2, 0xC8),
        Category::Math => Color32::from_rgb(0x4E, 0x84, 0xD6),
        Category::Modulation => Color32::from_rgb(0x6E, 0x66, 0xCC),
        Category::Triggers => Color32::from_rgb(0xD6, 0xBE, 0x40),
        Category::Sequencing => Color32::from_rgb(0x98, 0x6C, 0xD4),
        Category::Controls => Color32::from_rgb(0xD4, 0x64, 0x9E),
        Category::Display => Color32::from_rgb(0x82, 0x88, 0x92),
        Category::Utility => Color32::from_rgb(0x62, 0x73, 0x86),
    }
}

/// Header color for nodes whose kind is unknown (missing node).
pub const MISSING_HEADER: Color32 = Color32::from_rgb(0x6A, 0x3A, 0x3A);
/// Header color for group instances and the group interface nodes.
pub const GROUP_HEADER: Color32 = Color32::from_rgb(0x4C, 0x8C, 0x7A);

/// The header fill: the category color mixed into the card so white text
/// stays readable on every hue.
pub fn header_fill(accent: Color32) -> Color32 {
    mix(Color32::from_rgb(0x2B, 0x2E, 0x35), accent, 0.42)
}

/// Linear mix of two colors (`t` = 0 → `a`, 1 → `b`), in gamma space.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        l(a.r(), b.r()),
        l(a.g(), b.g()),
        l(a.b(), b.b()),
        l(a.a(), b.a()),
    )
}

/// `c` with its opacity multiplied by `alpha`.
pub fn fade(c: Color32, alpha: f32) -> Color32 {
    c.gamma_multiply(alpha.clamp(0.0, 1.0))
}

/// Meter color for a level in dBFS.
pub fn meter_color(db: f32) -> Color32 {
    if db > -3.0 {
        METER_HIGH
    } else if db > -12.0 {
        METER_MID
    } else {
        METER_LOW
    }
}

// ---------------------------------------------------------------- geometry

/// Node cards (canvas units; multiplied by zoom when drawn).
pub const NODE_WIDTH: f32 = 184.0;
pub const NODE_RADIUS: f32 = 7.0;
pub const HEADER_HEIGHT: f32 = 26.0;
pub const HEADER_STRIP: f32 = 3.0;
pub const ROW_HEIGHT: f32 = 22.0;
pub const NODE_PAD_X: f32 = 10.0;
pub const NODE_PAD_BOTTOM: f32 = 6.0;
pub const SOCKET_RADIUS: f32 = 5.0;
/// Pointer distance (screen points) within which a socket counts as hit.
pub const SOCKET_HIT: f32 = 10.0;
/// Pointer distance within which a dragged wire snaps to a socket.
pub const SOCKET_SNAP: f32 = 22.0;
pub const WIDGET_HEIGHT: f32 = 18.0;
pub const WIDGET_RADIUS: f32 = 4.0;
pub const BODY_PAD: f32 = 8.0;
/// Collapsed nodes become a pill of this height.
pub const PILL_HEIGHT: f32 = 26.0;
pub const REROUTE_RADIUS: f32 = 6.0;
/// Canvas grid spacing; Ctrl-drag snaps to it.
pub const GRID: f32 = 20.0;
pub const FRAME_RADIUS: f32 = 8.0;
pub const FRAME_PAD: f32 = 24.0;
pub const FRAME_HEADER: f32 = 28.0;

/// Wires.
pub const WIRE_WIDTH: f32 = 2.0;
pub const WIRE_GLOW: f32 = 7.0;
/// Pointer distance (screen points) within which a wire counts as hovered.
pub const WIRE_HIT: f32 = 6.0;

/// Zoom limits (UX.md: 10 % to 300 %).
pub const ZOOM_MIN: f32 = 0.1;
pub const ZOOM_MAX: f32 = 3.0;

/// Motion: nothing takes longer than 150 ms, nothing bounces (UX.md).
pub const ANIM_SECONDS: f32 = 0.15;
pub const SNAP_SECONDS: f32 = 0.08;

pub fn node_shadow() -> Shadow {
    Shadow {
        offset: [0, 3],
        blur: 14,
        spread: 0,
        color: Color32::from_black_alpha(110),
    }
}

pub fn popup_shadow() -> Shadow {
    Shadow {
        offset: [0, 6],
        blur: 22,
        spread: 0,
        color: Color32::from_black_alpha(140),
    }
}

pub fn radius(r: f32) -> CornerRadius {
    CornerRadius::same(r.round().clamp(0.0, 255.0) as u8)
}

// ---------------------------------------------------------------- type

/// Base sizes (points) of the type scale.
pub const FONT_UI: f32 = 13.5;
pub const FONT_SMALL: f32 = 11.5;
pub const FONT_HEADING: f32 = 16.0;
pub const FONT_TITLE: f32 = 20.0;
pub const FONT_VALUE: f32 = 12.0;
pub const FONT_NODE_TITLE: f32 = 13.0;
pub const FONT_NODE_LABEL: f32 = 12.0;
pub const FONT_ICON: f32 = 15.0;

/// The family holding the filled icon weight (transport buttons).
pub fn icons_fill_family() -> FontFamily {
    FontFamily::Name("icons-fill".into())
}

pub fn ui_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn mono_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// Font sizes on the canvas follow the zoom; quantizing them keeps egui's
/// glyph cache from filling up with every intermediate size during a zoom.
pub fn canvas_font(base: f32, zoom: f32, mono: bool) -> FontId {
    let size = (base * zoom * 2.0).round() / 2.0;
    let size = size.max(1.0);
    if mono { mono_font(size) } else { ui_font(size) }
}

/// Install Inter (UI), JetBrains Mono (values), and the Phosphor icon fonts.
pub fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let add = |fonts: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        fonts
            .font_data
            .insert(name.to_string(), FontData::from_static(bytes).into());
    };
    add(
        &mut fonts,
        "inter",
        include_bytes!("../assets/fonts/Inter-Variable.ttf"),
    );
    add(
        &mut fonts,
        "jetbrains-mono",
        include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    );
    add(
        &mut fonts,
        "phosphor",
        include_bytes!("../assets/fonts/Phosphor-Regular.ttf"),
    );
    add(
        &mut fonts,
        "phosphor-fill",
        include_bytes!("../assets/fonts/Phosphor-Fill.ttf"),
    );
    let defaults_prop = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let defaults_mono = fonts
        .families
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();
    // The icon fonts map only Private Use Area codepoints, so they go first:
    // Inter has glyphs of its own in that area that would otherwise win.
    let mut prop = vec!["phosphor".to_string(), "inter".to_string()];
    prop.extend(defaults_prop.iter().cloned());
    let mut mono = vec![
        "phosphor".to_string(),
        "jetbrains-mono".to_string(),
        "inter".to_string(),
    ];
    mono.extend(defaults_mono.iter().cloned());
    let mut fill = vec!["phosphor-fill".to_string(), "inter".to_string()];
    fill.extend(defaults_prop);
    fonts.families.insert(FontFamily::Proportional, prop);
    fonts.families.insert(FontFamily::Monospace, mono);
    fonts.families.insert(icons_fill_family(), fill);
    fonts
}

/// Apply the dark theme and type scale to a context.
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, style);
}

fn style(style: &mut egui::Style) {
    use egui::{FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Small, ui_font(FONT_SMALL)),
        (TextStyle::Body, ui_font(FONT_UI)),
        (TextStyle::Button, ui_font(FONT_UI)),
        (TextStyle::Heading, ui_font(FONT_HEADING)),
        (TextStyle::Monospace, mono_font(FONT_VALUE)),
    ]
    .into_iter()
    .collect::<std::collections::BTreeMap<TextStyle, FontId>>();

    let s = &mut style.spacing;
    s.item_spacing = egui::vec2(8.0, 6.0);
    s.button_padding = egui::vec2(8.0, 3.0);
    s.interact_size = egui::vec2(36.0, 22.0);
    s.menu_margin = egui::Margin::same(6);
    s.window_margin = egui::Margin::same(10);
    s.indent = 14.0;
    s.slider_width = 140.0;
    s.combo_width = 120.0;
    s.text_edit_width = 160.0;
    s.tooltip_width = 320.0;
    s.menu_width = 220.0;

    let v = &mut style.visuals;
    v.dark_mode = true;
    v.override_text_color = None;
    v.panel_fill = PANEL_BG;
    v.window_fill = Color32::from_rgb(0x25, 0x28, 0x2F);
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = radius(10.0);
    v.menu_corner_radius = radius(8.0);
    v.window_shadow = popup_shadow();
    v.popup_shadow = popup_shadow();
    v.extreme_bg_color = WELL_BG;
    v.faint_bg_color = Color32::from_rgb(0x24, 0x27, 0x2D);
    v.code_bg_color = Color32::from_rgb(0x2C, 0x30, 0x38);
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARNING;
    v.error_fg_color = ERROR;
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.slider_trailing_fill = true;
    v.striped = false;
    v.indent_has_left_vline = false;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PANEL_BG;
    w.noninteractive.weak_bg_fill = PANEL_BG;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.noninteractive.corner_radius = radius(WIDGET_RADIUS + 1.0);

    w.inactive.bg_fill = Color32::from_rgb(0x2E, 0x32, 0x3A);
    w.inactive.weak_bg_fill = Color32::from_rgb(0x2B, 0x2E, 0x35);
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = radius(WIDGET_RADIUS + 1.0);

    w.hovered.bg_fill = Color32::from_rgb(0x38, 0x3D, 0x47);
    w.hovered.weak_bg_fill = Color32::from_rgb(0x35, 0x39, 0x42);
    w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x4A, 0x50, 0x5B));
    w.hovered.fg_stroke = Stroke::new(1.5, TEXT_STRONG);
    w.hovered.corner_radius = radius(WIDGET_RADIUS + 1.0);
    w.hovered.expansion = 0.0;

    w.active.bg_fill = Color32::from_rgb(0x3F, 0x46, 0x53);
    w.active.weak_bg_fill = Color32::from_rgb(0x3F, 0x46, 0x53);
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.5, TEXT_STRONG);
    w.active.corner_radius = radius(WIDGET_RADIUS + 1.0);
    w.active.expansion = 0.0;

    w.open.bg_fill = Color32::from_rgb(0x33, 0x37, 0x40);
    w.open.weak_bg_fill = Color32::from_rgb(0x33, 0x37, 0x40);
    w.open.bg_stroke = Stroke::new(1.0, BORDER);
    w.open.fg_stroke = Stroke::new(1.0, TEXT_STRONG);
    w.open.corner_radius = radius(WIDGET_RADIUS + 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_has_a_distinct_color() {
        use SignalType::*;
        let types = [Audio, Float, Int, Bool, Trigger, Midi, Any];
        for (i, a) in types.iter().enumerate() {
            for b in &types[i + 1..] {
                assert_ne!(type_color(*a), type_color(*b), "{a} vs {b}");
            }
        }
        assert_eq!(socket_shape(Trigger), SocketShape::Diamond);
        assert_eq!(socket_shape(Audio), SocketShape::Circle);
    }

    #[test]
    fn mix_endpoints() {
        let a = Color32::from_rgb(0, 0, 0);
        let b = Color32::from_rgb(200, 100, 50);
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
    }

    #[test]
    fn canvas_fonts_are_quantized() {
        assert_eq!(canvas_font(13.0, 1.0, false).size, 13.0);
        assert_eq!(canvas_font(13.0, 0.51, false).size, 6.5);
        assert!(canvas_font(13.0, 0.01, true).size >= 1.0);
    }
}
