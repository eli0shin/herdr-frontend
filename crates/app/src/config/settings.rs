// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! `settings.json` in herdr-frontend's home: the person's look and terminal preferences, in
//! herdr-app's key names (`~/.config/herdr-app/settings.json`, `app:src/settings.rs` `Person`),
//! so a line copied from one works in the other.
//!
//! Every key is optional. A missing key, or a missing file, takes Eli's current value
//! (`docs/research/feature-inventory.md` §2), which is [`Settings::default`]. Keys herdr-frontend
//! doesn't read (`sounds`, `telemetry`, `font_ligatures`, …) are ignored: it has no sounds, no
//! telemetry and draws terminals without ligatures.
//!
//! The font style keys are Ghostty's (`font-style`, `font-style-bold`, `font-style-italic`,
//! `font-style-bold-italic`), which Herdr app doesn't have: each names the face of the `font`
//! family a style draws with, by its style name (`config::fonts`).

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

/// The zoom keys' range, as herdr-app's (`ZOOM_MIN`, `ZOOM_MAX`).
pub const ZOOM_MIN: f32 = 0.5;
pub const ZOOM_MAX: f32 = 3.0;

/// The panels' text size before zoom: herdr-app's `ui_font_size`, which no file changes.
pub const UI_FONT_SIZE: f32 = 13.0;

/// Eli's terminal and panel font family, as installed (`~/Library/Fonts/OperatorMono-*.otf`).
pub const FONT: &str = "Operator Mono";

/// The faces of Operator Mono the panels draw with, by style name as [`Settings::font_styles`]:
/// Medium where the terminals have Book. Herdr app draws its panels in Book through CoreText
/// with font smoothing on, which thickens light text on a dark canvas by about a third. The
/// window here draws without smoothing, as Ghostty does, so Medium, which carries about a
/// third more ink than Book, gives the panels the weight Herdr app's have
/// (`docs/settings.md` "Fonts").
pub const PANEL_STYLES: [Option<&str>; 4] = [
    Some("Medium"),
    Some("Bold"),
    Some("Medium Italic"),
    Some("Bold Italic"),
];

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
    /// The terminals' font family and size (points at zoom 1, 6 to 72): Ghostty's `font-family`
    /// and `font-size`.
    pub font: String,
    pub font_size: f32,
    /// The style name of the face each terminal style draws with (`"Book"`, `"Bold Italic"`), as
    /// Ghostty's `font-style*`. `None` finds the family's face by its bold and italic traits
    /// instead. A file that names another `font` and leaves these out gets `None`: the defaults
    /// are Operator Mono's style names.
    pub font_style: Option<String>,
    pub font_style_bold: Option<String>,
    pub font_style_italic: Option<String>,
    pub font_style_bold_italic: Option<String>,
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
    /// A scroll position thumb at a terminal pane's right edge, which can be dragged. Eli's is
    /// off.
    pub pane_scrollbars: bool,
    /// Letting go of a mouse selection copies it.
    pub copy_on_select: bool,
    pub appearance: Appearance,
    /// `arctic` (built in) or the name of a file in the home's `themes/`.
    pub theme: String,
    /// Theme color overrides by name (`{"accent": "#ff8800"}`), `#rrggbb` or `#rrggbbaa`.
    pub colors: BTreeMap<String, String>,
}

impl Default for Settings {
    /// Eli's `settings.json` (`docs/research/feature-inventory.md` §2), with Operator Mono's
    /// Book weight and its own Bold, as installed.
    fn default() -> Settings {
        Settings {
            zoom: 1.1,
            font: FONT.into(),
            font_size: 17.0,
            font_style: Some("Book".into()),
            font_style_bold: Some("Bold".into()),
            font_style_italic: Some("Book Italic".into()),
            font_style_bold_italic: Some("Bold Italic".into()),
            ui_font: FONT.into(),
            terminal_line_height: 1.0,
            terminal_cell_width: 1.0,
            terminal_scroll_speed: 1.0,
            minimum_contrast: 1.0,
            pane_scrollbars: false,
            copy_on_select: true,
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
            font_style: None,
            font_style_bold: None,
            font_style_italic: None,
            font_style_bold_italic: None,
            ui_font: "JetBrains Mono".into(),
            ..Settings::default()
        }
    }

