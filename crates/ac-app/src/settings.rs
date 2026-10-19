//! The editor's own settings (theme, UI scale), kept in `settings.json` in
//! the platform config folder next to the recent list, with the person's
//! own themes in a `themes` folder beside it. They belong to the person,
//! not to a project, so they never travel with project files.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::i18n::LanguageChoice;
use crate::theme::ThemeChoice;

/// UI scale steps (View ▸ UI Scale; UX.md "Accessibility and scaling").
pub const UI_SCALES: [f32; 8] = [0.75, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];

/// What is saved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub theme: ThemeChoice,
    /// egui's zoom factor: 1 is 100 %.
    pub ui_scale: f32,
    /// View ▸ Language.
    pub language: LanguageChoice,
    /// View ▸ Reduce Motion: no fades, flowing wires, or eased moves.
    pub reduce_motion: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            theme: ThemeChoice::Dark,
            ui_scale: 1.0,
            language: LanguageChoice::System,
            reduce_motion: false,
        }
    }
}

impl Prefs {
    /// Values a hand-edited or older file may hold, made usable: the scale
    /// within the steps' range (100 % if it isn't a number).
    fn sanitized(mut self) -> Prefs {
        self.ui_scale = clamp_scale(self.ui_scale);
        self
    }
}

/// A scale within 75 %..200 % (100 % for anything that isn't a number).
pub fn clamp_scale(s: f32) -> f32 {
    if s.is_finite() {
        s.clamp(UI_SCALES[0], UI_SCALES[UI_SCALES.len() - 1])
    } else {
        1.0
    }
}

/// The next step up from `current` (or the largest).
pub fn scale_up(current: f32) -> f32 {
    UI_SCALES
        .iter()
        .copied()
        .find(|s| *s > current + 1e-3)
        .unwrap_or(UI_SCALES[UI_SCALES.len() - 1])
}

/// The next step down from `current` (or the smallest).
pub fn scale_down(current: f32) -> f32 {
    UI_SCALES
        .iter()
        .rev()
        .copied()
        .find(|s| *s < current - 1e-3)
        .unwrap_or(UI_SCALES[0])
}

/// The settings and where they're stored.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppSettings {
    pub prefs: Prefs,
    /// `None` keeps them in memory (tests).
    file: Option<PathBuf>,
}

impl AppSettings {
    pub fn in_memory() -> AppSettings {
        AppSettings::default()
    }

    /// Load from the platform config directory.
    pub fn load_default() -> AppSettings {
        let file = directories::ProjectDirs::from("", "", "Audio Constructor")
            .map(|d| d.config_dir().join("settings.json"));
        AppSettings::load_from(file)
    }

    /// Load from `file`; a missing or unreadable file gives the defaults,
    /// and unknown or bad fields fall back one by one.
    pub fn load_from(file: Option<PathBuf>) -> AppSettings {
        let prefs = file
            .as_deref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|t| parse(&t))
            .unwrap_or_default();
        AppSettings { prefs, file }
    }

    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// The folder of the person's own themes (`themes/` beside the
    /// settings file), if the settings have a file.
    pub fn themes_dir(&self) -> Option<PathBuf> {
        Some(self.file.as_deref()?.parent()?.join("themes"))
    }

    pub fn set_theme(&mut self, theme: ThemeChoice) {
        if self.prefs.theme != theme {
            self.prefs.theme = theme;
            self.save();
        }
    }

    pub fn set_language(&mut self, language: LanguageChoice) {
        if self.prefs.language != language {
            self.prefs.language = language;
            self.save();
        }
    }

    pub fn set_reduce_motion(&mut self, on: bool) {
        if self.prefs.reduce_motion != on {
            self.prefs.reduce_motion = on;
            self.save();
        }
    }

    pub fn set_ui_scale(&mut self, scale: f32) {
        let scale = clamp_scale(scale);
        if (self.prefs.ui_scale - scale).abs() > 1e-4 {
            self.prefs.ui_scale = scale;
            self.save();
        }
    }

    /// Write the file. Failing to save settings is not worth interrupting
    /// anyone for; the editor keeps them for this session.
    fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&self.prefs) {
            let _ = std::fs::write(file, text);
        }
    }
}

