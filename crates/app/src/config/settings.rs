//! `settings.json` in herdr-frontend's home: the person's look and terminal preferences, in
//! herdr-app's key names (`~/.config/herdr-app/settings.json`, `app:src/settings.rs` `Person`),
//! so a line copied from one works in the other.
//!
//! Every key is optional. A missing key, or a missing file, takes Eli's current value
//! (`docs/research/feature-inventory.md` §2), which is [`Settings::default`]. Keys herdr-frontend
//! doesn't read (`sounds`, `telemetry`, `font_ligatures`, …) are ignored: it has no sounds, no
//! telemetry and draws terminals without ligatures.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

/// The zoom keys' range, as herdr-app's (`ZOOM_MIN`, `ZOOM_MAX`).
pub const ZOOM_MIN: f32 = 0.5;
pub const ZOOM_MAX: f32 = 3.0;

/// The panels' text size before zoom: herdr-app's `ui_font_size`, which no file changes.
pub const UI_FONT_SIZE: f32 = 13.0;

/// Light or dark: Eli's is always dark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    System,
    Light,
    #[default]
    Dark,
}

/// What the window runs with, resolved: [`Settings::parse`] filled every key the file left out
/// and replaced values it can't use.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// How big the whole window is drawn, 1 as designed: terminals, panels and the layout's
    /// sizes. The zoom keys change it, in tenths, from 0.5 to 3.
    pub zoom: f32,
    /// The terminals' font family and size (points at zoom 1, 6 to 72).
    pub font: String,
    pub font_size: f32,
    /// The panels' font family; its size is [`UI_FONT_SIZE`] at zoom 1.
    pub ui_font: String,
    /// Row height and column width multipliers, clamped to 0.8..2: Ghostty's `adjust-cell-height`
    /// and `adjust-cell-width` as a factor.
    pub terminal_line_height: f32,
    pub terminal_cell_width: f32,
    /// Wheel and trackpad multiplier, clamped to 0.1..5 (read by terminal scrolling).
    pub terminal_scroll_speed: f32,
    /// Text below this contrast ratio with its background is drawn white or black, whichever
    /// contrasts more (Ghostty's `minimum-contrast`). 1 to 21; 1 leaves program colors alone.
    pub minimum_contrast: f32,
    /// A scroll position thumb at a terminal pane's right edge. Eli's is off, and herdr-frontend
    /// draws none either way until scrollback lands.
    pub pane_scrollbars: bool,
    pub appearance: Appearance,
    /// `arctic` (built in) or the name of a file in the home's `themes/`.
    pub theme: String,
    /// Theme color overrides by name (`{"accent": "#ff8800"}`), `#rrggbb` or `#rrggbbaa`.
    pub colors: BTreeMap<String, String>,
}

impl Default for Settings {
    /// Eli's `settings.json` (`docs/research/feature-inventory.md` §2).
    fn default() -> Settings {
        Settings {
            zoom: 1.1,
            font: "Operator Mono Book Herdr".into(),
            font_size: 17.0,
            ui_font: "Operator Mono Book Herdr".into(),
            terminal_line_height: 1.0,
            terminal_cell_width: 1.0,
            terminal_scroll_speed: 1.0,
            minimum_contrast: 1.0,
            pane_scrollbars: false,
            appearance: Appearance::Dark,
            theme: "arctic".into(),
            colors: BTreeMap::new(),
        }
    }
}

impl Settings {
    /// Ghostty's defaults and no zoom: JetBrains Mono at 14 pt, what the renderer's fixtures, the
    /// Ghostty pixel diff and the shell's geometry tests draw at.
    pub fn fixture() -> Settings {
        Settings {
            zoom: 1.0,
            font: "JetBrains Mono".into(),
            font_size: 14.0,
            ui_font: "JetBrains Mono".into(),
            ..Settings::default()
        }
    }

    /// The settings a file's text gives, and what's wrong with it. Text that isn't a settings
    /// object is an `Err`: the caller keeps what it had. A value out of range takes its default
    /// (or is clamped, where herdr-app clamps) and is reported.
    pub fn parse(text: &str) -> Result<(Settings, Vec<String>), String> {
        let mut settings: Settings =
            serde_json::from_str(text).map_err(|e| format!("settings.json: {e}"))?;
        let problems = settings.check();
        Ok((settings, problems))
    }

    /// Puts every value in range, returning what it changed.
    fn check(&mut self) -> Vec<String> {
        let d = Settings::default();
        let mut problems = Vec::new();
        let mut within = |name: &str, value: &mut f32, lo: f32, hi: f32, default: f32| {
            if !value.is_finite() || !(lo..=hi).contains(value) {
                problems.push(format!(
                    "settings.json: {name} {value} is outside {lo}..{hi}; using {default}"
                ));
                *value = default;
            }
        };
        within("zoom", &mut self.zoom, ZOOM_MIN, ZOOM_MAX, d.zoom);
        within("font_size", &mut self.font_size, 6.0, 72.0, d.font_size);
        within(
            "minimum_contrast",
            &mut self.minimum_contrast,
            1.0,
            21.0,
            d.minimum_contrast,
        );
        let clamp = |v: f32, lo: f32, hi: f32| if v.is_finite() { v.clamp(lo, hi) } else { 1.0 };
        self.terminal_line_height = clamp(self.terminal_line_height, 0.8, 2.0);
        self.terminal_cell_width = clamp(self.terminal_cell_width, 0.8, 2.0);
        self.terminal_scroll_speed = clamp(self.terminal_scroll_speed, 0.1, 5.0);
        if self.font.trim().is_empty() {
            self.font = d.font;
        }
        if self.ui_font.trim().is_empty() {
            self.ui_font = d.ui_font;
        }
        if self.theme.trim().is_empty() {
            self.theme = d.theme;
        }
        problems
    }

