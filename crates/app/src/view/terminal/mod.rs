//! A terminal pane: one terminal snapshot drawn at its real grid on `rune::grid`, with the cursor
//! over it (`docs/research/terminal-rendering.md` §2, stage 4 of §8; `docs/research/rune.md` §3).
//!
//! The grid is the terminal's own (the core's), not the pane's: a pane bigger than the grid shows
//! the terminal's background around it, and a smaller one clips it. The grid sits at the pane's
//! top left after its padding, as Ghostty does with `window-padding-balance = false`, with no
//! balancing (`docs/research/feature-inventory.md` §1 "Terminal grid centering"). The app sets
//! the top padding to what the pane's height leaves under whole rows, so the bottom row sits on
//! the pane's bottom edge (`State::place_grids`).
//!
//! While the pane has the keyboard, the cursor's cell is the input method's caret
//! (`El::ime_cursor`), even when the program hides the cursor, so the candidate window opens
//! there. What the input method composes draws over that cell, underlined, until it commits:
//! libghostty-vt keeps no preedit (`docs/research/terminal-rendering.md` §3, "IME").
//!
//! Kitty images draw in Ghostty's order ([`images`]): those under the backgrounds before the
//! grid, those over the text after it and the cursor. Images between the backgrounds and the
//! text split the grid in two, its backgrounds and then its text, with the images between them;
//! a pane without such images draws one grid.

pub mod cursor;
pub mod font;
pub mod glyph;
pub mod images;
pub mod look;
pub mod rows;
pub mod scrollbar;
pub mod sprite;

use std::cell::RefCell;

use rune::{El, OptionAsAlt, Shape, Transition, canvas, el, grid, text};
use terminal::{Padding, Snapshot};

use self::font::Face;
use self::images::ImageCache;
use self::look::{Look, color};
use self::rows::RowCache;

/// One terminal as a pane shows it: the last snapshot taken, whether the pane has focus, and the
/// rows built from it.
pub struct TerminalPane {
    /// The pane's key, `pane-<id>`; its grid is `pane-<id>/screen`.
    pub key: String,
    pub snapshot: Snapshot,
    pub focused: bool,
    /// Space between the pane's edges and the grid, in device pixels.
    pub padding: Padding,
    /// What the input method is composing, drawn at the cursor until it commits.
    pub preedit: Option<String>,
    /// Which Option keys act as Alt while the pane has the keyboard.
    pub option_as_alt: OptionAsAlt,
    rows: RefCell<RowCache>,
    images: RefCell<ImageCache>,
}

impl TerminalPane {
    pub fn new(key: impl Into<String>, snapshot: Snapshot) -> TerminalPane {
        TerminalPane {
            key: key.into(),
            snapshot,
            focused: false,
            padding: Padding::default(),
            preedit: None,
            option_as_alt: OptionAsAlt::Both,
            rows: RefCell::new(RowCache::default()),
            images: RefCell::new(ImageCache::default()),
        }
    }

    /// Shows a newer snapshot. Rows it shares with the last one keep their built `GridRow`s.
    pub fn show(&mut self, snapshot: Snapshot) {
        self.snapshot = snapshot;
    }

    /// How many rows the last view built rather than reused.
    pub fn rows_built(&self) -> usize {
        self.rows.borrow().built
    }

    /// How many kitty images the pane holds pixels for: the ones its last view drew.
    pub fn images_held(&self) -> usize {
        self.images.borrow().len()
    }

    /// The pane: the terminal's background, the grid at its top left after the padding, and the
    /// cursor.
    ///
    /// It shows whole the frame it comes into view and is gone the frame it leaves, grid and
    /// cursor included: Rune's default entrance would fade it in over 0.3 s on the click or key
    /// that shows it, and the split gap's border color behind it would flash through. Its own
    /// frame still follows its pane's, so a resize moves both edges together.
    pub fn view(&self, face: &Face, look: &Look) -> El {
        self.draw(&self.key, self.focused, face, look, true)
            .transition(Transition::none())
    }

    /// [`TerminalPane::view`] under another key, showing focus or not: the same terminal drawn
    /// a second time, as the peek draws one whose pane is on screen too.
    pub fn view_as(&self, key: &str, focused: bool, face: &Face, look: &Look) -> El {
        self.draw(key, focused, face, look, false)
    }

