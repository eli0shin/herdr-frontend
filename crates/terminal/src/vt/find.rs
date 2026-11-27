//! The find bar's search over the screen and scrollback: libghostty-vt's incremental search
//! (`ghostty_search_*`), as Herdr app drives it (`herdr-app:src/vt.rs` `start_search`).
//!
//! A terminal owns its own search, made the first time a query is set. While it has a query,
//! every snapshot first brings it up to date with the terminal, so new output is searched too,
//! and marks the hits on the rows it shows ([`crate::Row::matches`]). "Next" goes up into
//! history, toward older output; after the oldest hit it wraps to the newest. Selecting a hit
//! scrolls the viewport to it if it isn't on screen.

use std::ptr;

use ghostty_vt_sys::*;

use copy_mode::Pos;

use super::{Vt, ok};
use crate::snapshot::{Found, Match};

/// A search and what it last found on the viewport.
pub(super) struct Finder {
    search: GhosttySearch,
    /// The query; empty while the bar has none (the search is idle).
    needle: String,
    found: Found,
    /// The viewport's hits by row, as of the last refresh.
    hits: Vec<(u16, Match)>,
}

impl Finder {
    /// An idle search over `term`; free it with [`Finder::free`] before the terminal.
    pub(super) fn new(term: GhosttyTerminal) -> Finder {
        let mut search: GhosttySearch = ptr::null_mut();
        // SAFETY: `term` is live; the caller frees the search before it.
        ok(
            unsafe { ghostty_search_new(ptr::null(), &mut search, term) },
            "ghostty_search_new",
        );
        Finder {
            search,
            needle: String::new(),
            found: Found::default(),
            hits: Vec::new(),
        }
    }

    /// The query searched for; empty while idle.
    pub(super) fn needle(&self) -> &str {
        &self.needle
    }

    /// Sets the query (empty: idle). False if it was already the query.
    pub(super) fn set(&mut self, needle: &str) -> bool {
        if self.needle == needle {
            return false;
        }
        self.needle = needle.to_owned();
        let value = GhosttyString {
            ptr: needle.as_ptr(),
            len: needle.len(),
        };
        // SAFETY: the search and its terminal are live; the bytes are copied during the call.
        ok(
            unsafe {
                ghostty_search_set(
                    self.search,
                    GHOSTTY_SEARCH_OPT_NEEDLE,
                    (&raw const value).cast(),
                )
            },
            "SEARCH_OPT_NEEDLE",
        );
        if needle.is_empty() {
            self.found = Found::default();
            self.hits.clear();
        }
        true
    }

    /// The hits on viewport row `y`, as of the last refresh.
    pub(super) fn hits_on(&self, y: u16) -> Vec<Match> {
        if self.needle.is_empty() {
            return Vec::new();
        }
        self.hits
            .iter()
            .filter(|(row, _)| *row == y)
            .map(|(_, m)| *m)
            .collect()
    }

    /// # Safety
    ///
    /// `self.search` was made for `term`, which is still live, and nothing uses `self` after.
    pub(super) unsafe fn free(&mut self) {
        // SAFETY: the caller's contract.
        unsafe { ghostty_search_free(self.search) };
    }
}

impl Vt {
    /// Searches the screen and scrollback for `needle` (ASCII letters match either case). An
    /// empty one stops searching. Each terminal keeps its own search.
    pub fn find(&mut self, needle: &str) {
        if self.finder.is_none() {
            if needle.is_empty() {
                return;
            }
            // Freed before the terminal in `Drop`.
            self.finder = Some(Finder::new(self.term));
        }
        let finder = self.finder.as_mut().expect("made above");
        if finder.set(needle) && !needle.is_empty() {
            self.refresh_find();
        }
    }

    /// The query searched for, if any.
    pub fn finding(&self) -> Option<&str> {
        self.finder
            .as_ref()
            .map(|f| f.needle.as_str())
            .filter(|n| !n.is_empty())
    }

