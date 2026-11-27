// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! Themes in herdr-app's file format (`app:src/settings.rs`): a terminal palette (`background`,
//! `foreground`, `selection`, `cursor`, 16 `ansi` colors) and `colors` overriding the panel
//! colors herdr-app derives from it. Eli's arctic (`assets/themes/arctic.json`, a copy of
//! `~/.dotfiles/home/.config/herdr-app/themes/arctic.json`) is built in; any other name is a
//! file in the home's `themes/`.
//!
//! [`Theme::resolve`] is herdr-app's `resolve`: every color by name, derived where the file is
//! silent, then the theme's and the settings' overrides. The window takes three views of it: the
//! terminals' colors ([`Theme::terminal`], [`Theme::look`]), the panels' ([`Chrome`]) and Rune's
//! tokens ([`Theme::tokens`]) for what Rune draws on its own.

use std::collections::BTreeMap;

use rune::Color;
use serde::Deserialize;
use terminal::Rgb;

use crate::view::terminal::look::Look;

/// The built-in theme's name and file.
pub const ARCTIC: &str = "arctic";
pub const ARCTIC_JSON: &str = include_str!("../../../../assets/themes/arctic.json");

/// `themes/<name>.json`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    background: String,
    foreground: String,
    selection: String,
    cursor: Option<String>,
    ansi: [String; 16],
    #[serde(default)]
    colors: BTreeMap<String, String>,
}

/// One color, `#rrggbb` or `#rrggbbaa` in sRGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub fn parse(s: &str) -> Option<Rgba> {
        let h = s.trim().strip_prefix('#')?;
        if !h.is_ascii() || !matches!(h.len(), 6 | 8) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
        Some(Rgba {
            r: byte(0)?,
            g: byte(2)?,
            b: byte(4)?,
            a: if h.len() == 8 { byte(6)? } else { 255 },
        })
    }

    fn opaque(v: u32) -> Rgba {
        Rgba {
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
            a: 255,
        }
    }

    fn with_alpha(v: u32, a: u8) -> Rgba {
        Rgba {
            a,
            ..Rgba::opaque(v)
        }
    }

    pub fn rgb(self) -> Rgb {
        Rgb::new(self.r, self.g, self.b)
    }

    /// As Rune paints it: linear light, straight alpha.
    pub fn color(self) -> Color {
        let channel = |v: u8| rune::color::srgb_decode(f32::from(v) / 255.0);
        Color {
            r: channel(self.r),
            g: channel(self.g),
            b: channel(self.b),
            a: f32::from(self.a) / 255.0,
        }
    }

    pub fn hex(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }
}

/// The terminal palette a theme file gives.
#[derive(Clone, Debug, PartialEq)]
struct Palette {
    background: u32,
    foreground: u32,
    selection: u32,
    cursor: Option<u32>,
    ansi: [u32; 16],
}

/// A theme with every color resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: String,
    colors: BTreeMap<String, Rgba>,
}

/// The panels' colors (herdr-app's theme names, `cfg/herdr.d.ts` `theme()`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chrome {
    /// The window behind everything.
    pub canvas: Color,
    /// Sidebars and bars.
    pub chrome: Color,
    pub surface: Color,
    pub popover: Color,
    /// A selected tab.
    pub raised: Color,
    pub hover: Color,
    pub selected: Color,
    /// The line between split panes.
    pub border: Color,
    pub input: Color,
    /// The panels' text.
    pub foreground: Color,
    pub muted: Color,
    pub dim: Color,
    pub accent: Color,
    pub accent_foreground: Color,
    pub ring: Color,
    pub working: Color,
    pub blocked: Color,
    pub done: Color,
    pub idle: Color,
}

impl Theme {
    /// The built-in arctic, with no overrides.
    pub fn arctic() -> Theme {
        Theme::from_file(ARCTIC, ARCTIC_JSON).expect("arctic.json parses")
    }

