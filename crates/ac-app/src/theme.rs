//! The look, in one place (docs/UX.md "Look"): every color, radius, spacing
//! value, and font size the editor draws with. Tune the look here.

use std::cell::Cell;

use ac_core::{Category, SignalType};
use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Shadow, Stroke};

// ---------------------------------------------------------------- palette

/// Every color the editor draws with, for one theme. Read the current one
/// through the accessor functions below (`theme::canvas_bg()`, ...).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Canvas background.
    pub canvas_bg: Color32,
    /// Dots of the canvas grid.
    pub grid_dot: Color32,
    /// Every fifth dot, slightly stronger, so the grid reads at a glance.
    pub grid_dot_major: Color32,
    /// Side panel, menu bar, transport bar.
    pub panel_bg: Color32,
    /// Recessed areas inside panels (text fields, meter wells).
    pub well_bg: Color32,
    /// Separators and subtle borders.
    pub border: Color32,
    /// Node card body. Slightly translucent so wires behind it read as dimmed.
    pub node_bg: Color32,
    /// Node outline.
    pub node_outline: Color32,
    /// Outline of a selected node.
    pub node_selected: Color32,
    /// Outline of a hovered node.
    pub node_hover: Color32,
    /// Inline widget wells inside nodes.
    pub widget_bg: Color32,
    pub widget_bg_hover: Color32,
    /// Value fill of drag sliders.
    pub widget_fill: Color32,
    pub widget_fill_hover: Color32,
    pub text: Color32,
    pub text_strong: Color32,
    pub text_muted: Color32,
    pub text_faint: Color32,
    /// Interface accent (focus, selection, active tab).
    pub accent: Color32,
    pub accent_dim: Color32,
    pub error: Color32,
    pub warning: Color32,
    pub ok: Color32,
    pub record: Color32,
    pub play: Color32,
    /// Box selection.
    pub select_fill: Color32,
    pub select_stroke: Color32,
    /// Knife line for cutting wires.
    pub cut_line: Color32,
    /// Line drawn while Shift+right-dragging to add reroutes.
    pub reroute_line: Color32,
    /// Frames: translucent fill, label color.
    pub frame_fill: Color32,
    pub frame_border: Color32,
    pub frame_label: Color32,
    /// Comment sticky notes.
    pub note_bg: Color32,
    pub note_header: Color32,
    pub note_text: Color32,
    /// Meters: green → yellow → red as the level approaches full scale.
    pub meter_low: Color32,
    pub meter_mid: Color32,
    pub meter_high: Color32,
    pub meter_bg: Color32,
    /// Scope trace and its background.
    pub scope_bg: Color32,
    pub scope_grid: Color32,
    /// Header color for nodes whose kind is unknown (missing node).
    pub missing_header: Color32,
    /// Header color for group instances and the group interface nodes.
    pub group_header: Color32,
    /// A dark theme (egui's visuals follow it).
    pub dark: bool,
    /// Windows and popups.
    pub window_bg: Color32,
    /// Faint backgrounds (striped rows).
    pub faint_bg: Color32,
    /// Inline code in docs: its background and text.
    pub code_bg: Color32,
    pub code_text: Color32,
    /// Panel buttons and other widgets: resting, hovered, pressed, and open
    /// (a menu button whose menu shows).
    pub button_bg: Color32,
    pub button_weak_bg: Color32,
    pub button_hover_bg: Color32,
    pub button_hover_weak_bg: Color32,
    pub button_hover_stroke: Color32,
    pub button_active_bg: Color32,
    pub button_open_bg: Color32,
    /// A hovered row in a list (library, palette).
    pub row_hover: Color32,
    /// Raised surfaces in widget bodies (button tops, knob caps), hovered,
    /// the lip under a button, and the shade under caps and dots.
    pub raised: Color32,
    pub raised_hover: Color32,
    pub raised_lip: Color32,
    pub body_shadow: Color32,
    /// A switched-off toggle's track, and hovered.
    pub toggle_off: Color32,
    pub toggle_hover: Color32,
    /// The ring around socket dots.
    pub socket_edge: Color32,
    /// Marks drawn on an accent fill (a checked box's tick).
    pub on_accent: Color32,
    /// The casing under wires, so crossing wires stay apart.
    pub wire_casing: Color32,
    /// What event flashes and flow dashes on wires brighten towards.
    pub wire_flash: Color32,
    /// Pills over the canvas: the group breadcrumb and gesture hints.
    pub breadcrumb_bg: Color32,
    pub hint_bg: Color32,
    /// Shadows under node cards and popups.
    pub node_shadow: Color32,
    pub popup_shadow: Color32,
    /// Node headers mix their category color into this base, by
    /// `header_mix`, so the title stays readable on every hue.
    pub header_base: Color32,
    pub header_mix: f32,
    /// Signal type colors, in [`TYPE_ORDER`].
    pub types: [Color32; 9],
    /// Category header colors, in `Category::ALL` order.
    pub categories: [Color32; 13],
}

/// The order of [`Palette::types`].
pub const TYPE_ORDER: [SignalType; 9] = [
    SignalType::Audio,
    SignalType::Float,
    SignalType::Int,
    SignalType::Bool,
    SignalType::Trigger,
    SignalType::Midi,
    SignalType::Spectrum,
    SignalType::Buffer,
    SignalType::Any,
];

