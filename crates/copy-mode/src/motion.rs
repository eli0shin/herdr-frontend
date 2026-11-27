//! Where each [`Motion`] goes, as Vim's `normal.c` and `textobject.c` move: words by Vim's
//! character classes (`fwd_word`, `bck_word`, `end_word`, `bckend_word`), paragraphs by empty
//! lines (`findpar`), `%` by bracket nesting, and the view keys by rows.

use crate::buffer::{Buffer, Pos, Spot, Text, is_word};
use crate::command::{FindKind, Motion};
use crate::engine::{Search, Session, Want};

/// Where a motion leaves the cursor, the column `j` and `k` keep after it, and the view's new
/// first row for the motions that scroll.
pub(crate) struct Moved {
    pub to: Pos,
    pub want: Want,
    pub top: Option<usize>,
}

impl Moved {
    fn to(to: Pos) -> Moved {
        Moved {
            to,
            want: Want::Col(to.col),
            top: None,
        }
    }
}

/// Why a motion didn't move: `Some` says so in the bar ("Pattern not found"); `None` is a
/// motion with nowhere to go, which Vim answers with a beep and this with nothing.
pub(crate) type Stuck = Option<String>;

/// Where `motion` goes from the session's cursor. A search remembers its query in the session.
pub(crate) fn go<B: Buffer + ?Sized>(
    motion: &Motion,
    count: Option<usize>,
    s: &mut Session,
    t: &mut Text<B>,
) -> Result<Moved, Stuck> {
    let n = count.unwrap_or(1).max(1);
    let c = t.clamp(s.cursor);
    let (top, height) = (s.top, s.height.max(1));
    let last = t.last_row();
    let want_at = |t: &mut Text<B>, row: usize, want: Want| match want {
        Want::Col(col) => Pos::new(row, t.head(row, col.min(t.cols - 1))),
        Want::End => t.last_text(row),
    };
    use Motion as M;
    Ok(match motion {
        M::Left => {
            let mut col = c.col;
            for _ in 0..n {
                let row = t.row(c.row);
                match (0..col).rev().find(|&x| row.cells[x].is_some()) {
                    Some(x) => col = x,
                    None => break,
                }
            }
            Moved::to(Pos::new(c.row, col))
        }
        M::Right => {
            let mut col = c.col;
            for _ in 0..n {
                let row = t.row(c.row);
                match (col + 1..row.cells.len()).find(|&x| row.cells[x].is_some()) {
                    Some(x) => col = x,
                    None => break,
                }
            }
            Moved::to(Pos::new(c.row, col))
        }
        M::Up | M::Down => {
            let row = if *motion == M::Up {
                c.row.saturating_sub(n)
            } else {
                (c.row + n).min(last)
            };
            Moved {
                to: want_at(t, row, s.want),
                want: s.want,
                top: None,
            }
        }
        M::LineStart => Moved::to(Pos::new(c.row, 0)),
        M::FirstText => Moved::to(t.first_text(c.row)),
        M::LineEnd => Moved {
            to: t.last_text((c.row + n - 1).min(last)),
            want: Want::End,
            top: None,
        },
        M::Column => Moved::to(Pos::new(c.row, t.head(c.row, (n - 1).min(t.cols - 1)))),
        M::NextLine => Moved::to(t.first_text((c.row + n).min(last))),
        M::PrevLine => Moved::to(t.first_text(c.row.saturating_sub(n))),
        M::WordForward { big } => words(t, c, n, |t, s| word_forward(t, s, *big)),
        M::WordBack { big } => words(t, c, n, |t, s| word_back(t, s, *big)),
        M::WordEnd { big } => words(t, c, n, |t, s| word_end(t, s, *big)),
        M::WordEndBack { big } => words(t, c, n, |t, s| word_end_back(t, s, *big)),
        M::Top => Moved::to(t.first_text(count.map_or(0, |n| n.max(1) - 1).min(last))),
        M::Bottom => {
            let row = match count {
                Some(n) => (n.max(1) - 1).min(last),
                None => last_with_text(t),
            };
            Moved::to(t.first_text(row))
        }
        M::ViewTop | M::ViewMiddle | M::ViewBottom => {
            let bottom = (top + height - 1).min(last);
            let row = match motion {
                M::ViewTop => (top + n - 1).min(bottom),
                M::ViewBottom => bottom.saturating_sub(n - 1).max(top),
                _ => top + (bottom - top) / 2,
            };
            Moved::to(t.first_text(row))
        }
        M::Find { kind, ch, repeat } => Moved::to(find(t, c, *kind, *ch, *repeat, n).ok_or(None)?),
        M::Match => match count {
            Some(percent) => {
                let row = (percent.min(100) * t.total).div_ceil(100).saturating_sub(1);
                Moved::to(t.first_text(row.min(last)))
            }
            None => Moved::to(matching(t, c).ok_or(None)?),
        },
        M::ParagraphForward => Moved::to(paragraph(t, c, true, n)),
        M::ParagraphBack => Moved::to(paragraph(t, c, false, n)),
        M::HalfPageDown | M::HalfPageUp | M::PageDown | M::PageUp | M::ScrollDown | M::ScrollUp => {
            let down = matches!(motion, M::HalfPageDown | M::PageDown | M::ScrollDown);
            let by = match motion {
                M::HalfPageDown | M::HalfPageUp => count.unwrap_or(height / 2).max(1),
                M::PageDown | M::PageUp => n * height.saturating_sub(2).max(1),
                _ => n,
            };
            let max_top = t.total.saturating_sub(height);
            let new_top = if down {
                (top + by).min(max_top)
            } else {
                top.saturating_sub(by)
            };
            let view_bottom = (new_top + height - 1).min(last);
            let row = match motion {
                M::HalfPageDown => (c.row + by).min(last),
                M::HalfPageUp => c.row.saturating_sub(by),
                // At the end there's nowhere to scroll: the cursor goes to the edge instead.
                M::PageDown if new_top == top => view_bottom,
                M::PageUp if new_top == top => new_top,
                _ => c.row,
            };
            Moved {
                to: want_at(t, row.clamp(new_top, view_bottom), s.want),
                want: s.want,
                top: Some(new_top),
            }
        }
        M::Search { query, backward } => {
            let search = if query.is_empty() {
                let Some(last) = &s.search else {
                    return Err(Some("No previous search".into()));
                };
                Search {
                    backward: *backward,
                    ..last.clone()
                }
            } else {
                Search {
                    query: query.clone(),
                    backward: *backward,
                    whole: false,
                }
            };
            s.search = Some(search.clone());
            Moved::to(search_from(t, c, &search, search.backward, n)?)
        }
        M::SearchNext { reverse } => {
            let Some(search) = s.search.clone() else {
                return Err(Some("No previous search".into()));
            };
            Moved::to(search_from(t, c, &search, search.backward != *reverse, n)?)
        }
        M::SearchWord { backward } => {
            let Some(word) = word_at(t, c) else {
                return Err(Some("No word under the cursor".into()));
            };
            let search = Search {
                query: word,
                backward: *backward,
                whole: true,
            };
            s.search = Some(search.clone());
            Moved::to(search_from(t, c, &search, *backward, n)?)
        }
        M::Mark { name, exact } => {
            let name = if *name == '`' { '\'' } else { *name };
            let Some(&at) = s.marks.get(&name) else {
                return Err(Some(format!("Mark not set: {name}")));
            };
            let at = t.clamp(at);
            Moved::to(if *exact { at } else { t.first_text(at.row) })
        }
    })
}