    /// A theme file's text, resolved. The settings' `colors` go over it with [`Theme::apply`].
    pub fn from_file(name: &str, text: &str) -> Result<Theme, String> {
        let f: File = serde_json::from_str(text).map_err(|e| format!("themes/{name}.json: {e}"))?;
        let rgb = |key: &str, v: &str| {
            Rgba::parse(v)
                .filter(|c| c.a == 255)
                .map(|c| u32::from(c.r) << 16 | u32::from(c.g) << 8 | u32::from(c.b))
                .ok_or(format!("themes/{name}.json: {key} {v:?} is not #rrggbb"))
        };
        let mut ansi = [0; 16];
        for (i, v) in f.ansi.iter().enumerate() {
            ansi[i] = rgb(&format!("ansi[{i}]"), v)?;
        }
        let palette = Palette {
            background: rgb("background", &f.background)?,
            foreground: rgb("foreground", &f.foreground)?,
            selection: rgb("selection", &f.selection)?,
            cursor: f.cursor.as_deref().map(|v| rgb("cursor", v)).transpose()?,
            ansi,
        };
        let mut colors = resolve(&palette);
        for (k, v) in &f.colors {
            let c = Rgba::parse(v).ok_or(format!(
                "themes/{name}.json: colors.{k}: {v:?} is not #rrggbb or #rrggbbaa"
            ))?;
            colors.insert(k.clone(), c);
        }
        Ok(Theme {
            name: name.to_owned(),
            colors,
        })
    }

    /// The settings' `colors` over the theme's. A value that isn't a color is skipped and
    /// reported; the others apply.
    pub fn apply(&mut self, overrides: &BTreeMap<String, String>) -> Vec<String> {
        let mut problems = Vec::new();
        for (k, v) in overrides {
            match Rgba::parse(v) {
                Some(c) => {
                    self.colors.insert(k.clone(), c);
                }
                None => problems.push(format!(
                    "settings.json: colors.{k}: {v:?} is not #rrggbb or #rrggbbaa"
                )),
            }
        }
        problems
    }

    /// A color by name (`chrome`, `ansi4`, `terminal_foreground`, …).
    pub fn get(&self, name: &str) -> Option<Rgba> {
        self.colors.get(name).copied()
    }

    /// A color by name, magenta when the theme has none, so a mistake shows (as herdr-app does).
    fn paint(&self, name: &str) -> Color {
        self.get(name).unwrap_or(Rgba::opaque(0xff00ff)).color()
    }

    /// The colors terminals start with: the theme's background, foreground, cursor and 16 ANSI
    /// colors.
    pub fn terminal(&self) -> terminal::Theme {
        let rgb = |name: &str| self.get(name).map(Rgba::rgb);
        terminal::Theme {
            foreground: rgb("terminal_foreground"),
            background: rgb("background"),
            cursor: rgb("cursor"),
            palette: (0..16u8)
                .filter_map(|i| rgb(&format!("ansi{i}")).map(|c| (i, c)))
                .collect(),
        }
    }

    /// What a terminal view adds: the selection, and the cursor unless the program set one.
    pub fn look(&self, minimum_contrast: f32) -> Look {
        Look {
            selection_background: self.get("selection").map(Rgba::rgb),
            cursor: self.get("cursor").map(Rgba::rgb),
            minimum_contrast,
            ..Look::default()
        }
    }

    pub fn chrome(&self) -> Chrome {
        let c = |name: &str| self.paint(name);
        Chrome {
            canvas: c("canvas"),
            chrome: c("chrome"),
            surface: c("surface"),
            popover: c("popover"),
            raised: c("raised"),
            hover: c("hover"),
            selected: c("selected"),
            border: c("border"),
            input: c("input"),
            foreground: c("foreground"),
            muted: c("muted"),
            dim: c("dim"),
            accent: c("accent"),
            accent_foreground: c("accent_foreground"),
            ring: c("ring"),
            working: c("working"),
            blocked: c("blocked"),
            done: c("done"),
            idle: c("idle"),
        }
    }

    /// Rune's semantic tokens in the theme's colors, for what Rune draws with them (the hello
    /// window, focus rings, Rune's own controls). Shadows keep Rune's.
    pub fn tokens(&self, dark: bool) -> rune::tok::Theme {
        let c = self.chrome();
        rune::tok::Theme {
            bg: c.canvas,
            card: c.surface,
            card_border: c.border,
            text: c.foreground,
            text_secondary: c.muted,
            separator: c.border,
            control_track: c.input,
            control_off: c.dim,
            indicator: c.accent,
            knob: c.foreground,
            hover: c.hover,
            press: c.selected,
            ..rune::tok::theme(dark)
        }
    }
}

