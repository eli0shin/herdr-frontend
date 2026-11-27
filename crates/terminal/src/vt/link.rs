//! The link under a cell: an OSC 8 hyperlink, else a URL or file path found in the text by
//! Ghostty's pattern ([`crate::links`]), over the soft-wrapped line the cell is on. Links are
//! read from the rows the last snapshot showed, so a click opens what's on screen.

use std::sync::Arc;

use ghostty_vt_sys::*;

use super::Vt;
use crate::snapshot::{Row, Span, Width};

/// A link in a terminal: what it opens, and where it is on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// An OSC 8 hyperlink's URI, or the text that reads as a URL or path (`src/main.rs:42`).
    pub target: String,
    /// The program marked it with OSC 8.
    pub hyperlink: bool,
    /// The cells it covers, one span per viewport row.
    pub spans: Vec<Span>,
}

impl Vt {
    /// The link at a pixel position (from the grid's top left), if there is one.
    pub fn link_at(&self, x: f32, y: f32) -> Option<Link> {
        let (col, row) = self.cell_at(x, y)?;
        self.link_at_cell(col, row)
    }

    /// The link on viewport cell `(col, row)` of the last snapshot.
    pub fn link_at_cell(&self, col: u16, row: u16) -> Option<Link> {
        let screen = &self.shown;
        let cell = screen.get(usize::from(row))?.cells.get(usize::from(col))?;
        if cell.hyperlink {
            return self.hyperlink(col, row);
        }
        found_in(screen, col, row)
    }

    /// Underlines the link at a pixel position while Cmd is held, or nothing with `None`. True
    /// if what's underlined changed.
    pub fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let cell = at.and_then(|(x, y)| self.cell_at(x, y));
        let link = cell.and_then(|(col, row)| self.link_at_cell(col, row));
        self.hovered = cell;
        let spans = link.map(|l| l.spans).unwrap_or_default();
        let changed = spans != self.underlined;
        self.underlined = spans;
        changed
    }

    /// The hovered link, found again on rows just read: output may have moved it.
    pub(super) fn refresh_hover(&mut self) {
        if let Some((col, row)) = self.hovered {
            self.underlined = self
                .link_at_cell(col, row)
                .map(|l| l.spans)
                .unwrap_or_default();
        }
    }

    /// The OSC 8 hyperlink on a cell, and the run of cells on its row with the same URI.
    fn hyperlink(&self, col: u16, row: u16) -> Option<Link> {
        let uri = self.uri_at(col, row)?;
        let cells = &self.shown.get(usize::from(row))?.cells;
        let same = |c: u16| {
            cells.get(usize::from(c)).is_some_and(|cell| cell.hyperlink)
                && self.uri_at(c, row).as_deref() == Some(uri.as_str())
        };
        let mut start = col;
        while start > 0 && same(start - 1) {
            start -= 1;
        }
        let mut end = col;
        while usize::from(end) + 1 < cells.len() && same(end + 1) {
            end += 1;
        }
        Some(Link {
            target: uri,
            hyperlink: true,
            spans: vec![Span { row, start, end }],
        })
    }

    fn uri_at(&self, col: u16, row: u16) -> Option<String> {
        let r = self.grid_ref(col, row)?;
        let mut len = 0usize;
        // SAFETY: `r` is fresh; a null buffer asks for the size, then `buf` has room for it.
        unsafe {
            match ghostty_grid_ref_hyperlink_uri(&r, std::ptr::null_mut(), 0, &mut len) {
                GHOSTTY_OUT_OF_SPACE if len > 0 => {}
                _ => return None,
            }
            let mut buf = vec![0u8; len];
            if ghostty_grid_ref_hyperlink_uri(&r, buf.as_mut_ptr(), buf.len(), &mut len)
                != GHOSTTY_SUCCESS
            {
                return None;
            }
            buf.truncate(len);
            String::from_utf8(buf).ok().filter(|u| !u.is_empty())
        }
    }
}

/// A URL or path the pattern finds over the soft-wrapped line through `(col, row)`.
fn found_in(screen: &[Arc<Row>], col: u16, row: u16) -> Option<Link> {
    let row = usize::from(row);
    let mut first = row;
    while first > 0 && screen[first - 1].wrapped {
        first -= 1;
    }
    let mut last = row;
    while last + 1 < screen.len() && screen[last].wrapped {
        last += 1;
    }
    // The line's text, and the cell each character starts in.
    let mut text = String::new();
    let mut places: Vec<(usize, u16, u16, u16)> = Vec::new();
    let mut at = None;
    for (y, r) in screen.iter().enumerate().take(last + 1).skip(first) {
        for (x, cell) in r.cells.iter().enumerate() {
            let x = x as u16;
            if matches!(cell.width, Width::SpacerTail | Width::SpacerHead) {
                if y == row && x == col && cell.width == Width::SpacerTail {
                    at = places.last().map(|p| p.0);
                }
                continue;
            }
            if y == row && x == col {
                at = Some(text.len());
            }
            let width = if cell.width == Width::Wide { 2 } else { 1 };
            places.push((text.len(), y as u16, x, width));
            match cell.text.as_str() {
                "" => text.push(' '),
                t => text.push_str(t),
            }
        }
    }
    let range = crate::links::find(&text, at?)?;
    let mut spans: Vec<Span> = Vec::new();
    for &(byte, y, x, width) in &places {
        if !range.contains(&byte) {
            continue;
        }
        let end = x + width - 1;
        match spans.last_mut() {
            Some(span) if span.row == y => span.end = end,
            _ => spans.push(Span {
                row: y,
                start: x,
                end,
            }),
        }
    }
    Some(Link {
        target: text[range].to_owned(),
        hyperlink: false,
        spans,
    })
}
