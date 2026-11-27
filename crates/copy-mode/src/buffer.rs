//! The text copy mode moves over: a [`Buffer`] of rows, and [`Text`], which reads them lazily,
//! once per command, and walks them as Vim walks a file.
//!
//! Vim's cursor steps over each line's characters and then the line's end (`inc_cursor`), a
//! place of its own that counts as a blank. [`Spot`] is either: a cell, or the end of a line.
//! A line here is a row and the rows its soft wrap continues on. Its blank cells after its last
//! text aren't part of it (a terminal row is padded with them), so an empty row is an empty
//! line, just its end.

use std::collections::HashMap;

/// A cell: `row` counts from the oldest scrollback row (0), `col` from the left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    pub row: usize,
    pub col: usize,
}

impl Pos {
    pub const fn new(row: usize, col: usize) -> Pos {
        Pos { row, col }
    }
}

/// One row as copy mode reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    /// One entry per column: the cell's character (a space for a blank cell), or `None` for a
    /// place no character starts: a wide character's second half, or the spacer a wide
    /// character that didn't fit leaves at a row's end.
    pub cells: Vec<Option<char>>,
    /// The row's line goes on in the next row (a soft wrap).
    pub wrapped: bool,
}

impl Row {
    /// A row of narrow characters.
    pub fn from_text(text: &str, cols: usize, wrapped: bool) -> Row {
        let mut cells: Vec<Option<char>> = text.chars().take(cols).map(Some).collect();
        cells.resize(cols, Some(' '));
        Row { cells, wrapped }
    }

    /// The last column with text.
    pub fn last_text(&self) -> Option<usize> {
        self.cells
            .iter()
            .rposition(|c| c.is_some_and(|c| !is_blank(c)))
    }

    /// The first column with text.
    pub fn first_text(&self) -> Option<usize> {
        self.cells
            .iter()
            .position(|c| c.is_some_and(|c| !is_blank(c)))
    }

    /// The row's text, blank cells as spaces, wide characters once.
    pub fn text(&self) -> String {
        self.cells.iter().flatten().collect()
    }

    /// The last column that belongs to the row's line: every cell up to a soft wrap, else up to
    /// the last text.
    fn limit(&self) -> Option<usize> {
        if self.wrapped {
            self.cells.iter().rposition(Option::is_some)
        } else {
            self.last_text()
        }
    }
}

/// A blank cell, as Vim's `cls()` counts one.
pub fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t' || c == '\0'
}

/// A terminal's text: its rows, its view, and a search over them.
pub trait Buffer {
    /// Rows in the scrollback and on the screen together. At least one.
    fn rows(&self) -> usize;
    fn cols(&self) -> usize;
    /// Row `y` (`y < rows()`).
    fn row(&mut self, y: usize) -> Row;
    /// The view's first row and how many rows it shows.
    fn view(&self) -> (usize, usize);
    /// Where each match for `query` starts and the cell it ends on, in reading order. Matches
    /// are literal; ASCII letters match either case.
    fn matches(&mut self, query: &str) -> Vec<(Pos, Pos)>;
}

/// A place the cursor steps over: a cell, or the end of the line whose last row is the one
/// given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Spot {
    Cell(Pos),
    End(usize),
}

/// Vim's character classes: blanks, punctuation, word characters, and other scripts' letters,
/// so a word stops where the script changes. For `W`, `B`, `E`, anything not blank is one class.
pub(crate) fn class(c: char, big: bool) -> u8 {
    if is_blank(c) || c == '\n' {
        0
    } else if big {
        1
    } else if c.is_ascii() {
        if c.is_ascii_alphanumeric() || c == '_' {
            2
        } else {
            1
        }
    } else if c.is_alphanumeric() {
        if (c as u32) < 0x2000 { 2 } else { 3 }
    } else {
        1
    }
}

/// A word character, for `*` and `#` and `iw`'s idea of a keyword.
pub(crate) fn is_word(c: char) -> bool {
    class(c, false) >= 2
}

/// A buffer read for one command: each row at most once.
pub(crate) struct Text<'a, B: Buffer + ?Sized> {
    pub buffer: &'a mut B,
    rows: HashMap<usize, Row>,
    pub total: usize,
    pub cols: usize,
}

