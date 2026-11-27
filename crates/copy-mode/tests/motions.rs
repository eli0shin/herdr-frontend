//! Copy mode's keys over fixture buffers: the grammar reads the keys, the engine runs each
//! command, as the model and the terminal's worker do.

use copy_mode::fixture::Fixture;
use copy_mode::{Key, Pos, Range, Session, Vim, Visual};

/// The keys in Vim's notation: characters, and `<Esc>`, `<CR>`, `<BS>`, `<C-x>`.
fn keys(text: &str) -> Vec<Key> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c == '<'
            && let Some(end) = rest.find('>')
            && end > 1
        {
            let name = &rest[1..end];
            let key = match name {
                "Esc" => Some(Key::Escape),
                "CR" => Some(Key::Enter),
                "BS" => Some(Key::Backspace),
                "Up" => Some(Key::Up),
                "Down" => Some(Key::Down),
                "Left" => Some(Key::Left),
                "Right" => Some(Key::Right),
                "Home" => Some(Key::Home),
                "End" => Some(Key::End),
                "PageUp" => Some(Key::PageUp),
                "PageDown" => Some(Key::PageDown),
                _ => name
                    .strip_prefix("C-")
                    .and_then(|l| l.chars().next())
                    .map(Key::Ctrl),
            };
            if let Some(key) = key {
                out.push(key);
                rest = &rest[end + 1..];
                continue;
            }
        }
        out.push(Key::Char(c));
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// A copy mode over a fixture: what it did, as the terminal would see it.
struct Copy {
    buffer: Fixture,
    vim: Vim,
    session: Session,
    yanked: Option<String>,
    message: Option<String>,
    left: bool,
}

impl Copy {
    fn new(buffer: Fixture, at: (usize, usize)) -> Copy {
        Copy {
            buffer,
            vim: Vim::default(),
            session: Session::new(Pos::new(at.0, at.1)),
            yanked: None,
            message: None,
            left: false,
        }
    }

    fn lines(cols: usize, lines: &[&str], at: (usize, usize)) -> Copy {
        Copy::new(Fixture::new(cols, lines), at)
    }

    /// Types `text`; the view follows the cursor, as the terminal scrolls it.
    fn keys(&mut self, text: &str) -> &mut Copy {
        for key in keys(text) {
            assert!(!self.left, "a key after copy mode ended: {text}");
            let step = self.vim.key(key);
            for command in &step.commands {
                let out = self.session.run(command, &mut self.buffer);
                let height = self.buffer.view.1;
                self.buffer.view = (self.session.top, height);
                if let Some(range) = out.yank {
                    self.yanked = Some(self.text(range));
                }
                self.message = out.message;
            }
            self.left |= step.leave;
        }
        self
    }