/// The dark theme, the default.
pub const DARK: Palette = Palette {
    canvas_bg: Color32::from_rgb(0x18, 0x1A, 0x1F),
    grid_dot: Color32::from_rgb(0x30, 0x34, 0x3C),
    grid_dot_major: Color32::from_rgb(0x3C, 0x41, 0x4A),
    panel_bg: Color32::from_rgb(0x20, 0x22, 0x28),
    well_bg: Color32::from_rgb(0x15, 0x16, 0x1A),
    border: Color32::from_rgb(0x30, 0x33, 0x3A),
    node_bg: Color32::from_rgba_premultiplied(0x25, 0x28, 0x2E, 0xEA),
    node_outline: Color32::from_rgb(0x38, 0x3C, 0x45),
    node_selected: Color32::from_rgb(0xE6, 0xEA, 0xF0),
    node_hover: Color32::from_rgb(0x5A, 0x60, 0x6B),
    widget_bg: Color32::from_rgb(0x1C, 0x1E, 0x23),
    widget_bg_hover: Color32::from_rgb(0x22, 0x25, 0x2B),
    widget_fill: Color32::from_rgb(0x34, 0x4A, 0x66),
    widget_fill_hover: Color32::from_rgb(0x3D, 0x57, 0x78),
    text: Color32::from_rgb(0xD4, 0xD8, 0xDF),
    text_strong: Color32::from_rgb(0xF2, 0xF4, 0xF7),
    text_muted: Color32::from_rgb(0x95, 0x9C, 0xA7),
    text_faint: Color32::from_rgb(0x5E, 0x65, 0x70),
    accent: Color32::from_rgb(0x7C, 0xA7, 0xF5),
    accent_dim: Color32::from_rgb(0x2E, 0x3D, 0x59),
    error: Color32::from_rgb(0xF0, 0x6A, 0x6A),
    warning: Color32::from_rgb(0xE8, 0xB0, 0x4A),
    ok: Color32::from_rgb(0x5F, 0xC8, 0x8F),
    record: Color32::from_rgb(0xE8, 0x4A, 0x4A),
    play: Color32::from_rgb(0x5F, 0xC8, 0x8F),
    select_fill: Color32::from_rgba_premultiplied(0x18, 0x22, 0x33, 0x40),
    select_stroke: Color32::from_rgb(0x7C, 0xA7, 0xF5),
    cut_line: Color32::from_rgb(0xF0, 0x6A, 0x6A),
    reroute_line: Color32::from_rgb(0xA9, 0xB4, 0xC2),
    frame_fill: Color32::from_rgba_premultiplied(0x2A, 0x2E, 0x36, 0x70),
    frame_border: Color32::from_rgb(0x3A, 0x3F, 0x48),
    frame_label: Color32::from_rgb(0xA8, 0xAF, 0xBA),
    note_bg: Color32::from_rgb(0x3B, 0x37, 0x26),
    note_header: Color32::from_rgb(0x4A, 0x44, 0x2A),
    note_text: Color32::from_rgb(0xEC, 0xE4, 0xC4),
    meter_low: Color32::from_rgb(0x5F, 0xC8, 0x8F),
    meter_mid: Color32::from_rgb(0xE8, 0xC2, 0x4A),
    meter_high: Color32::from_rgb(0xF0, 0x6A, 0x6A),
    meter_bg: Color32::from_rgb(0x14, 0x15, 0x19),
    scope_bg: Color32::from_rgb(0x12, 0x14, 0x17),
    scope_grid: Color32::from_rgb(0x24, 0x28, 0x2F),
    missing_header: Color32::from_rgb(0x6A, 0x3A, 0x3A),
    group_header: Color32::from_rgb(0x4C, 0x8C, 0x7A),
    dark: true,
    window_bg: Color32::from_rgb(0x25, 0x28, 0x2F),
    faint_bg: Color32::from_rgb(0x24, 0x27, 0x2D),
    code_bg: Color32::from_rgb(0x2C, 0x30, 0x38),
    code_text: Color32::from_rgb(0xE6, 0xC0, 0x8A),
    button_bg: Color32::from_rgb(0x2E, 0x32, 0x3A),
    button_weak_bg: Color32::from_rgb(0x2B, 0x2E, 0x35),
    button_hover_bg: Color32::from_rgb(0x38, 0x3D, 0x47),
    button_hover_weak_bg: Color32::from_rgb(0x35, 0x39, 0x42),
    button_hover_stroke: Color32::from_rgb(0x4A, 0x50, 0x5B),
    button_active_bg: Color32::from_rgb(0x3F, 0x46, 0x53),
    button_open_bg: Color32::from_rgb(0x33, 0x37, 0x40),
    row_hover: Color32::from_rgb(0x2B, 0x2F, 0x37),
    raised: Color32::from_rgb(0x32, 0x36, 0x3F),
    raised_hover: Color32::from_rgb(0x3A, 0x3F, 0x49),
    raised_lip: Color32::from_rgb(0x1A, 0x1C, 0x21),
    body_shadow: Color32::from_rgb(0x15, 0x16, 0x1A),
    toggle_off: Color32::from_rgb(0x30, 0x34, 0x3C),
    toggle_hover: Color32::from_rgb(0x3A, 0x3F, 0x49),
    socket_edge: Color32::from_rgb(0x12, 0x13, 0x16),
    on_accent: Color32::from_rgb(0x10, 0x14, 0x1C),
    wire_casing: Color32::from_black_alpha(70),
    wire_flash: Color32::WHITE,
    breadcrumb_bg: Color32::from_rgba_premultiplied(0x2A, 0x3A, 0x36, 0xE8),
    hint_bg: Color32::from_rgba_premultiplied(0x10, 0x11, 0x14, 0xD8),
    node_shadow: Color32::from_black_alpha(110),
    popup_shadow: Color32::from_black_alpha(140),
    header_base: Color32::from_rgb(0x2B, 0x2E, 0x35),
    header_mix: 0.42,
    // UX.md's socket table.
    types: [
        Color32::from_rgb(0xF0, 0xA0, 0x30),
        Color32::from_rgb(0x6F, 0xB1, 0xE6),
        Color32::from_rgb(0x5F, 0xC8, 0x8F),
        Color32::from_rgb(0xE0, 0x6C, 0x9F),
        Color32::from_rgb(0xF2, 0xD1, 0x5C),
        Color32::from_rgb(0xA9, 0x8B, 0xF0),
        Color32::from_rgb(0x4F, 0xD1, 0xD9),
        Color32::from_rgb(0xC8, 0xA8, 0x78),
        Color32::from_rgb(0x9A, 0xA0, 0xA8),
    ],
    categories: [
        Color32::from_rgb(0xC8, 0x55, 0x55),
        Color32::from_rgb(0xDB, 0x7E, 0x3E),
        Color32::from_rgb(0xC9, 0xA4, 0x3E),
        Color32::from_rgb(0x5C, 0xA6, 0x5E),
        Color32::from_rgb(0x3A, 0xA3, 0x98),
        Color32::from_rgb(0x3C, 0xB2, 0xC8),
        Color32::from_rgb(0x4E, 0x84, 0xD6),
        Color32::from_rgb(0x6E, 0x66, 0xCC),
        Color32::from_rgb(0xD6, 0xBE, 0x40),
        Color32::from_rgb(0x98, 0x6C, 0xD4),
        Color32::from_rgb(0xD4, 0x64, 0x9E),
        Color32::from_rgb(0x82, 0x88, 0x92),
        Color32::from_rgb(0x62, 0x73, 0x86),
    ],
};

