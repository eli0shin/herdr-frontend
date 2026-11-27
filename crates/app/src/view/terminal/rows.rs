//! Terminal rows as `rune::GridRow`s: the port of Ghostty's `rebuildRow` color and run rules onto
//! Rune's grid, and the cache that hands an unchanged row back as the same `Rc`.
//!
//! A row becomes (`docs/research/terminal-rendering.md` §2):
//!
//! - its narrow cells' text as one line, with a span wherever the color, weight or slant
//!   changes;
//! - a fill for each run of cells with the same non-default background, selection included;
//! - pinned cells for what mustn't move the columns after it: wide characters, clusters of
//!   several code points, and characters the regular face has no glyph for. Those the glyph atlas
//!   draws ([`glyph`]) come out as images over the grid, and their pinned text is drawn
//!   transparent so the row still reads;
//! - Ghostty's sprites in the vector layer, at whole device pixels: underlines of every style,
//!   strikethrough, overline, and the characters Ghostty draws itself (box drawing, blocks,
//!   braille, Powerline, [`sprite`] has the list). A sprite character also stays in the text,
//!   drawn transparent, so the row reads as the terminal's text. The part of a sprite below the
//!   row (an undercurl's lower half) is kept apart, for the view to draw over the next row's
//!   background, as Ghostty draws every background first.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use rune::{Color, GridRow, Shape, Span, TextStyle};
use terminal::{CellFlags, Colors, Rgb, Row, RowId, Underline, Width};

use super::font::{Face, FaceKey};
use super::glyph::{self, AtlasCell, GlyphImage};
use super::look::{Look, color};
use super::sprite::{self, AlphaBox, Kind};

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
    /// Sprites over the text, in device pixels from the row's top left, each box in its color
    /// at its coverage.
    pub boxes: Vec<(AlphaBox, Color)>,
    /// The parts of sprites below the row, from the same top left. The grid would paint them
    /// under the next row's background fill, so the view draws them over the grid instead.
    pub spill: Vec<(AlphaBox, Color)>,
    /// Cells the glyph atlas draws, over the grid.
    pub atlas: Vec<AtlasCell>,
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

/// What a cell is marked as, in Ghostty's order of precedence: the selection over a find hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mark {
    None,
    Selection,
    /// A find hit; `true` for the selected one.
    Hit(bool),
}