    /// The terminal under `key`. `whole`: the grid and what's drawn over it mount at once.
    fn draw(&self, key: &str, focused: bool, face: &Face, look: &Look, whole: bool) -> El {
        let snapshot = &self.snapshot;
        // Copy mode's cursor draws solid while its bar has the keyboard.
        let cursor_focused = focused || snapshot.copy.is_some();
        let (cell_w, cell_h) = face.cell_pt();
        let (grid_w, grid_h) = (
            f32::from(snapshot.cols) * cell_w,
            f32::from(snapshot.rows) * cell_h,
        );
        let layers = self
            .images
            .borrow_mut()
            .layers(snapshot, face, (grid_w, grid_h));
        let (fills, rows, atlas, spill) = {
            let mut cache = self.rows.borrow_mut();
            let (fills, rows) = if layers.below_text.is_some() {
                let (fills, text) = cache.split(&snapshot.screen, &snapshot.colors, face, look);
                (Some(fills), text)
            } else {
                (
                    None,
                    cache.rows(&snapshot.screen, &snapshot.colors, face, look),
                )
            };
            (
                fills,
                rows,
                std::mem::take(&mut cache.atlas),
                std::mem::take(&mut cache.spill),
            )
        };
        // The backgrounds alone, under the images between them and the text.
        let fills = fills.map(|rows| {
            grid(snapshot.cols, cell_w, cell_h, rows)
                .key("screen-backgrounds")
                .absolute(0.0, 0.0)
                .size(grid_w, grid_h)
                .a11y_hidden()
                .inert()
        });
        let mut screen = grid(snapshot.cols, cell_w, cell_h, rows)
            .key("screen")
            .label(if snapshot.title.is_empty() {
                "Terminal".to_owned()
            } else {
                snapshot.title.clone()
            })
            .size(grid_w, grid_h);
        if let Some(place) = cursor::place(snapshot, true) {
            screen = screen.grid_caret(place.col, place.row);
        }
        let s = face.scale;
        let mut at_grid = el()
            .absolute(self.padding.left as f32 / s, self.padding.top as f32 / s)
            .size(grid_w, grid_h);
        if whole {
            at_grid = at_grid.instant_changes().transition(Transition::none());
        }
        el().key(key.to_owned())
            .w_full()
            .h_full()
            .fill(color(snapshot.colors.background))
            .clip_soft(0.0)
            .child(
                at_grid
                    .children(layers.below_background)
                    .children(fills)
                    .children(layers.below_text)
                    .child(screen)
                    .children(spill_view(&spill, face, grid_w, grid_h))
                    .children(atlas.iter().map(|(row, cell)| cell.view(*row, face)))
                    .children(cursor::overlay(snapshot, cursor_focused, face, look))
                    .children(layers.above_text)
                    .children(self.ime(focused, face)),
            )
    }

    /// The input method's caret at the cursor's cell while the pane has the keyboard, with what
    /// it's composing drawn over the cell.
    fn ime(&self, focused: bool, face: &Face) -> Option<El> {
        if !focused {
            return None;
        }
        let snapshot = &self.snapshot;
        let (col, row) = snapshot.cursor.position?;
        let (cell_w, cell_h) = face.cell_pt();
        let (x, y) = (f32::from(col) * cell_w, f32::from(row) * cell_h);
        let caret = el()
            .key("ime")
            .absolute(0.0, 0.0)
            .size(cell_w, cell_h)
            .a11y_hidden()
            .ime_cursor()
            .ime_option_as_alt_mode(self.option_as_alt);
        // It comes and goes with the composition at once, as the cursor does.
        let mut at = el()
            .key("ime-at")
            .absolute(x, y)
            .inert()
            .instant_changes()
            .transition(Transition::none())
            .child(caret);
        if let Some(composing) = self.preedit.as_deref().filter(|t| !t.is_empty()) {
            // On the terminal's background, so the cells under it don't show through, and as
            // wide as its text (a CJK character takes two cells).
            let colors = &snapshot.colors;
            let line = 1.0 / face.scale;
            at = at.child(
                el().key("preedit")
                    .absolute(0.0, 0.0)
                    .h(cell_h)
                    .fill(color(colors.background))
                    .child(
                        text(composing.to_owned())
                            .key("preedit-text")
                            .font(face.style())
                            .color(color(colors.foreground)),
                    )
                    .child(
                        el().absolute(0.0, cell_h - line)
                            .w_full()
                            .h(line)
                            .fill(color(colors.foreground)),
                    ),
            );
        }
        Some(at)
    }
}

/// The sprite parts that reach below their rows ([`rows::BuiltRow::spill`]), over the grid: there
/// they draw over the next row's background, as in Ghostty, rather than under its fill.
fn spill_view(spill: &[(u16, rows::Spill)], face: &Face, grid_w: f32, grid_h: f32) -> Option<El> {
    if spill.is_empty() {
        return None;
    }
    let cell_h = face.metrics.cell_height as i32;
    let mut shape = Shape::new();
    for (row, boxes) in spill {
        let dy = i32::from(*row) * cell_h;
        for &(b, color) in boxes.iter() {
            let b = sprite::AlphaBox {
                y0: b.y0 + dy,
                y1: b.y1 + dy,
                ..b
            };
            shape = shape.fill(sprite::alpha_box_pt(b, 0, face.scale), color);
        }
    }
    Some(
        canvas(shape)
            .key("spill")
            .absolute(0.0, 0.0)
            .size(grid_w, grid_h)
            .a11y_hidden()
            .inert(),
    )
}