/// The light theme: the dark theme's hues, deepened so text, wires, and
/// sockets keep their contrast on light cards (body text at least 4.5:1,
/// signal colors at least 3:1; the tests check).
pub const LIGHT: Palette = Palette {
    canvas_bg: Color32::from_rgb(0xE9, 0xEB, 0xEF),
    grid_dot: Color32::from_rgb(0xC9, 0xCD, 0xD5),
    grid_dot_major: Color32::from_rgb(0xB4, 0xBA, 0xC4),
    panel_bg: Color32::from_rgb(0xF6, 0xF7, 0xF9),
    well_bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    border: Color32::from_rgb(0xD3, 0xD7, 0xDE),
    node_bg: Color32::from_rgba_unmultiplied_const(0xFD, 0xFD, 0xFE, 0xF0),
    node_outline: Color32::from_rgb(0xC3, 0xC8, 0xD0),
    node_selected: Color32::from_rgb(0x2B, 0x30, 0x3A),
    node_hover: Color32::from_rgb(0x8A, 0x91, 0x9C),
    widget_bg: Color32::from_rgb(0xE6, 0xE9, 0xEE),
    widget_bg_hover: Color32::from_rgb(0xDC, 0xE0, 0xE6),
    widget_fill: Color32::from_rgb(0xC5, 0xD6, 0xEF),
    widget_fill_hover: Color32::from_rgb(0xB3, 0xC9, 0xE9),
    text: Color32::from_rgb(0x26, 0x2A, 0x31),
    text_strong: Color32::from_rgb(0x11, 0x14, 0x18),
    text_muted: Color32::from_rgb(0x58, 0x61, 0x70),
    text_faint: Color32::from_rgb(0x7A, 0x82, 0x90),
    accent: Color32::from_rgb(0x25, 0x60, 0xC4),
    accent_dim: Color32::from_rgb(0xDD, 0xE8, 0xFA),
    error: Color32::from_rgb(0xC0, 0x26, 0x26),
    warning: Color32::from_rgb(0x94, 0x57, 0x00),
    ok: Color32::from_rgb(0x16, 0x73, 0x3D),
    record: Color32::from_rgb(0xD3, 0x2F, 0x2F),
    play: Color32::from_rgb(0x16, 0x73, 0x3D),
    select_fill: Color32::from_rgba_unmultiplied_const(0x25, 0x60, 0xC4, 0x30),
    select_stroke: Color32::from_rgb(0x25, 0x60, 0xC4),
    cut_line: Color32::from_rgb(0xC0, 0x26, 0x26),
    reroute_line: Color32::from_rgb(0x58, 0x61, 0x70),
    frame_fill: Color32::from_rgba_unmultiplied_const(0x7A, 0x85, 0x99, 0x26),
    frame_border: Color32::from_rgb(0xB9, 0xBF, 0xC9),
    frame_label: Color32::from_rgb(0x4A, 0x51, 0x5C),
    note_bg: Color32::from_rgb(0xFF, 0xF3, 0xC4),
    note_header: Color32::from_rgb(0xF5, 0xE3, 0x9C),
    note_text: Color32::from_rgb(0x3D, 0x34, 0x15),
    meter_low: Color32::from_rgb(0x2E, 0x9E, 0x5B),
    meter_mid: Color32::from_rgb(0xD9, 0xA5, 0x20),
    meter_high: Color32::from_rgb(0xD6, 0x45, 0x45),
    meter_bg: Color32::from_rgb(0xDA, 0xDE, 0xE4),
    scope_bg: Color32::from_rgb(0xF3, 0xF5, 0xF8),
    scope_grid: Color32::from_rgb(0xDC, 0xE0, 0xE6),
    missing_header: Color32::from_rgb(0xB0, 0x5A, 0x5A),
    group_header: Color32::from_rgb(0x3D, 0x82, 0x70),
    dark: false,
    window_bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    faint_bg: Color32::from_rgb(0xEE, 0xF0, 0xF3),
    code_bg: Color32::from_rgb(0xE8, 0xEB, 0xF0),
    code_text: Color32::from_rgb(0x8A, 0x4B, 0x00),
    button_bg: Color32::from_rgb(0xE3, 0xE6, 0xEB),
    button_weak_bg: Color32::from_rgb(0xE8, 0xEB, 0xEF),
    button_hover_bg: Color32::from_rgb(0xD8, 0xDC, 0xE3),
    button_hover_weak_bg: Color32::from_rgb(0xDC, 0xE0, 0xE6),
    button_hover_stroke: Color32::from_rgb(0xB3, 0xBA, 0xC5),
    button_active_bg: Color32::from_rgb(0xCB, 0xD2, 0xDD),
    button_open_bg: Color32::from_rgb(0xDC, 0xE0, 0xE6),
    row_hover: Color32::from_rgb(0xE6, 0xE9, 0xEE),
    raised: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    raised_hover: Color32::from_rgb(0xEE, 0xF1, 0xF5),
    raised_lip: Color32::from_rgb(0xC3, 0xC8, 0xD0),
    body_shadow: Color32::from_rgb(0xA9, 0xB0, 0xBB),
    toggle_off: Color32::from_rgb(0xCD, 0xD2, 0xDA),
    toggle_hover: Color32::from_rgb(0xBE, 0xC4, 0xCE),
    socket_edge: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    on_accent: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    wire_casing: Color32::from_rgba_unmultiplied_const(0xFF, 0xFF, 0xFF, 150),
    wire_flash: Color32::from_rgb(0x1B, 0x1F, 0x27),
    breadcrumb_bg: Color32::from_rgba_unmultiplied_const(0xD5, 0xEC, 0xE6, 0xE8),
    hint_bg: Color32::from_rgba_unmultiplied_const(0xFF, 0xFF, 0xFF, 0xE8),
    node_shadow: Color32::from_black_alpha(40),
    popup_shadow: Color32::from_black_alpha(56),
    header_base: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    header_mix: 0.2,
    types: [
        Color32::from_rgb(0xC4, 0x6F, 0x00),
        Color32::from_rgb(0x2B, 0x7B, 0xC0),
        Color32::from_rgb(0x1F, 0x8A, 0x55),
        Color32::from_rgb(0xC2, 0x40, 0x7D),
        Color32::from_rgb(0x97, 0x7C, 0x00),
        Color32::from_rgb(0x7A, 0x55, 0xD6),
        Color32::from_rgb(0x13, 0x8A, 0x94),
        Color32::from_rgb(0x9A, 0x74, 0x44),
        Color32::from_rgb(0x7A, 0x81, 0x8C),
    ],
    categories: [
        Color32::from_rgb(0xC0, 0x48, 0x48),
        Color32::from_rgb(0xD0, 0x6A, 0x22),
        Color32::from_rgb(0xA8, 0x82, 0x1E),
        Color32::from_rgb(0x3F, 0x96, 0x45),
        Color32::from_rgb(0x22, 0x97, 0x8B),
        Color32::from_rgb(0x1F, 0x9D, 0xB5),
        Color32::from_rgb(0x3A, 0x72, 0xC8),
        Color32::from_rgb(0x5B, 0x52, 0xC0),
        Color32::from_rgb(0xA6, 0x8A, 0x10),
        Color32::from_rgb(0x86, 0x56, 0xC8),
        Color32::from_rgb(0xC6, 0x4F, 0x8C),
        Color32::from_rgb(0x7A, 0x81, 0x8C),
        Color32::from_rgb(0x5A, 0x6B, 0x80),
    ],
};

