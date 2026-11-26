//! The cursor, drawn over the grid in Ghostty's shapes (`ghostty:src/font/sprite/draw/special.zig`
//! `cursor_*`), with the glyph under a block cursor drawn again in the cursor-text color.

use rune::{El, Shape, Transition, canvas, el, text};
use terminal::{CursorStyle, Snapshot, Width};

use super::font::Face;
use super::look::{Look, color};
use super::sprite::{self, PxRect, alpha_box_pt, rect_pt};

/// What the cursor looks like this frame: its cell, how many columns it covers, and its shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorPlace {
    pub col: u16,
    pub row: u16,
    /// 2 on a wide character.
    pub width: u16,
    pub style: CursorStyle,
}

/// Where the cursor draws, if it shows: the program's style while the pane is focused, a hollow
/// block otherwise, as Ghostty draws an unfocused surface's cursor. On the spacer half of a wide
/// character it covers the whole character.
pub fn place(snapshot: &Snapshot, focused: bool) -> Option<CursorPlace> {
    let cursor = &snapshot.cursor;
    let (mut col, row) = cursor.position?;
    if !cursor.visible {
        return None;
    }
    if cursor.wide_tail {
        col = col.saturating_sub(1);
    }
    let wide = snapshot
        .screen
        .get(usize::from(row))
        .and_then(|r| r.cells.get(usize::from(col)))
        .is_some_and(|c| c.width == Width::Wide);
    Some(CursorPlace {
        col,
        row,
        width: if wide { 2 } else { 1 },
        style: if focused {
            cursor.style
        } else {
            CursorStyle::BlockHollow
        },
    })
}

/// The cursor's boxes in device pixels from its cell's top left.
pub fn boxes(place: &CursorPlace, face: &Face) -> Vec<PxRect> {
    let m = &face.metrics;
    let width = m.cell_width * u32::from(place.width);
    let height = m.cell_height;
    let thickness = m.cursor_thickness;
    match place.style {
        CursorStyle::Block => vec![PxRect {
            x0: 0,
            y0: 0,
            x1: width,
            y1: height,
        }],
        CursorStyle::BlockHollow => {
            let inner_right = width.saturating_sub(thickness);
            let inner_bottom = height.saturating_sub(thickness);
            vec![
                PxRect {
                    x0: 0,
                    y0: 0,
                    x1: width,
                    y1: thickness,
                },
                PxRect {
                    x0: 0,
                    y0: inner_bottom,
                    x1: width,
                    y1: height,
                },
                PxRect {
                    x0: 0,
                    y0: thickness,
                    x1: thickness,
                    y1: inner_bottom,
                },
                PxRect {
                    x0: inner_right,
                    y0: thickness,
                    x1: width,
                    y1: inner_bottom,
                },
            ]
        }
        // Ghostty centers the bar on the cell's left edge, half its thickness (rounded up) to the
        // left. The overlay can't draw left of its cell, so it starts at the edge.
        CursorStyle::Bar => vec![PxRect {
            x0: 0,
            y0: 0,
            x1: thickness,
            y1: height,
        }],
        CursorStyle::Underline => {
            let below = height + height / 4;
            let y = m
                .underline_position
                .min(below.saturating_sub(m.underline_thickness));
            vec![PxRect {
                x0: 0,
                y0: y,
                x1: width,
                y1: y + thickness,
            }]
        }
    }
}

/// The cursor overlay, positioned in the grid's points, or `None` while it's hidden.
pub fn overlay(snapshot: &Snapshot, focused: bool, face: &Face, look: &Look) -> Option<El> {
    let place = place(snapshot, focused)?;
    let s = face.scale;
    let (cell_w, cell_h) = face.cell_pt();
    let colors = &snapshot.colors;
    let cursor = color(colors.cursor.or(look.cursor).unwrap_or(colors.foreground));
    let mut shape = Shape::new();
    for b in boxes(&place, face) {
        shape = shape.fill(rect_pt(b, s), cursor);
    }
    let x = f32::from(place.col) * cell_w;
    let y = f32::from(place.row) * cell_h;
    let w = f32::from(place.width) * cell_w;
    let mut cursor_el = el()
        .key("cursor")
        .absolute(x, y)
        .size(w, cell_h)
        .a11y_hidden()
        .inert()
        .instant_changes()
        .transition(Transition::none())
        .child(
            canvas(shape)
                .key("cursor-shape")
                .absolute(0.0, 0.0)
                .size(w, cell_h),
        );
    if place.style == CursorStyle::Block {
        cursor_el = cursor_el.children(under_block(snapshot, &place, face, look));
    }
    Some(cursor_el)
}