    /// Whether the window is dark.
    pub fn dark(&self, system_dark: bool) -> bool {
        match self.appearance {
            Appearance::Dark => true,
            Appearance::Light => false,
            Appearance::System => system_dark,
        }
    }
}

/// The zoom `steps` tenths from `zoom`, within the zoom keys' range; 0 steps resets it to 1
/// (herdr-app's `next_zoom`).
pub fn next_zoom(zoom: f32, steps: i32) -> f32 {
    if steps == 0 {
        return 1.0;
    }
    ((zoom * 10.0).round() + steps as f32).clamp(ZOOM_MIN * 10.0, ZOOM_MAX * 10.0) / 10.0
}

/// `text` (the settings file as it is, or `None` with no file) with `zoom` set and every other
/// key kept: what the zoom keys write. `None` when the text isn't a JSON object, which is never
/// overwritten.
pub fn with_zoom(text: Option<&str>, zoom: f32) -> Option<String> {
    let mut object = match text.map(str::trim) {
        None | Some("") => serde_json::Map::new(),
        Some(text) => match serde_json::from_str::<Value>(text).ok()? {
            Value::Object(object) => object,
            _ => return None,
        },
    };
    // In tenths, as the keys step it, so the file says 1.1 and not 1.100000023841858.
    let tenths = (f64::from(zoom) * 10.0).round() / 10.0;
    object.insert("zoom".into(), serde_json::json!(tenths));
    let mut out = serde_json::to_string_pretty(&Value::Object(object)).ok()?;
    out.push('\n');
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_is_elis_settings() {
        let (s, problems) = Settings::parse("{}").unwrap();
        assert!(problems.is_empty());
        assert_eq!(s, Settings::default());
        assert_eq!(s.zoom, 1.1);
        assert_eq!(s.font, "Operator Mono Book Herdr");
        assert_eq!(s.ui_font, "Operator Mono Book Herdr");
        assert_eq!(s.font_size, 17.0);
        assert_eq!(
            (
                s.terminal_line_height,
                s.terminal_cell_width,
                s.terminal_scroll_speed
            ),
            (1.0, 1.0, 1.0)
        );
        assert_eq!(s.minimum_contrast, 1.0);
        assert!(!s.pane_scrollbars);
        assert_eq!(s.appearance, Appearance::Dark);
        assert_eq!(s.theme, "arctic");
    }

    #[test]
    fn elis_herdr_app_settings_file_reads_as_it_is() {
        let text = r#"{
          "zoom": 1.100000023841858,
          "sounds": { "enabled": false },
          "font": "Operator Mono Book Herdr",
          "font_size": 17.0,
          "font_ligatures": false,
          "terminal_line_height": 1.0,
          "terminal_cell_width": 1.0,
          "terminal_scroll_speed": 1.0,
          "ui_font": "Operator Mono Book Herdr",
          "telemetry": false,
          "pane_scrollbars": false,
          "minimum_contrast": 1.0,
          "appearance": "dark",
          "theme": "arctic"
        }"#;
        let (s, problems) = Settings::parse(text).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn values_out_of_range_take_their_default_or_are_clamped() {
        let (s, problems) = Settings::parse(
            r#"{"zoom": 9, "font_size": 2, "minimum_contrast": 30,
                "terminal_line_height": 3, "terminal_cell_width": 0.1, "terminal_scroll_speed": 9,
                "font": " "}"#,
        )
        .unwrap();
        assert_eq!((s.zoom, s.font_size, s.minimum_contrast), (1.1, 17.0, 1.0));
        assert_eq!(
            (
                s.terminal_line_height,
                s.terminal_cell_width,
                s.terminal_scroll_speed
            ),
            (2.0, 0.8, 5.0)
        );
        assert_eq!(s.font, "Operator Mono Book Herdr");
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("zoom 9 is outside 0.5..3"));
    }

    #[test]
    fn a_broken_file_is_an_error() {
        assert!(
            Settings::parse("{")
                .unwrap_err()
                .starts_with("settings.json:")
        );
        assert!(Settings::parse(r#"{"zoom": "big"}"#).is_err());
        assert!(Settings::parse(r#"{"appearance": "sepia"}"#).is_err());
    }

    #[test]
    fn zoom_steps_in_tenths_within_range_and_resets_to_one() {
        assert_eq!(next_zoom(1.1, 1), 1.2);
        assert_eq!(next_zoom(1.1, -1), 1.0);
        assert_eq!(next_zoom(1.1, 0), 1.0);
        assert_eq!(next_zoom(3.0, 1), 3.0);
        assert_eq!(next_zoom(0.5, -1), 0.5);
        // A zoom between tenths lands on the nearest one first.
        assert_eq!(next_zoom(1.13, 1), 1.2);
    }

    #[test]
    fn the_zoom_keys_rewrite_only_the_zoom() {
        let text = r#"{"font_size": 17, "zoom": 1.1, "theme": "arctic"}"#;
        let out = with_zoom(Some(text), 1.2).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["zoom"], 1.2);
        assert_eq!(v["font_size"], 17);
        assert_eq!(v["theme"], "arctic");
        assert!(out.ends_with('\n'));
        assert_eq!(
            with_zoom(None, 1.3).as_deref(),
            Some("{\n  \"zoom\": 1.3\n}\n")
        );
        assert_eq!(with_zoom(Some("{ broken"), 1.3), None);
        assert_eq!(with_zoom(Some("[1]"), 1.3), None);
    }
}
