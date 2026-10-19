//! The person's own themes: JSON files in the `themes` folder beside the
//! settings file (docs/guide/themes.md). Each names itself, picks a base
//! (Dark, Light, or High Contrast), and overrides any of the base's colors
//! by their `Palette` field names. A file that can't be used, a color that
//! doesn't parse, or a name the editor doesn't know is reported in View ▸
//! Theme and skipped; nothing in a theme file can stop the editor.

use std::path::{Path, PathBuf};

use egui::Color32;

use crate::theme::{self, HIGH_CONTRAST, Palette, ThemeChoice};

/// Below this, body text is hard to read (WCAG AA); View ▸ Theme warns.
pub const MIN_TEXT_CONTRAST: f64 = 4.5;

/// Theme files larger than this aren't themes (a few kilobytes at most).
const MAX_FILE_BYTES: u64 = 256 * 1024;

/// A theme file's base: the built-in theme its colors start from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    Dark,
    Light,
    HighContrast,
}

impl Base {
    pub fn palette(self) -> &'static Palette {
        match self {
            Base::Dark => &theme::DARK,
            Base::Light => &theme::LIGHT,
            Base::HighContrast => &HIGH_CONTRAST,
        }
    }

    fn parse(s: &str) -> Option<Base> {
        match s.trim().to_lowercase().replace([' ', '-'], "_").as_str() {
            "dark" => Some(Base::Dark),
            "light" => Some(Base::Light),
            "high_contrast" => Some(Base::HighContrast),
            _ => None,
        }
    }
}

/// One usable theme file.
#[derive(Clone, Debug)]
pub struct UserTheme {
    pub name: String,
    /// The file it came from (its name, for reports).
    pub file: String,
    pub base: Base,
    pub palette: &'static Palette,
    /// The lowest contrast of body text on its surfaces, and where.
    pub text_contrast: (f64, String),
    /// What in the file was ignored, and why.
    pub notes: Vec<String>,
}

impl UserTheme {
    /// Body text under [`MIN_TEXT_CONTRAST`] somewhere.
    pub fn low_contrast(&self) -> bool {
        self.text_contrast.0 < MIN_TEXT_CONTRAST
    }
}

/// The themes folder's usable themes, in file-name order, and the files
/// that couldn't be used.
#[derive(Clone, Debug, Default)]
pub struct UserThemes {
    pub themes: Vec<UserTheme>,
    /// `file: why`, for files with no usable theme in them.
    pub problems: Vec<String>,
    /// The folder they were read from (none in memory, as in tests).
    pub dir: Option<PathBuf>,
}

impl UserThemes {
    /// Read every `*.json` in `dir`. A missing folder has no themes.
    pub fn load(dir: Option<PathBuf>) -> UserThemes {
        let mut themes = UserThemes {
            dir: dir.clone(),
            ..UserThemes::default()
        };
        let Some(dir) = dir else { return themes };
        for (file, text) in read_files(&dir) {
            match text {
                Ok(text) => themes.add(&file, &text),
                Err(e) => themes.problems.push(format!("{file}: {e}")),
            }
        }
        themes
    }

    /// Themes from `(file name, text)` pairs (tests, and the loader).
    pub fn from_texts(files: &[(&str, &str)]) -> UserThemes {
        let mut themes = UserThemes::default();
        for (file, text) in files {
            themes.add(file, text);
        }
        themes
    }

    fn add(&mut self, file: &str, text: &str) {
        match parse(file, text) {
            Ok(t) if self.get(&t.name).is_some() => self.problems.push(format!(
                "{file}: another theme file is already named “{}”",
                t.name
            )),
            Ok(t) => self.themes.push(t),
            Err(e) => self.problems.push(format!("{file}: {e}")),
        }
    }

    /// The theme called `name`.
    pub fn get(&self, name: &str) -> Option<&UserTheme> {
        self.themes.iter().find(|t| t.name == name)
    }

    /// What a choice shows: built-ins as they are, a theme file's palette
    /// in its slot, and Dark (with why) for a name no file has.
    pub fn look(&self, choice: &ThemeChoice) -> (theme::Look, Option<String>) {
        match choice {
            ThemeChoice::User(name) => match self.get(name) {
                Some(t) => (theme::Look::of(t.palette), None),
                None => (
                    theme::Look::built_in(&ThemeChoice::Dark),
                    Some(name.clone()),
                ),
            },
            builtin => (theme::Look::built_in(builtin), None),
        }
    }
}

