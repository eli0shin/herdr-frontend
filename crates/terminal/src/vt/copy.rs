//! Copy mode on a terminal's own buffer (`docs/research/terminal-rendering.md` §6): the
//! [`copy_mode`] engine run over the screen and scrollback, read through grid references.
//!
//! - The cursor, the visual anchor and the marks are tracked references, so they stay on their
//!   text while output scrolls it, the scrollback is pruned or the grid reflows. Before each
//!   command they go into the [`Session`] as screen points; after, they're set from it.
//! - The view is pinned: its top row is a tracked reference too, scrolled back to after each
//!   write, so output arriving doesn't move it, even when copy mode started at the bottom
//!   (where `Scroll::Row` alone keeps following the active area). It follows the cursor.
//! - The selection is the terminal's own (`OPT_SELECTION`, a rectangle for a block), so the
//!   render state draws it and the formatter copies it as Ghostty copies a selection.
//! - `/` and `?` use a search of their own (`ghostty_search`), apart from the find bar's: its
//!   matches decide where `n` goes and are marked on the rows, the one under the cursor as the
//!   selected one, while the find bar's query and counts stay as they were.
//! - Snapshots carry the copy cursor as the cursor (a block) and [`CopyView`] for the bar.

use std::collections::BTreeMap;
use std::ptr;

use copy_mode::{Buffer, Command, Pos, Range, Row, Session, Visual};
use ghostty_vt_sys::*;

use super::find::Finder;
use super::{Vt, read_meta, set, terminal_get};
use crate::snapshot::{ActiveScreen, CopyView, Cursor, CursorStyle, Match};

/// A tracked grid reference, freed when dropped (which may be after its terminal).
struct Tracked(GhosttyTrackedGridRef);

impl Tracked {
    fn new(term: GhosttyTerminal, at: Pos) -> Option<Tracked> {
        let mut out: GhosttyTrackedGridRef = ptr::null_mut();
        // SAFETY: `term` is live and `out` is writable.
        let result = unsafe { ghostty_terminal_grid_ref_track(term, screen_point(at), &mut out) };
        (result == GHOSTTY_SUCCESS && !out.is_null()).then_some(Tracked(out))
    }

    /// Where it is now, unless the text it was on was pruned.
    fn get(&self, tag: GhosttyPointTag) -> Option<Pos> {
        let mut at = GhosttyPointCoordinate::default();
        // SAFETY: the handle is live until dropped; `at` is writable.
        let result = unsafe { ghostty_tracked_grid_ref_point(self.0, tag, &mut at) };
        (result == GHOSTTY_SUCCESS).then(|| Pos::new(at.y as usize, usize::from(at.x)))
    }

    fn set(&mut self, term: GhosttyTerminal, at: Pos) {
        // SAFETY: the handle and `term`, which made it, are live. On failure (out of memory)
        // it keeps its old place.
        unsafe { ghostty_tracked_grid_ref_set(self.0, term, screen_point(at)) };
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        // SAFETY: made by `ghostty_terminal_grid_ref_track`, freed once.
        unsafe { ghostty_tracked_grid_ref_free(self.0) };
    }
}

/// A screen point: `row` from the oldest scrollback row.
fn screen_point(at: Pos) -> GhosttyPoint {
    GhosttyPoint {
        tag: GHOSTTY_POINT_TAG_SCREEN,
        value: GhosttyPointValue {
            coordinate: GhosttyPointCoordinate {
                x: at.col.min(usize::from(u16::MAX)) as u16,
                y: at.row.min(u32::MAX as usize) as u32,
            },
        },
    }
}

/// Copy mode running in a terminal.
pub(super) struct Copying {
    session: Session,
    cursor: Tracked,
    anchor: Option<Tracked>,
    marks: BTreeMap<char, Tracked>,
    /// The view's top row, which output doesn't move while copy mode runs.
    pin: Option<Tracked>,
    /// The screen it started on; the program switching screens starts it over.
    screen: ActiveScreen,
    message: Option<String>,
    /// `/` and `?`'s search. Freed before the terminal.
    search: Finder,
}

impl Copying {
    /// # Safety
    ///
    /// The terminal the search was made for is live, and nothing uses `self` after.
    pub(super) unsafe fn free(&mut self) {
        // SAFETY: the caller's contract.
        unsafe { self.search.free() };
    }
}

