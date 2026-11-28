//! A terminal pane: one terminal snapshot drawn at its real grid on `rune::grid`, with the cursor
//! over it (`docs/research/terminal-rendering.md` §2, stage 4 of §8; `docs/research/rune.md` §3).
//!
//! The grid is the terminal's own (the core's), not the pane's: a pane bigger than the grid shows
//! the terminal's background around it, and a smaller one clips it. The grid sits at the pane's
//! top left after its padding, as Ghostty does with `window-padding-balance = false`, with no
//! balancing (`docs/research/feature-inventory.md` §1 "Terminal grid centering"). The app sets
//! the top padding to what the pane's height leaves under whole rows, so a grid at the pane's
//! size has its bottom row on the pane's bottom edge (`State::place_grids`).
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
//!
//! Scrolling smoothly, the screen sits [`TerminalPane::nudge`] rows lower than its place (higher
//! when negative), and the scrollback rows the snapshot carries around it
//! ([`Snapshot::above`], [`Snapshot::below`]) show in the space it leaves; the rows slide under
//! the grid's top edge (the top padding stays bare) and the pane's bottom edge. The nudge is the
//! snapshot's own part row plus the scrolling the window has sent the worker that the snapshot
//! doesn't show yet ([`TerminalPane::ahead`]), so a trackpad moves the screen the frame its event
//! lands, whole rows included.

pub mod cursor;
pub mod font;
pub mod glyph;
pub mod images;
pub mod look;
pub mod rows;
pub mod scrollbar;
pub mod sprite;

use std::cell::RefCell;
use std::sync::Arc;

