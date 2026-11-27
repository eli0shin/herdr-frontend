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
            let mut search: GhosttySearch = ptr::null_mut();
            // SAFETY: `term` is live; the search is freed before it in `Drop`.
            ok(
                unsafe { ghostty_search_new(ptr::null(), &mut search, self.term) },
                "ghostty_search_new",
            );
            self.finder = Some(Finder {
                search,
                needle: String::new(),
                found: Found::default(),
                hits: Vec::new(),
            });
        }
        let finder = self.finder.as_mut().expect("made above");
        if finder.needle == needle {
            return;
        }
        finder.needle = needle.to_owned();
        let value = GhosttyString {
            ptr: needle.as_ptr(),
            len: needle.len(),
        };
        // SAFETY: the search and its terminal are live; the bytes are copied during the call.
        ok(
            unsafe {
                ghostty_search_set(
                    finder.search,
                    GHOSTTY_SEARCH_OPT_NEEDLE,
                    (&raw const value).cast(),
                )
            },
            "SEARCH_OPT_NEEDLE",
        );
        if needle.is_empty() {
            finder.found = Found::default();
            finder.hits.clear();
        } else {
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

    /// The hits on viewport row `y`, as of the last refresh.
    pub(super) fn matches_on(&self, y: u16) -> Vec<Match> {
        match &self.finder {
            Some(f) if !f.needle.is_empty() => f
                .hits
                .iter()
                .filter(|(row, _)| *row == y)
                .map(|(_, m)| *m)
                .collect(),
            _ => Vec::new(),
        }
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
        let Some(search) = self.active_search() else {
            return;
        };
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

            let mut buffer = GhosttySelectionBuffer {
                ptr: ptr::null_mut(),
                cap: 0,
                len: 0,
            };
            let mut matches = Vec::new();
            match ghostty_search_get(
                search,
                GHOSTTY_SEARCH_DATA_VIEWPORT_MATCHES,
                (&raw mut buffer).cast(),
            ) {
                GHOSTTY_OUT_OF_SPACE if buffer.len > 0 => {
                    matches = vec![GhosttySelection::default(); buffer.len];
                    buffer = GhosttySelectionBuffer {
                        ptr: matches.as_mut_ptr(),
                        cap: matches.len(),
                        len: 0,
                    };
                    if ghostty_search_get(
                        search,
                        GHOSTTY_SEARCH_DATA_VIEWPORT_MATCHES,
                        (&raw mut buffer).cast(),
                    ) != GHOSTTY_SUCCESS
                    {
                        buffer.len = 0;
                    }
                    matches.truncate(buffer.len);
                }
                _ => {}
            }

            let size = self.effects().size;
            let (cols, rows) = (size.columns.max(1), size.rows.max(1));
            let mut hits = Vec::new();
            for m in &matches {
                let mut equal = false;
                if has_chosen {
                    ghostty_terminal_selection_equal(self.term, m, &chosen, &mut equal);
                }
                let mut start = GhosttyPointCoordinate::default();
                let mut end = GhosttyPointCoordinate::default();
                let ends = (
                    ghostty_terminal_point_from_grid_ref(
                        self.term,
                        &m.start,
                        GHOSTTY_POINT_TAG_VIEWPORT,
                        &mut start,
                    ),
                    ghostty_terminal_point_from_grid_ref(
                        self.term,
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
            let finder = self.finder.as_mut().expect("searching");
            finder.found = Found {
                total,
                selected,
                complete: status == GHOSTTY_SEARCH_STATUS_COMPLETE,
            };
            finder.hits = hits;
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