/// A word motion `n` times from `c`.
fn words<B: Buffer + ?Sized>(
    t: &mut Text<B>,
    c: Pos,
    n: usize,
    step: impl Fn(&mut Text<B>, Spot) -> Spot,
) -> Moved {
    let mut spot = t.spot(c);
    for _ in 0..n {
        spot = step(t, spot);
    }
    Moved::to(t.pos(spot))
}

/// `w`: past this word and the blanks after it, stopping at an empty line (`fwd_word`).
pub(crate) fn word_forward<B: Buffer + ?Sized>(t: &mut Text<B>, start: Spot, big: bool) -> Spot {
    let class = t.class_at(start, big);
    let Some(mut s) = t.next(start) else {
        return start;
    };
    if class != 0 {
        while t.class_at(s, big) == class {
            match t.next(s) {
                Some(next) => s = next,
                None => return s,
            }
        }
    }
    while t.class_at(s, big) == 0 {
        if t.empty_line(s) {
            break;
        }
        match t.next(s) {
            Some(next) => s = next,
            None => return s,
        }
    }
    s
}

/// `b`: back over blanks (stopping at an empty line), then to the start of the word
/// (`bck_word`).
pub(crate) fn word_back<B: Buffer + ?Sized>(t: &mut Text<B>, start: Spot, big: bool) -> Spot {
    let Some(mut s) = t.prev(start) else {
        return start;
    };
    while t.class_at(s, big) == 0 {
        if t.empty_line(s) {
            return s;
        }
        match t.prev(s) {
            Some(prev) => s = prev,
            None => return s,
        }
    }
    let class = t.class_at(s, big);
    while let Some(prev) = t.prev(s) {
        if t.class_at(prev, big) != class {
            break;
        }
        s = prev;
    }
    s
}