/// The theme in a file's text, or why there's none. Colors that don't
/// parse and names that aren't colors are noted and skipped.
pub fn parse(file: &str, text: &str) -> Result<UserTheme, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("isn't a theme file (not JSON: {e})"))?;
    let serde_json::Value::Object(map) = value else {
        return Err("isn't a theme file (it should be one JSON object)".into());
    };
    let name = match map.get("name") {
        Some(serde_json::Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
        _ => return Err("has no \"name\"".into()),
    };
    if ThemeChoice::is_reserved(&name) {
        return Err(format!(
            "is named “{name}”, like a built-in theme: give it a name of its own"
        ));
    }
    let mut notes = Vec::new();
    let base = match map.get("base") {
        None => {
            notes.push("no \"base\": built on Dark".to_string());
            Base::Dark
        }
        Some(v) => match v.as_str().and_then(Base::parse) {
            Some(b) => b,
            None => {
                notes.push(format!(
                    "\"base\" {v} isn't dark, light, or high_contrast: built on Dark"
                ));
                Base::Dark
            }
        },
    };
    let mut palette = *base.palette();
    for (key, v) in &map {
        match key.as_str() {
            "name" | "base" => {}
            "header_mix" => match v.as_f64() {
                Some(m) if (0.0..=1.0).contains(&m) => palette.header_mix = m as f32,
                _ => notes.push(format!(
                    "\"header_mix\" {v} isn't a number from 0 to 1 (kept the base's)"
                )),
            },
            "colors" => match v {
                serde_json::Value::Object(colors) => {
                    for (field, value) in colors {
                        override_color(&mut palette, field, value, &mut notes);
                    }
                }
                _ => notes.push("\"colors\" should be an object of colors (ignored)".into()),
            },
            other => notes.push(format!("unknown field \"{other}\" (ignored)")),
        }
    }
    Ok(UserTheme {
        name,
        file: file.to_string(),
        base,
        text_contrast: theme::text_contrast(&palette),
        palette: theme::intern(palette),
        notes,
    })
}

fn override_color(
    palette: &mut Palette,
    field: &str,
    value: &serde_json::Value,
    notes: &mut Vec<String>,
) {
    let Some(slot) = palette.color_mut(field) else {
        let hint = closest_field(field)
            .map(|f| format!(", did you mean \"{f}\"?"))
            .unwrap_or_default();
        notes.push(format!("unknown color \"{field}\" (ignored{hint})"));
        return;
    };
    match value.as_str().and_then(parse_color) {
        Some(c) => *slot = c,
        None => notes.push(format!(
            "\"{field}\": {value} isn't a color like \"#RRGGBB\" or \"#RRGGBBAA\" (kept the base's)"
        )),
    }
}

/// `#RRGGBB` or `#RRGGBBAA` (straight alpha, as in CSS).
pub fn parse_color(s: &str) -> Option<Color32> {
    let hex = s.trim().strip_prefix('#')?;
    if !(hex.len() == 6 || hex.len() == 8) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    let a = if hex.len() == 8 { byte(6)? } else { 255 };
    Some(Color32::from_rgba_unmultiplied(
        byte(0)?,
        byte(2)?,
        byte(4)?,
        a,
    ))
}

/// A color as a theme file writes it.
pub fn format_color(c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 {
        format!("#{r:02X}{g:02X}{b:02X}")
    } else {
        format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
    }
}

/// The field name nearest a misspelled one (a typo or two away).
fn closest_field(field: &str) -> Option<&'static str> {
    Palette::COLOR_FIELDS
        .iter()
        .map(|f| (edit_distance(field, f), *f))
        .filter(|(d, _)| *d <= 2)
        .min()
        .map(|(_, f)| f)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = if ca == *cb {
                prev
            } else {
                1 + prev.min(cur).min(row[j])
            };
            prev = cur;
        }
    }
    row[b.len()]
}

/// The `*.json` files in `dir`, by name, with their text or why it
/// couldn't be read. The browser build has no folder to read.
#[cfg(not(target_arch = "wasm32"))]
fn read_files(dir: &Path) -> Vec<(String, Result<String, String>)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        })
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let name = p
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().to_string());
            let text = match std::fs::metadata(&p) {
                Ok(m) if m.len() > MAX_FILE_BYTES => {
                    Err(format!("is {} KB, too big to be a theme", m.len() / 1024))
                }
                _ => std::fs::read_to_string(&p).map_err(|e| format!("couldn't be read ({e})")),
            };
            (name, text)
        })
        .collect()
}

#[cfg(target_arch = "wasm32")]
fn read_files(_dir: &Path) -> Vec<(String, Result<String, String>)> {
    let _ = MAX_FILE_BYTES;
    Vec::new()
}

