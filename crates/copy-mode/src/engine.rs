//! [`Session`]: copy mode's cursor, selection, marks and last search, and how each
//! [`Command`] changes them over a [`Buffer`].
//!
//! The terminal keeps the session's positions as tracked references, so they follow the text
//! as output scrolls it and history is pruned; before each command it puts them in the session,
//! and after, it reads them back, scrolls to [`Session::top`], installs the selection and
//! yanks what [`Outcome::yank`] names.

use std::collections::BTreeMap;

use crate::buffer::{Buffer, Pos, Spot, Text};
use crate::command::{Command, Motion, Object, Reach, Visual};

/// How many objects further out a repeated text object looks for one past the selection.
const GROW_LIMIT: usize = 64;
use crate::{motion, object};

/// The column `j` and `k` keep: the one the cursor was last put in, or a row's end after `$`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want {
    Col(usize),
    End,
}

/// The last search, for `n` and `N`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Search {
    pub query: String,
    pub backward: bool,
    /// From `*` or `#`: only whole words match.
    pub whole: bool,
}

/// Cells from `start` to `end`, both included. For [`Visual::Lines`] they span whole rows; for
/// [`Visual::Block`] they're opposite corners of a rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: Pos,
    pub end: Pos,
    pub kind: Visual,
}

/// What a command leaves for the terminal to do besides moving.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The text to copy to the clipboard.
    pub yank: Option<Range>,
    /// Something to say in the bar ("Pattern not found: foo").
    pub message: Option<String>,
}

/// One copy mode's state over one terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub cursor: Pos,
    /// Where the selection started, while there is one.
    pub anchor: Option<Pos>,
    pub visual: Option<Visual>,
    pub want: Want,
    /// `m` marks, and `'` for the place before the last jump.
    pub marks: BTreeMap<char, Pos>,
    pub(crate) search: Option<Search>,
    /// The view's first row: as the buffer had it, then where the command leaves it.
    pub top: usize,
    /// The view's height, as the buffer last had it.
    pub height: usize,
}

impl Session {
    /// Copy mode starting at `cursor`, nothing selected.
    pub fn new(cursor: Pos) -> Session {
        Session {
            cursor,
            anchor: None,
            visual: None,
            want: Want::Col(cursor.col),
            marks: BTreeMap::new(),
            search: None,
            top: 0,
            height: 1,
        }
    }

    /// The last search's query, for highlighting its matches.
    pub fn query(&self) -> Option<&str> {
        self.search.as_ref().map(|s| s.query.as_str())
    }

    /// What's selected: between the anchor and the cursor, as the visual mode covers.
    pub fn selection(&self, cols: usize) -> Option<Range> {
        let (anchor, kind) = (self.anchor?, self.visual?);
        let (a, b) = (anchor.min(self.cursor), anchor.max(self.cursor));
        Some(match kind {
            Visual::Chars => Range {
                start: a,
                end: b,
                kind,
            },
            Visual::Lines => Range {
                start: Pos::new(a.row, 0),
                end: Pos::new(b.row, cols.saturating_sub(1)),
                kind,
            },
            Visual::Block => Range {
                start: anchor,
                end: self.cursor,
                kind,
            },
        })
    }

    /// Runs one command over `buffer`, then scrolls [`Session::top`] so the cursor shows.
    pub fn run<B: Buffer + ?Sized>(&mut self, command: &Command, buffer: &mut B) -> Outcome {
        let (top, height) = buffer.view();
        self.top = top;
        self.height = height.max(1);
        let mut t = Text::new(buffer);
        self.cursor = t.clamp(self.cursor);
        self.anchor = self.anchor.map(|a| t.clamp(a));
        let mut out = Outcome::default();
        match command {
            Command::Move { motion, count } => match motion::go(motion, *count, self, &mut t) {
                Ok(moved) => {
                    if motion.jumps() && moved.to != self.cursor {
                        self.marks.insert('\'', self.cursor);
                    }
                    self.cursor = moved.to;
                    self.want = moved.want;
                    if let Some(top) = moved.top {
                        self.top = top;
                    }
                }
                Err(message) => out.message = message,
            },
            Command::Visual(Some(kind)) => {
                if self.visual.is_none() {
                    self.anchor = Some(self.cursor);
                }
                self.visual = Some(*kind);
            }
            Command::Visual(None) => {
                self.visual = None;
                self.anchor = None;
            }
            Command::SwapEnds => {
                if let Some(anchor) = self.anchor {
                    self.anchor = Some(self.cursor);
                    self.cursor = anchor;
                    self.want = Want::Col(anchor.col);
                }
            }
            Command::Select {
                object,
                inner,
                count,
            } => {
                if let Some((start, end)) = self.grow(&mut t, *object, *inner, *count) {
                    if self.visual.is_none() {
                        self.visual = Some(if object.linewise() {
                            Visual::Lines
                        } else {
                            Visual::Chars
                        });
                    }
                    self.anchor = Some(start);
                    self.cursor = end;
                    self.want = Want::Col(end.col);
                }
            }
            Command::Mark(name) => {
                self.marks.insert(*name, self.cursor);
            }
            Command::Yank => out.yank = self.selection(t.cols),
            Command::YankMotion { motion, count } => {
                let mut probe = self.clone();
                match motion::go(motion, *count, &mut probe, &mut t) {
                    Ok(moved) => {
                        self.search = probe.search;
                        out.yank = reach(&mut t, motion, self.cursor, moved.to);
                    }
                    Err(message) => out.message = message,
                }
            }
            Command::YankObject {
                object,
                inner,
                count,
            } => {
                out.yank = object::find(&mut t, self.cursor, *object, *inner, *count).map(
                    |(start, end)| Range {
                        start,
                        end,
                        kind: if object.linewise() {
                            Visual::Lines
                        } else {
                            Visual::Chars
                        },
                    },
                );
            }
            Command::YankLines { count } => {
                let end = (self.cursor.row + count.max(&1) - 1).min(t.last_row());
                out.yank = Some(Range {
                    start: Pos::new(self.cursor.row, 0),
                    end: Pos::new(end, t.cols - 1),
                    kind: Visual::Lines,
                });
            }
        }
        if command.yanks() && out.yank.is_none() && out.message.is_none() {
            out.message = Some("Nothing to copy".into());
        }
        self.follow(t.total);
        out
    }