/// `e`: to the end of this word, or past blanks to the end of the next (`end_word`).
pub(crate) fn word_end<B: Buffer + ?Sized>(t: &mut Text<B>, start: Spot, big: bool) -> Spot {
    let class = t.class_at(start, big);
    let Some(mut s) = t.next(start) else {
        return start;
    };
    if t.class_at(s, big) != class || class == 0 {
        while t.class_at(s, big) == 0 {
            match t.next(s) {
                Some(next) => s = next,
                None => return start,
            }
        }
    }
    let class = t.class_at(s, big);
    while let Some(next) = t.next(s) {
        if t.class_at(next, big) != class {
            break;
        }
        s = next;
    }
    s
}

/// `ge`: back before this word's start, then over blanks to the end of the word before
/// (`bckend_word`).
pub(crate) fn word_end_back<B: Buffer + ?Sized>(t: &mut Text<B>, start: Spot, big: bool) -> Spot {
    let class = t.class_at(start, big);
    let Some(mut s) = t.prev(start) else {
        return start;
    };
    if class != 0 {
        while t.class_at(s, big) == class {
            match t.prev(s) {
                Some(prev) => s = prev,
                None => return s,
            }
        }
    }
    while t.class_at(s, big) == 0 {
        if t.empty_line(s) {
            break;
        }
        match t.prev(s) {
            Some(prev) => s = prev,
            None => return s,
        }
    }
    s
}

/// The last row with text, or the last row when none has any.
fn last_with_text<B: Buffer + ?Sized>(t: &mut Text<B>) -> usize {
    let last = t.last_row();
    (0..=last)
        .rev()
        .find(|&y| t.row(y).last_text().is_some())
        .unwrap_or(last)
}

/// `f`, `F`, `t`, `T`: the `n`th `ch` along the cursor's line.
fn find<B: Buffer + ?Sized>(
    t: &mut Text<B>,
    c: Pos,
    kind: FindKind,
    ch: char,
    repeat: bool,
    n: usize,
) -> Option<Pos> {
    let line = t.line(c.row);
    let at = line.partition_point(|(p, _)| *p < c);
    let on = line.get(at).is_some_and(|(p, _)| *p == c);
    let till = matches!(kind, FindKind::Till | FindKind::TillBack);
    let skip = usize::from(till && repeat);
    if kind.forward() {
        let from = at + usize::from(on) + skip;
        let j = (from..line.len()).filter(|&i| line[i].1 == ch).nth(n - 1)?;
        // `from` is past the cursor's cell, so `j - 1` is at worst the cursor's own.
        Some(line[if till { j - 1 } else { j }].0)
    } else {
        let below = at.checked_sub(1 + skip)?;
        let j = (0..=below).rev().filter(|&i| line[i].1 == ch).nth(n - 1)?;
        let found = line[j].0;
        if !till {
            return Some(found);
        }
        // Just after it: the line's next cell, or, past the line's last text (where a prompt's
        // cursor starts), the row's next cell.
        Some(match line.get(j + 1) {
            Some(&(next, _)) => next,
            None => {
                let row = t.row(found.row);
                let next = (found.col + 1..row.cells.len()).find(|&x| row.cells[x].is_some());
                Pos::new(found.row, next.unwrap_or(found.col))
            }
        })
    }
}

const OPEN: [char; 3] = ['(', '[', '{'];
const CLOSE: [char; 3] = [')', ']', '}'];