/// Whether a color is light, by its luminance (herdr-app's `is_light`).
fn is_light(c: u32) -> bool {
    let ch = |s: u32| ((c >> s) & 255) as f32 / 255.0;
    0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0) > 0.5
}

/// `a` mixed toward `b` by `t`, per sRGB channel (herdr-app's `mix`).
fn mix(a: u32, b: u32, t: f32) -> u32 {
    [16, 8, 0].iter().fold(0, |acc, s| {
        let (x, y) = (((a >> s) & 255) as f32, ((b >> s) & 255) as f32);
        acc | (((x * (1.0 - t) + y * t).round() as u32) << s)
    })
}

/// herdr-app's `resolve`: the panels' colors around a palette, before overrides. `chrome` and
/// `canvas` step away from the terminal's background, darker on a dark theme, toward warm grey
/// on a light one.
fn resolve(p: &Palette) -> BTreeMap<String, Rgba> {
    let bg = p.background;
    let light = is_light(bg);
    let fg = if light { 0x1b1b1e } else { 0xeeeeef };
    // Theme files take the status colors from ansi (yellow, red, blue).
    let [working, blocked, done] = [p.ansi[3], p.ansi[1], p.ansi[4]];
    let o = Rgba::opaque;
    let (popover, raised, hover, selected, border, input, canvas, chrome) = if light {
        let warm = 0xe8e8e2;
        (
            o(bg),
            o(mix(bg, 0, 0.035)),
            Rgba::with_alpha(0, 0x0c),
            Rgba::with_alpha(0, 0x12),
            Rgba::with_alpha(0, 0x17),
            Rgba::with_alpha(0, 0x26),
            o(mix(bg, warm, 0.2)),
            o(mix(bg, warm, 0.6)),
        )
    } else {
        (
            o(mix(bg, 0xffffff, 0.06)),
            o(mix(bg, 0xffffff, 0.09)),
            Rgba::with_alpha(0xffffff, 0x0e),
            Rgba::with_alpha(0xffffff, 0x17),
            Rgba::with_alpha(0xffffff, 0x18),
            Rgba::with_alpha(0xffffff, 0x24),
            o(mix(bg, 0, 0.16)),
            o(mix(bg, 0, 0.32)),
        )
    };
    let mut t: BTreeMap<String, Rgba> = [
        ("background", o(bg)),
        ("terminal_foreground", o(p.foreground)),
        ("foreground", o(fg)),
        ("surface", o(bg)),
        ("canvas", canvas),
        ("chrome", chrome),
        ("popover", popover),
        ("raised", raised),
        ("hover", hover),
        ("selected", selected),
        ("border", border),
        ("input", input),
        ("muted", o(mix(bg, fg, if light { 0.66 } else { 0.6 }))),
        ("dim", o(mix(bg, fg, if light { 0.46 } else { 0.38 }))),
        ("accent", o(p.ansi[4])),
        ("accent_foreground", o(bg)),
        ("solid", o(fg)),
        (
            "solid_foreground",
            o(if light { 0xffffff } else { mix(bg, 0, 0.28) }),
        ),
        ("ring", o(mix(bg, fg, 0.7))),
        ("selection", o(p.selection)),
        ("cursor", o(p.cursor.unwrap_or(p.foreground))),
        ("working", o(working)),
        ("blocked", o(blocked)),
        ("done", o(done)),
        ("idle", o(mix(bg, fg, 0.38))),
        ("unknown", o(mix(bg, fg, 0.38))),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    for (i, c) in p.ansi.iter().enumerate() {
        t.insert(format!("ansi{i}"), o(*c));
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(t: &Theme, name: &str) -> String {
        t.get(name).unwrap().hex()
    }

    #[test]
    fn arctic_is_elis_theme() {
        let t = Theme::arctic();
        // The terminal: Ghostty's and Neovim's colors.
        let term = t.terminal();
        assert_eq!(term.background, Some(Rgb::new(0x1e, 0x1e, 0x1e)));
        assert_eq!(term.foreground, Some(Rgb::new(0xd4, 0xd4, 0xd4)));
        assert_eq!(term.cursor, Some(Rgb::new(0xd4, 0xd4, 0xd4)));
        assert_eq!(term.palette.len(), 16);
        assert_eq!(term.palette[1], (1, Rgb::new(0xf4, 0x47, 0x47)));
        assert_eq!(term.palette[15], (15, Rgb::new(0xff, 0xff, 0xff)));
        let look = t.look(1.0);
        assert_eq!(look.selection_background, Some(Rgb::new(0x04, 0x39, 0x5e)));
        // The panels: arctic's overrides (`feature-inventory.md` §2).
        for (name, want) in [
            ("canvas", "#1e1e1e"),
            ("chrome", "#1e1e1e"),
            ("surface", "#1e1e1e"),
            ("popover", "#1e1e1e"),
            ("raised", "#2a2d2e"),
            ("hover", "#2d2d2d"),
            ("selected", "#04395e"),
            ("border", "#858585"),
            ("input", "#a9a9a9"),
            ("muted", "#8f8f8f"),
            ("dim", "#8f8f8f"),
            ("accent", "#858585"),
            ("working", "#3794ff"),
            ("blocked", "#cca700"),
            ("done", "#89d185"),
        ] {
            assert_eq!(hex(&t, name), want, "{name}");
        }
        // What arctic leaves to herdr-app's derivation.
        assert_eq!(hex(&t, "foreground"), "#eeeeef");
        assert_eq!(hex(&t, "accent_foreground"), "#1e1e1e");
    }

    #[test]
    fn a_theme_without_overrides_is_derived_as_herdr_app_derives_it() {
        let text = r##"{"background": "#17191d", "foreground": "#d2d4d9", "selection": "#394454",
            "ansi": ["#45484f", "#dc8c95", "#a6bd91", "#d8be87", "#96b2d7", "#b5a2ce", "#8cbdc3",
                     "#d2d4d9", "#818793", "#f0a5ad", "#bad5a2", "#ead2a1", "#b0cbef", "#cbb7e6",
                     "#a4d5db", "#f0f0f2"]}"##;
        let t = Theme::from_file("plain", text).unwrap();
        assert_eq!(hex(&t, "chrome"), hex_of(mix(0x17191d, 0, 0.32)));
        assert_eq!(hex(&t, "canvas"), hex_of(mix(0x17191d, 0, 0.16)));
        assert_eq!(hex(&t, "hover"), "#ffffff0e");
        assert_eq!(hex(&t, "accent"), "#96b2d7");
        assert_eq!(hex(&t, "working"), "#d8be87");
        assert_eq!(hex(&t, "cursor"), "#d2d4d9");
    }

    fn hex_of(v: u32) -> String {
        Rgba::opaque(v).hex()
    }

    #[test]
    fn settings_colors_go_over_the_themes_and_reach_terminals() {
        let overrides: BTreeMap<String, String> = [
            ("accent".to_owned(), "#ff8800".to_owned()),
            ("background".to_owned(), "#000000".to_owned()),
        ]
        .into();
        let mut t = Theme::arctic();
        assert!(t.apply(&overrides).is_empty());
        assert_eq!(hex(&t, "accent"), "#ff8800");
        assert_eq!(t.terminal().background, Some(Rgb::new(0, 0, 0)));
        // A bad one is skipped and said; the good ones still apply.
        let mixed: BTreeMap<String, String> = [
            ("accent".to_owned(), "orange".to_owned()),
            ("chrome".to_owned(), "#202020".to_owned()),
        ]
        .into();
        let mut t = Theme::arctic();
        let problems = t.apply(&mixed);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("colors.accent"));
        assert_eq!(hex(&t, "accent"), "#858585");
        assert_eq!(hex(&t, "chrome"), "#202020");
    }

    #[test]
    fn a_bad_theme_file_says_what_is_wrong() {
        assert!(Theme::from_file("x", "{}").is_err());
        let bad = ARCTIC_JSON.replace("\"#f44747\"", "\"red\"");
        let err = Theme::from_file("x", &bad).unwrap_err();
        assert!(err.contains("ansi[1]"), "{err}");
    }

    #[test]
    fn rune_tokens_take_the_panels_colors() {
        let t = Theme::arctic();
        let tokens = t.tokens(true);
        assert_eq!(tokens.bg, t.chrome().canvas);
        assert_eq!(tokens.text, t.chrome().foreground);
        assert_eq!(tokens.text_secondary, t.chrome().muted);
    }
}
