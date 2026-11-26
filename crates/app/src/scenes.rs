//! Named states for `--snap --scene NAME` (`--list-scenes` lists them).

use crate::State;
use crate::fixtures::{self, BOXES, COLORS, Fixture, NVIM_SPLITS};
use crate::view::terminal::TerminalPane;

/// A scene: its name and how it changes the first state.
type Scene = (&'static str, fn(&mut State));

pub(crate) const SCENES: &[Scene] = &[
    ("hello", |_| {}),
    ("terminal-colors", |s| one(s, &COLORS)),
    ("terminal-boxes", |s| one(s, &BOXES)),
    ("terminal-nvim", |s| one(s, &NVIM_SPLITS)),
    ("terminal-selection", selection),
    ("terminal-panes", panes),
    ("screen-file", screen_file),
];

/// `fixture` alone in a focused pane: what the Ghostty pixel diff compares.
fn one(s: &mut State, fixture: &Fixture) {
    let mut pane = TerminalPane::new("pane-1", fixture.snapshot(&s.theme));
    pane.focused = true;
    s.panes = vec![pane];
}

/// The colors screen with three rows selected, in an unfocused pane (a hollow cursor).
fn selection(s: &mut State) {
    let mut snapshot = COLORS.snapshot(&s.theme);
    fixtures::select(&mut snapshot, 15..=17, 8, 40);
    s.panes = vec![TerminalPane::new("pane-1", snapshot)];
}

/// Four panes, 2 × 2, the Neovim one focused.
fn panes(s: &mut State) {
    let mut selected = COLORS.snapshot(&s.theme);
    fixtures::select(&mut selected, 15..=17, 8, 40);
    let snapshots = [
        NVIM_SPLITS.snapshot(&s.theme),
        BOXES.snapshot(&s.theme),
        COLORS.snapshot(&s.theme),
        selected,
    ];
    s.panes = snapshots
        .into_iter()
        .enumerate()
        .map(|(i, snapshot)| {
            let mut pane = TerminalPane::new(format!("pane-{}", i + 1), snapshot);
            pane.focused = i == 0;
            pane
        })
        .collect();
}

/// A screen file in a focused pane, at the grid `HERDR_FRONTEND_GRID` names
/// ([`fixtures::screen_file`]); an empty 80 × 30 screen without one.
fn screen_file(s: &mut State) {
    let (bytes, cols, rows) = fixtures::screen_file().unwrap_or_default();
    let (cols, rows) = if cols == 0 {
        (fixtures::COLS, fixtures::ROWS)
    } else {
        (cols, rows)
    };
    let mut pane = TerminalPane::new("pane-1", fixtures::replay(&bytes, cols, rows, &s.theme));
    pane.focused = true;
    s.panes = vec![pane];
}