    /// The four `font_style*` keys, regular first.
    pub fn font_styles(&self) -> [Option<&str>; 4] {
        [
            &self.font_style,
            &self.font_style_bold,
            &self.font_style_italic,
            &self.font_style_bold_italic,
        ]
        .map(|s| s.as_deref())
    }

    /// The settings a file's text gives, and what's wrong with it. Text that isn't a settings
    /// object is an `Err`: the caller keeps what it had. A value out of range takes its default
    /// (or is clamped, where herdr-app clamps) and is reported.
    pub fn parse(text: &str) -> Result<(Settings, Vec<String>), String> {
        let value: Value = serde_json::from_str(text).map_err(|e| format!("settings.json: {e}"))?;
        let mut settings: Settings =
            Settings::deserialize(&value).map_err(|e| format!("settings.json: {e}"))?;
        let problems = settings.check();
        // The style names are Operator Mono's: another family the file names keeps none it
        // doesn't give, so its faces are found by their traits. After `check`, so an empty
        // `font` (Operator Mono again) keeps them.
        if !settings.font.trim().eq_ignore_ascii_case(FONT) {
            let given = |key: &str| value.get(key).is_some();
            for (key, style) in [
                ("font_style", &mut settings.font_style),
                ("font_style_bold", &mut settings.font_style_bold),
                ("font_style_italic", &mut settings.font_style_italic),
                (
                    "font_style_bold_italic",
                    &mut settings.font_style_bold_italic,
                ),
            ] {
                if !given(key) {
                    *style = None;
                }
            }
        }
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
        for style in [
            &mut self.font_style,
            &mut self.font_style_bold,
            &mut self.font_style_italic,
            &mut self.font_style_bold_italic,
        ] {
            if style.as_deref().is_some_and(|s| s.trim().is_empty()) {
                *style = None;
            }
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
        assert_eq!(s.font, "Operator Mono");
        assert_eq!(s.ui_font, "Operator Mono");
        assert_eq!(s.font_size, 17.0);
        assert_eq!(
            s.font_styles(),
            [
                Some("Book"),
                Some("Bold"),
                Some("Book Italic"),
                Some("Bold Italic")
            ]
        );
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
        assert!(s.copy_on_select);
        assert_eq!(s.appearance, Appearance::Dark);
        assert_eq!(s.theme, "arctic");
    }

    #[test]
    fn elis_herdr_app_settings_file_reads_as_it_is() {
        let text = r#"{
          "zoom": 1.100000023841858,
          "sounds": { "enabled": false },
          "font": "Operator Mono",
          "font_size": 17.0,
          "font_ligatures": false,
          "terminal_line_height": 1.0,
          "terminal_cell_width": 1.0,
          "terminal_scroll_speed": 1.0,
          "ui_font": "Operator Mono",
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
        assert_eq!(s.font, "Operator Mono");
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("zoom 9 is outside 0.5..3"));
    }

    #[test]
    fn the_style_names_go_with_operator_mono() {
        // Another family keeps only the styles the file gives; the rest are found by trait.
        let (s, _) = Settings::parse(r#"{"font": "Menlo", "font_style_bold": "Bold"}"#).unwrap();
        assert_eq!(s.font_styles(), [None, Some("Bold"), None, None]);
        // Operator Mono, in any case, keeps Book and Bold; null or "" asks for traits.
        let (s, _) = Settings::parse(
            r#"{"font": "operator mono", "font_style_italic": null, "font_style": " "}"#,
        )
        .unwrap();
        assert_eq!(
            s.font_styles(),
            [None, Some("Bold"), None, Some("Bold Italic")]
        );
        // An empty font is Operator Mono, with its style names.
        let (s, _) = Settings::parse(r#"{"font": ""}"#).unwrap();
        assert_eq!(s, Settings::default());
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
