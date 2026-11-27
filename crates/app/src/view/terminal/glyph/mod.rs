//! The glyph atlas: cells the row's shaped line can't draw the way Ghostty does (wide characters,
//! characters the terminal font lacks, emoji, Nerd Font icons), rasterized as Ghostty's macOS
//! face rasterizes them and drawn as images over the grid at whole device pixels
//! (`docs/research/terminal-rendering.md` §2 "Glyph atlas" and "Font fallback and shaping").
//!
//! For each such cell:
//!
//! - **Font fallback** follows Ghostty's order (`font/CodepointResolver.zig`,
//!   `font/SharedGridSet.zig`): for a terminal font other than JetBrains Mono, the JetBrains Mono
//!   Ghostty embeds as its backup (`SharedGridSet.zig:260-316`), then the Symbols Nerd Font Ghostty embeds, then Apple Color Emoji for
//!   emoji presentation, then the fallback faces already loaded, then CoreText's fallback for the
//!   character, scaled so its ideograph width matches the terminal font's
//!   (`Collection.SizeAdjustment.ic_width`). A character is emoji when a variation selector says
//!   so, or by default when it has the Unicode `Emoji_Presentation` property.
//! - **Constraints** ([`constraint`]) size and place the glyph: every emoji covers its cells,
//!   Nerd Font icons follow Ghostty's table, other symbols shrink to fit, and a symbol followed by
//!   a blank cell may spread into it.
//! - **Rasterizing** makes the same CoreGraphics calls as Ghostty's `coretext.zig`
//!   `renderGlyph`: antialiased, no font smoothing, subpixel positioning without quantizing, the
//!   glyph's box placed by its constraint and centered in the rounded cell. Color glyphs are drawn
//!   in sRGB (Ghostty uses Display P3 for its P3 surface; ours is sRGB), text glyphs as an alpha
//!   mask in the cell's color.
//!
//! What it doesn't take: clusters of several code points other than a variation selector
//! (combining marks, emoji ZWJ sequences, skin tones). Those stay pinned cells Rune shapes, as do
//! all cells off macOS.

pub mod constraint;
#[cfg(target_os = "macos")]
mod coretext;
mod tables;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rune::Pixels;
use terminal::Rgb;

use super::font::Face;

/// The Symbols Nerd Font 3.4.0 Ghostty embeds as a fallback (`ghostty:src/font/embedded.zig`).
pub const SYMBOLS_NERD_FONT: &[u8] =
    include_bytes!("../../../../../../assets/fonts/SymbolsNerdFont-Regular.ttf");

/// A rasterized glyph: its pixels and where they sit relative to its cell.
pub struct GlyphImage {
    /// The left edge, in device pixels right of the cell's left edge.
    pub x: i32,
    /// The top edge, in device pixels below the cell's top edge.
    pub top: i32,
    pub width: u32,
    pub height: u32,
    /// A color glyph (emoji): `data` is straight-alpha RGBA, drawn as it is. Otherwise `data` is
    /// an alpha mask drawn in the cell's color.
    pub color: bool,
    data: Vec<u8>,
    /// The images made from it, by the color they're drawn in.
    tinted: RefCell<HashMap<(Rgb, u8), Pixels>>,
}

impl GlyphImage {
    /// The glyph as an image: a color glyph as it is, faded to `alpha`; a mask in `ink`.
    pub fn pixels(&self, ink: Rgb, alpha: f32) -> Pixels {
        let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
        let key = if self.color {
            (Rgb::default(), a)
        } else {
            (ink, a)
        };
        if let Some(p) = self.tinted.borrow().get(&key) {
            return p.clone();
        }
        let scale = |v: u8| ((u16::from(v) * u16::from(a) + 127) / 255) as u8;
        let rgba: Vec<u8> = if self.color {
            self.data
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2], scale(p[3])])
                .collect()
        } else {
            self.data
                .iter()
                .flat_map(|&m| [ink.r, ink.g, ink.b, scale(m)])
                .collect()
        };
        let pixels = Pixels::new();
        pixels.set(self.width, self.height, rgba);
        self.tinted.borrow_mut().insert(key, pixels.clone());
        pixels
    }
}

/// A cell to draw from the atlas, as a row hands it to the pane.
#[derive(Clone)]
pub struct AtlasCell {
    pub col: u16,
    pub glyph: Rc<GlyphImage>,
    pub pixels: Pixels,
}

