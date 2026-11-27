//! The colors a terminal view adds to the ones the terminal resolves: selection, cursor and faint
//! text, as Ghostty's `selection-*`, `cursor-*` and `faint-opacity` settings give them.

use rune::Color;
use rune::color::srgb_decode;
use terminal::{Rgb, Theme};

/// How a view colors what the program doesn't: the defaults are Ghostty's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// The selection's background. `None` draws selected cells in the terminal's default
    /// background on its default foreground, as Ghostty does with no `selection-background`.
    pub selection_background: Option<Rgb>,
    /// The selected text. `None` keeps each cell's own foreground when the background is set,
    /// as Herdr app does (`herdr-app:src/vt.rs:1647`), and is the default background when it
    /// isn't, as in Ghostty.
    pub selection_foreground: Option<Rgb>,
    /// The cursor, unless the program set one (OSC 12). `None` is the foreground.
    pub cursor: Option<Rgb>,
    /// The glyph under a block cursor. `None` is the terminal's background.
    pub cursor_text: Option<Rgb>,
    /// How opaque faint (SGR 2) text is.
    pub faint_opacity: f32,
    /// Text whose contrast ratio with its background is below this is drawn white or black,
    /// whichever contrasts more (Ghostty's `minimum-contrast`). 1 leaves every color alone.
    pub minimum_contrast: f32,
}

impl Default for Look {
    fn default() -> Look {
        Look {
            selection_background: None,
            selection_foreground: None,
            cursor: None,
            cursor_text: None,
            faint_opacity: 0.5,
            minimum_contrast: 1.0,
        }
    }
}

impl Look {
    /// What changes a built row: rows built under another look are built again.
    pub(crate) fn key(&self) -> [u32; 6] {
        let rgb = |c: Option<Rgb>| {
            c.map_or(u32::MAX, |c| {
                u32::from(c.r) << 16 | u32::from(c.g) << 8 | u32::from(c.b)
            })
        };
        [
            rgb(self.selection_background),
            rgb(self.selection_foreground),
            rgb(self.cursor),
            rgb(self.cursor_text),
            self.faint_opacity.to_bits(),
            self.minimum_contrast.to_bits(),
        ]
    }
}

/// Eli's arctic theme (`docs/research/feature-inventory.md` §2): the terminal colors his Ghostty
/// and Neovim share, and Herdr app's `#04395E` selection ([`crate::config::theme::Theme::arctic`]).
pub fn arctic() -> (Theme, Look) {
    let arctic = crate::config::theme::Theme::arctic();
    (arctic.terminal(), arctic.look(1.0))
}

/// A terminal color as Rune's linear-light color.
pub fn color(c: Rgb) -> Color {
    let channel = |v: u8| srgb_decode(f32::from(v) / 255.0);
    Color {
        r: channel(c.r),
        g: channel(c.g),
        b: channel(c.b),
        a: 1.0,
    }
}