impl<'a, B: Buffer + ?Sized> Text<'a, B> {
    pub fn new(buffer: &'a mut B) -> Text<'a, B> {
        let total = buffer.rows().max(1);
        let cols = buffer.cols().max(1);
        Text {
            buffer,
            rows: HashMap::new(),
            total,
            cols,
        }
    }

    pub fn row(&mut self, y: usize) -> &Row {
        let y = y.min(self.total - 1);
        let cols = self.cols;
        let buffer = &mut *self.buffer;
        self.rows.entry(y).or_insert_with(|| {
            let mut row = buffer.row(y);
            row.cells.resize(cols, Some(' '));
            row
        })
    }

    pub fn last_row(&self) -> usize {
        self.total - 1
    }

    /// The character at a spot: a line's end is a newline.
    pub fn char_at(&mut self, spot: Spot) -> char {
        match spot {
            Spot::Cell(p) => self.row(p.row).cells[p.col].unwrap_or(' '),
            Spot::End(_) => '\n',
        }
    }

    pub fn class_at(&mut self, spot: Spot, big: bool) -> u8 {
        class(self.char_at(spot), big)
    }

    /// The spot after `spot`, or `None` at the end of the buffer.
    pub fn next(&mut self, spot: Spot) -> Option<Spot> {
        match spot {
            Spot::Cell(p) => {
                let row = self.row(p.row);
                let limit = row.limit();
                let later = (p.col + 1..row.cells.len())
                    .take_while(|&c| limit.is_some_and(|l| c <= l))
                    .find(|&c| row.cells[c].is_some());
                if let Some(col) = later {
                    return Some(Spot::Cell(Pos::new(p.row, col)));
                }
                if row.wrapped && p.row < self.last_row() {
                    Some(self.row_start(p.row + 1))
                } else {
                    Some(Spot::End(p.row))
                }
            }
            Spot::End(r) if r < self.last_row() => Some(self.row_start(r + 1)),
            Spot::End(_) => None,
        }
    }

    /// The spot before `spot`, or `None` at the start of the buffer.
    pub fn prev(&mut self, spot: Spot) -> Option<Spot> {
        match spot {
            Spot::Cell(p) => {
                let row = self.row(p.row);
                let earlier = (0..p.col).rev().find(|&c| row.cells[c].is_some());
                match earlier {
                    Some(col) => Some(Spot::Cell(Pos::new(p.row, col))),
                    None => self.before_row(p.row),
                }
            }
            Spot::End(r) => match self.row(r).limit() {
                Some(col) => Some(Spot::Cell(Pos::new(r, self.head(r, col)))),
                None => self.before_row(r),
            },
        }
    }

    /// The first spot of row `y`: its first cell, or the line's end for an empty row.
    pub fn row_start(&mut self, y: usize) -> Spot {
        let row = self.row(y);
        match row.limit() {
            Some(limit) => {
                let col = (0..=limit).find(|&c| row.cells[c].is_some()).unwrap_or(0);
                Spot::Cell(Pos::new(y, col))
            }
            None => Spot::End(y),
        }
    }

    /// The spot just before row `y` starts.
    fn before_row(&mut self, y: usize) -> Option<Spot> {
        if y == 0 {
            return None;
        }
        let above = self.row(y - 1);
        if !above.wrapped {
            return Some(Spot::End(y - 1));
        }
        match above.limit() {
            Some(col) => Some(Spot::Cell(Pos::new(y - 1, self.head(y - 1, col)))),
            None => self.before_row(y - 1),
        }
    }

    /// `col`, or the start of the wide character it's the second half of.
    pub fn head(&mut self, y: usize, col: usize) -> usize {
        let row = self.row(y);
        let col = col.min(row.cells.len() - 1);
        (0..=col)
            .rev()
            .find(|&c| row.cells[c].is_some())
            .unwrap_or(0)
    }

    /// The spot the cursor is on: its cell, or past the end of its row's text, the line's end.
    pub fn spot(&mut self, pos: Pos) -> Spot {
        let y = pos.row.min(self.last_row());
        let col = self.head(y, pos.col);
        match self.row(y).limit() {
            Some(limit) if col <= limit => Spot::Cell(Pos::new(y, col)),
            Some(limit) if self.row(y).wrapped => Spot::Cell(Pos::new(y, self.head(y, limit))),
            _ => Spot::End(y),
        }
    }

    /// Where the cursor lands on a spot. As in Vim's normal mode, never past a line's last
    /// character: a line's end is its last text, or the first column of an empty line.
    pub fn pos(&mut self, spot: Spot) -> Pos {
        match spot {
            Spot::Cell(p) => p,
            Spot::End(y) => match self.row(y).last_text() {
                Some(col) => Pos::new(y, col),
                None => Pos::new(y, 0),
            },
        }
    }

    /// The end of an empty line: where `w`, `b` and `}` stop.
    pub fn empty_line(&mut self, spot: Spot) -> bool {
        match spot {
            Spot::End(y) => self.row(y).limit().is_none() && (y == 0 || !self.row(y - 1).wrapped),
            Spot::Cell(_) => false,
        }
    }

    /// The first and last rows of the line row `y` is on.
    pub fn line_rows(&mut self, y: usize) -> (usize, usize) {
        let y = y.min(self.last_row());
        let mut first = y;
        while first > 0 && self.row(first - 1).wrapped {
            first -= 1;
        }
        let mut last = y;
        while last < self.last_row() && self.row(last).wrapped {
            last += 1;
        }
        (first, last)
    }

    /// The cells of the line row `y` is on, in order, with their characters.
    pub fn line(&mut self, y: usize) -> Vec<(Pos, char)> {
        let (first, last) = self.line_rows(y);
        let mut cells = Vec::new();
        for r in first..=last {
            let row = self.row(r);
            let Some(limit) = row.limit() else { continue };
            for c in 0..=limit {
                if let Some(ch) = row.cells[c] {
                    cells.push((Pos::new(r, c), ch));
                }
            }
        }
        cells
    }

    /// Whether the line starting on row `y` has no text.
    pub fn line_empty(&mut self, y: usize) -> bool {
        let (first, last) = self.line_rows(y);
        (first..=last).all(|r| self.row(r).last_text().is_none())
    }

    /// The first text on row `y`, or its first column.
    pub fn first_text(&mut self, y: usize) -> Pos {
        Pos::new(y, self.row(y).first_text().unwrap_or(0))
    }

    /// The last text on row `y`, or its first column.
    pub fn last_text(&mut self, y: usize) -> Pos {
        let col = self.row(y).last_text().unwrap_or(0);
        Pos::new(y, self.head(y, col))
    }

    /// `pos`, kept inside the buffer and on a character's first cell.
    pub fn clamp(&mut self, pos: Pos) -> Pos {
        let row = pos.row.min(self.last_row());
        Pos::new(row, self.head(row, pos.col.min(self.cols - 1)))
    }
}