impl std::fmt::Debug for AtlasCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AtlasCell")
            .field("col", &self.col)
            .field("x", &self.glyph.x)
            .field("top", &self.glyph.top)
            .field("size", &(self.glyph.width, self.glyph.height))
            .field("color", &self.glyph.color)
            .finish()
    }
}

impl PartialEq for AtlasCell {
    fn eq(&self, other: &AtlasCell) -> bool {
        self.col == other.col
            && Rc::ptr_eq(&self.glyph, &other.glyph)
            && self.pixels.signature() == other.pixels.signature()
    }
}

impl AtlasCell {
    /// The image over the grid: its glyph's box from its cell, in the grid's points.
    pub fn view(&self, row: u16, face: &Face) -> rune::El {
        let m = &face.metrics;
        let s = face.scale;
        let x = (i32::from(self.col) * m.cell_width as i32 + self.glyph.x) as f32 / s;
        let y = (i32::from(row) * m.cell_height as i32 + self.glyph.top) as f32 / s;
        rune::el()
            .absolute(x, y)
            .size(self.glyph.width as f32 / s, self.glyph.height as f32 / s)
            .pixels(&self.pixels)
            .a11y_hidden()
            .inert()
    }
}

type Key = (super::font::FaceKey, String, bool, u32);

thread_local! {
    static GLYPHS: RefCell<HashMap<Key, Option<Rc<GlyphImage>>>> = RefCell::new(HashMap::new());
}

/// The glyph for a cell holding `text`, `wide` or not, with `constraint_width` cells free, if
/// the atlas draws it.
pub fn render(
    face: &Face,
    text: &str,
    wide: bool,
    constraint_width: u32,
) -> Option<Rc<GlyphImage>> {
    let key = (face.key(), text.to_owned(), wide, constraint_width);
    if let Some(hit) = GLYPHS.with(|g| g.borrow().get(&key).cloned()) {
        return hit;
    }
    let made = rasterize(face, text, constraint_width).map(Rc::new);
    GLYPHS.with(|g| g.borrow_mut().insert(key, made.clone()));
    made
}

#[cfg(target_os = "macos")]
fn rasterize(face: &Face, text: &str, constraint_width: u32) -> Option<GlyphImage> {
    let (c, presentation) = single(text)?;
    let r = coretext::render(face, c, presentation, constraint_width)?;
    Some(GlyphImage {
        x: r.x,
        top: r.top,
        width: r.width,
        height: r.height,
        color: r.color,
        data: r.data,
        tinted: RefCell::new(HashMap::new()),
    })
}

#[cfg(not(target_os = "macos"))]
fn rasterize(_: &Face, _: &str, _: u32) -> Option<GlyphImage> {
    None
}

/// What presentation a cell asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation {
    /// Its own: emoji if it has the Unicode `Emoji_Presentation` property, text otherwise.
    Default,
    /// VS15.
    Text,
    /// VS16.
    Emoji,
}

/// Whether `c` is emoji by default (the Unicode `Emoji_Presentation` property).
pub fn is_emoji_presentation(c: char) -> bool {
    let cp = c as u32;
    let at = tables::EMOJI_PRESENTATION.partition_point(|&(_, hi)| hi < cp);
    tables::EMOJI_PRESENTATION
        .get(at)
        .is_some_and(|&(lo, _)| lo <= cp)
}

/// The one character a cell holds, with a variation selector read as its presentation.
fn single(text: &str) -> Option<(char, Presentation)> {
    let mut chars = text.chars();
    let c = chars.next()?;
    let presentation = match (chars.next(), chars.next()) {
        (None, _) => Presentation::Default,
        (Some('\u{FE0E}'), None) => Presentation::Text,
        (Some('\u{FE0F}'), None) => Presentation::Emoji,
        _ => return None,
    };
    Some((c, presentation))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_variation_selector_sets_the_presentation_and_other_clusters_are_left_to_rune() {
        assert_eq!(single("a"), Some(('a', Presentation::Default)));
        assert_eq!(single("☺\u{FE0F}"), Some(('☺', Presentation::Emoji)));
        assert_eq!(single("☺\u{FE0E}"), Some(('☺', Presentation::Text)));
        assert_eq!(single("e\u{301}"), None);
        assert_eq!(single("👍🏽"), None);
    }

    #[test]
    fn emoji_presentation_is_unicodes() {
        assert!(is_emoji_presentation('🙂') && is_emoji_presentation('⌚'));
        // Wide, and in Apple Color Emoji, but text by default.
        assert!(!is_emoji_presentation('〰') && !is_emoji_presentation('㊙'));
        assert!(!is_emoji_presentation('☺') && !is_emoji_presentation('日'));
    }
}