use rune::{El, OptionAsAlt, Shape, Transition, canvas, el, grid, text};
use terminal::{Padding, Row, Snapshot};

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
    /// Rows the window has scrolled the scrollback, up for positive, that the snapshot doesn't
    /// show yet: the smooth scrolls sent to the worker that it hasn't answered.
    pub ahead: f32,
    rows: RefCell<RowCache>,
    /// The rows drawn above and below the screen, built apart so the screen's cache counts only
    /// its own.
    above: RefCell<RowCache>,
    below: RefCell<RowCache>,
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
            ahead: 0.0,
            rows: RefCell::new(RowCache::default()),
            above: RefCell::new(RowCache::default()),
            below: RefCell::new(RowCache::default()),
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

    /// Rows the screen draws below its place, scrolling smoothly; negative is above it. It's
    /// where the window has scrolled to (the snapshot's viewport and part row, less what it's
    /// [`TerminalPane::ahead`] by), as far as the scrollback reaches, and no further than the rows
    /// the snapshot carries around the screen.
    pub fn nudge(&self) -> f32 {
        let s = &self.snapshot;
        let bar = s.scrollbar;
        let offset = bar.offset as f64;
        let bottom = bar.total.saturating_sub(bar.len).max(bar.offset) as f64;
        // The screen's top, in rows from the top of the scrollback.
        let top = (offset - f64::from(s.nudge) - f64::from(self.ahead)).clamp(0.0, bottom);
        ((offset - top) as f32).clamp(-(s.below.len() as f32), s.above.len() as f32)
    }

    /// [`TerminalPane::nudge`] in whole device pixels of `face`'s cell.
    pub fn nudge_px(&self, face: &Face) -> f32 {
        (self.nudge() * face.metrics.cell_height as f32).round()
    }

    /// The part of [`TerminalPane::nudge_px`] the worker's own viewport sits low by, under a
    /// cell: by the time anything sent now reaches the worker, it has moved by every scroll sent
    /// before it, whole rows included, and only its part row is left.
    pub fn worker_nudge_px(&self, face: &Face) -> f32 {
        self.nudge_px(face)
            .rem_euclid(face.metrics.cell_height.max(1) as f32)
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

    /// The terminal under `key`. `whole`: the grid mounts at once. What's drawn over it, and
    /// its moves as it scrolls, always land at once: it moves with the fingers, never easing
    /// after them.
    fn draw(&self, key: &str, focused: bool, face: &Face, look: &Look, whole: bool) -> El {
        let snapshot = &self.snapshot;
        // Copy mode's cursor draws solid while its bar has the keyboard.
        let cursor_focused = focused || snapshot.copy.is_some();
        let (cell_w, cell_h) = face.cell_pt();
        let (grid_w, grid_h) = (
            f32::from(snapshot.cols) * cell_w,
            f32::from(snapshot.rows) * cell_h,
        );
        let nudge = self.nudge();
        // The rows the screen leaves room for above or below it.
        let above = &snapshot.above[snapshot.above.len() - nudge.max(0.0).ceil() as usize..];
        let below = &snapshot.below[..(-nudge).max(0.0).ceil() as usize];
        let layers = self.images.borrow_mut().layers(
            snapshot,
            face,
            (grid_w, grid_h),
            (above.len() as u16, below.len() as u16),
        );
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
            .absolute(
                self.padding.left as f32 / s,
                (self.padding.top as f32 + self.nudge_px(face)) / s,
            )
            .size(grid_w, grid_h)
            .instant_changes();
        if whole {
            at_grid = at_grid.transition(Transition::none());
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
                    .children(edge_rows(&self.above, snapshot, above, false, face, look))
                    .children(edge_rows(&self.below, snapshot, below, true, face, look))
                    .child(screen)
                    .children(spill_view(&spill, face, grid_w, grid_h))
                    .children(atlas.iter().map(|(row, cell)| cell.view(*row, face)))
                    .children(cursor::overlay(snapshot, cursor_focused, face, look))
                    .children(layers.above_text)
                    .children(self.ime(focused, face)),
            )
            .children(self.top_margin(snapshot, face))
    }

    /// The space above the grid, in the terminal's background over whatever a smooth scroll
    /// slides into it: rows scroll under the grid's top edge, and it stays as bare as it is on
    /// a screen sitting on its rows.
    fn top_margin(&self, snapshot: &Snapshot, face: &Face) -> Option<El> {
        (self.padding.top > 0).then(|| {
            el().key("top-margin")
                .absolute(0.0, 0.0)
                .w_full()
                .h(self.padding.top as f32 / face.scale)
                .fill(color(snapshot.colors.background))
                .a11y_hidden()
                .inert()
                .transition(Transition::none())
        })
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

/// Scrollback rows drawn beside the screen while it sits off its place: `rows` of `snapshot`
/// from `cache`, ending just above the grid when `below` is false, else from just under it.
fn edge_rows(
    cache: &RefCell<RowCache>,
    snapshot: &Snapshot,
    rows: &[Arc<Row>],
    below: bool,
    face: &Face,
    look: &Look,
) -> Option<El> {
    let mut cache = cache.borrow_mut();
    // Built even when there are none, so the cache keeps no rows that left.
    let built = cache.rows(rows, &snapshot.colors, face, look);
    if rows.is_empty() {
        return None;
    }
    let (atlas, spill) = (
        std::mem::take(&mut cache.atlas),
        std::mem::take(&mut cache.spill),
    );
    let (cell_w, cell_h) = face.cell_pt();
    let (w, h) = (
        f32::from(snapshot.cols) * cell_w,
        rows.len() as f32 * cell_h,
    );
    let y = if below {
        f32::from(snapshot.rows) * cell_h
    } else {
        -h
    };
    Some(
        el().key(if below { "below" } else { "above" })
            .absolute(0.0, y)
            .size(w, h)
            .a11y_hidden()
            .inert()
            .child(grid(snapshot.cols, cell_w, cell_h, built).size(w, h))
            .children(spill_view(&spill, face, w, h))
            .children(atlas.iter().map(|(row, cell)| cell.view(*row, face))),
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::terminal::look::arctic;
    use terminal::{Config, OVERSCAN, Vt};

    /// A 20 × 10 terminal that printed a hundred lines, and its snapshot after `then`.
    fn scrolled(then: impl FnOnce(&mut Vt)) -> Snapshot {
        let mut vt = Vt::new(&Config {
            cols: 20,
            rows: 10,
            theme: arctic().0,
            ..Config::default()
        });
        let lines: String = (1..=100).map(|i| format!("line {i}\r\n")).collect();
        vt.write(lines.as_bytes());
        then(&mut vt);
        vt.snapshot()
    }

    #[test]
    fn the_nudge_runs_ahead_of_the_snapshot_as_far_as_its_rows_reach() {
        let mut pane = TerminalPane::new("pane-1", scrolled(|_| {}));
        assert_eq!(pane.nudge(), 0.0, "on its rows at the bottom");
        // Scrolled up two and a half rows the worker hasn't moved yet.
        pane.ahead = 2.5;
        assert_eq!(pane.nudge(), 2.5);
        // Down from the bottom: nowhere to go.
        pane.ahead = -1.0;
        assert_eq!(pane.nudge(), 0.0);
        // Further than the rows the snapshot carries: as far as they reach.
        pane.ahead = 40.0;
        assert_eq!(pane.nudge(), f32::from(OVERSCAN));

        // Its own part row, and down toward the bottom from there.
        pane.show(scrolled(|vt| vt.scroll_rows(20.25, 1)));
        pane.ahead = 0.0;
        assert_eq!(pane.nudge(), 0.25);
        pane.ahead = -3.0;
        assert_eq!(pane.nudge(), -2.75, "rows below come into view");
        pane.ahead = -40.0;
        assert_eq!(pane.nudge(), -f32::from(OVERSCAN));

        // Near the top it stops at the first row.
        pane.show(scrolled(|vt| vt.scroll(terminal::Scroll::Row(1))));
        pane.ahead = 5.0;
        assert_eq!(pane.nudge(), 1.0);
    }
}