    /// Selects the next hit: toward older output (up), or with `back`, toward newer. The
    /// viewport scrolls to it when it isn't on screen. False with no hits.
    pub fn find_next(&mut self, back: bool) -> bool {
        let Some(search) = self.active_search() else {
            return false;
        };
        // SAFETY: the search and its terminal are live.
        let result = unsafe {
            if back {
                ghostty_search_select_prev(search)
            } else {
                ghostty_search_select_next(search)
            }
        };
        self.refresh_find();
        result == GHOSTTY_SUCCESS
    }

    /// How the search stands, while there is a query.
    pub fn found(&self) -> Option<Found> {
        self.finding()?;
        self.finder.as_ref().map(|f| f.found)
    }

    /// The hits on viewport row `y`, as of the last refresh: copy mode's search while it has
    /// one, else the find bar's.
    pub(super) fn matches_on(&self, y: u16) -> Vec<Match> {
        if let Some(hits) = self.copy_matches_on(y) {
            return hits;
        }
        self.finder
            .as_ref()
            .map(|f| f.hits_on(y))
            .unwrap_or_default()
    }

    fn active_search(&self) -> Option<GhosttySearch> {
        self.finder
            .as_ref()
            .filter(|f| !f.needle.is_empty())
            .map(|f| f.search)
    }

    /// Brings the search up to date with the terminal and reads its counts and the viewport's
    /// hits. A snapshot calls it first, so new output is searched as it arrives.
    pub(super) fn refresh_find(&mut self) {
        let size = self.effects().size;
        if let Some(finder) = self.finder.as_mut().filter(|f| !f.needle.is_empty()) {
            // SAFETY: the finder was made for this live terminal.
            unsafe { finder.refresh(self.term, size.columns, size.rows) };
        }
    }
}

impl Finder {
    /// Brings the search up to date with `term` and reads its counts and the viewport's hits.
    ///
    /// # Safety
    ///
    /// The search was made for `term`, which is live, and the search has a query.
    pub(super) unsafe fn refresh(&mut self, term: GhosttyTerminal, cols: u16, rows: u16) {
        let search = self.search;
        // SAFETY: the search and its terminal are live; every out pointer has its key's type,
        // and the selection buffer has room for `cap` entries. The matches are grid refs that
        // die with the next terminal change, so they're turned into viewport cells here.
        unsafe {
            ok(ghostty_search_run(search), "ghostty_search_run");
            let mut status = GHOSTTY_SEARCH_STATUS_COMPLETE;
            ghostty_search_get(search, GHOSTTY_SEARCH_DATA_STATUS, (&raw mut status).cast());
            let mut total = 0usize;
            ghostty_search_get(
                search,
                GHOSTTY_SEARCH_DATA_TOTAL_MATCHES,
                (&raw mut total).cast(),
            );
            let mut index = 0usize;
            let selected = (ghostty_search_get(
                search,
                GHOSTTY_SEARCH_DATA_SELECTED_INDEX,
                (&raw mut index).cast(),
            ) == GHOSTTY_SUCCESS)
                .then_some(index);
            let mut chosen = GhosttySelection::default();
            let has_chosen = ghostty_search_get(
                search,
                GHOSTTY_SEARCH_DATA_SELECTED_MATCH,
                (&raw mut chosen).cast(),
            ) == GHOSTTY_SUCCESS;

            let matches = selections(search, GHOSTTY_SEARCH_DATA_VIEWPORT_MATCHES);
            let (cols, rows) = (cols.max(1), rows.max(1));
            let mut hits = Vec::new();
            for m in &matches {
                let mut equal = false;
                if has_chosen {
                    ghostty_terminal_selection_equal(term, m, &chosen, &mut equal);
                }
                let mut start = GhosttyPointCoordinate::default();
                let mut end = GhosttyPointCoordinate::default();
                let ends = (
                    ghostty_terminal_point_from_grid_ref(
                        term,
                        &m.start,
                        GHOSTTY_POINT_TAG_VIEWPORT,
                        &mut start,
                    ),
                    ghostty_terminal_point_from_grid_ref(
                        term,
                        &m.end,
                        GHOSTTY_POINT_TAG_VIEWPORT,
                        &mut end,
                    ),
                );
                // Hits on the pages around the viewport but outside it don't convert.
                if ends != (GHOSTTY_SUCCESS, GHOSTTY_SUCCESS) {
                    continue;
                }
                let mut last_ref = m.end;
                if (start.y, start.x) > (end.y, end.x) {
                    std::mem::swap(&mut start, &mut end);
                    last_ref = m.start;
                }
                // A hit ending on a wide character covers its second cell too.
                if wide_at(&last_ref) {
                    end.x += 1;
                }
                let last_row = end.y.min(u32::from(rows) - 1);
                for y in start.y..=last_row {
                    let first = if y == start.y { start.x } else { 0 };
                    let last = if y == end.y { end.x } else { cols - 1 };
                    if first <= last && first < cols {
                        hits.push((
                            y as u16,
                            Match {
                                start: first,
                                end: last.min(cols - 1),
                                selected: equal,
                            },
                        ));
                    }
                }
            }
            hits.sort_by_key(|(y, m)| (*y, m.start));
            self.found = Found {
                total,
                selected,
                complete: status == GHOSTTY_SEARCH_STATUS_COMPLETE,
            };
            self.hits = hits;
        }
    }