/// Which theme the editor shows (View ▸ Theme), kept in the app settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    #[default]
    Dark,
    Light,
    /// The operating system's light or dark mode.
    System,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::Dark, ThemeChoice::Light, ThemeChoice::System];

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::Dark => "Dark",
            ThemeChoice::Light => "Light",
            ThemeChoice::System => "Follow System",
        }
    }

    fn preference(self) -> egui::ThemePreference {
        match self {
            ThemeChoice::Dark => egui::ThemePreference::Dark,
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::System => egui::ThemePreference::System,
        }
    }
}

/// The palette for one of egui's two themes.
pub fn palette_of(theme: egui::Theme) -> &'static Palette {
    match theme {
        egui::Theme::Dark => &DARK,
        egui::Theme::Light => &LIGHT,
    }
}

/// Show `choice` from now on: egui's visuals switch at once, and so does
/// the palette (Follow System resolves through the OS's mode).
pub fn set_choice(ctx: &egui::Context, choice: ThemeChoice) {
    ctx.set_theme(choice.preference());
    follow(ctx);
}

/// Draw with the palette of the theme egui shows. Called at the start of
/// every frame, so a change of the OS's mode follows too.
pub fn follow(ctx: &egui::Context) {
    set_palette(palette_of(ctx.theme()));
}

