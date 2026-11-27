//! Text objects (`:help text-objects`): where `iw`, `aW`, `ip`, `i"`, `a(` and the rest start
//! and end around the cursor, both ends included.

use crate::buffer::{Buffer, Pos, Spot, Text, class};
use crate::command::Object;
use crate::motion::pair;

/// The object around `c`, `count` deep: its first and last cells. `None` where there's none
/// (no quotes on the line, no brackets around the cursor, the cursor past a line's text).
pub(crate) fn find<B: Buffer + ?Sized>(
    t: &mut Text<B>,
    c: Pos,
    object: Object,
    inner: bool,
    count: usize,
) -> Option<(Pos, Pos)> {
    let count = count.max(1);
    match object {
        Object::Word { big } => word(t, c, big, inner, count),
        Object::Paragraph => paragraph(t, c, inner, count),
        Object::Quote(q) => quote(t, c, q, inner),
        Object::Bracket(open) => bracket(t, c, open, inner, count),
    }
}

/// `iw` and `aw` on the cursor's line: runs of one class; a run of blanks counts as a word for
/// `iw`, and `aw` takes the blanks after the word (or, with none after, before it).
fn word<B: Buffer + ?Sized>(
    t: &mut Text<B>,
    c: Pos,
    big: bool,
    inner: bool,
    count: usize,
) -> Option<(Pos, Pos)> {
    let line = t.line(c.row);
    let at = line.iter().position(|(p, _)| *p == c)?;
    let cls = |i: usize| class(line[i].1, big);
    let run_end = |i: usize| {
        let mut j = i;
        while j + 1 < line.len() && cls(j + 1) == cls(i) {
            j += 1;
        }
        j
    };
    let mut start = at;
    while start > 0 && cls(start - 1) == cls(at) {
        start -= 1;
    }
    let mut end = run_end(at);
    if inner {
        for _ in 1..count {
            if end + 1 >= line.len() {
                break;
            }
            end = run_end(end + 1);
        }
        return Some((line[start].0, line[end].0));
    }
    let on_blank = cls(at) == 0;
    for i in 0..count {
        if i > 0 && end + 1 >= line.len() {
            break;
        }
        if i > 0 {
            end = run_end(end + 1);
        }
        // A word takes the blanks after it; blanks take the word after them.
        if end + 1 < line.len() {
            let next = run_end(end + 1);
            if on_blank || cls(end + 1) == 0 {
                end = next;
            } else if i == 0 && !on_blank {
                // No blanks after the word: `aw` takes the ones before instead.
                while start > 0 && cls(start - 1) == 0 {
                    start -= 1;
                }
            }
        } else if i == 0 && !on_blank {
            while start > 0 && cls(start - 1) == 0 {
                start -= 1;
            }
        }
    }
    Some((line[start].0, line[end].0))
}

/// `ip`: the run of lines with text (or of empty lines) the cursor is in; `ap` adds the empty
/// lines after it (or, with none after, before it).
fn paragraph<B: Buffer + ?Sized>(
    t: &mut Text<B>,
    c: Pos,
    inner: bool,
    count: usize,
) -> Option<(Pos, Pos)> {
    let last = t.last_row();
    let (mut start, mut end) = t.line_rows(c.row);
    let empty = t.line_empty(start);
    // Lines above and below while they're the same kind as `kind`.
    let grow_down = |t: &mut Text<B>, mut end: usize, kind: bool| {
        while end < last && t.line_empty(end + 1) == kind {
            end = t.line_rows(end + 1).1;
        }
        end
    };
    while start > 0 && t.line_empty(start - 1) == empty {
        start = t.line_rows(start - 1).0;
    }
    end = grow_down(t, end, empty);
    let mut kind = empty;
    let runs = if inner { count } else { count * 2 };
    let mut taken = 1;
    while taken < runs && end < last {
        kind = !kind;
        let next = t.line_rows(end + 1).1;
        end = grow_down(t, next, kind);
        taken += 1;
    }
    if !inner && taken < runs && !empty {
        // No empty lines after the paragraph: take the ones before it.
        while start > 0 && t.line_empty(start - 1) {
            start = t.line_rows(start - 1).0;
        }
    }
    Some((Pos::new(start, 0), Pos::new(end, t.cols - 1)))
}

/// `i"` and `a"` on the cursor's line: the quotes around the cursor, or the next pair after
/// it. `a` takes the quotes and the blanks after them (or, with none, before them).
fn quote<B: Buffer + ?Sized>(t: &mut Text<B>, c: Pos, q: char, inner: bool) -> Option<(Pos, Pos)> {
    let line = t.line(c.row);
    let at = line.partition_point(|(p, _)| *p < c);
    let quotes: Vec<usize> = (0..line.len())
        .filter(|&i| line[i].1 == q && (i == 0 || line[i - 1].1 != '\\'))
        .collect();
    let before = quotes.iter().filter(|&&i| i < at).count();
    let on = quotes.contains(&at);
    // An odd count before the cursor means it's inside a pair (or on its closing quote).
    let (open, close) = if before % 2 == 1 {
        (quotes[before - 1], *quotes.get(before)?)
    } else if on {
        (at, *quotes.get(before + 1)?)
    } else {
        let first = *quotes.get(before)?;
        (first, *quotes.get(before + 1)?)
    };
    if inner {
        if close == open + 1 {
            return None;
        }
        return Some((line[open + 1].0, line[close - 1].0));
    }
    let blank = |i: usize| class(line[i].1, false) == 0;
    let mut end = close;
    while end + 1 < line.len() && blank(end + 1) {
        end += 1;
    }
    let mut start = open;
    if end == close {
        while start > 0 && blank(start - 1) {
            start -= 1;
        }
    }
    Some((line[start].0, line[end].0))
}

/// `i(` and `a(` (and `[`, `{`, `<`): the brackets around the cursor, `count` levels out, over
/// as many lines as they span. `i` leaves out a line break right inside either bracket.
fn bracket<B: Buffer + ?Sized>(
    t: &mut Text<B>,
    c: Pos,
    open: char,
    inner: bool,
    count: usize,
) -> Option<(Pos, Pos)> {
    let close = match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        _ => return None,
    };
    let start = t.spot(c);
    let here = t.char_at(start);
    // On an opening bracket, that's the first level. Elsewhere, the unmatched one before the
    // cursor is (a closing bracket under the cursor counts from the cell before it, so it finds
    // its own pair).
    let mut from = if here == open {
        start
    } else {
        pair(t, start, open, close, false)?
    };
    for _ in 1..count {
        from = pair(t, from, open, close, false)?;
    }
    let to = pair(t, from, open, close, true)?;
    if !inner {
        return Some((t.pos(from), t.pos(to)));
    }
    let mut first = t.next(from)?;
    if matches!(first, Spot::End(_)) {
        first = t.next(first)?;
    }
    let mut last = t.prev(to)?;
    if matches!(last, Spot::End(_)) && last != first {
        last = t.prev(last)?;
    }
    if first == to {
        return None;
    }
    Some((t.pos(first), t.pos(last)))
}