/// `%`: the first bracket at or after the cursor on its line, and the one matching it.
fn matching<B: Buffer + ?Sized>(t: &mut Text<B>, c: Pos) -> Option<Pos> {
    let line = t.line(c.row);
    let at = line.partition_point(|(p, _)| *p < c);
    let (from, bracket) = line[at..]
        .iter()
        .find(|(_, ch)| OPEN.contains(ch) || CLOSE.contains(ch))
        .copied()?;
    let (open, close, forward) = match OPEN.iter().position(|&o| o == bracket) {
        Some(i) => (bracket, CLOSE[i], true),
        None => {
            let i = CLOSE.iter().position(|&x| x == bracket)?;
            (OPEN[i], bracket, false)
        }
    };
    pair(t, Spot::Cell(from), open, close, forward).map(|s| t.pos(s))
}

/// From a bracket, the one that closes (or opens) it, nesting counted.
pub(crate) fn pair<B: Buffer + ?Sized>(
    t: &mut Text<B>,
    from: Spot,
    open: char,
    close: char,
    forward: bool,
) -> Option<Spot> {
    let (inward, outward) = if forward {
        (open, close)
    } else {
        (close, open)
    };
    let mut depth = 0usize;
    let mut s = from;
    loop {
        s = if forward { t.next(s)? } else { t.prev(s)? };
        let ch = t.char_at(s);
        if ch == inward {
            depth += 1;
        } else if ch == outward {
            if depth == 0 {
                return Some(s);
            }
            depth -= 1;
        }
    }
}

/// `}` and `{`: `n` times to the next empty line after a line with text (`findpar`). Past the
/// last one, `}` stops at the end of the last line and `{` at the first.
fn paragraph<B: Buffer + ?Sized>(t: &mut Text<B>, c: Pos, forward: bool, n: usize) -> Pos {
    let mut y = t.line_rows(c.row).0;
    for _ in 0..n {
        let mut skipped = false;
        let mut first = true;
        loop {
            let empty = t.line_empty(y);
            if !empty {
                skipped = true;
            }
            if !first && skipped && empty {
                break;
            }
            let next = if forward {
                let (_, end) = t.line_rows(y);
                (end < t.last_row()).then_some(end + 1)
            } else {
                (y > 0).then(|| t.line_rows(y - 1).0)
            };
            match next {
                Some(next) => y = next,
                None => {
                    return if forward {
                        let (_, end) = t.line_rows(y);
                        t.last_text(end)
                    } else {
                        Pos::new(y, 0)
                    };
                }
            }
            first = false;
        }
    }
    Pos::new(y, 0)
}

/// The keyword under the cursor, or the next one on its line (`*`, `#`).
fn word_at<B: Buffer + ?Sized>(t: &mut Text<B>, c: Pos) -> Option<String> {
    let line = t.line(c.row);
    let at = line.partition_point(|(p, _)| *p < c);
    let hit = (at..line.len()).find(|&i| is_word(line[i].1))?;
    let mut start = hit;
    while start > 0 && is_word(line[start - 1].1) {
        start -= 1;
    }
    let mut end = hit;
    while end + 1 < line.len() && is_word(line[end + 1].1) {
        end += 1;
    }
    Some(line[start..=end].iter().map(|(_, ch)| *ch).collect())
}

/// The `n`th match from `c` (not counting one starting at `c`), wrapping around the buffer.
fn search_from<B: Buffer + ?Sized>(
    t: &mut Text<B>,
    c: Pos,
    search: &Search,
    backward: bool,
    n: usize,
) -> Result<Pos, Stuck> {
    let mut starts: Vec<Pos> = t
        .buffer
        .matches(&search.query)
        .into_iter()
        .filter(|&(start, end)| !search.whole || whole_word(t, start, end))
        .map(|(start, _)| start)
        .collect();
    starts.sort();
    starts.dedup();
    if starts.is_empty() {
        return Err(Some(format!("Pattern not found: {}", search.query)));
    }
    let len = starts.len();
    // The index of the first match after the cursor, or of the last before it.
    let i = if backward {
        let before = starts.partition_point(|p| *p < c);
        (before + len - 1 - (n - 1) % len) % len
    } else {
        let after = starts.partition_point(|p| *p <= c);
        (after + (n - 1)) % len
    };
    Ok(starts[i])
}

/// No word character touches the match on either side.
fn whole_word<B: Buffer + ?Sized>(t: &mut Text<B>, start: Pos, end: Pos) -> bool {
    let before = t.prev(Spot::Cell(start));
    let after = t.next(Spot::Cell(end));
    let touches = |t: &mut Text<B>, s: Option<Spot>| s.is_some_and(|s| is_word(t.char_at(s)));
    !touches(t, before) && !touches(t, after)
}