thread_local! {
    /// The palette the UI thread draws with. Per thread, so headless tests
    /// running side by side can't change each other's colors.
    static CURRENT: Cell<&'static Palette> = const { Cell::new(&DARK) };
}

/// The palette in use on this thread.
pub fn palette() -> &'static Palette {
    CURRENT.with(Cell::get)
}

/// Draw with `p` from now on (on this thread).
pub fn set_palette(p: &'static Palette) {
    CURRENT.with(|c| c.set(p));
}

/// Canvas background.
pub fn canvas_bg() -> Color32 {
    palette().canvas_bg
}
/// Dots of the canvas grid.
pub fn grid_dot() -> Color32 {
    palette().grid_dot
}
/// Every fifth dot, slightly stronger, so the grid reads at a glance.
pub fn grid_dot_major() -> Color32 {
    palette().grid_dot_major
}

/// Side panel, menu bar, transport bar.
pub fn panel_bg() -> Color32 {
    palette().panel_bg
}
/// Recessed areas inside panels (text fields, meter wells).
pub fn well_bg() -> Color32 {
    palette().well_bg
}
/// Separators and subtle borders.
pub fn border() -> Color32 {
    palette().border
}

/// Node card body. Slightly translucent so wires behind it read as dimmed.
pub fn node_bg() -> Color32 {
    palette().node_bg
}
/// Node outline.
pub fn node_outline() -> Color32 {
    palette().node_outline
}
/// Outline of a selected node.
pub fn node_selected() -> Color32 {
    palette().node_selected
}
/// Outline of a hovered node.
pub fn node_hover() -> Color32 {
    palette().node_hover
}
/// Inline widget wells inside nodes.
pub fn widget_bg() -> Color32 {
    palette().widget_bg
}
pub fn widget_bg_hover() -> Color32 {
    palette().widget_bg_hover
}
/// Value fill of drag sliders.
pub fn widget_fill() -> Color32 {
    palette().widget_fill
}
pub fn widget_fill_hover() -> Color32 {
    palette().widget_fill_hover
}

pub fn text() -> Color32 {
    palette().text
}
pub fn text_strong() -> Color32 {
    palette().text_strong
}
pub fn text_muted() -> Color32 {
    palette().text_muted
}
pub fn text_faint() -> Color32 {
    palette().text_faint
}

/// Interface accent (focus, selection, active tab).
pub fn accent() -> Color32 {
    palette().accent
}
pub fn accent_dim() -> Color32 {
    palette().accent_dim
}

pub fn error() -> Color32 {
    palette().error
}
pub fn warning() -> Color32 {
    palette().warning
}
pub fn ok() -> Color32 {
    palette().ok
}
pub fn record() -> Color32 {
    palette().record
}
pub fn play() -> Color32 {
    palette().play
}

/// Box selection.
pub fn select_fill() -> Color32 {
    palette().select_fill
}
pub fn select_stroke() -> Color32 {
    palette().select_stroke
}
/// Knife line for cutting wires.
pub fn cut_line() -> Color32 {
    palette().cut_line
}
/// Line drawn while Shift+right-dragging to add reroutes.
pub fn reroute_line() -> Color32 {
    palette().reroute_line
}

/// Frames: translucent fill, label color.
pub fn frame_fill() -> Color32 {
    palette().frame_fill
}
pub fn frame_border() -> Color32 {
    palette().frame_border
}
pub fn frame_label() -> Color32 {
    palette().frame_label
}

/// Comment sticky notes.
pub fn note_bg() -> Color32 {
    palette().note_bg
}
pub fn note_header() -> Color32 {
    palette().note_header
}
pub fn note_text() -> Color32 {
    palette().note_text
}

/// Meters: green → yellow → red as the level approaches full scale.
pub fn meter_low() -> Color32 {
    palette().meter_low
}
pub fn meter_mid() -> Color32 {
    palette().meter_mid
}
pub fn meter_high() -> Color32 {
    palette().meter_high
}
pub fn meter_bg() -> Color32 {
    palette().meter_bg
}

/// Scope trace and its background.
pub fn scope_bg() -> Color32 {
    palette().scope_bg
}
pub fn scope_grid() -> Color32 {
    palette().scope_grid
}