    /// Every match on the screen and in the scrollback, run to completion first: where each
    /// starts and the cell it ends on, as screen points (rows from the oldest scrollback row),
    /// in reading order. Literal; ASCII letters match either case.
    ///
    /// # Safety
    ///
    /// As [`Finder::refresh`].
    pub(super) unsafe fn all(&mut self, term: GhosttyTerminal) -> Vec<(Pos, Pos)> {
        // SAFETY: the caller's contract; each match is converted before the terminal changes.
        unsafe {
            ok(ghostty_search_run(self.search), "ghostty_search_run");
            let matches = selections(self.search, GHOSTTY_SEARCH_DATA_MATCHES);
            let point = |r: &GhosttyGridRef| {
                let mut at = GhosttyPointCoordinate::default();
                (ghostty_terminal_point_from_grid_ref(term, r, GHOSTTY_POINT_TAG_SCREEN, &mut at)
                    == GHOSTTY_SUCCESS)
                    .then(|| Pos::new(at.y as usize, usize::from(at.x)))
            };
            let mut all: Vec<(Pos, Pos)> = matches
                .iter()
                .filter_map(|m| {
                    let (a, b) = (point(&m.start)?, point(&m.end)?);
                    Some((a.min(b), a.max(b)))
                })
                .collect();
            all.sort();
            all
        }
    }
}

/// A search's list of matches (`what`: all of them, or the viewport's), copied out.
///
/// # Safety
///
/// `search` is live; the matches' grid refs are valid until the terminal next changes.
unsafe fn selections(search: GhosttySearch, what: GhosttySearchData) -> Vec<GhosttySelection> {
    let mut buffer = GhosttySelectionBuffer {
        ptr: ptr::null_mut(),
        cap: 0,
        len: 0,
    };
    // SAFETY: the caller's contract; a null buffer asks for the count, then `matches` has room.
    unsafe {
        match ghostty_search_get(search, what, (&raw mut buffer).cast()) {
            GHOSTTY_OUT_OF_SPACE if buffer.len > 0 => {
                let mut matches = vec![GhosttySelection::default(); buffer.len];
                buffer = GhosttySelectionBuffer {
                    ptr: matches.as_mut_ptr(),
                    cap: matches.len(),
                    len: 0,
                };
                if ghostty_search_get(search, what, (&raw mut buffer).cast()) != GHOSTTY_SUCCESS {
                    buffer.len = 0;
                }
                matches.truncate(buffer.len);
                matches
            }
            _ => Vec::new(),
        }
    }
}

/// The cell at `r` is the first half of a wide character.
unsafe fn wide_at(r: &GhosttyGridRef) -> bool {
    let mut cell: GhosttyCell = 0;
    // SAFETY: the caller passes a ref valid until the next terminal change.
    unsafe {
        if ghostty_grid_ref_cell(r, &mut cell) != GHOSTTY_SUCCESS {
            return false;
        }
        let mut wide: GhosttyCellWide = GHOSTTY_CELL_WIDE_NARROW;
        ghostty_cell_get(cell, GHOSTTY_CELL_DATA_WIDE, (&raw mut wide).cast());
        wide == GHOSTTY_CELL_WIDE_WIDE
    }
}