/// The screen and scrollback as copy mode reads them.
struct Reader<'a> {
    term: GhosttyTerminal,
    cols: usize,
    total: usize,
    view: (usize, usize),
    search: &'a mut Finder,
}

impl Buffer for Reader<'_> {
    fn rows(&self) -> usize {
        self.total
    }

    fn cols(&self) -> usize {
        self.cols
    }

    fn row(&mut self, y: usize) -> Row {
        // SAFETY: `term` is live and nothing changes it while a command runs.
        unsafe { read_row(self.term, y, self.cols) }
    }

    fn view(&self) -> (usize, usize) {
        self.view
    }

    fn matches(&mut self, query: &str) -> Vec<(Pos, Pos)> {
        self.search.set(query);
        // SAFETY: the search was made for `term`, which is live.
        unsafe { self.search.all(self.term) }
    }
}

/// Row `y` of the screen (from the oldest scrollback row), a cell at a time.
///
/// # Safety
///
/// `term` is live.
unsafe fn read_row(term: GhosttyTerminal, y: usize, cols: usize) -> Row {
    let mut cells = Vec::with_capacity(cols);
    let mut wrapped = false;
    // SAFETY: the caller's contract; each grid ref is read before the terminal changes, and
    // every out pointer has its key's type.
    unsafe {
        for x in 0..cols {
            let mut r = GhosttyGridRef::default();
            if ghostty_terminal_grid_ref(term, screen_point(Pos::new(y, x)), &mut r)
                != GHOSTTY_SUCCESS
            {
                cells.push(Some(' '));
                continue;
            }
            if x == 0 {
                let mut row: GhosttyRow = 0;
                if ghostty_grid_ref_row(&r, &mut row) == GHOSTTY_SUCCESS {
                    ghostty_row_get(row, GHOSTTY_ROW_DATA_WRAP, (&raw mut wrapped).cast());
                }
            }
            let mut cell: GhosttyCell = 0;
            if ghostty_grid_ref_cell(&r, &mut cell) != GHOSTTY_SUCCESS {
                cells.push(Some(' '));
                continue;
            }
            let mut wide: GhosttyCellWide = GHOSTTY_CELL_WIDE_NARROW;
            ghostty_cell_get(cell, GHOSTTY_CELL_DATA_WIDE, (&raw mut wide).cast());
            let mut codepoint = 0u32;
            ghostty_cell_get(
                cell,
                GHOSTTY_CELL_DATA_CODEPOINT,
                (&raw mut codepoint).cast(),
            );
            cells.push(match wide {
                GHOSTTY_CELL_WIDE_SPACER_TAIL | GHOSTTY_CELL_WIDE_SPACER_HEAD => None,
                _ => Some(
                    char::from_u32(codepoint)
                        .filter(|&c| c != '\0')
                        .unwrap_or(' '),
                ),
            });
        }
    }
    Row { cells, wrapped }
}

/// The scrollbar as it is now: rows in all, the view's first row, and its height.
fn scroll_state(term: GhosttyTerminal) -> (usize, usize, usize) {
    // SAFETY: `term` is live and the key's output type is the scrollbar struct.
    let bar: GhosttyTerminalScrollbar =
        unsafe { terminal_get(term, GHOSTTY_TERMINAL_DATA_SCROLLBAR) };
    (
        (bar.total as usize).max(1),
        bar.offset as usize,
        (bar.len as usize).max(1),
    )
}

impl Vt {
    /// Starts copy mode at the program's cursor, or at the bottom of the view when the cursor
    /// is scrolled out of it. Pins the view and drops any selection.
    pub fn copy_start(&mut self) {
        self.copy_end();
        let term = self.term;
        let (total, top, height) = scroll_state(term);
        let at = self.program_cursor(total, height);
        let at = match at {
            Some(at) if at.row >= top && at.row < top + height => at,
            _ => Pos::new((top + height - 1).min(total - 1), 0),
        };
        let Some(cursor) = Tracked::new(term, at) else {
            return;
        };
        // SAFETY: `term` is live.
        let screen = unsafe { read_meta(term) }.active_screen;
        self.copying = Some(Copying {
            session: Session::new(at),
            cursor,
            anchor: None,
            marks: BTreeMap::new(),
            pin: None,
            screen,
            message: None,
            search: Finder::new(term),
        });
        self.clear_selection();
        self.scroll(crate::Scroll::Row(top));
    }