/// Windows and popups.
pub fn window_bg() -> Color32 {
    palette().window_bg
}
/// Inline code in docs.
pub fn code_bg() -> Color32 {
    palette().code_bg
}
pub fn code_text() -> Color32 {
    palette().code_text
}
/// A hovered row in a list (library, palette).
pub fn row_hover() -> Color32 {
    palette().row_hover
}
/// Raised surfaces in widget bodies (button tops, knob caps).
pub fn raised() -> Color32 {
    palette().raised
}
pub fn raised_hover() -> Color32 {
    palette().raised_hover
}
/// The lip under a raised button.
pub fn raised_lip() -> Color32 {
    palette().raised_lip
}
/// The shade under knob caps and dots, and key edges.
pub fn body_shadow() -> Color32 {
    palette().body_shadow
}
/// A switched-off toggle's track, and hovered.
pub fn toggle_off() -> Color32 {
    palette().toggle_off
}
pub fn toggle_hover() -> Color32 {
    palette().toggle_hover
}
/// The ring around socket dots.
pub fn socket_edge() -> Color32 {
    palette().socket_edge
}
/// Marks drawn on an accent fill (a checked box's tick).
pub fn on_accent() -> Color32 {
    palette().on_accent
}
/// The casing under wires.
pub fn wire_casing() -> Color32 {
    palette().wire_casing
}
/// What event flashes and flow dashes on wires brighten towards.
pub fn wire_flash() -> Color32 {
    palette().wire_flash
}
/// The group breadcrumb pill over the canvas.
pub fn breadcrumb_bg() -> Color32 {
    palette().breadcrumb_bg
}
/// The gesture hint pill over the canvas.
pub fn hint_bg() -> Color32 {
    palette().hint_bg
}

/// Piano keys look like a piano's in either theme: white and black keys,
/// hovered, and the octave names on the C keys.
pub const KEY_WHITE: Color32 = Color32::from_rgb(0xE6, 0xE8, 0xEC);
pub const KEY_WHITE_HOVER: Color32 = Color32::from_rgb(0xC9, 0xCD, 0xD4);
pub const KEY_BLACK: Color32 = Color32::from_rgb(0x1E, 0x20, 0x25);
pub const KEY_BLACK_HOVER: Color32 = Color32::from_rgb(0x44, 0x48, 0x52);
pub const KEY_LABEL: Color32 = Color32::from_rgb(0x6A, 0x70, 0x7A);

/// Signal type colors (UX.md socket table, deepened in the light theme).
/// `Any` (reroutes before they connect) is a neutral grey.
pub fn type_color(ty: SignalType) -> Color32 {
    let i = TYPE_ORDER.iter().position(|t| *t == ty).unwrap_or(8);
    palette().types[i]
}

/// Socket shape: stream types are circles, events diamonds, and data
/// handles (Spectrum, Buffer) squares (the colorblind-safe channel).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketShape {
    Circle,
    Diamond,
    Square,
}

pub fn socket_shape(ty: SignalType) -> SocketShape {
    if ty.is_event() {
        SocketShape::Diamond
    } else if ty.is_data() {
        SocketShape::Square
    } else {
        SocketShape::Circle
    }
}

/// Header color per library category (UX.md).
pub fn category_color(cat: Category) -> Color32 {
    let i = Category::ALL.iter().position(|c| *c == cat).unwrap_or(0);
    palette().categories[i]
}

/// Header color for nodes whose kind is unknown (missing node).
pub fn missing_header() -> Color32 {
    palette().missing_header
}
/// Header color for group instances and the group interface nodes.
pub fn group_header() -> Color32 {
    palette().group_header
}

/// The header fill: the category color mixed into the card so the title
/// stays readable on every hue (light text in the dark theme, dark text in
/// the light one).
pub fn header_fill(accent: Color32) -> Color32 {
    let p = palette();
    mix(p.header_base, accent, p.header_mix)
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

/// Per-node timing (`Ctrl+T`): cool for cheap nodes, through the warning
/// color, to the error color for a large share of the block's budget. `k`
/// runs from 0 to 1 (see `timing::heat`).
pub fn heat(k: f32) -> Color32 {
    let k = k.clamp(0.0, 1.0);
    let cool = mix(text_faint(), type_color(SignalType::Float), 0.45);
    if k < 0.5 {
        mix(cool, warning(), k * 2.0)
    } else {
        mix(warning(), error(), k * 2.0 - 1.0)
    }
}

/// Meter color for a level in dBFS.
pub fn meter_color(db: f32) -> Color32 {
    if db > -3.0 {
        meter_high()
    } else if db > -12.0 {
        meter_mid()
    } else {
        meter_low()
    }
}

// ---------------------------------------------------------------- geometry

/// Node cards (canvas units; multiplied by zoom when drawn).
pub const NODE_WIDTH: f32 = 196.0;
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
        color: palette().node_shadow,
    }
}

pub fn popup_shadow() -> Shadow {
    popup_shadow_of(palette())
}

