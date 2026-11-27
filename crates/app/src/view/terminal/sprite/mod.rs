//! Ghostty's sprite face: the characters and decorations Ghostty draws itself at the cell's exact
//! pixels rather than taking from a font (`ghostty:src/font/sprite/Face.zig` and
//! `sprite/draw/*.zig` at `v1.4.0`).
//!
//! Each sprite is drawn once per (code point, cell) on [`canvas::Canvas`], with the cell's size
//! plus a quarter cell of padding, through a port of the z2d rasterizer Ghostty uses
//! ([`raster`]). The pixels come out as boxes of one alpha each, which the row's vector layer
//! fills at whole device pixels in the cell's color. Rune blends them in sRGB-encoded space, as
//! Ghostty does on macOS (`alpha-blending = native`), so a partly covered pixel gets the same
//! color in both.
//!
//! Ported: Box Drawing (U+2500 to U+257F), Block Elements (U+2580 to U+259F), Braille
//! (U+2800 to U+28FF), the geometric shapes Ghostty draws (U+25E2 to U+25E5, U+25F8 to U+25FA,
//! U+25FF), Powerline (U+E0B0 to U+E0BF, U+E0D2, U+E0D4), and the five underline styles,
//! strikethrough and overline. Ghostty also draws Symbols for Legacy Computing (U+1FB00 to
//! U+1FBEF and its supplement) and branch drawing (U+F5D0 to U+F60D); those still come from the
//! font ([`ghostty_draws`] tells them apart).

mod blocks;
mod box_drawing;
pub mod canvas;
mod powerline;
pub mod raster;
mod special;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use terminal::{Metrics, Underline};

use self::canvas::Canvas;
pub use self::canvas::{AlphaBox, Sprite};

/// A filled box in a cell, in device pixels from the cell's top left: `x0..x1` by `y0..y1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PxRect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