/// A cell's colors. Inverse swaps foreground and background, defaults included, as Ghostty's
/// `rebuildRow` does. A selected cell takes the selection background; with no selection
/// foreground it keeps its own ink, as Herdr app does (`herdr-app:src/vt.rs:1647`). With no
/// selection background it's Ghostty's: the default background on the default foreground. A
/// find hit takes Ghostty's search colors.
pub(super) fn cell_colors(
    cell: &terminal::Cell,
    mark: Mark,
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
    if let Mark::Hit(selected) = mark {
        let (text, under) = if selected {
            look.search_selected
        } else {
            look.search
        };
        return CellColors {
            fg: text,
            bg: Some(under),
        };
    }
    if mark == Mark::Selection {
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
        spill: Vec::new(),
        atlas: Vec::new(),
        wraps: row.wrapped,
    };
    let cps = code_points(row);
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
        let hit = row
            .matches
            .iter()
            .find(|m| (m.start..=m.end).contains(&at))
            .map(|m| m.selected);
        let mark = match (selected, hit) {
            (true, _) => Mark::Selection,
            (false, Some(chosen)) => Mark::Hit(chosen),
            (false, None) => Mark::None,
        };
        let resolved = cell_colors(cell, mark, colors, look);

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
        let x0 = (u32::from(col) * cell_w) as i32;
        // Every cell draws its own decorations, a wide character's spacer too, as Ghostty does:
        // a curl or dots start again in each cell. Invisible text draws none, as in xterm and
        // Ghostty.
        if !cell.flags.contains(CellFlags::INVISIBLE) {
            // The link under the pointer while Cmd is held is underlined, twice if it already
            // was once, as Ghostty marks it.
            let hovered = row
                .link
                .is_some_and(|(start, end)| (start..=end).contains(&col));
            let underline = match cell.underline {
                Underline::Single if hovered => Underline::Double,
                _ if hovered => Underline::Single,
                own => own,
            };
            decorations(cell, underline, fg, x0, face, &mut built.boxes);
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

        // Text too close to its background is drawn white or black (`minimum_contrast`).
        let mut ink_fg = fg;
        let mut text_rgb = resolved.fg;
        if look.minimum_contrast > 1.0 && !single.is_some_and(no_min_contrast) {
            let bg = resolved.bg.unwrap_or(colors.background);
            text_rgb = contrasted(resolved.fg, bg, look.minimum_contrast);
            if text_rgb != resolved.fg {
                ink_fg = color(text_rgb).alpha(fg.a);
            }
        }

        // A character Ghostty draws itself draws as its sprite, in the vector layer. It stays in
        // the row, drawn transparent, so a screen reader still reads it.
        if drawn
            && let Some(c) = single
            && let Some(glyph) = sprite::glyph(c, span_cols as u8, metrics)
        {
            // `ink_fg`: `minimum_contrast` recolors the sprites it doesn't exempt (braille).
            push_sprite(&glyph, x0, ink_fg, &mut built.boxes);
            ink_fg = Color::TRANSPARENT;
        }

        let weight = if cell.flags.contains(CellFlags::BOLD) {
            BOLD
        } else {
            style.weight
        };
        let italic = cell.flags.contains(CellFlags::ITALIC);
        let on_line =
            drawn && cell.width == Width::Narrow && single.is_some_and(|c| on_line(c, cell, face));
        if drawn
            && !on_line
            && ink_fg != Color::TRANSPARENT
            && let Some(g) = atlas_glyph(row, &cps, x, face)
        {
            let alpha = if cell.flags.contains(CellFlags::FAINT) {
                look.faint_opacity
            } else {
                1.0
            };
            built.atlas.push(AtlasCell {
                col,
                pixels: g.pixels(text_rgb, alpha),
                glyph: g,
            });
            ink_fg = Color::TRANSPARENT;
        }
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
    built.spill = split_spill(&mut built.boxes, metrics.cell_height as i32);
    built
}

/// Takes the parts of `boxes` below a row `height` pixels tall out of them.
fn split_spill(boxes: &mut Vec<(AlphaBox, Color)>, height: i32) -> Vec<(AlphaBox, Color)> {
    let mut spill = Vec::new();
    boxes.retain_mut(|(b, color)| {
        if b.y1 <= height {
            return true;
        }
        spill.push((
            AlphaBox {
                y0: b.y0.max(height),
                ..*b
            },
            *color,
        ));
        b.y1 = height;
        b.y0 < height
    });
    spill
}

/// Each cell's code point, 0 for an empty one, as `constraintWidth` reads its neighbors.
fn code_points(row: &Row) -> Vec<u32> {
    row.cells
        .iter()
        .map(|c| c.text.as_str().chars().next().map_or(0, |c| c as u32))
        .collect()
}

/// Whether a narrow `c` in `cell` draws on the row's line: the terminal font has it, and Ghostty
/// draws it with no constraint. A glyph Ghostty constrains (a Nerd Font or other symbol, even
/// from the terminal font, as `addGlyph` does) goes through the atlas, which has only the
/// terminal font's regular face: in a bold or italic cell it stays on the line, in its style but
/// unconstrained.
pub fn on_line(c: char, cell: &terminal::Cell, face: &Face) -> bool {
    face.covers(c)
        && (cell.flags.contains(CellFlags::BOLD)
            || cell.flags.contains(CellFlags::ITALIC)
            || !glyph::constraint::for_text(c as u32).does_anything())
}

/// The atlas glyph for column `x` of `row`, if the atlas draws that cell.
pub(super) fn atlas_glyph(
    row: &Row,
    cps: &[u32],
    x: usize,
    face: &Face,
) -> Option<std::rc::Rc<GlyphImage>> {
    let cell = row.cells.get(x)?;
    let text = cell.text.as_str();
    let mut chars = text.chars();
    // A narrow character the row's line draws stays there, and a sprite draws as itself.
    if let (Some(c), None) = (chars.next(), chars.next())
        && ((cell.width == Width::Narrow && on_line(c, cell, face)) || sprite::drawn(c))
    {
        return None;
    }
    let wide = cell.width == Width::Wide;
    let width = glyph::constraint::constraint_width(cps, wide, x);
    glyph::render(face, text, wide, width)
}

/// [`atlas_glyph`] for a cell, its row's neighbors read from `row`.
pub(super) fn atlas_glyph_at(row: &Row, x: usize, face: &Face) -> Option<std::rc::Rc<GlyphImage>> {
    atlas_glyph(row, &code_points(row), x, face)
}

/// Ghostty's `contrasted_color`: `fg` when its contrast ratio with `bg` reaches `min`, else white
/// or black, whichever contrasts more with `bg`.
pub(super) fn contrasted(fg: Rgb, bg: Rgb, min: f32) -> Rgb {
    if contrast_ratio(fg, bg) >= min {
        return fg;
    }
    let (white, black) = (Rgb::new(255, 255, 255), Rgb::new(0, 0, 0));
    if contrast_ratio(white, bg) > contrast_ratio(black, bg) {
        white
    } else {
        black
    }
}

/// WCAG's contrast ratio, from relative luminance in linear light (Ghostty's `contrast_ratio`).
fn contrast_ratio(a: Rgb, b: Rgb) -> f32 {
    let luminance = |c: Rgb| {
        let ch = |v: u8| rune::color::srgb_decode(f32::from(v) / 255.0);
        0.2126 * ch(c.r) + 0.7152 * ch(c.g) + 0.0722 * ch(c.b)
    };
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Characters whose color `minimum_contrast` never changes: box drawing, block elements, legacy
/// computing and Powerline, which draw shapes rather than text (Ghostty's `noMinContrast`,
/// `ghostty:src/renderer/cell.zig`).
fn no_min_contrast(c: char) -> bool {
    matches!(
        c,
        '\u{2500}'..='\u{259F}'
            | '\u{1FB00}'..='\u{1FBFF}'
            | '\u{1CC00}'..='\u{1CEBF}'
            | '\u{E0B0}'..='\u{E0D7}'
    )
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

/// A sprite's boxes at `x0` in the row, in `color` at each box's coverage.
fn push_sprite(glyph: &sprite::Sprite, x0: i32, color: Color, boxes: &mut Vec<(AlphaBox, Color)>) {
    boxes.extend(glyph.boxes.iter().map(|b| {
        let at = AlphaBox {
            x0: b.x0 + x0,
            x1: b.x1 + x0,
            ..*b
        };
        (at, color.alpha(color.a * f32::from(b.alpha) / 255.0))
    }));
}

/// Underline, strikethrough and overline sprites for one cell, as Ghostty's `rebuildRow` adds
/// them: the underline (`underline`, the cell's own or a hovered link's) in the underline color
/// (the foreground's if unset), the others in the foreground, all at the cell's faint opacity.
fn decorations(
    cell: &terminal::Cell,
    underline: Underline,
    fg: Color,
    x0: i32,
    face: &Face,
    boxes: &mut Vec<(AlphaBox, Color)>,
) {
    let m = &face.metrics;
    if underline != Underline::None {
        let color = cell
            .underline_color
            .map_or(fg, |c| super::look::color(c).alpha(fg.a));
        push_sprite(
            &sprite::decoration(Kind::Underline(underline), m),
            x0,
            color,
            boxes,
        );
    }
    if cell.flags.contains(CellFlags::OVERLINE) {
        push_sprite(&sprite::decoration(Kind::Overline, m), x0, fg, boxes);
    }
    if cell.flags.contains(CellFlags::STRIKETHROUGH) {
        push_sprite(&sprite::decoration(Kind::Strikethrough, m), x0, fg, boxes);
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
            shape = shape.fill(sprite::alpha_box_pt(b, 0, face.scale), color);
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
    rows: HashMap<RowId, CachedRow>,
    /// Rows built (not reused) by the last [`RowCache::rows`] call.
    pub built: usize,
    /// The atlas cells of the last [`RowCache::rows`] call's rows, by row.
    pub atlas: Vec<(u16, AtlasCell)>,
    /// The sprite parts below the last call's rows ([`BuiltRow::spill`]), by row.
    pub spill: Vec<(u16, Spill)>,
}

/// A row's sprite parts below it ([`BuiltRow::spill`]), each box in its color at its coverage.
pub type Spill = Rc<[(AlphaBox, Color)]>;

/// A built row: the snapshot row it was built from, what Rune draws, its atlas cells and its
/// sprites' spill.
type CachedRow = (Arc<Row>, Rc<GridRow>, Rc<[AtlasCell]>, Spill);

#[derive(Clone, PartialEq)]
struct CacheKey {
    face: FaceKey,
    look: [u32; 10],
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
        self.atlas.clear();
        self.spill.clear();
        let rows = screen
            .iter()
            .enumerate()
            .map(|(y, row)| {
                let (grid_row, atlas, spill) = match self.rows.remove(&row.id) {
                    Some((was, grid_row, atlas, spill)) if Arc::ptr_eq(&was, row) => {
                        (grid_row, atlas, spill)
                    }
                    _ => {
                        self.built += 1;
                        let built = build(row, colors, face, look);
                        let atlas: Rc<[AtlasCell]> = built.atlas.clone().into();
                        let spill: Spill = built.spill.clone().into();
                        (Rc::new(built.grid_row(face)), atlas, spill)
                    }
                };
                self.atlas
                    .extend(atlas.iter().map(|cell| (y as u16, cell.clone())));
                if !spill.is_empty() {
                    self.spill.push((y as u16, spill.clone()));
                }
                kept.insert(row.id, (row.clone(), grid_row.clone(), atlas, spill));
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
    fn minimum_contrast_whitens_or_blackens_dim_text_but_never_graphics() {
        // Black text (ANSI 0 is arctic's background) on the default background, and a box line.
        let bytes = "\x1b[30mab\x1b[0m cd\x1b[30m\u{2500}";
        let snapshot = screen(bytes);
        let row = |min: f32| {
            let look = Look {
                minimum_contrast: min,
                ..arctic().1
            };
            build(&snapshot.screen[0], &snapshot.colors, &face(), &look)
        };
        // Off (Eli's 1): the program's colors stay.
        assert_eq!(row(1.0).spans[0].color, Some(rgb(BG)));
        // On: the invisible text turns white, the readable text keeps its color.
        let on = row(4.5);
        assert_eq!(on.spans[0].color, Some(rgb(0xffffff)));
        assert!(on.spans.iter().any(|s| s.color == Some(rgb(FG))));
        // The box-drawing line keeps its color.
        assert!(on.boxes.iter().all(|(_, c)| *c == rgb(BG)));
        // A braille sprite is text to Ghostty: it turns white too.
        let braille = screen("\x1b[30m\u{2836}");
        let look = Look {
            minimum_contrast: 4.5,
            ..arctic().1
        };
        let row = build(&braille.screen[0], &braille.colors, &face(), &look);
        assert!(!row.boxes.is_empty());
        assert!(row.boxes.iter().all(|(_, c)| *c == rgb(0xffffff)));
        assert_eq!(
            contrasted(Rgb::new(250, 250, 250), Rgb::new(255, 255, 255), 3.0),
            Rgb::new(0, 0, 0)
        );
        assert_eq!(
            contrasted(Rgb::new(0, 0, 0), Rgb::new(255, 255, 255), 3.0),
            Rgb::new(0, 0, 0)
        );
    }

    #[test]
    fn find_hits_take_ghosttys_search_colors_under_the_selection() {
        let snapshot = screen("a \x1b[41mfoo\x1b[0m foo");
        let mut row = (*snapshot.screen[0]).clone();
        row.matches = vec![
            terminal::Match {
                start: 2,
                end: 4,
                selected: false,
            },
            terminal::Match {
                start: 6,
                end: 8,
                selected: true,
            },
        ];
        // The selection covers the last cell of the selected hit.
        row.selection = Some((8, 8));
        let look = arctic().1;
        let built = build(&row, &snapshot.colors, &face(), &look);
        let yellow = rgb(0xffe082);
        let peach = rgb(0xf2a57e);
        let selection = color(look.selection_background.unwrap());
        assert_eq!(
            built.fills,
            [(2, 5, yellow), (6, 8, peach), (8, 9, selection)],
            "the program's red background gives way to the hit"
        );
        // Both hits' text is black (the space between doesn't break the run); the selected
        // cell keeps its own ink.
        let black = Some(rgb(0x000000));
        assert_eq!(
            built
                .spans
                .iter()
                .map(|s| (s.start, s.color))
                .collect::<Vec<_>>()[1..3],
            [(2, black), (8, Some(rgb(FG)))]
        );
    }

    /// A hovered link is underlined as Ghostty marks it: once, or twice where the program
    /// already underlined it (Pi's links are SGR 4 and OSC 8), in the cell's own ink.
    #[test]
    fn a_hovered_link_is_underlined_once_or_twice() {
        let snapshot = screen("ab \x1b[4mcd\x1b[0m");
        let mut row = (*snapshot.screen[0]).clone();
        let underlines = |row: &Row| -> Vec<(i32, usize)> {
            let built = build(row, &snapshot.colors, &face(), &arctic().1);
            let w = face().metrics.cell_width as i32;
            (0..5)
                .map(|col| {
                    let n = built
                        .boxes
                        .iter()
                        .filter(|(b, _)| b.x0 >= col * w && b.x0 < (col + 1) * w)
                        .count();
                    (col, n)
                })
                .collect()
        };
        let plain = underlines(&row);
        row.link = Some((0, 4));
        let hovered = underlines(&row);
        // `ab ` gains an underline; `cd`, underlined already, gains a second line.
        let single = plain[3].1;
        assert!(single > 0);
        assert_eq!(plain[0].1, 0);
        assert_eq!(hovered[0].1, single, "{hovered:?}");
        assert!(hovered[3].1 > single, "double: {hovered:?}");
    }

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
        let cell_w = face.metrics.cell_width as i32;
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
                .filter(|(b, _)| b.x0 == (col * m.cell_width) as i32)
                .map(|(b, _)| (b.y0 as u32, (b.y1 - b.y0) as u32))
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
        assert_eq!(
            at(3),
            [(m.overline_position.max(0) as u32, m.overline_thickness)]
        );
    }

    #[test]
    fn a_curly_underline_in_its_own_color_starts_again_in_every_cell() {
        let face = face();
        let w = face.metrics.cell_width as i32;
        // Neovim's diagnostic: undercurl (4:3) in a 24-bit underline color (58:2).
        let row = built("\x1b[4:3;58:2::255:0:0mab\x1b[0m");
        let red = rgb(0xff0000);
        assert!(!row.boxes.is_empty());
        assert!(row.boxes.iter().all(|(_, c)| c.r == red.r && c.g == 0.0));
        // Each cell pushes its own sprite, so the second half is the first moved a cell right.
        let (first, second) = row.boxes.split_at(row.boxes.len() / 2);
        let moved: Vec<_> = first
            .iter()
            .map(|(b, c)| (b.x0 + w, b.y0, b.x1 + w, b.y1, *c))
            .collect();
        let second: Vec<_> = second
            .iter()
            .map(|(b, c)| (b.x0, b.y0, b.x1, b.y1, *c))
            .collect();
        assert_eq!(moved, second, "the same sprite in each cell");
        assert!(row.boxes.iter().any(|(_, c)| c.a < 1.0), "antialiased");
    }

    #[test]
    fn an_undercurls_lower_half_is_kept_apart_to_draw_over_the_next_row() {
        let h = face().metrics.cell_height as i32;
        let row = built("\x1b[4:3mab\x1b[0m");
        // JetBrains Mono's undercurl reaches below the cell, as Ghostty draws it.
        assert!(!row.spill.is_empty(), "{:?}", row.boxes);
        assert!(row.boxes.iter().all(|(b, _)| b.y1 <= h && b.y0 < b.y1));
        assert!(row.spill.iter().all(|(b, _)| b.y0 >= h && b.y0 < b.y1));
    }

    #[test]
    fn a_box_across_the_rows_bottom_is_cut_there_and_one_below_moves_whole() {
        let b = |y0, y1, alpha| AlphaBox {
            x0: 0,
            y0,
            x1: 4,
            y1,
            alpha,
        };
        let red = rgb(0xff0000);
        let mut boxes = vec![
            (b(30, 40, 1), red),
            (b(37, 39, 2), red),
            (b(10, 20, 3), red),
        ];
        let spill = split_spill(&mut boxes, 37);
        assert_eq!(boxes, [(b(30, 37, 1), red), (b(10, 20, 3), red)]);
        assert_eq!(spill, [(b(37, 40, 1), red), (b(37, 39, 2), red)]);
    }

    #[test]
    fn a_glyph_ghostty_constrains_leaves_the_line_even_from_the_terminal_font() {
        let face = face();
        // JetBrains Mono has the Powerline branch symbol, but Ghostty draws it to its Nerd Font
        // constraint; a letter it draws as it is.
        assert!(face.covers('\u{e0a0}'));
        let row = built("\u{e0a0} a");
        assert_eq!(row.text, "  a");
        if cfg!(target_os = "macos") {
            assert_eq!(row.atlas.iter().map(|c| c.col).collect::<Vec<_>>(), [0]);
        }
    }

    #[test]
    fn a_bold_or_italic_symbol_from_the_terminal_font_stays_on_the_line_in_its_style() {
        // The atlas has only the regular face; a bold prompt's ❯ keeps its weight on the line.
        let row = built("\x1b[1m❯\x1b[0;3m→\x1b[0m");
        assert_eq!(row.text, "❯→");
        assert!(row.atlas.is_empty(), "{:?}", row.atlas);
        let styles: Vec<_> = row.spans.iter().map(|s| (s.weight, s.italic)).collect();
        assert_eq!(styles, [(Some(BOLD), false), (None, true)]);
    }

    #[test]
    fn a_wide_characters_spacer_draws_its_own_underline() {
        let face = face();
        let m = face.metrics;
        let row = built("\x1b[4m日\x1b[0m");
        let starts: Vec<i32> = row.boxes.iter().map(|(b, _)| b.x0).collect();
        assert_eq!(starts, [0, m.cell_width as i32]);
    }

    #[test]
    fn blocks_and_braille_draw_as_sprites() {
        let row = built("█▒⣿");
        assert_eq!(
            row.text.chars().filter(|&c| c != ' ').count() + row.cells.len(),
            3
        );
        // █ is one opaque box filling its cell; ▒ is half-alpha.
        let face = face();
        let (w, h) = (
            face.metrics.cell_width as i32,
            face.metrics.cell_height as i32,
        );
        assert!(
            row.boxes
                .iter()
                .any(|(b, c)| (b.x0, b.y0, b.x1, b.y1) == (0, 0, w, h) && c.a == 1.0)
        );
        assert!(
            row.boxes
                .iter()
                .any(|(b, c)| b.x0 == w && (c.a - 128.0 / 255.0).abs() < 1e-6)
        );
        assert!(row.boxes.iter().filter(|(b, _)| b.x0 >= 2 * w).count() >= 2);
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