    /// Pins the view where it is now, while copy mode runs: after any scroll.
    pub(super) fn copy_pin(&mut self) {
        let term = self.term;
        let Some(copying) = self.copying.as_mut() else {
            return;
        };
        let (_, top, _) = scroll_state(term);
        let at = Pos::new(top, 0);
        match &mut copying.pin {
            Some(pin) => pin.set(term, at),
            None => copying.pin = Tracked::new(term, at),
        }
    }

    /// Scrolls the view back to its pinned row if output moved it: after each write. A view
    /// at the bottom follows the active area, so output would carry it (and the cursor's text)
    /// away.
    pub(super) fn copy_keep_pinned(&mut self) {
        let Some(row) = self
            .copying
            .as_ref()
            .and_then(|c| c.pin.as_ref())
            .and_then(|p| p.get(GHOSTTY_POINT_TAG_SCREEN))
            .map(|p| p.row)
        else {
            return;
        };
        let (_, top, _) = scroll_state(self.term);
        if row != top {
            self.scroll(crate::Scroll::Row(row));
        }
    }

    /// Whether copy mode is running.
    pub fn copying(&self) -> bool {
        self.copying.is_some()
    }

    /// Runs one copy-mode command: moves, selects, scrolls the view after the cursor. A yank
    /// returns the text, as Ghostty copies a selection.
    pub fn copy_run(&mut self, command: &Command) -> Option<String> {
        let mut copying = self.copying.take()?;
        let term = self.term;
        let (total, top, height) = scroll_state(term);
        let cols = usize::from(self.effects().size.columns.max(1));
        // SAFETY: `term` is live.
        let screen = unsafe { read_meta(term) }.active_screen;
        let cursor = copying.cursor.get(GHOSTTY_POINT_TAG_SCREEN);
        match cursor {
            Some(at) if screen == copying.screen => {
                copying.session.cursor = at;
                copying.session.anchor = copying
                    .anchor
                    .as_ref()
                    .and_then(|a| a.get(GHOSTTY_POINT_TAG_SCREEN));
                if copying.session.anchor.is_none() {
                    copying.session.visual = None;
                }
                copying
                    .marks
                    .retain(|_, m| m.get(GHOSTTY_POINT_TAG_SCREEN).is_some());
                copying.session.marks = copying
                    .marks
                    .iter()
                    .filter_map(|(&name, m)| Some((name, m.get(GHOSTTY_POINT_TAG_SCREEN)?)))
                    .collect();
            }
            // The cursor's text was pruned, or the program switched screens: start over at
            // the bottom of the view.
            _ => {
                let at = Pos::new((top + height - 1).min(total - 1), 0);
                copying.session = Session::new(at);
                copying.anchor = None;
                copying.marks.clear();
                copying.screen = screen;
                match Tracked::new(term, at) {
                    Some(cursor) => copying.cursor = cursor,
                    None => {
                        self.copying = Some(copying);
                        return None;
                    }
                }
            }
        }
        let out = {
            let mut reader = Reader {
                term,
                cols,
                total,
                view: (top, height),
                search: &mut copying.search,
            };
            copying.session.run(command, &mut reader)
        };
        let session = &copying.session;
        copying.cursor.set(term, session.cursor);
        copying.anchor = match (copying.anchor.take(), session.anchor) {
            (Some(mut tracked), Some(at)) => {
                tracked.set(term, at);
                Some(tracked)
            }
            (None, Some(at)) => Tracked::new(term, at),
            (_, None) => None,
        };
        let marks = std::mem::take(&mut copying.marks);
        copying.marks = session
            .marks
            .iter()
            .filter_map(|(&name, &at)| Some((name, Tracked::new(term, at)?)))
            .collect();
        drop(marks);
        copying.message = out.message;
        let (new_top, selection) = (session.top, session.selection(cols));
        self.copying = Some(copying);
        if new_top != top {
            self.scroll(crate::Scroll::Row(new_top));
        }
        match selection {
            Some(range) => self.install(range),
            None => self.clear_selection(),
        }
        let range = out.yank?;
        self.install(range);
        self.selection_text()
    }