/// A box as a polygon in points, at display scale `s`.
pub fn rect_pt(b: PxRect, s: f32) -> Vec<[f32; 2]> {
    let (x0, y0, x1, y1) = (
        b.x0 as f32 / s,
        b.y0 as f32 / s,
        b.x1 as f32 / s,
        b.y1 as f32 / s,
    );
    vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

/// A sprite's box as a polygon in points, `dx` device pixels right of the cell's left, at display
/// scale `s`.
pub fn alpha_box_pt(b: AlphaBox, dx: i32, s: f32) -> Vec<[f32; 2]> {
    let (x0, y0, x1, y1) = (
        (b.x0 + dx) as f32 / s,
        b.y0 as f32 / s,
        (b.x1 + dx) as f32 / s,
        b.y1 as f32 / s,
    );
    vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

/// What a sprite is drawn for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A character, in a cell one or two columns wide.
    Char(u32, u8),
    Underline(Underline),
    Strikethrough,
    Overline,
}

/// Whether this port draws `c` as a sprite.
pub fn drawn(c: char) -> bool {
    matches!(c as u32,
        0x2500..=0x259F | 0x2800..=0x28FF | 0x25E2..=0x25E5 | 0x25F8..=0x25FA | 0x25FF
        | 0xE0B0..=0xE0BF | 0xE0D2 | 0xE0D4)
}

/// Whether Ghostty draws `c` as a sprite (`Face.zig`'s ranges), ported here or not.
pub fn ghostty_draws(c: char) -> bool {
    drawn(c)
        || matches!(c as u32,
            0x1FB00..=0x1FBEF | 0x1CC1B..=0x1CEAF | 0xF5D0..=0xF60D)
}

/// The cell metrics a sprite depends on.
type MetricsKey = [u32; 10];

fn metrics_key(m: &Metrics) -> MetricsKey {
    [
        m.cell_width,
        m.cell_height,
        m.box_thickness,
        m.underline_position,
        m.underline_thickness,
        m.strikethrough_position,
        m.strikethrough_thickness,
        m.overline_position as u32,
        m.overline_thickness,
        m.cursor_thickness,
    ]
}

thread_local! {
    /// Sprites drawn so far, for every face and size this thread drew.
    static SPRITES: RefCell<HashMap<(Kind, MetricsKey), Rc<Sprite>>> = RefCell::new(HashMap::new());
}

/// Character `c`'s sprite in a cell `cols` columns wide, if it's one this port draws.
pub fn glyph(c: char, cols: u8, m: &Metrics) -> Option<Rc<Sprite>> {
    drawn(c).then(|| get(Kind::Char(c as u32, cols.max(1)), m))
}

/// The underline, strikethrough or overline sprite for one cell.
pub fn decoration(kind: Kind, m: &Metrics) -> Rc<Sprite> {
    get(kind, m)
}

fn get(kind: Kind, m: &Metrics) -> Rc<Sprite> {
    let key = (kind, metrics_key(m));
    if let Some(s) = SPRITES.with(|c| c.borrow().get(&key).cloned()) {
        return s;
    }
    let sprite = Rc::new(draw(kind, m));
    SPRITES.with(|c| c.borrow_mut().insert(key, sprite.clone()));
    sprite
}

/// Draws a sprite (`Face.renderGlyph`): on a canvas as wide as the cell's columns and one cell
/// tall.
fn draw(kind: Kind, m: &Metrics) -> Sprite {
    let cols = match kind {
        Kind::Char(_, cols) => u32::from(cols),
        _ => 1,
    };
    let (width, height) = (m.cell_width * cols, m.cell_height);
    let mut canvas = Canvas::new(width, height);
    match kind {
        Kind::Char(cp, _) => match cp {
            0x2500..=0x257F => box_drawing::draw(cp, &mut canvas, m),
            0x2580..=0x259F => blocks::draw_block(cp, &mut canvas, m),
            0x2800..=0x28FF => blocks::draw_braille(cp, &mut canvas, width, height),
            0x25E2..=0x25FF => blocks::draw_geometric(cp, &mut canvas, m),
            0xE0B0..=0xE0D4 => powerline::draw(cp, &mut canvas, width, height, m),
            _ => {}
        },
        Kind::Underline(style) => special::underline(style, &mut canvas, width, height, m),
        Kind::Strikethrough => special::strikethrough(&mut canvas, width, m),
        Kind::Overline => special::overline(&mut canvas, width, m),
    }
    canvas.sprite()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// JetBrains Mono's metrics at 14 pt and 2x, with the cell and line thickness set.
    pub(crate) fn metrics(width: u32, height: u32, box_thickness: u32) -> Metrics {
        let mut m = Metrics::calc(&terminal::FaceMetrics {
            px_per_em: 28.0,
            cell_width: 16.8,
            ascent: 28.56,
            descent: -8.4,
            ..Default::default()
        });
        (m.cell_width, m.cell_height, m.box_thickness) = (width, height, box_thickness);
        m
    }

    #[test]
    fn every_drawn_character_draws_something() {
        let m = metrics(17, 37, 2);
        let ranges = [
            0x2500..=0x259F,
            0x2801..=0x28FF,
            0x25E2..=0x25E5,
            0x25F8..=0x25FA,
            0x25FF..=0x25FF,
            0xE0B0..=0xE0BF,
            0xE0D2..=0xE0D2,
            0xE0D4..=0xE0D4,
        ];
        for cp in ranges.into_iter().flatten() {
            let c = char::from_u32(cp).unwrap();
            assert!(drawn(c), "U+{cp:04X}");
            let s = glyph(c, 1, &m).unwrap();
            assert!(!s.boxes.is_empty(), "U+{cp:04X} draws nothing");
        }
        assert!(glyph('a', 1, &m).is_none());
        assert!(ghostty_draws('\u{1FB00}') && !drawn('\u{1FB00}'));
    }

    #[test]
    fn a_sprite_is_drawn_once_per_cell() {
        let m = metrics(17, 37, 2);
        let a = glyph('╭', 1, &m).unwrap();
        assert!(Rc::ptr_eq(&a, &glyph('╭', 1, &m).unwrap()));
        let other = metrics(10, 22, 1);
        assert!(!Rc::ptr_eq(&a, &glyph('╭', 1, &other).unwrap()));
    }

    #[test]
    fn an_arc_meets_the_lines_beside_it() {
        // ╭ ends where ─ and │ run: its stem in │'s columns at the bottom, its arm in ─'s rows at
        // the right.
        let m = metrics(17, 37, 2);
        let arc = glyph('╭', 1, &m).unwrap();
        assert!(arc.covers(7, 36) && arc.covers(8, 36) && !arc.covers(6, 36) && !arc.covers(9, 36));
        assert!(arc.covers(16, 17) && arc.covers(16, 18) && !arc.covers(16, 16));
        assert!(
            arc.boxes.iter().any(|b| b.alpha < 255),
            "the bend is antialiased"
        );
    }
}
