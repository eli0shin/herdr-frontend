//! Terminal rows as `rune::GridRow`s: the port of Ghostty's `rebuildRow` color and run rules onto
//! Rune's grid, and the cache that hands an unchanged row back as the same `Rc`.
//!
//! A row becomes (`docs/research/terminal-rendering.md` §2):
//!
//! - its narrow cells' text as one line, with a span wherever the color, weight or slant
//!   changes;
//! - a fill for each run of cells with the same non-default background, selection included;
//! - pinned cells for what mustn't move the columns after it: wide characters, clusters of
//!   several code points, and characters the regular face has no glyph for;
//! - boxes in the vector layer for underlines, strikethrough, overline and box-drawing lines, at
//!   Ghostty's whole-pixel positions. A box-drawing character also stays in the text, drawn
//!   transparent, so the row reads as the terminal's text.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use rune::{Color, GridRow, Shape, Span, TextStyle};
use terminal::{CellFlags, Colors, Rgb, Row, RowId, Underline, Width};

use super::font::Face;
use super::look::{Look, color};
use super::sprite::{self, PxRect};

/// A text weight Ghostty's bold face draws at.
const BOLD: f32 = 700.0;

/// One row, built: what [`BuiltRow::grid_row`] hands Rune, kept apart so tests can read it.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltRow {
    /// One character per column (a space for empty, pinned and sprite cells), trailing spaces
    /// trimmed.
    pub text: String,
    pub spans: Vec<Span>,
    /// Background runs: columns `start..end` and their color.
    pub fills: Vec<(u16, u16, Color)>,
    pub cells: Vec<Pinned>,
    /// Lines and sprites over the text, in device pixels from the row's top left.
    pub boxes: Vec<(PxRect, Color)>,
    pub wraps: bool,
}

/// A cell drawn on its own at its column.
#[derive(Clone, Debug, PartialEq)]
pub struct Pinned {
    pub col: u16,
    pub width: u16,
    pub text: String,
    pub style: TextStyle,
    pub color: Color,
}

/// The colors and style one cell draws with.
#[derive(Clone, Copy, PartialEq)]
struct Ink {
    fg: Color,
    weight: f32,
    italic: bool,
}

/// The resolved colors of a cell: inverse and selection applied.
pub(super) struct CellColors {
    pub fg: Rgb,
    /// `None` for the terminal's default background, which the view fills under the grid.
    pub bg: Option<Rgb>,
}

/// A cell's colors as Ghostty resolves them in `rebuildRow`: inverse swaps foreground and
/// background (defaults included). A selected cell takes the selection colors, or with no
/// selection background the terminal's default foreground behind its default background.
pub(super) fn cell_colors(
    cell: &terminal::Cell,
    selected: bool,
    colors: &Colors,
    look: &Look,
) -> CellColors {
    let mut fg = cell.fg.unwrap_or(colors.foreground);
    let mut bg = cell.bg;
    if cell.flags.contains(CellFlags::INVERSE) {
        let swapped = bg.unwrap_or(colors.background);
        bg = Some(fg);
        fg = swapped;
    }
    if selected {
        match look.selection_background {
            Some(selection) => {
                bg = Some(selection);
                fg = look.selection_foreground.unwrap_or(fg);
            }
            None => {
                bg = Some(colors.foreground);
                fg = look.selection_foreground.unwrap_or(colors.background);
            }
        }
    }
    CellColors { fg, bg }
}

