//! A [`Buffer`] made from text, for tests: each line of text fills rows `cols` wide, soft
//! wrapping onto the next row when it's longer, as a terminal shows it. Wide characters (CJK)
//! take two cells.

use crate::buffer::{Buffer, Pos, Row};

#[derive(Clone, Debug)]
pub struct Fixture {
    rows: Vec<Row>,
    cols: usize,
    /// The view's first row and height.
    pub view: (usize, usize),
}

impl Fixture {
    /// The lines, `cols` wide; the view shows them all.
    pub fn new(cols: usize, lines: &[&str]) -> Fixture {
        let mut rows = Vec::new();
        for line in lines {
            let mut cells: Vec<Option<char>> = Vec::new();
            for c in line.chars() {
                let wide = is_wide(c);
                let at = cells.len() % cols;
                if wide && at == cols - 1 {
                    // A wide character doesn't fit at the row's end: a spacer, then the next row.
                    cells.push(None);
                }
                cells.push(Some(c));
                if wide {
                    cells.push(None);
                }
            }
            if cells.is_empty() {
                cells.push(Some(' '));
            }
            let chunks: Vec<&[Option<char>]> = cells.chunks(cols).collect();
            let last = chunks.len() - 1;
            for (i, chunk) in chunks.into_iter().enumerate() {
                let mut cells = chunk.to_vec();
                cells.resize(cols, Some(' '));
                rows.push(Row {
                    cells,
                    wrapped: i < last,
                });
            }
        }
        if rows.is_empty() {
            rows.push(Row::from_text("", cols, false));
        }
        let height = rows.len();
        Fixture {
            rows,
            cols,
            view: (0, height),
        }
    }

    /// Shows `height` rows from `top`.
    pub fn with_view(mut self, top: usize, height: usize) -> Fixture {
        self.view = (top, height);
        self
    }

    /// The text from `start` to `end`, both included, as Ghostty copies it (its formatter's
    /// `TRIM`): rows joined with newlines unless soft-wrapped, and the blanks at the end of each
    /// line taken, even a space the range ends on, left out.
    pub fn text(&self, start: Pos, end: Pos) -> String {
        let mut out = String::new();
        for y in start.row..=end.row {
            let row = &self.rows[y];
            let from = if y == start.row { start.col } else { 0 };
            let to = if y == end.row { end.col } else { self.cols - 1 };
            let piece: String = row.cells[from..=to.min(self.cols - 1)]
                .iter()
                .flatten()
                .collect();
            if row.wrapped && y != end.row {
                out.push_str(&piece);
            } else {
                out.push_str(piece.trim_end());
                if y != end.row {
                    out.push('\n');
                }
            }
        }
        out
    }
}

/// East Asian wide characters, roughly: enough for tests.
fn is_wide(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFF00..=0xFF60 | 0x1F300..=0x1FAFF)
}

impl Buffer for Fixture {
    fn rows(&self) -> usize {
        self.rows.len()
    }

    fn cols(&self) -> usize {
        self.cols
    }

    fn row(&mut self, y: usize) -> Row {
        self.rows[y].clone()
    }

    fn view(&self) -> (usize, usize) {
        self.view
    }

    fn matches(&mut self, query: &str) -> Vec<(Pos, Pos)> {
        let needle: Vec<char> = query.chars().map(|c| c.to_ascii_lowercase()).collect();
        if needle.is_empty() {
            return Vec::new();
        }
        // Each line's cells in order, joined across soft wraps, as the terminal's search reads.
        let mut hits = Vec::new();
        let mut y = 0;
        while y < self.rows.len() {
            let mut cells: Vec<(Pos, char)> = Vec::new();
            loop {
                for (x, c) in self.rows[y].cells.iter().enumerate() {
                    if let Some(c) = c {
                        cells.push((Pos::new(y, x), c.to_ascii_lowercase()));
                    }
                }
                let wrapped = self.rows[y].wrapped;
                y += 1;
                if !wrapped || y >= self.rows.len() {
                    break;
                }
            }
            for i in 0..cells.len() {
                if cells.len() - i >= needle.len()
                    && cells[i..i + needle.len()]
                        .iter()
                        .map(|(_, c)| *c)
                        .eq(needle.iter().copied())
                {
                    hits.push((cells[i].0, cells[i + needle.len() - 1].0));
                }
            }
        }
        hits
    }
}