    /// The text object a visual `i`/`a` selects: the one around the cursor, or, when the
    /// selection already covers it, the next bigger one (`viwiw` takes the next word, blanks
    /// counting as one, as Vim's `iw` does; `va(a(` the pair around), so repeating it grows the
    /// selection. Its ends take in the selection's.
    fn grow<B: Buffer + ?Sized>(
        &self,
        t: &mut Text<B>,
        object: Object,
        inner: bool,
        count: usize,
    ) -> Option<(Pos, Pos)> {
        let selected = self
            .anchor
            .filter(|_| self.visual.is_some())
            .map(|a| (a.min(self.cursor), a.max(self.cursor)));
        let Some((a, b)) = selected else {
            return object::find(t, self.cursor, object, inner, count);
        };
        let back = self.anchor.is_some_and(|anchor| self.cursor < anchor);
        // A selection that hasn't grown yet is where the object starts, not part of it: one
        // cell, or for a paragraph one line, soft wraps and all (Vim's `current_word` and
        // `current_par`).
        let fresh = match object {
            Object::Paragraph => t.line_rows(a.row).0 == t.line_rows(b.row).0,
            _ => a == b,
        };
        let found = match object {
            Object::Word { .. } | Object::Paragraph if fresh => {
                return object::find(t, self.cursor, object, inner, count);
            }
            // Words and paragraphs grow by the next ones past the cursor's end of the
            // selection, as Vim's `current_word` and `current_par` do.
            Object::Word { .. } => {
                let mut spot = Spot::Cell(self.cursor);
                let at = loop {
                    spot = if back { t.prev(spot)? } else { t.next(spot)? };
                    if let Spot::Cell(p) = spot {
                        break p;
                    }
                };
                object::find(t, at, object, inner, count)?
            }
            Object::Paragraph => {
                let (first, last) = t.line_rows(self.cursor.row);
                let row = if back {
                    first.checked_sub(1)?
                } else if last < t.last_row() {
                    last + 1
                } else {
                    return None;
                };
                object::find(t, Pos::new(row, 0), object, inner, count)?
            }
            // Brackets go a level out at a time until one is bigger than the selection, as Vim's
            // `current_block` does (so `vi(` on one character takes the level around). Quotes
            // take no count: there, as for anything with nothing bigger, the selection stays.
            Object::Bracket(_) | Object::Quote(_) => {
                let covered = |(start, end): (Pos, Pos)| a <= start && end <= b;
                let mut more = count;
                let mut found = object::find(t, self.cursor, object, inner, more)?;
                while covered(found) {
                    more += 1;
                    if more > count + GROW_LIMIT {
                        return None;
                    }
                    found = object::find(t, self.cursor, object, inner, more)?;
                }
                found
            }
        };
        if fresh {
            return Some(found);
        }
        Some((found.0.min(a), found.1.max(b)))
    }

    /// Scrolls the view the least that shows the cursor; to a cursor more than half a view
    /// away, the view centers on it, as Vim does after a long jump.
    fn follow(&mut self, total: usize) {
        let height = self.height;
        let row = self.cursor.row;
        if row < self.top {
            self.top = if self.top - row > height / 2 {
                row.saturating_sub(height / 2)
            } else {
                row
            };
        } else if row >= self.top + height {
            let past = row + 1 - (self.top + height);
            self.top = if past > height / 2 {
                row.saturating_sub(height / 2)
            } else {
                row + 1 - height
            };
        }
        self.top = self.top.min(total.saturating_sub(height));
    }
}

/// What yanking from `from` over `motion` to `to` takes (`:help exclusive`): an exclusive
/// motion leaves its last cell out, and one that ends at a row's start ends at the previous
/// line's end instead. `yw` stops at its line's end rather than taking the next line's start.
fn reach<B: Buffer + ?Sized>(
    t: &mut Text<B>,
    motion: &Motion,
    from: Pos,
    to: Pos,
) -> Option<Range> {
    let (a, b) = (from.min(to), from.max(to));
    match motion.kind() {
        Reach::Lines => Some(Range {
            start: Pos::new(a.row, 0),
            end: Pos::new(b.row, t.cols - 1),
            kind: Visual::Lines,
        }),
        Reach::Inclusive => Some(Range {
            start: a,
            end: b,
            kind: Visual::Chars,
        }),
        Reach::Exclusive => {
            if a == b {
                return None;
            }
            let (a_line, _) = t.line_rows(a.row);
            let (b_line, _) = t.line_rows(b.row);
            let mut end = if matches!(motion, Motion::WordForward { .. }) && b_line > a_line {
                // The line before `b`'s: its end.
                let (_, last) = t.line_rows(b_line - 1);
                t.last_text(last)
            } else {
                let spot = t.spot(b);
                match t.prev(spot)? {
                    Spot::End(y) => t.last_text(y),
                    s => t.pos(s),
                }
            };
            if end < a {
                end = a;
            }
            Some(Range {
                start: a,
                end,
                kind: Visual::Chars,
            })
        }
    }
}