/// The character under a block cursor, in the cursor-text color: the block covers the grid's
/// own glyph.
fn under_block(snapshot: &Snapshot, place: &CursorPlace, face: &Face, look: &Look) -> Option<El> {
    let row = snapshot.screen.get(usize::from(place.row))?;
    let cell = row.cells.get(usize::from(place.col))?;
    let text_str = cell.text.as_str();
    if text_str.is_empty() || text_str == " " || cell.flags.contains(terminal::CellFlags::INVISIBLE)
    {
        return None;
    }
    let colors = &snapshot.colors;
    let ink = color(look.cursor_text.unwrap_or(colors.background));
    let (cell_w, cell_h) = face.cell_pt();
    let w = f32::from(place.width) * cell_w;
    let mut chars = text_str.chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && let Some(glyph) = sprite::glyph(c, place.width as u8, &face.metrics)
    {
        let mut shape = Shape::new();
        for &b in &glyph.boxes {
            let ink = ink.alpha(f32::from(b.alpha) / 255.0);
            shape = shape.fill(alpha_box_pt(b, 0, face.scale), ink);
        }
        return Some(
            canvas(shape)
                .key("cursor-text")
                .absolute(0.0, 0.0)
                .size(w, cell_h),
        );
    }
    let mut style = face.style();
    style.tracking = 0.0;
    if cell.flags.contains(terminal::CellFlags::BOLD) {
        style.weight = 700.0;
    }
    style.italic = cell.flags.contains(terminal::CellFlags::ITALIC);
    Some(
        el().key("cursor-text")
            .absolute(0.0, 0.0)
            .size(w, cell_h)
            .child(text(text_str.to_owned()).font(style).color(ink)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;
    use crate::view::terminal::font::Font;
    use crate::view::terminal::look::arctic;

    fn screen(bytes: &str) -> Snapshot {
        fixtures::replay(bytes.as_bytes(), 20, 2, &arctic().0)
    }

    #[test]
    fn the_programs_style_while_focused_a_hollow_block_otherwise() {
        let bar = screen("ab\x1b[6 q");
        let focused = place(&bar, true).unwrap();
        assert_eq!(
            (focused.col, focused.row, focused.style),
            (2, 0, CursorStyle::Bar)
        );
        assert_eq!(place(&bar, false).unwrap().style, CursorStyle::BlockHollow);
        assert_eq!(place(&screen("ab\x1b[?25l"), true), None, "hidden");
    }

    #[test]
    fn on_a_wide_character_it_covers_both_cells() {
        let wide = screen("a日\x1b[2G");
        assert_eq!(place(&wide, true).unwrap().width, 2);
        let tail = screen("a日\x1b[3G");
        let place = place(&tail, true).unwrap();
        assert_eq!((place.col, place.width), (1, 2), "from the spacer half too");
    }

    #[test]
    fn a_block_shows_the_character_under_it_unless_its_invisible() {
        let face = Font::jetbrains_mono(14.0).at(2.0).unwrap();
        let look = arctic().1;
        let shown = screen("ab\x1b[1G");
        let place = place(&shown, true).unwrap();
        assert!(under_block(&shown, &place, &face, &look).is_some());
        let hidden = screen("\x1b[8mab\x1b[1G");
        assert!(under_block(&hidden, &place, &face, &look).is_none());
    }

    #[test]
    fn shapes_in_whole_pixels() {
        let face = Font::jetbrains_mono(14.0).at(2.0).unwrap();
        let m = face.metrics;
        let at = |style| CursorPlace {
            col: 0,
            row: 0,
            width: 1,
            style,
        };
        let full = PxRect {
            x0: 0,
            y0: 0,
            x1: m.cell_width,
            y1: m.cell_height,
        };
        assert_eq!(boxes(&at(CursorStyle::Block), &face), [full]);
        assert_eq!(
            boxes(&at(CursorStyle::Bar), &face)[0].x1,
            m.cursor_thickness
        );
        let hollow = boxes(&at(CursorStyle::BlockHollow), &face);
        assert_eq!(hollow.len(), 4);
        assert!(hollow.iter().all(|b| b.x1 <= full.x1 && b.y1 <= full.y1));
        let under = boxes(&at(CursorStyle::Underline), &face)[0];
        assert_eq!(
            (under.y0, under.y1 - under.y0),
            (m.underline_position, m.cursor_thickness)
        );
    }
}
