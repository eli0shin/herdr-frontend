//! [`Session`]: copy mode's cursor, selection, marks and last search, and how each
//! [`Command`] changes them over a [`Buffer`].
//!
//! The terminal keeps the session's positions as tracked references, so they follow the text
//! as output scrolls it and history is pruned; before each command it puts them in the session,
//! and after, it reads them back, scrolls to [`Session::top`], installs the selection and
//! yanks what [`Outcome::yank`] names.

use std::collections::BTreeMap;

use crate::buffer::{Buffer, Pos, Spot, Text};
use crate::command::{Command, Motion, Reach, Visual};
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
                let found = object::find(&mut t, self.cursor, *object, *inner, *count);
                if let Some((start, end)) = found {
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
        self.follow(t.total);
        out
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