    fn text(&self, range: Range) -> String {
        match range.kind {
            Visual::Block => {
                let (top, bottom) = (
                    range.start.row.min(range.end.row),
                    range.start.row.max(range.end.row),
                );
                let (left, right) = (
                    range.start.col.min(range.end.col),
                    range.start.col.max(range.end.col),
                );
                (top..=bottom)
                    .map(|y| self.buffer.text(Pos::new(y, left), Pos::new(y, right)))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            _ => self.buffer.text(range.start, range.end),
        }
    }

    fn at(&self) -> (usize, usize) {
        (self.session.cursor.row, self.session.cursor.col)
    }

    /// The selection's text.
    fn selected(&self) -> Option<String> {
        self.session.selection(80).map(|r| self.text(r))
    }
}

/// Types `text` from `at` and says where the cursor ends.
fn after(lines: &[&str], at: (usize, usize), text: &str) -> (usize, usize) {
    Copy::lines(80, lines, at).keys(text).at()
}

#[test]
fn hjkl_move_by_cells_and_rows_with_counts_keeping_the_column() {
    let lines = ["alpha beta", "x", "", "gamma delta epsilon"];
    assert_eq!(after(&lines, (0, 0), "l"), (0, 1));
    assert_eq!(after(&lines, (0, 0), "3l"), (0, 3));
    assert_eq!(after(&lines, (0, 5), "2h"), (0, 3));
    assert_eq!(after(&lines, (0, 1), "5h"), (0, 0), "stops at the edge");
    assert_eq!(after(&lines, (0, 0), "j"), (1, 0));
    assert_eq!(after(&lines, (3, 2), "10k"), (0, 2));
    assert_eq!(after(&lines, (0, 0), "9j"), (3, 0), "stops at the last row");
    assert_eq!(after(&lines, (0, 0), "<Down><Right><Up><Left>"), (0, 0));
    // The column is kept across a shorter row, and `$` sticks to rows' ends.
    let mut c = Copy::lines(80, &lines, (3, 12));
    assert_eq!(c.keys("2k").at(), (1, 12));
    assert_eq!(c.keys("k").at(), (0, 12));
    assert_eq!(c.keys("3j").at(), (3, 12));
    assert_eq!(c.keys("$").at(), (3, 18));
    assert_eq!(c.keys("3k").at(), (0, 9));
    assert_eq!(c.keys("j").at(), (1, 0));
}

#[test]
fn row_starts_first_text_ends_and_columns() {
    let lines = ["   indented text  ", "two"];
    assert_eq!(after(&lines, (0, 10), "0"), (0, 0));
    assert_eq!(after(&lines, (0, 10), "<Home>"), (0, 0));
    assert_eq!(after(&lines, (0, 10), "^"), (0, 3));
    assert_eq!(after(&lines, (0, 0), "$"), (0, 15));
    assert_eq!(after(&lines, (0, 0), "<End>"), (0, 15));
    assert_eq!(after(&lines, (0, 0), "2$"), (1, 2));
    assert_eq!(after(&lines, (0, 0), "7|"), (0, 6));
    assert_eq!(after(&lines, (0, 0), "+"), (1, 0));
    assert_eq!(after(&lines, (1, 2), "-"), (0, 3));
    assert_eq!(after(&lines, (0, 9), "g0"), (0, 0));
}

#[test]
fn small_words_stop_at_punctuation_and_big_words_at_blanks() {
    let lines = ["foo.bar baz-qux", "", "  next(line)"];
    assert_eq!(after(&lines, (0, 0), "w"), (0, 3));
    assert_eq!(after(&lines, (0, 0), "2w"), (0, 4));
    assert_eq!(after(&lines, (0, 0), "W"), (0, 8));
    assert_eq!(after(&lines, (0, 8), "w"), (0, 11));
    assert_eq!(
        after(&lines, (0, 8), "W"),
        (1, 0),
        "an empty line is a word"
    );
    assert_eq!(after(&lines, (1, 0), "w"), (2, 2));
    assert_eq!(after(&lines, (2, 2), "3w"), (2, 11));
    assert_eq!(after(&lines, (2, 2), "b"), (1, 0));
    assert_eq!(after(&lines, (1, 0), "b"), (0, 12));
    assert_eq!(after(&lines, (1, 0), "B"), (0, 8));
    assert_eq!(after(&lines, (0, 14), "b"), (0, 12));
    assert_eq!(after(&lines, (0, 0), "e"), (0, 2));
    assert_eq!(after(&lines, (0, 2), "e"), (0, 3));
    assert_eq!(after(&lines, (0, 0), "E"), (0, 6));
    assert_eq!(after(&lines, (0, 8), "E"), (0, 14));
    assert_eq!(after(&lines, (0, 14), "e"), (2, 5), "e skips empty lines");
    assert_eq!(
        after(&lines, (2, 2), "ge"),
        (1, 0),
        "ge stops at an empty line too"
    );
    assert_eq!(after(&lines, (1, 0), "ge"), (0, 14));
    assert_eq!(after(&lines, (0, 12), "ge"), (0, 11));
    assert_eq!(after(&lines, (0, 12), "gE"), (0, 6));
    // The last word: `w` stops on the last character.
    assert_eq!(after(&lines, (2, 7), "5w"), (2, 11));
    assert_eq!(after(&lines, (0, 0), "5b"), (0, 0));
}

#[test]
fn words_run_on_across_a_soft_wrap() {
    // Ten columns: "one two three four" wraps after "one two th".
    let c = Copy::lines(10, &["one two three four", "five"], (0, 4));
    assert_eq!(
        c.buffer.clone().text(Pos::new(0, 0), Pos::new(1, 9)),
        "one two three four"
    );
    let mut c = c;
    assert_eq!(c.keys("w").at(), (0, 8), "three starts on the first row");
    assert_eq!(c.keys("e").at(), (1, 2), "and ends on the next");
    assert_eq!(c.keys("w").at(), (1, 4));
    assert_eq!(c.keys("w").at(), (2, 0));
    assert_eq!(c.keys("b").at(), (1, 4));
    assert_eq!(c.keys("b").at(), (0, 8));
    // `$` and `0` go by the row, as j and k do.
    assert_eq!(c.keys("$").at(), (0, 9));
    assert_eq!(c.keys("j0").at(), (1, 0));
}

#[test]
fn wide_characters_are_one_place_for_the_cursor() {
    // 日本 takes four cells.
    let mut c = Copy::lines(20, &["a日本b c"], (0, 0));
    assert_eq!(c.keys("l").at(), (0, 1));
    assert_eq!(c.keys("l").at(), (0, 3));
    assert_eq!(c.keys("l").at(), (0, 5));
    assert_eq!(c.keys("h").at(), (0, 3));
    assert_eq!(c.keys("0w").at(), (0, 1), "another script is another word");
    assert_eq!(c.keys("w").at(), (0, 5));
    assert_eq!(c.keys("$").at(), (0, 7));
}

#[test]
fn top_bottom_and_line_numbers() {
    let lines = ["first", "  second", "third", "", ""];
    assert_eq!(after(&lines, (2, 3), "gg"), (0, 0));
    assert_eq!(after(&lines, (0, 3), "G"), (2, 0), "the last row with text");
    assert_eq!(after(&lines, (0, 3), "2G"), (1, 2));
    assert_eq!(after(&lines, (0, 3), "2gg"), (1, 2));
    assert_eq!(after(&lines, (0, 0), "99G"), (4, 0));
    assert_eq!(after(&lines, (0, 0), "60%"), (2, 0));
}

#[test]
fn view_top_middle_and_bottom() {
    let lines: Vec<String> = (1..=30).map(|i| format!("  line {i}")).collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let view = |text: &str| {
        let mut c = Copy::new(Fixture::new(40, &lines).with_view(10, 10), (12, 0));
        c.keys(text).at()
    };
    assert_eq!(view("H"), (10, 2));
    assert_eq!(view("3H"), (12, 2));
    assert_eq!(view("M"), (14, 2));
    assert_eq!(view("L"), (19, 2));
    assert_eq!(view("2L"), (18, 2));
}

#[test]
fn find_and_till_along_the_line_with_repeats() {
    let lines = ["a,b,c,d", "e,f"];
    assert_eq!(after(&lines, (0, 0), "f,"), (0, 1));
    assert_eq!(after(&lines, (0, 0), "2f,"), (0, 3));
    assert_eq!(after(&lines, (0, 0), "f,;"), (0, 3));
    assert_eq!(after(&lines, (0, 0), "f,;;,"), (0, 3));
    assert_eq!(
        after(&lines, (0, 0), "t,"),
        (0, 0),
        "just before the next one"
    );
    assert_eq!(
        after(&lines, (0, 0), "t,;"),
        (0, 2),
        "; skips the one right there"
    );
    assert_eq!(after(&lines, (0, 6), "F,"), (0, 5));
    assert_eq!(after(&lines, (0, 6), "T,"), (0, 6));
    assert_eq!(after(&lines, (0, 6), "T,;"), (0, 4));
    assert_eq!(
        after(&lines, (0, 6), "fz"),
        (0, 6),
        "not on the line: no move"
    );
    assert_eq!(after(&lines, (0, 0), "fe"), (0, 0), "only this line");
    // Past the line's text, as at a prompt: T lands just after the match.
    assert_eq!(after(&lines, (1, 6), "Tf"), (1, 3));
    assert_eq!(after(&lines, (1, 6), "Ff"), (1, 2));
    assert_eq!(after(&lines, (1, 6), "T,"), (1, 2));
    assert_eq!(
        after(&lines, (1, 6), "tf"),
        (1, 6),
        "nothing after the cursor"
    );
}

#[test]
fn percent_matches_brackets_across_lines() {
    let lines = ["fn main() {", "    let a = [1, (2)];", "}"];
    assert_eq!(
        after(&lines, (0, 0), "%"),
        (0, 8),
        "the first bracket on the line"
    );
    assert_eq!(after(&lines, (0, 10), "%"), (2, 0));
    assert_eq!(after(&lines, (2, 0), "%"), (0, 10));
    assert_eq!(after(&lines, (1, 12), "%"), (1, 19));
    assert_eq!(after(&lines, (1, 16), "%"), (1, 18));
    assert_eq!(
        after(&lines, (1, 0), "x%"),
        (1, 19),
        "a key copy mode doesn't use"
    );
}

#[test]
fn paragraphs_go_to_empty_lines() {
    let lines = ["one", "two", "", "three", "four", "", "", "five six"];
    assert_eq!(after(&lines, (0, 1), "}"), (2, 0));
    assert_eq!(after(&lines, (0, 1), "2}"), (5, 0));
    assert_eq!(
        after(&lines, (5, 0), "}"),
        (7, 7),
        "past the last: the end of the last line"
    );
    assert_eq!(after(&lines, (7, 3), "{"), (6, 0));
    assert_eq!(after(&lines, (4, 0), "{"), (2, 0));
    assert_eq!(after(&lines, (4, 0), "3{"), (0, 0));
}

#[test]
fn half_pages_pages_and_lines_move_the_view_and_the_cursor() {
    let lines: Vec<String> = (0..100).map(|i| format!("row {i}")).collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut c = Copy::new(Fixture::new(40, &lines).with_view(80, 20), (95, 0));
    c.keys("<C-u>");
    assert_eq!((c.at(), c.session.top), ((85, 0), 70));
    c.keys("<C-d>");
    assert_eq!((c.at(), c.session.top), ((95, 0), 80));
    c.keys("<C-b>");
    assert_eq!(
        (c.at(), c.session.top),
        ((81, 0), 62),
        "a view less two rows"
    );
    c.keys("<C-f>");
    assert_eq!(
        (c.at(), c.session.top),
        ((81, 0), 80),
        "the cursor stays in view"
    );
    c.keys("<C-y>");
    assert_eq!((c.at(), c.session.top), ((81, 0), 79));
    c.keys("<C-f>");
    assert_eq!((c.at(), c.session.top), ((81, 0), 80));
    c.keys("<C-f>");
    assert_eq!(
        (c.at(), c.session.top),
        ((99, 0), 80),
        "at the end, to the view's bottom"
    );
    c.keys("gg");
    assert_eq!((c.at(), c.session.top), ((0, 0), 0));
    c.keys("<PageDown>");
    assert_eq!(c.session.top, 18);
    c.keys("10<C-d>");
    assert_eq!(c.session.top, 28);
    c.keys("5<C-e>");
    assert_eq!(
        (c.at(), c.session.top),
        ((33, 0), 33),
        "pushed down with the view's top"
    );
}

#[test]
fn the_view_follows_the_cursor_and_centers_after_a_long_jump() {
    let lines: Vec<String> = (0..100).map(|i| format!("row {i}")).collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut c = Copy::new(Fixture::new(40, &lines).with_view(80, 20), (80, 0));
    c.keys("k");
    assert_eq!(c.session.top, 79, "one row up");
    c.keys("gg");
    assert_eq!(c.session.top, 0);
    c.keys("50G");
    assert_eq!(c.session.top, 39, "centered");
    c.keys("G");
    assert_eq!(c.session.top, 80, "never past the bottom");
}

#[test]
fn search_forward_and_back_with_n_and_n_wrapping() {
    let lines = ["error one", "fine", "ERROR two", "error three"];
    let mut c = Copy::lines(80, &lines, (0, 0));
    assert_eq!(c.keys("/error<CR>").at(), (2, 0), "either case");
    assert_eq!(c.keys("n").at(), (3, 0));
    assert_eq!(c.keys("n").at(), (0, 0), "wraps");
    assert_eq!(c.keys("N").at(), (3, 0));
    assert_eq!(c.keys("?two<CR>").at(), (2, 6));
    assert_eq!(
        c.keys("?<CR>").at(),
        (2, 6),
        "an empty query is the last one"
    );
    assert_eq!(c.keys("gg/<CR>").at(), (2, 6), "forward this time");
    assert_eq!(c.keys("/nope<CR>").at(), (2, 6));
    assert_eq!(c.message.as_deref(), Some("Pattern not found: nope"));
    assert_eq!(
        c.keys("2/e<CR>").at(),
        (3, 0),
        "a count before / is dropped"
    );
    // The prompt edits.
    assert_eq!(c.keys("gg/fx<BS>ine<CR>").at(), (1, 0));
    assert_eq!(c.keys("/abc<C-u>three<CR>").at(), (3, 6));
    assert_eq!(c.keys("/zzz<Esc>").at(), (3, 6), "Esc cancels the search");
    assert!(!c.left);
}

#[test]
fn star_and_hash_search_the_word_under_the_cursor_whole() {
    let lines = ["foo food", "x foo", "foo.bar"];
    let mut c = Copy::lines(80, &lines, (0, 1));
    assert_eq!(c.keys("*").at(), (1, 2), "food isn't foo");
    assert_eq!(c.keys("n").at(), (2, 0));
    assert_eq!(c.keys("#").at(), (1, 2));
    assert_eq!(c.keys("N").at(), (2, 0));
}

#[test]
fn marks_and_the_place_before_a_jump() {
    let lines = ["  alpha", "beta", "gamma"];
    let mut c = Copy::lines(80, &lines, (0, 4));
    c.keys("ma").keys("G");
    assert_eq!(c.at(), (2, 0));
    assert_eq!(c.keys("`a").at(), (0, 4));
    assert_eq!(c.keys("G'a").at(), (0, 2), "' goes to the row's first text");
    assert_eq!(c.keys("''").at(), (2, 0), "back before the jump");
    assert_eq!(c.keys("``").at(), (0, 2));
    assert_eq!(c.keys("'z").at(), (0, 2));
    assert_eq!(c.message.as_deref(), Some("Mark not set: z"));
}

#[test]
fn visual_modes_select_from_the_anchor_and_o_swaps_the_ends() {
    let lines = ["one two three", "four five six", "seven"];
    let mut c = Copy::lines(80, &lines, (0, 4));
    c.keys("vjll");
    assert_eq!(c.vim.visual(), Some(Visual::Chars));
    assert_eq!(c.selected().as_deref(), Some("two three\nfour fi"));
    c.keys("V");
    assert_eq!(
        c.selected().as_deref(),
        Some("one two three\nfour five six")
    );
    c.keys("<C-v>");
    assert_eq!(c.selected().as_deref(), Some("two\n fi"));
    c.keys("o");
    assert_eq!(c.at(), (0, 4));
    c.keys("<C-v>");
    assert_eq!(c.vim.visual(), None, "the same key drops it");
    assert_eq!(c.selected(), None);
    c.keys(" e");
    assert_eq!(c.selected().as_deref(), Some("two"), "Space selects too");
    c.keys("<Esc>");
    assert_eq!(c.selected(), None, "Esc drops the selection");
    assert!(!c.left);
    c.keys("<Esc>");
    assert!(c.left, "then leaves");
}

#[test]
fn text_objects_select_words_quotes_brackets_and_paragraphs() {
    let lines = [
        r#"let s = "a quoted string"; call(one, (two));"#,
        "",
        "para one",
        "para two",
        "",
        "last",
    ];
    let object = |at: (usize, usize), keys: &str| {
        let mut c = Copy::lines(80, &lines, at);
        c.keys(keys);
        c.selected()
    };
    let some = |s: &str| Some(s.to_owned());
    assert_eq!(object((0, 1), "viw"), some("let"));
    let ends = |at: (usize, usize), keys: &str| {
        let mut c = Copy::lines(80, &lines, at);
        c.keys(keys);
        let anchor = c.session.anchor.unwrap();
        ((anchor.row, anchor.col), c.at())
    };
    assert_eq!(ends((0, 1), "vaw"), ((0, 0), (0, 3)), "and the blank after");
    assert_eq!(
        ends((0, 3), "viw"),
        ((0, 3), (0, 3)),
        "blanks are a word for iw"
    );
    assert_eq!(
        ends((0, 3), "vaw"),
        ((0, 3), (0, 4)),
        "blanks and the word after"
    );
    assert_eq!(object((0, 1), "v3iw"), some("let s"));
    assert_eq!(object((0, 11), "viW"), some("quoted"));
    assert_eq!(object((0, 11), "vi\""), some("a quoted string"));
    assert_eq!(
        object((0, 11), "va\""),
        some(" \"a quoted string\""),
        "no blanks after: the ones before"
    );
    assert_eq!(
        object((0, 28), "vaw"),
        some(" call"),
        "no blanks after: before"
    );
    assert_eq!(
        object((0, 1), "vi\""),
        some("a quoted string"),
        "the next pair"
    );
    assert_eq!(object((0, 40), "vi("), some("two"));
    assert_eq!(object((0, 40), "va("), some("(two)"));
    assert_eq!(object((0, 40), "v2i("), some("one, (two)"));
    assert_eq!(object((0, 33), "vib"), some("one, (two)"));
    assert_eq!(
        object((0, 31), "vi)"),
        some("one, (two)"),
        "on the bracket itself"
    );
    assert_eq!(
        object((0, 2), "vi("),
        some("t"),
        "no brackets: the selection stays"
    );
    assert_eq!(object((3, 2), "vip"), some("para one\npara two"));
    assert_eq!(object((3, 2), "vap"), some("para one\npara two\n"));
    let mut c = Copy::lines(80, &lines, (3, 2));
    c.keys("vip");
    assert_eq!(
        c.vim.visual(),
        Some(Visual::Lines),
        "a paragraph is whole lines"
    );
}

#[test]
fn y_yanks_the_selection_and_leaves() {
    let lines = ["one two three", "four"];
    let mut c = Copy::lines(80, &lines, (0, 4));
    c.keys("vey");
    assert_eq!((c.yanked.as_deref(), c.left), (Some("two"), true));
    let mut c = Copy::lines(80, &lines, (0, 4));
    c.keys("vj<CR>");
    assert_eq!(
        (c.yanked.as_deref(), c.left),
        (Some("two three\nfour"), true)
    );
    let mut c = Copy::lines(80, &lines, (0, 4));
    c.keys("vY");
    assert_eq!(
        c.yanked.as_deref(),
        Some("one two three"),
        "Y takes the rows"
    );
    let mut c = Copy::lines(80, &lines, (0, 4));
    c.keys("<CR>");
    assert_eq!(
        (c.yanked, c.left),
        (None, true),
        "nothing selected: just leave"
    );
    for leave in ["q", "<C-c>", "<Esc>", "vq"] {
        let mut c = Copy::lines(80, &lines, (0, 4));
        c.keys(leave);
        assert_eq!((c.yanked.clone(), c.left), (None, true), "{leave}");
    }
}

#[test]
fn y_with_a_motion_or_an_object_yanks_without_selecting() {
    let lines = ["one two three", "four five", "six"];
    let yank = |at: (usize, usize), keys: &str| {
        let mut c = Copy::lines(80, &lines, at);
        c.keys(keys);
        assert!(c.left, "{keys} leaves");
        c.yanked
    };
    let some = |s: &str| Some(s.to_owned());
    assert_eq!(
        yank((0, 4), "yw"),
        some("two"),
        "trimmed, as Ghostty copies"
    );
    assert_eq!(
        yank((0, 4), "y2w"),
        some("two three"),
        "not the next line's start"
    );
    assert_eq!(yank((0, 4), "2yw"), some("two three"));
    assert_eq!(yank((0, 4), "ye"), some("two"));
    assert_eq!(yank((0, 4), "y$"), some("two three"));
    assert_eq!(yank((0, 4), "y0"), some("one"));
    assert_eq!(yank((0, 4), "yfe"), some("two thre"));
    assert_eq!(yank((0, 4), "yte"), some("two thr"));
    assert_eq!(yank((0, 4), "yj"), some("one two three\nfour five"));
    assert_eq!(yank((0, 4), "yy"), some("one two three"));
    assert_eq!(yank((0, 4), "2yy"), some("one two three\nfour five"));
    assert_eq!(yank((1, 0), "Y"), some("four five"));
    assert_eq!(yank((1, 0), "3Y"), some("four five\nsix"));
    assert_eq!(yank((0, 5), "yiw"), some("two"));
    assert_eq!(yank((0, 5), "yaw"), some("two"));
    assert_eq!(yank((0, 0), "y/fi<CR>"), some("one two three\nfour"));
    assert_eq!(yank((0, 0), "yl"), some("o"));
    assert_eq!(yank((0, 0), "yh"), None, "nowhere to go: nothing");
    // Esc drops the y.
    let mut c = Copy::lines(80, &lines, (0, 4));
    c.keys("y<Esc>w");
    assert_eq!((c.at(), c.left), ((0, 8), false));
}

#[test]
fn the_grammar_shows_what_is_typed_and_passes_keys_it_doesnt_use() {
    let mut vim = Vim::default();
    vim.key(Key::Char('2'));
    vim.key(Key::Char('y'));
    vim.key(Key::Char('3'));
    assert_eq!(vim.typed(), "2y3");
    vim.key(Key::Char('f'));
    assert_eq!(vim.typed(), "2y3f");
    let step = vim.key(Key::Escape);
    assert!(step.taken && !step.leave);
    assert_eq!(vim.typed(), "");
    vim.key(Key::Char('/'));
    vim.key(Key::Char('a'));
    assert_eq!(vim.prompt().map(|p| p.query.as_str()), Some("a"));
    vim.key(Key::Backspace);
    vim.key(Key::Backspace);
    assert_eq!(vim.prompt(), None, "backspace on an empty query closes it");
    let step = vim.key(Key::Char('x'));
    assert!(!step.taken, "x isn't copy mode's");
    let step = vim.key(Key::Ctrl('h'));
    assert!(!step.taken, "Ctrl-H is the window's");
    let step = vim.key(Key::Char('0'));
    assert_eq!(step.commands.len(), 1, "0 alone is a motion");
}