fn popup_shadow_of(p: &Palette) -> Shadow {
    Shadow {
        offset: [0, 6],
        blur: 22,
        spread: 0,
        color: p.popup_shadow,
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

/// Apply the look to a context: the fonts and type scale, and egui's
/// visuals for both themes (which one shows is [`set_choice`]'s business).
/// egui's own Ctrl+plus/minus zoom is off: View ▸ UI Scale owns the zoom
/// factor, in steps, and saves it.
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.style_mut_of(egui::Theme::Dark, |s| style(s, &DARK));
    ctx.style_mut_of(egui::Theme::Light, |s| style(s, &LIGHT));
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
}

fn style(style: &mut egui::Style, p: &Palette) {
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
    v.override_text_color = None;
    v.dark_mode = p.dark;
    v.panel_fill = p.panel_bg;
    v.window_fill = p.window_bg;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = radius(10.0);
    v.menu_corner_radius = radius(8.0);
    v.window_shadow = popup_shadow_of(p);
    v.popup_shadow = popup_shadow_of(p);
    v.extreme_bg_color = p.well_bg;
    v.faint_bg_color = p.faint_bg;
    v.code_bg_color = p.code_bg;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.error;
    v.selection.bg_fill = p.accent_dim;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.slider_trailing_fill = true;
    v.striped = false;
    v.indent_has_left_vline = false;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.panel_bg;
    w.noninteractive.weak_bg_fill = p.panel_bg;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = radius(WIDGET_RADIUS + 1.0);

    w.inactive.bg_fill = p.button_bg;
    w.inactive.weak_bg_fill = p.button_weak_bg;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = radius(WIDGET_RADIUS + 1.0);

    w.hovered.bg_fill = p.button_hover_bg;
    w.hovered.weak_bg_fill = p.button_hover_weak_bg;
    w.hovered.bg_stroke = Stroke::new(1.0, p.button_hover_stroke);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text_strong);
    w.hovered.corner_radius = radius(WIDGET_RADIUS + 1.0);
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.button_active_bg;
    w.active.weak_bg_fill = p.button_active_bg;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.5, p.text_strong);
    w.active.corner_radius = radius(WIDGET_RADIUS + 1.0);
    w.active.expansion = 0.0;

    w.open.bg_fill = p.button_open_bg;
    w.open.weak_bg_fill = p.button_open_bg;
    w.open.bg_stroke = Stroke::new(1.0, p.border);
    w.open.fg_stroke = Stroke::new(1.0, p.text_strong);
    w.open.corner_radius = radius(WIDGET_RADIUS + 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws with `p` on this thread until dropped, then with the dark
    /// palette again (tests can share a thread).
    struct Using;
    impl Using {
        fn palette(p: &'static Palette) -> Using {
            set_palette(p);
            Using
        }
    }
    impl Drop for Using {
        fn drop(&mut self) {
            set_palette(&DARK);
        }
    }

    const BOTH: [(&str, &Palette); 2] = [("dark", &DARK), ("light", &LIGHT)];

    /// WCAG 2 relative luminance of an opaque color.
    fn luminance(c: Color32) -> f64 {
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }

    /// WCAG 2 contrast ratio (1 to 21).
    fn contrast(a: Color32, b: Color32) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    /// A translucent color as it shows over an opaque one.
    fn over(top: Color32, under: Color32) -> Color32 {
        let a = f32::from(top.a()) / 255.0;
        let ch = |t: u8, u: u8| (f32::from(t) + f32::from(u) * (1.0 - a)).round() as u8;
        Color32::from_rgb(
            ch(top.r(), under.r()),
            ch(top.g(), under.g()),
            ch(top.b(), under.b()),
        )
    }

    /// The surfaces body text sits on: panels, wells, windows, panel
    /// widgets, the canvas, and node cards (over the canvas).
    fn surfaces(p: &Palette) -> [(&'static str, Color32); 7] {
        [
            ("panel", p.panel_bg),
            ("well", p.well_bg),
            ("window", p.window_bg),
            ("button", p.button_bg),
            ("widget", p.widget_bg),
            ("canvas", p.canvas_bg),
            ("card", over(p.node_bg, p.canvas_bg)),
        ]
    }

    fn assert_contrast(theme: &str, what: &str, fg: Color32, bg: Color32, min: f64) {
        let r = contrast(fg, bg);
        assert!(
            r >= min,
            "{theme}: {what} has contrast {r:.2}, needs {min} ({fg:?} on {bg:?})"
        );
    }

    #[test]
    fn contrast_math_matches_wcag() {
        let black = Color32::BLACK;
        let white = Color32::WHITE;
        assert!((contrast(black, white) - 21.0).abs() < 1e-9);
        assert!((contrast(white, white) - 1.0).abs() < 1e-9);
        // #767676 on white is the classic 4.54:1.
        let grey = Color32::from_rgb(0x76, 0x76, 0x76);
        assert!((contrast(grey, white) - 4.54).abs() < 0.01);
    }

    /// WCAG AA: body text at least 4.5:1 on every panel and card, in both
    /// themes. Faint hint text in the light theme at least 3:1.
    #[test]
    fn body_text_meets_wcag_aa() {
        for (theme, p) in BOTH {
            for (bg_name, bg) in surfaces(p) {
                for (fg_name, fg) in [
                    ("text", p.text),
                    ("strong text", p.text_strong),
                    ("muted text", p.text_muted),
                ] {
                    assert_contrast(theme, &format!("{fg_name} on {bg_name}"), fg, bg, 4.5);
                }
            }
            // Status messages, links, and labels in color.
            for (name, fg) in [
                ("accent", p.accent),
                ("error", p.error),
                ("warning", p.warning),
                ("ok", p.ok),
            ] {
                assert_contrast(theme, &format!("{name} on panel"), fg, p.panel_bg, 4.5);
            }
            assert_contrast(theme, "note text", p.note_text, p.note_bg, 4.5);
            assert_contrast(theme, "frame label", p.frame_label, p.canvas_bg, 4.5);
            assert_contrast(theme, "inline code", p.code_text, p.code_bg, 4.5);
            assert_contrast(
                theme,
                "hint pill",
                p.text,
                over(p.hint_bg, p.canvas_bg),
                4.5,
            );
            // Node titles on every category's header.
            for (i, c) in p.categories.iter().enumerate() {
                let _on = Using::palette(p);
                let header = over(header_fill(*c), p.canvas_bg);
                assert_contrast(
                    theme,
                    &format!("title on the {} header", Category::ALL[i].name()),
                    p.text_strong,
                    header,
                    4.5,
                );
            }
        }
        for (bg_name, bg) in surfaces(&LIGHT) {
            assert_contrast(
                "light",
                &format!("faint text on {bg_name}"),
                LIGHT.text_faint,
                bg,
                3.0,
            );
        }
    }

    /// Wires and sockets are graphics: WCAG asks 3:1 against what they sit
    /// on (the canvas and node cards).
    #[test]
    fn signal_colors_stand_out_from_the_canvas_and_cards() {
        for (theme, p) in BOTH {
            for (i, c) in p.types.iter().enumerate() {
                let ty = TYPE_ORDER[i];
                assert_contrast(theme, &format!("{ty} on canvas"), *c, p.canvas_bg, 3.0);
                let card = over(p.node_bg, p.canvas_bg);
                assert_contrast(theme, &format!("{ty} on a card"), *c, card, 3.0);
            }
        }
    }

    #[test]
    fn every_type_has_a_distinct_color() {
        use SignalType::*;
        let types = [
            Audio, Float, Int, Bool, Trigger, Midi, Spectrum, Buffer, Any,
        ];
        for (theme, p) in BOTH {
            let _on = Using::palette(p);
            for (i, a) in types.iter().enumerate() {
                for b in &types[i + 1..] {
                    assert_ne!(type_color(*a), type_color(*b), "{theme}: {a} vs {b}");
                }
            }
        }
        assert_eq!(socket_shape(Trigger), SocketShape::Diamond);
        assert_eq!(socket_shape(Audio), SocketShape::Circle);
        assert_eq!(socket_shape(Spectrum), SocketShape::Square);
        assert_eq!(socket_shape(Buffer), SocketShape::Square);
    }

    #[test]
    fn the_dark_types_are_the_socket_table() {
        assert_eq!(DARK.types[0], Color32::from_rgb(0xF0, 0xA0, 0x30));
        assert_eq!(DARK.types[4], Color32::from_rgb(0xF2, 0xD1, 0x5C));
        assert_eq!(TYPE_ORDER[4], SignalType::Trigger);
    }

    #[test]
    fn switching_palettes_changes_what_accessors_return() {
        assert_eq!(canvas_bg(), DARK.canvas_bg, "dark by default");
        {
            let _on = Using::palette(&LIGHT);
            assert_eq!(canvas_bg(), LIGHT.canvas_bg);
            assert_eq!(text(), LIGHT.text);
            assert_eq!(type_color(SignalType::Audio), LIGHT.types[0]);
            assert_eq!(
                category_color(Category::Filters),
                LIGHT.categories[3],
                "Category::ALL order"
            );
            assert_eq!(
                header_fill(LIGHT.categories[0]),
                mix(LIGHT.header_base, LIGHT.categories[0], LIGHT.header_mix)
            );
            assert_eq!(node_shadow().color, LIGHT.node_shadow);
            // Another thread still draws with its own palette.
            let other = std::thread::spawn(canvas_bg).join().unwrap();
            assert_eq!(other, DARK.canvas_bg, "per thread");
        }
        assert_eq!(canvas_bg(), DARK.canvas_bg);
    }

    #[test]
    fn choosing_a_theme_switches_egui_and_the_palette() {
        let ctx = egui::Context::default();
        apply(&ctx);
        set_choice(&ctx, ThemeChoice::Light);
        let v = ctx.global_style().visuals.clone();
        assert!(!v.dark_mode);
        assert_eq!(v.panel_fill, LIGHT.panel_bg);
        assert_eq!(v.window_fill, LIGHT.window_bg);
        assert_eq!(canvas_bg(), LIGHT.canvas_bg);
        set_choice(&ctx, ThemeChoice::Dark);
        let v = ctx.global_style().visuals.clone();
        assert!(v.dark_mode);
        assert_eq!(v.panel_fill, DARK.panel_bg);
        assert_eq!(canvas_bg(), DARK.canvas_bg);
        // Following the system with no system mode known shows dark.
        set_choice(&ctx, ThemeChoice::System);
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        assert_eq!(canvas_bg(), DARK.canvas_bg);
        assert!(!ctx.options(|o| o.zoom_with_keyboard), "UI Scale owns zoom");
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