/// Builds one row.
pub fn build(row: &Row, colors: &Colors, face: &Face, look: &Look) -> BuiltRow {
    let style = face.style();
    let metrics = &face.metrics;
    let cell_w = metrics.cell_width;
    let mut built = BuiltRow {
        text: String::with_capacity(row.cells.len()),
        spans: Vec::new(),
        fills: Vec::new(),
        cells: Vec::new(),
        boxes: Vec::new(),
        wraps: row.wrapped,
    };
    let mut fill: Option<(u16, u16, Rgb)> = None;
    let mut ink: Option<(usize, Ink)> = None;

    for (x, cell) in row.cells.iter().enumerate() {
        let col = x as u16;
        // A wide character's spacer is selected with it: the render state can end a selection
        // on the character's first cell (Ghostty's renderer compares spacers one to the left).
        let at = if cell.width == Width::SpacerTail {
            col.saturating_sub(1)
        } else {
            col
        };
        let selected = row
            .selection
            .is_some_and(|(start, end)| (start..=end).contains(&at));
        let resolved = cell_colors(cell, selected, colors, look);

        // Background runs, merged while the color holds.
        match (&mut fill, resolved.bg) {
            (Some((_, end, run)), Some(bg)) if *run == bg && *end == col => *end = col + 1,
            (_, bg) => {
                if let Some((start, end, run)) = fill.take() {
                    built.fills.push((start, end, color(run)));
                }
                fill = bg.map(|bg| (col, col + 1, bg));
            }
        }

        let span_cols = match cell.width {
            Width::Wide => 2,
            _ => 1,
        };
        let mut fg = color(resolved.fg);
        if cell.flags.contains(CellFlags::FAINT) {
            fg = fg.alpha(look.faint_opacity);
        }
        let x0 = u32::from(col) * cell_w;
        let x1 = x0 + span_cols * cell_w;
        // A wide character's lines span both its cells; its spacer draws none of its own.
        // Invisible text draws no lines either, as in xterm and Ghostty.
        if matches!(cell.width, Width::Narrow | Width::Wide)
            && !cell.flags.contains(CellFlags::INVISIBLE)
        {
            decorations(cell, fg, x0, x1, face, &mut built.boxes);
        }

        let text = cell.text.as_str();
        let drawn = !text.is_empty()
            && text != " "
            && !cell.flags.contains(CellFlags::INVISIBLE)
            && matches!(cell.width, Width::Narrow | Width::Wide);
        let mut chars = text.chars();
        let single = match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        };

        // A box-drawing line draws as Ghostty's sprite, in the vector layer. Its character
        // stays in the row, drawn transparent, so a screen reader still reads it.
        let mut ink_fg = fg;
        if drawn
            && let Some(c) = single
            && let Some(boxes) = sprite::lines(c, metrics)
        {
            built.boxes.extend(boxes.into_iter().map(|b| {
                let shifted = PxRect {
                    x0: b.x0 + x0,
                    x1: b.x1 + x0,
                    ..b
                };
                (shifted, fg)
            }));
            ink_fg = Color::TRANSPARENT;
        }

        let weight = if cell.flags.contains(CellFlags::BOLD) {
            BOLD
        } else {
            style.weight
        };
        let italic = cell.flags.contains(CellFlags::ITALIC);
        let on_line =
            drawn && cell.width == Width::Narrow && single.is_some_and(|c| face.covers(c));
        if drawn && !on_line {
            let mut cell_style = style;
            cell_style.weight = weight;
            cell_style.italic = italic;
            // A pinned cell is shaped on its own: it needs no tracking to stay on its column.
            cell_style.tracking = 0.0;
            built.cells.push(Pinned {
                col,
                width: span_cols as u16,
                text: text.to_owned(),
                style: cell_style,
                color: ink_fg,
            });
        }
        if !on_line {
            built.text.push(' ');
            continue;
        }

        let start = built.text.len();
        built.text.push_str(text);
        let this = Ink {
            fg: ink_fg,
            weight,
            italic,
        };
        match &mut ink {
            Some((_, run)) if *run == this => {}
            _ => {
                if let Some((from, run)) = ink.take() {
                    built.spans.push(span(from, start, run, &style));
                }
                ink = Some((start, this));
            }
        }
    }
    if let Some((start, end, run)) = fill {
        built.fills.push((start, end, color(run)));
    }
    let trimmed = built.text.trim_end_matches(' ').len();
    if let Some((from, run)) = ink
        && from < trimmed
    {
        built.spans.push(span(from, trimmed, run, &style));
    }
    built.text.truncate(trimmed);
    // Spans run to the next one's start; none may reach past the trimmed text.
    for s in &mut built.spans {
        s.end = s.end.min(trimmed as u32);
    }
    built.spans.retain(|s| s.end > s.start);
    built
}

