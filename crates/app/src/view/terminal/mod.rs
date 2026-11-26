//! A terminal pane: one terminal snapshot drawn at its real grid on `rune::grid`, with the cursor
//! over it (`docs/research/terminal-rendering.md` §2, stage 4 of §8; `docs/research/rune.md` §3).
//!
//! The grid is the terminal's own (the core's), not the pane's: a pane bigger than the grid shows
//! the terminal's background around it, and a smaller one clips it. The grid hugs the pane's top
//! left after its padding, as Ghostty does with `window-padding-balance = false`; that's what Eli
//! wants (`docs/research/feature-inventory.md` §1 "Terminal grid centering"). A caller that wants
//! Ghostty's balanced padding computes it with `terminal::Size` and passes it in.

pub mod cursor;
pub mod font;
pub mod look;
pub mod rows;
pub mod sprite;

use std::cell::RefCell;

use rune::{El, el, grid};
use terminal::{Padding, Snapshot};

use self::font::Face;
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
    rows: RefCell<RowCache>,
}

impl TerminalPane {
    pub fn new(key: impl Into<String>, snapshot: Snapshot) -> TerminalPane {
        TerminalPane {
            key: key.into(),
            snapshot,
            focused: false,
            padding: Padding::default(),
            rows: RefCell::new(RowCache::default()),
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

    /// The pane: the terminal's background, the grid at its top left after the padding, and the
    /// cursor.
    pub fn view(&self, face: &Face, look: &Look) -> El {
        let snapshot = &self.snapshot;
        let (cell_w, cell_h) = face.cell_pt();
        let rows = self
            .rows
            .borrow_mut()
            .rows(&snapshot.screen, &snapshot.colors, face, look);
        let (grid_w, grid_h) = (
            f32::from(snapshot.cols) * cell_w,
            f32::from(snapshot.rows) * cell_h,
        );
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
        el().key(self.key.clone())
            .w_full()
            .h_full()
            .fill(color(snapshot.colors.background))
            .clip_soft(0.0)
            .child(
                el().absolute(self.padding.left as f32 / s, self.padding.top as f32 / s)
                    .size(grid_w, grid_h)
                    .child(screen)
                    .children(cursor::overlay(snapshot, self.focused, face, look)),
            )
    }
}
