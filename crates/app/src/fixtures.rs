//! Fixture screens: recorded or generated program output (`fixtures/`), replayed through a real
//! terminal into the snapshot a pane draws. The scenes, the headless tests and the Ghostty pixel
//! diff all draw these.

use std::sync::Arc;

use terminal::{Config, Row, Snapshot, Theme, Vt};

/// A fixture: its name and the bytes a program wrote to a `COLS` × `ROWS` terminal.
pub struct Fixture {
    pub name: &'static str,
    pub bytes: &'static [u8],
}

/// Every fixture is 80 × 30, the size it was recorded or generated at.
pub const COLS: u16 = 80;
pub const ROWS: u16 = 30;

/// 16, 256 and 24-bit colors, SGR styles, wide characters and combining marks (`make.py`).
pub const COLORS: Fixture = Fixture {
    name: "colors",
    bytes: include_bytes!("../fixtures/colors.bin"),
};

/// Box drawing: lines, corners, tees and crosses in light, heavy and double; arcs, dashes,
/// blocks, braille and Powerline (`make.py`).
pub const BOXES: Fixture = Fixture {
    name: "boxes",
    bytes: include_bytes!("../fixtures/boxes.bin"),
};

/// Neovim 0.12 with three splits, line numbers, a cursorline and a visual selection
/// (`fixtures/README.md` has the command line).
pub const NVIM_SPLITS: Fixture = Fixture {
    name: "nvim-splits",
    bytes: include_bytes!("../fixtures/nvim-splits.bin"),
};

pub const ALL: [&Fixture; 3] = [&COLORS, &BOXES, &NVIM_SPLITS];

impl Fixture {
    /// The fixture's screen, replayed into a fresh terminal with `theme`'s colors.
    pub fn snapshot(&self, theme: &Theme) -> Snapshot {
        replay(self.bytes, COLS, ROWS, theme)
    }
}

/// `bytes` written to a fresh `cols` × `rows` terminal with `theme`'s colors.
pub fn replay(bytes: &[u8], cols: u16, rows: u16, theme: &Theme) -> Snapshot {
    let mut vt = Vt::new(&Config {
        cols,
        rows,
        theme: theme.clone(),
        ..Config::default()
    });
    vt.write(bytes);
    vt.snapshot()
}

/// The screen file and grid the `screen-file` scene shows, from `HERDR_FRONTEND_SCREEN` (a file
/// of program output) and `HERDR_FRONTEND_GRID` (`COLSxROWS`, 80x30 if unset): how the Ghostty
/// pixel diff draws what Ghostty drew.
pub fn screen_file() -> Option<(Vec<u8>, u16, u16)> {
    let path = std::env::var_os("HERDR_FRONTEND_SCREEN")?;
    let bytes = std::fs::read(path).ok()?;
    let grid = std::env::var("HERDR_FRONTEND_GRID").unwrap_or_default();
    let (cols, rows) = grid
        .split_once('x')
        .and_then(|(c, r)| Some((c.parse().ok()?, r.parse().ok()?)))
        .unwrap_or((COLS, ROWS));
    Some((bytes, cols, rows))
}

/// `snapshot` with columns `start..=end` of rows `rows` selected, as the terminal reports a
/// selection it holds.
pub fn select(
    snapshot: &mut Snapshot,
    rows: std::ops::RangeInclusive<usize>,
    start: u16,
    end: u16,
) {
    for y in rows {
        if let Some(row) = snapshot.screen.get_mut(y) {
            *row = Arc::new(Row {
                selection: Some((start, end)),
                ..(**row).clone()
            });
        }
    }
}