/// Settings from JSON, keeping every field that reads and defaulting the
/// rest (a theme name from a newer version shouldn't lose the scale).
fn parse(text: &str) -> Prefs {
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(text) else {
        return Prefs::default();
    };
    let mut prefs = Prefs::default();
    // Built-in themes by id, theme files by name; a name that matches no
    // file shows Dark until one does.
    if let Some(t) = map.get("theme").and_then(serde_json::Value::as_str) {
        prefs.theme = ThemeChoice::from_name(t);
    }
    if let Some(s) = map.get("ui_scale").and_then(serde_json::Value::as_f64) {
        prefs.ui_scale = s as f32;
    }
    if let Some(l) = map
        .get("language")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
    {
        prefs.language = l;
    }
    if let Some(r) = map
        .get("reduce_motion")
        .and_then(serde_json::Value::as_bool)
    {
        prefs.reduce_motion = r;
    }
    prefs.sanitized()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_go_up_and_down_and_stop_at_the_ends() {
        assert_eq!(scale_up(1.0), 1.1);
        assert_eq!(scale_up(1.1), 1.25);
        assert_eq!(scale_up(2.0), 2.0);
        assert_eq!(scale_down(1.0), 0.9);
        assert_eq!(scale_down(0.9), 0.75);
        assert_eq!(scale_down(0.75), 0.75);
        // Between steps, the next one in that direction.
        assert_eq!(scale_up(1.3), 1.5);
        assert_eq!(scale_down(1.3), 1.25);
        assert_eq!(clamp_scale(9.0), 2.0);
        assert_eq!(clamp_scale(0.1), 0.75);
        assert_eq!(clamp_scale(f32::NAN), 1.0);
    }

    #[test]
    fn settings_round_trip_through_their_file() {
        let dir = std::env::temp_dir().join(format!("ac-settings-{}", std::process::id()));
        let file = dir.join("nested").join("settings.json");
        let mut s = AppSettings::load_from(Some(file.clone()));
        assert_eq!(s.prefs, Prefs::default(), "no file yet");
        s.set_theme(ThemeChoice::Light);
        s.set_ui_scale(1.25);
        let again = AppSettings::load_from(Some(file.clone()));
        assert_eq!(again.prefs.theme, ThemeChoice::Light);
        assert_eq!(s.themes_dir(), Some(dir.join("nested").join("themes")));
        s.set_language(LanguageChoice::English);
        s.set_reduce_motion(true);
        let again = AppSettings::load_from(Some(file.clone()));
        assert_eq!(again.prefs.language, LanguageChoice::English);
        assert!(again.prefs.reduce_motion);
        s.set_theme(ThemeChoice::User("Night Shift".into()));
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains(r#""theme": "Night Shift""#), "{text}");
        assert_eq!(again.prefs.ui_scale, 1.25);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn bad_files_give_defaults_field_by_field() {
        assert_eq!(parse("not json"), Prefs::default());
        assert_eq!(parse("[1, 2]"), Prefs::default());
        let p = parse(r#"{"theme": 7, "ui_scale": 1.5}"#);
        assert_eq!(p.theme, ThemeChoice::Dark);
        assert_eq!(p.ui_scale, 1.5);
        // A name that isn't a built-in is a theme file's, kept by name.
        let p = parse(r#"{"theme": "Sepia"}"#);
        assert_eq!(p.theme, ThemeChoice::User("Sepia".into()));
        assert_eq!(
            parse(r#"{"theme": "high_contrast"}"#).theme,
            ThemeChoice::HighContrast
        );
        let p = parse(r#"{"theme": "system", "ui_scale": "big"}"#);
        assert_eq!(p.theme, ThemeChoice::System);
        assert_eq!(p.ui_scale, 1.0);
        assert_eq!(parse(r#"{"ui_scale": 40}"#).ui_scale, 2.0);
        let p = parse(r#"{"language": "pseudo", "reduce_motion": true}"#);
        assert_eq!(p.language, LanguageChoice::Pseudo);
        assert!(p.reduce_motion);
        let p = parse(r#"{"language": "klingon", "reduce_motion": "yes"}"#);
        assert_eq!(p.language, LanguageChoice::System);
        assert!(!p.reduce_motion);
    }
}