fn span(start: usize, end: usize, ink: Ink, style: &TextStyle) -> Span {
    let mut span = Span::new(start, end).color(ink.fg);
    if ink.weight != style.weight {
        span = span.weight(ink.weight);
    }
    if ink.italic {
        span = span.italic();
    }
    span
}

/// Underline, strikethrough and overline as boxes at Ghostty's positions
/// (`ghostty:src/font/sprite/draw/special.zig`). Curly, dotted and dashed underlines draw as
/// single ones until the fidelity ticket ports their sprites.
fn decorations(
    cell: &terminal::Cell,
    fg: Color,
    x0: u32,
    x1: u32,
    face: &Face,
    boxes: &mut Vec<(PxRect, Color)>,
) {
    let m = &face.metrics;
    // The sprite canvas reaches a quarter cell past the cell's top and bottom.
    let below = m.cell_height + m.cell_height / 4;
    let mut line = |y0: u32, thickness: u32, color: Color| {
        boxes.push((
            PxRect {
                x0,
                y0,
                x1,
                y1: y0 + thickness,
            },
            color,
        ));
    };
    if cell.underline != Underline::None {
        let color = cell.underline_color.map_or(fg, |c| {
            let c = super::look::color(c);
            c.alpha(fg.a)
        });
        let thickness = m.underline_thickness;
        match cell.underline {
            Underline::Double => {
                let y = m
                    .underline_position
                    .min(below.saturating_sub(2 * thickness));
                line(y.saturating_sub(thickness), thickness, color);
                line(y + thickness, thickness, color);
            }
            _ => {
                let y = m.underline_position.min(below.saturating_sub(thickness));
                line(y, thickness, color);
            }
        }
    }
    if cell.flags.contains(CellFlags::STRIKETHROUGH) {
        line(m.strikethrough_position, m.strikethrough_thickness, fg);
    }
    if cell.flags.contains(CellFlags::OVERLINE) {
        // The row's vector layer can't reach above the row, so an overline above the cell sits
        // at its top edge.
        line(m.overline_position.max(0) as u32, m.overline_thickness, fg);
    }
}

impl BuiltRow {
    /// The row as Rune's grid draws it.
    pub fn grid_row(&self, face: &Face) -> GridRow {
        let mut row =
            GridRow::new(self.text.clone(), self.spans.clone(), face.style()).wraps(self.wraps);
        for &(start, end, color) in &self.fills {
            row = row.fill(start, end, color);
        }
        for cell in &self.cells {
            row = row.cell(
                cell.col,
                cell.width,
                cell.text.clone(),
                cell.style,
                cell.color,
            );
        }
        let mut shape = Shape::new();
        for &(b, color) in &self.boxes {
            shape = shape.fill(sprite::rect_pt(b, face.scale), color);
        }
        row.shape(shape)
    }
}

/// The rows a view built last frame, by row id: a row whose snapshot `Arc` is the same one
/// comes back as the same `Rc<GridRow>`, so Rune neither shapes nor paints it again.
#[derive(Default)]
pub struct RowCache {
    /// What every cached row was built with: the face, the look and the default colors.
    key: Option<CacheKey>,
    rows: HashMap<RowId, (Arc<Row>, Rc<GridRow>)>,
    /// Rows built (not reused) by the last [`RowCache::rows`] call.
    pub built: usize,
}

#[derive(Clone, PartialEq)]
struct CacheKey {
    face: (usize, u32, u32),
    look: [u32; 5],
    foreground: Rgb,
    background: Rgb,
}

