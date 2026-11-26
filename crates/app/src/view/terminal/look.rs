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
}

impl Default for Look {
    fn default() -> Look {
        Look {
            selection_background: None,
            selection_foreground: None,
            cursor: None,
            cursor_text: None,
            faint_opacity: 0.5,
        }
    }
}

impl Look {
    /// What changes a built row: rows built under another look are built again.
    pub(crate) fn key(&self) -> [u32; 5] {
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
        ]
    }
}

/// Eli's arctic theme (`docs/research/feature-inventory.md` §2): the terminal colors his Ghostty
/// and Neovim share, and Herdr app's `#04395E` selection.
pub fn arctic() -> (Theme, Look) {
    let hex = |v: u32| Rgb::new((v >> 16) as u8, (v >> 8) as u8, v as u8);
    let ansi = [
        0x1e1e1e, 0xf44747, 0x6a9955, 0xd7ba7d, 0x569cd6, 0xc586c0, 0x4dc9b0, 0xdddddd, 0x808080,
        0xf55757, 0x80ad6b, 0xe1cb9e, 0x6fabdc, 0xd6a9d2, 0x70d3bf, 0xffffff,
    ];
    let theme = Theme {
        foreground: Some(hex(0xd4d4d4)),
        background: Some(hex(0x1e1e1e)),
        cursor: None,
        palette: ansi
            .iter()
            .enumerate()
            .map(|(i, &c)| (i as u8, hex(c)))
            .collect(),
    };
    let look = Look {
        selection_background: Some(hex(0x04395e)),
        ..Look::default()
    };
    (theme, look)
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