/// A theme file holding every color of `palette`: a complete starting
/// point to edit (`base` names the built-in it was made from).
pub fn to_json(name: &str, base: Base, palette: &Palette) -> String {
    let mut colors = serde_json::Map::new();
    for field in Palette::COLOR_FIELDS {
        if let Some(c) = palette.color(field) {
            colors.insert((*field).to_string(), format_color(c).into());
        }
    }
    let base = match base {
        Base::Dark => "dark",
        Base::Light => "light",
        Base::HighContrast => "high_contrast",
    };
    let doc = serde_json::json!({
        "name": name,
        "base": base,
        "header_mix": f64::from(palette.header_mix),
        "colors": colors,
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_parse_with_and_without_alpha() {
        assert_eq!(
            parse_color("#FFB000"),
            Some(Color32::from_rgb(0xFF, 0xB0, 0x00))
        );
        assert_eq!(
            parse_color(" #ffb000 "),
            Some(Color32::from_rgb(0xFF, 0xB0, 0x00))
        );
        assert_eq!(
            parse_color("#10203040"),
            Some(Color32::from_rgba_unmultiplied(0x10, 0x20, 0x30, 0x40))
        );
        for bad in ["FFB000", "#FFB00", "#GGGGGG", "#FFB0000", "", "#", "#ééé"] {
            assert_eq!(parse_color(bad), None, "{bad}");
        }
        let c = Color32::from_rgba_unmultiplied(0x12, 0x34, 0x56, 0x78);
        let back = parse_color(&format_color(c)).unwrap();
        assert_eq!(back, c);
        assert_eq!(format_color(Color32::from_rgb(1, 2, 255)), "#0102FF");
    }

    #[test]
    fn a_partial_theme_overrides_only_what_it_names() {
        let t = parse(
            "night.json",
            r##"{"name": "Night Shift", "base": "dark",
                 "colors": {"accent": "#FFB000", "type_audio": "#FF8800",
                            "category_filters": "#00FF00"}}"##,
        )
        .unwrap();
        assert_eq!(t.name, "Night Shift");
        assert_eq!(t.base, Base::Dark);
        assert!(t.notes.is_empty(), "{:?}", t.notes);
        let p = t.palette;
        assert_eq!(p.accent, Color32::from_rgb(0xFF, 0xB0, 0x00));
        assert_eq!(p.types[0], Color32::from_rgb(0xFF, 0x88, 0x00));
        assert_eq!(p.categories[3], Color32::from_rgb(0, 0xFF, 0));
        assert_eq!(p.canvas_bg, theme::DARK.canvas_bg, "the rest is the base's");
        assert!(p.dark);
        assert!(!t.low_contrast());

        let light = parse("l.json", r#"{"name": "Paper", "base": "light"}"#).unwrap();
        assert!(!light.palette.dark);
        assert_eq!(*light.palette, theme::LIGHT);
        let hc = parse("h.json", r#"{"name": "Loud", "base": "High Contrast"}"#).unwrap();
        assert_eq!(hc.base, Base::HighContrast);
        assert!(hc.palette.strong_outlines);
    }

    #[test]
    fn bad_colors_and_unknown_names_are_noted_and_skipped() {
        let t = parse(
            "messy.json",
            r##"{"name": "Messy", "base": "sepia", "flavor": 3, "header_mix": 4,
                 "colors": {"acent": "#FFFFFF", "text": "white", "panel_bg": 12,
                            "type_noise": "#000000", "border": "#123456"}}"##,
        )
        .unwrap();
        assert_eq!(t.base, Base::Dark);
        assert_eq!(t.palette.border, Color32::from_rgb(0x12, 0x34, 0x56));
        assert_eq!(t.palette.text, theme::DARK.text);
        assert_eq!(t.palette.panel_bg, theme::DARK.panel_bg);
        assert_eq!(t.palette.header_mix, theme::DARK.header_mix);
        let notes = t.notes.join("\n");
        for expected in [
            "\"base\" \"sepia\" isn't dark, light, or high_contrast",
            "unknown field \"flavor\"",
            "\"header_mix\" 4 isn't a number from 0 to 1",
            "unknown color \"acent\" (ignored, did you mean \"accent\"?)",
            "\"text\": \"white\" isn't a color",
            "\"panel_bg\": 12 isn't a color",
            "unknown color \"type_noise\"",
        ] {
            assert!(notes.contains(expected), "{expected} in:\n{notes}");
        }
    }

    #[test]
    fn files_that_arent_themes_are_problems_not_panics() {
        let themes = UserThemes::from_texts(&[
            ("a.json", "{ not json"),
            ("b.json", "[1, 2]"),
            ("c.json", r#"{"base": "dark"}"#),
            ("d.json", r#"{"name": "  "}"#),
            ("e.json", r#"{"name": "Dark"}"#),
            ("f.json", r#"{"name": "high contrast"}"#),
            ("g.json", r#"{"name": "Mine"}"#),
            ("h.json", r#"{"name": "Mine", "base": "light"}"#),
            ("i.json", ""),
        ]);
        assert_eq!(themes.themes.len(), 1);
        assert_eq!(themes.themes[0].file, "g.json");
        let problems = themes.problems.join("\n");
        assert_eq!(themes.problems.len(), 8, "{problems}");
        for expected in [
            "a.json: isn't a theme file (not JSON",
            "b.json: isn't a theme file (it should be one JSON object)",
            "c.json: has no \"name\"",
            "d.json: has no \"name\"",
            "e.json: is named “Dark”, like a built-in theme",
            "f.json: is named “high contrast”",
            "h.json: another theme file is already named “Mine”",
            "i.json: isn't a theme file",
        ] {
            assert!(problems.contains(expected), "{expected} in:\n{problems}");
        }
    }

    #[test]
    fn low_text_contrast_is_flagged() {
        let t = parse(
            "dim.json",
            r##"{"name": "Dim", "base": "dark", "colors": {"text_muted": "#3A3D44"}}"##,
        )
        .unwrap();
        assert!(t.low_contrast(), "{:?}", t.text_contrast);
        assert!(
            t.text_contrast.1.starts_with("text_muted on "),
            "{:?}",
            t.text_contrast
        );
    }

    #[test]
    fn a_missing_theme_shows_dark() {
        let themes = UserThemes::from_texts(&[("m.json", r#"{"name": "Mine", "base": "light"}"#)]);
        let (look, missing) = themes.look(&ThemeChoice::User("Mine".into()));
        assert!(missing.is_none());
        assert_eq!(look.preference, egui::ThemePreference::Light);
        assert_eq!(*look.light, theme::LIGHT);
        let (look, missing) = themes.look(&ThemeChoice::User("Gone".into()));
        assert_eq!(missing.as_deref(), Some("Gone"));
        assert_eq!(look, theme::Look::built_in(&ThemeChoice::Dark));
        let (look, _) = themes.look(&ThemeChoice::HighContrast);
        assert_eq!(*look.dark, HIGH_CONTRAST);
    }

    #[test]
    fn a_saved_palette_reads_back_the_same() {
        for (base, p) in [
            (Base::Dark, &theme::DARK),
            (Base::Light, &theme::LIGHT),
            (Base::HighContrast, &HIGH_CONTRAST),
        ] {
            let text = to_json("Copy", base, p);
            let t = parse("copy.json", &text).unwrap();
            assert!(t.notes.is_empty(), "{:?}", t.notes);
            assert_eq!(*t.palette, *p, "{base:?}");
        }
    }

    /// docs/guide/themes.md lists every color a theme file can change,
    /// with the built-in themes' values as they are.
    #[test]
    fn the_guide_lists_every_color_with_its_values() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/guide/themes.md");
        let doc = std::fs::read_to_string(path).expect("docs/guide/themes.md");
        for field in Palette::COLOR_FIELDS {
            let row = doc
                .lines()
                .find(|l| l.starts_with(&format!("| `{field}` |")))
                .unwrap_or_else(|| panic!("docs/guide/themes.md has no row for `{field}`"));
            for (theme, p) in [
                ("Dark", &theme::DARK),
                ("Light", &theme::LIGHT),
                ("High Contrast", &HIGH_CONTRAST),
            ] {
                let value = p.color(field).map(format_color).unwrap_or_default();
                assert!(
                    row.contains(&format!("`{value}`")),
                    "the guide's {theme} value for `{field}` should be {value}: {row}"
                );
            }
        }
        for field in ["name", "base", "colors", "header_mix"] {
            assert!(doc.contains(&format!("| `{field}` |")), "{field}");
        }
    }

    #[test]
    fn the_folder_is_read_in_name_order_and_skips_other_files() {
        let dir = std::env::temp_dir().join(format!(
            "ac-themes-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.json"), r#"{"name": "Bravo"}"#).unwrap();
        std::fs::write(dir.join("a.JSON"), r#"{"name": "Alpha", "base": "light"}"#).unwrap();
        std::fs::write(dir.join("notes.txt"), "not a theme").unwrap();
        std::fs::write(dir.join("c.json"), "{").unwrap();
        let themes = UserThemes::load(Some(dir.clone()));
        let names: Vec<&str> = themes.themes.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "Bravo"]);
        assert_eq!(themes.problems.len(), 1);
        assert!(themes.problems[0].starts_with("c.json: "));
        assert!(UserThemes::load(Some(dir.join("nope"))).themes.is_empty());
        std::fs::remove_dir_all(dir).ok();
    }
}