impl RowCache {
    /// The grid rows for `screen`, reusing every row that didn't change.
    pub fn rows(
        &mut self,
        screen: &[Arc<Row>],
        colors: &Colors,
        face: &Face,
        look: &Look,
    ) -> Vec<Rc<GridRow>> {
        let key = CacheKey {
            face: face.key(),
            look: look.key(),
            foreground: colors.foreground,
            background: colors.background,
        };
        if self.key.as_ref() != Some(&key) {
            self.rows.clear();
            self.key = Some(key);
        }
        let mut kept = HashMap::with_capacity(screen.len());
        self.built = 0;
        let rows = screen
            .iter()
            .map(|row| {
                let grid_row = match self.rows.remove(&row.id) {
                    Some((was, grid_row)) if Arc::ptr_eq(&was, row) => grid_row,
                    _ => {
                        self.built += 1;
                        Rc::new(build(row, colors, face, look).grid_row(face))
                    }
                };
                kept.insert(row.id, (row.clone(), grid_row.clone()));
                grid_row
            })
            .collect();
        self.rows = kept;
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;
    use crate::view::terminal::font::Font;
    use crate::view::terminal::look::arctic;
    use terminal::Snapshot;

    fn face() -> Face {
        Font::jetbrains_mono(14.0).at(2.0).unwrap()
    }

    fn screen(bytes: &str) -> Snapshot {
        fixtures::replay(bytes.as_bytes(), 20, 2, &arctic().0)
    }

    /// Row 0 of `bytes` on a 20 × 2 terminal, built under arctic.
    fn built(bytes: &str) -> BuiltRow {
        let snapshot = screen(bytes);
        build(&snapshot.screen[0], &snapshot.colors, &face(), &arctic().1)
    }

    fn rgb(v: u32) -> Color {
        color(Rgb::new((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    const FG: u32 = 0xd4d4d4;
    const BG: u32 = 0x1e1e1e;

    #[test]
    fn a_span_per_change_of_color_weight_or_slant() {
        let row = built("ab\x1b[31mcd\x1b[1mef\x1b[0;3mgh\x1b[0m ij");
        assert_eq!(row.text, "abcdefgh ij");
        let spans: Vec<_> = row
            .spans
            .iter()
            .map(|s| (s.start, s.end, s.color, s.weight, s.italic))
            .collect();
        let red = rgb(0xf44747);
        assert_eq!(
            spans,
            [
                (0, 2, Some(rgb(FG)), None, false),
                (2, 4, Some(red), None, false),
                (4, 6, Some(red), Some(BOLD), false),
                // Spaces don't break a run, so the italic one runs to the next change.
                (6, 9, Some(rgb(FG)), None, true),
                (9, 11, Some(rgb(FG)), None, false),
            ]
        );
        assert!(row.fills.is_empty(), "the default background isn't filled");
    }

    #[test]
    fn backgrounds_merge_into_runs() {
        let row = built("\x1b[41mab\x1b[42mc\x1b[0m d\x1b[41me");
        assert_eq!(
            row.fills,
            [
                (0, 2, rgb(0xf44747)),
                (2, 3, rgb(0x6a9955)),
                (5, 6, rgb(0xf44747))
            ]
        );
    }

    #[test]
    fn inverse_swaps_the_defaults_too() {
        let row = built("\x1b[7mx\x1b[0m");
        assert_eq!(row.fills, [(0, 1, rgb(FG))]);
        assert_eq!(row.spans[0].color, Some(rgb(BG)));
    }

    #[test]
    fn faint_text_is_half_opaque_and_invisible_text_isnt_drawn() {
        let row = built("\x1b[2mab\x1b[0;8mcd\x1b[0mef");
        assert_eq!(row.text, "ab  ef");
        assert_eq!(row.spans[0].color, Some(rgb(FG).alpha(0.5)));
    }

    #[test]
    fn selection_takes_the_selection_background_and_keeps_the_ink() {
        let mut snapshot = screen("\x1b[31mabcdef");
        fixtures::select(&mut snapshot, 0..=0, 1, 3);
        let row = build(&snapshot.screen[0], &snapshot.colors, &face(), &arctic().1);
        assert_eq!(row.fills, [(1, 4, rgb(0x04395e))]);
        assert_eq!(row.spans.len(), 1, "the ink didn't change");

        // With no selection background, Ghostty draws the default background on the default
        // foreground, whatever the cell's own colors.
        let row = build(
            &snapshot.screen[0],
            &snapshot.colors,
            &face(),
            &Look::default(),
        );
        assert_eq!(row.fills, [(1, 4, rgb(FG))]);
        assert_eq!(row.spans[1].color, Some(rgb(BG)));
    }

    #[test]
    fn a_selection_ending_on_a_wide_character_covers_all_of_it() {
        let mut snapshot = screen("a日b");
        fixtures::select(&mut snapshot, 0..=0, 0, 1);
        let row = build(&snapshot.screen[0], &snapshot.colors, &face(), &arctic().1);
        assert_eq!(row.fills, [(0, 3, rgb(0x04395e))]);
    }

    #[test]
    fn invisible_text_draws_no_lines() {
        let row = built("\x1b[8;4;9;53mhidden\x1b[0m");
        assert!(row.text.is_empty());
        assert!(row.boxes.is_empty(), "{:?}", row.boxes);
    }

    #[test]
    fn wide_fallback_and_combined_cells_are_pinned() {
        let row = built("a日b\u{a66e}e\u{301}z");
        // a, then 日 over two columns, b, a letter JetBrains Mono lacks, e + accent, z.
        assert_eq!(row.text, "a  b  z");
        let pinned: Vec<_> = row
            .cells
            .iter()
            .map(|c| (c.col, c.width, c.text.as_str()))
            .collect();
        assert_eq!(
            pinned,
            [(1, 2, "日"), (4, 1, "\u{a66e}"), (5, 1, "e\u{301}")]
        );
        assert!(row.cells.iter().all(|c| c.style.tracking == 0.0));
    }

    #[test]
    fn box_lines_draw_as_sprites_and_still_read() {
        let face = face();
        let row = built("│─x");
        assert_eq!(row.text, "│─x");
        assert_eq!(row.spans[0].color, Some(Color::TRANSPARENT));
        assert_eq!(row.spans[1].color, Some(rgb(FG)));
        // │ centered in column 0, ─ across column 1, at Ghostty's 2 px.
        let cell_w = face.metrics.cell_width;
        assert!(row.boxes.iter().all(|(_, c)| *c == rgb(FG)));
        assert!(
            row.boxes
                .iter()
                .any(|(b, _)| b.x0 == 7 && b.x1 == 9 && b.y0 == 0)
        );
        assert!(
            row.boxes
                .iter()
                .any(|(b, _)| b.x0 == cell_w && b.y1 - b.y0 == 2)
        );
    }

    #[test]
    fn decorations_sit_at_ghosttys_positions() {
        let face = face();
        let m = face.metrics;
        let row = built("\x1b[4ma\x1b[0;9mb\x1b[0;4:2mc\x1b[0;53md");
        let at = |col: u32| -> Vec<(u32, u32)> {
            row.boxes
                .iter()
                .filter(|(b, _)| b.x0 == col * m.cell_width)
                .map(|(b, _)| (b.y0, b.y1 - b.y0))
                .collect()
        };
        assert_eq!(at(0), [(m.underline_position, m.underline_thickness)]);
        assert_eq!(
            at(1),
            [(m.strikethrough_position, m.strikethrough_thickness)]
        );
        let t = m.underline_thickness;
        assert_eq!(
            at(2),
            [(m.underline_position - t, t), (m.underline_position + t, t)]
        );
        assert_eq!(at(3), [(0, m.overline_thickness)]);
    }

    #[test]
    fn the_cache_hands_back_unchanged_rows() {
        let face = face();
        let look = arctic().1;
        let first = screen("one\r\ntwo");
        let mut cache = RowCache::default();
        let rows = cache.rows(&first.screen, &first.colors, &face, &look);
        assert_eq!(cache.built, 2);

        // The same `Arc`s: the same `Rc`s back.
        let again = cache.rows(&first.screen, &first.colors, &face, &look);
        assert_eq!(cache.built, 0);
        assert!(rows.iter().zip(&again).all(|(a, b)| Rc::ptr_eq(a, b)));

        // A changed look builds everything again.
        cache.rows(&first.screen, &first.colors, &face, &Look::default());
        assert_eq!(cache.built, 2);
    }
}