    /// Leaves copy mode: the selection goes and, with history to scroll back from, the view
    /// returns to the bottom.
    pub fn copy_end(&mut self) {
        let Some(mut copying) = self.copying.take() else {
            return;
        };
        // SAFETY: the search was made for this live terminal and goes with `copying`.
        unsafe { copying.free() };
        self.clear_selection();
        let (total, _, height) = scroll_state(self.term);
        if total > height {
            self.scroll(crate::Scroll::Bottom);
        }
    }

    /// The program's cursor as a screen point.
    fn program_cursor(&self, total: usize, height: usize) -> Option<Pos> {
        // SAFETY: `term` is live and the keys' output types are `uint16_t`.
        let (x, y): (u16, u16) = unsafe {
            (
                terminal_get(self.term, GHOSTTY_TERMINAL_DATA_CURSOR_X),
                terminal_get(self.term, GHOSTTY_TERMINAL_DATA_CURSOR_Y),
            )
        };
        let row = total.saturating_sub(height) + usize::from(y);
        (row < total).then(|| Pos::new(row, usize::from(x)))
    }

    /// Makes `range` the terminal's selection.
    fn install(&mut self, range: Range) {
        let point = |at: Pos| {
            let mut out = GhosttyGridRef::default();
            // SAFETY: `term` is live and `out` is writable.
            (unsafe { ghostty_terminal_grid_ref(self.term, screen_point(at), &mut out) }
                == GHOSTTY_SUCCESS)
                .then_some(out)
        };
        let (Some(start), Some(end)) = (point(range.start), point(range.end)) else {
            return;
        };
        let selection = GhosttySelection {
            start,
            end,
            rectangle: range.kind == Visual::Block,
        };
        // SAFETY: `term` is live and the selection's refs are fresh from it.
        unsafe {
            set(
                self.term,
                GHOSTTY_TERMINAL_OPT_SELECTION,
                (&raw const selection).cast(),
            )
        };
    }

    /// The copy cursor as the snapshot's cursor: a steady block on its cell, if that's in
    /// view.
    pub(super) fn copy_cursor(&self) -> Option<Cursor> {
        let copying = self.copying.as_ref()?;
        let at = copying.cursor.get(GHOSTTY_POINT_TAG_VIEWPORT);
        Some(Cursor {
            position: at.map(|p| (p.col as u16, p.row as u16)),
            wide_tail: false,
            visible: true,
            blinking: false,
            password_input: false,
            style: CursorStyle::Block,
        })
    }

    /// What the copy-mode bar shows.
    pub(super) fn copy_view(&self) -> Option<CopyView> {
        let copying = self.copying.as_ref()?;
        let (total, _, _) = scroll_state(self.term);
        let row = copying
            .cursor
            .get(GHOSTTY_POINT_TAG_SCREEN)
            .map_or(copying.session.cursor.row, |p| p.row);
        Some(CopyView {
            row: row + 1,
            total,
            message: copying.message.clone(),
        })
    }

    /// Brings copy mode's search up to date, for its hits on the rows.
    pub(super) fn refresh_copy_search(&mut self) {
        let size = self.effects().size;
        let term = self.term;
        if let Some(copying) = self.copying.as_mut()
            && !copying.search.needle().is_empty()
        {
            // SAFETY: the search was made for this live terminal and has a query.
            unsafe { copying.search.refresh(term, size.columns, size.rows) };
        }
    }

    /// Copy mode's hits on viewport row `y`, while it has a search: the one the cursor is on
    /// is the selected one.
    pub(super) fn copy_matches_on(&self, y: u16) -> Option<Vec<Match>> {
        let copying = self.copying.as_ref()?;
        if copying.search.needle().is_empty() {
            return None;
        }
        let cursor = copying.cursor.get(GHOSTTY_POINT_TAG_VIEWPORT);
        let mut hits = copying.search.hits_on(y);
        for hit in &mut hits {
            hit.selected = cursor.is_some_and(|c| {
                c.row == usize::from(y)
                    && (usize::from(hit.start)..=usize::from(hit.end)).contains(&c.col)
            });
        }
        Some(hits)
    }
}
