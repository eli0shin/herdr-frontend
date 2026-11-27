//! Copy mode on a real terminal's buffer: the engine over libghostty-vt's screen and
//! scrollback, its cursor and marks tracked as output arrives, the view following it, the
//! selection installed and yanked as Ghostty copies it.

use copy_mode::{Command, FindKind, Motion, Visual};
use terminal::{Config, Vt};

fn vt() -> Vt {
    Vt::new(&Config {
        cell_width: 10,
        cell_height: 22,
        ..Config::default()
    })
}

fn lines(range: std::ops::RangeInclusive<usize>) -> Vec<u8> {
    range
        .flat_map(|i| format!("line {i}\r\n").into_bytes())
        .collect()
}

fn go(vt: &mut Vt, motion: Motion) -> Option<String> {
    vt.copy_run(&Command::Move {
        motion,
        count: None,
    })
}

fn go_n(vt: &mut Vt, motion: Motion, count: usize) -> Option<String> {
    vt.copy_run(&Command::Move {
        motion,
        count: Some(count),
    })
}

#[test]
fn copy_mode_starts_at_the_programs_cursor_and_moves_over_history() {
    let mut vt = vt();
    vt.write(&lines(1..=100));
    vt.write(b"$ ");
    vt.copy_start();
    let s = vt.snapshot();
    let copy = s.copy.clone().expect("copying");
    assert_eq!((copy.row, copy.total), (101, 101));
    assert_eq!(s.cursor.position, Some((2, 23)), "at the prompt");
    assert!(s.cursor.visible && !s.cursor.blinking);

    go_n(&mut vt, Motion::Up, 3);
    let s = vt.snapshot();
    assert_eq!(s.cursor.position, Some((2, 20)));
    assert_eq!(s.copy.unwrap().row, 98);
    assert_eq!(s.scrollbar.offset, 77, "still in view: the view stays");

    go(&mut vt, Motion::Top);
    let s = vt.snapshot();
    assert_eq!(s.scrollbar.offset, 0, "the view follows into history");
    assert_eq!(s.line(0), "line 1");
    assert_eq!(s.cursor.position, Some((0, 0)));

    go(&mut vt, Motion::WordForward { big: false });
    assert_eq!(vt.snapshot().cursor.position, Some((5, 0)));
    go(&mut vt, Motion::LineEnd);
    assert_eq!(vt.snapshot().cursor.position, Some((5, 0)));
    go_n(&mut vt, Motion::Down, 2);
    assert_eq!(
        vt.snapshot().cursor.position,
        Some((5, 2)),
        "$ sticks to the end"
    );

    go(&mut vt, Motion::Bottom);
    let s = vt.snapshot();
    assert_eq!(
        s.copy.unwrap().row,
        101,
        "the last row with text: the prompt"
    );
    assert_eq!(s.scrollbar.offset, 77);

    vt.copy_end();
    let s = vt.snapshot();
    assert_eq!(s.copy, None);
    assert!(s.scrollbar.at_bottom);
    assert_eq!(
        s.cursor.position,
        Some((2, 23)),
        "the program's cursor again"
    );
}

#[test]
fn a_selection_shows_in_the_rows_and_yanks_as_ghostty_copies() {
    let mut vt = vt();
    vt.write(b"one two three\r\nfour five six\r\nseven\r\n");
    vt.copy_start();
    go(&mut vt, Motion::Top);
    go(&mut vt, Motion::WordForward { big: false });
    vt.copy_run(&Command::Visual(Some(Visual::Chars)));
    go(&mut vt, Motion::Down);
    go(&mut vt, Motion::WordEnd { big: false });
    let s = vt.snapshot();
    assert_eq!(s.screen[0].selection, Some((4, 79)));
    assert_eq!(s.screen[1].selection, Some((0, 8)));
    assert_eq!(
        vt.copy_run(&Command::Yank).as_deref(),
        Some("two three\nfour five")
    );

    vt.copy_run(&Command::Visual(Some(Visual::Block)));
    let s = vt.snapshot();
    assert_eq!(s.screen[0].selection, Some((4, 8)));
    assert_eq!(s.screen[1].selection, Some((4, 8)));
    assert_eq!(vt.copy_run(&Command::Yank).as_deref(), Some("two t\n five"));

    vt.copy_run(&Command::Visual(Some(Visual::Lines)));
    assert_eq!(
        vt.copy_run(&Command::Yank).as_deref(),
        Some("one two three\nfour five six")
    );
    vt.copy_run(&Command::Visual(None));
    assert_eq!(vt.snapshot().screen[0].selection, None);

    // A yank over a motion takes its text without selecting.
    go(&mut vt, Motion::Top);
    let text = vt.copy_run(&Command::YankMotion {
        motion: Motion::Find {
            kind: FindKind::Onto,
            ch: 'w',
            repeat: false,
        },
        count: None,
    });
    assert_eq!(text.as_deref(), Some("one tw"));
    let text = vt.copy_run(&Command::YankMotion {
        motion: Motion::WordForward { big: false },
        count: None,
    });
    assert_eq!(
        text.as_deref(),
        Some("one"),
        "trimmed at its end, as Ghostty copies"
    );
    assert_eq!(
        vt.copy_run(&Command::YankLines { count: 2 }).as_deref(),
        Some("one two three\nfour five six")
    );
}

#[test]
fn the_cursor_stays_on_its_text_while_output_arrives_and_the_view_stays_pinned() {
    let mut vt = vt();
    vt.write(&lines(1..=100));
    vt.copy_start();
    go_n(&mut vt, Motion::Top, 50);
    let s = vt.snapshot();
    assert_eq!(s.copy.as_ref().unwrap().row, 50);
    let top = s.scrollbar.offset;
    let first = s.line(0);
    assert_eq!(s.line(usize::from(s.cursor.position.unwrap().1)), "line 50");

    vt.write(&lines(101..=140));
    let s = vt.snapshot();
    assert_eq!(s.scrollbar.offset, top, "pinned");
    assert_eq!(s.line(0), first);
    assert_eq!(s.copy.as_ref().unwrap().row, 50);
    assert_eq!(s.line(usize::from(s.cursor.position.unwrap().1)), "line 50");

    // Marks follow their text too.
    vt.copy_run(&Command::Mark('a'));
    go(&mut vt, Motion::Bottom);
    vt.write(&lines(141..=150));
    go(
        &mut vt,
        Motion::Mark {
            name: 'a',
            exact: true,
        },
    );
    let s = vt.snapshot();
    assert_eq!(s.copy.unwrap().row, 50);
}

#[test]
fn started_at_the_bottom_output_leaves_the_view_and_the_cursor_where_they_are() {
    let mut vt = vt();
    vt.write(&lines(1..=100));
    vt.write(b"$ ");
    vt.copy_start();
    go(&mut vt, Motion::Up);
    let s = vt.snapshot();
    let (top, first, cursor) = (s.scrollbar.offset, s.line(0), s.cursor.position);
    assert_eq!(s.line(usize::from(cursor.unwrap().1)), "line 100");

    vt.write(b"\r\n");
    vt.write(&lines(101..=110));
    let s = vt.snapshot();
    assert_eq!(s.scrollbar.offset, top, "the view stays");
    assert!(!s.scrollbar.at_bottom);
    assert_eq!(s.line(0), first);
    assert_eq!(s.cursor.position, cursor);
    assert_eq!(s.line(usize::from(cursor.unwrap().1)), "line 100");

    // The view still follows the cursor, and leaving goes back to the bottom.
    go(&mut vt, Motion::Bottom);
    let s = vt.snapshot();
    assert_eq!(
        s.line(usize::from(s.cursor.position.unwrap().1)),
        "line 110"
    );
    vt.copy_end();
    vt.write(&lines(111..=112));
    assert!(vt.snapshot().scrollbar.at_bottom, "following output again");
}

#[test]
fn a_yank_of_blanks_copies_nothing_and_leaves_no_selection_drawn() {
    let mut vt = vt();
    vt.write(b"text\r\n\r\n$ ");
    vt.copy_start();
    go(&mut vt, Motion::Up);
    assert_eq!(vt.copy_run(&Command::YankLines { count: 1 }), None);
    let s = vt.snapshot();
    assert!(s.screen.iter().all(|r| r.selection.is_none()));
    assert_eq!(s.copy.unwrap().message.as_deref(), Some("Nothing to copy"));
    // A selection made before stays as it was.
    go(&mut vt, Motion::Up);
    vt.copy_run(&Command::Visual(Some(Visual::Chars)));
    go(&mut vt, Motion::Down);
    assert_eq!(vt.copy_run(&Command::YankLines { count: 1 }), None);
    let s = vt.snapshot();
    assert_eq!(
        s.screen[0].selection,
        Some((2, 79)),
        "from the column kept from the prompt"
    );
    assert_eq!(s.screen[1].selection, Some((0, 2)));
    assert_eq!(s.screen[2].selection, None);
}

#[test]
fn search_jumps_to_matches_and_marks_them_on_the_rows() {
    let mut vt = vt();
    vt.write(&lines(1..=100));
    vt.copy_start();
    go(&mut vt, Motion::Top);
    go(
        &mut vt,
        Motion::Search {
            query: "LINE 5".into(),
            backward: false,
        },
    );
    let s = vt.snapshot();
    assert_eq!(s.copy.as_ref().unwrap().row, 5, "either case");
    go(&mut vt, Motion::SearchNext { reverse: false });
    let s = vt.snapshot();
    assert_eq!(s.copy.as_ref().unwrap().row, 50);
    assert_eq!(s.scrollbar.offset, 37, "a long jump centers");
    let y = usize::from(s.cursor.position.unwrap().1);
    assert_eq!(s.line(y), "line 50");
    let hit = s.screen[y].matches[0];
    assert!(hit.selected && hit.start == 0 && hit.end == 5);
    let other = s
        .screen
        .iter()
        .position(|r| r.line_text_is("line 51"))
        .unwrap();
    assert!(!s.screen[other].matches[0].selected);

    go(&mut vt, Motion::SearchNext { reverse: true });
    assert_eq!(vt.snapshot().copy.unwrap().row, 5);
    go(
        &mut vt,
        Motion::Search {
            query: "nowhere".into(),
            backward: true,
        },
    );
    let s = vt.snapshot();
    assert_eq!(s.copy.as_ref().unwrap().row, 5);
    assert_eq!(
        s.copy.unwrap().message.as_deref(),
        Some("Pattern not found: nowhere")
    );
    // The find bar's search is its own.
    assert_eq!(vt.finding(), None);
    vt.copy_end();
    assert!(vt.snapshot().screen.iter().all(|r| r.matches.is_empty()));
}

trait LineText {
    fn line_text_is(&self, text: &str) -> bool;
}

impl LineText for std::sync::Arc<terminal::Row> {
    fn line_text_is(&self, text: &str) -> bool {
        self.text().trim_end() == text
    }
}

#[test]
fn soft_wrapped_lines_and_wide_characters_read_as_text() {
    let mut vt = Vt::new(&Config {
        cols: 10,
        rows: 5,
        cell_width: 10,
        cell_height: 22,
        ..Config::default()
    });
    vt.write("one two three four\r\na日本b c\r\n".as_bytes());
    vt.copy_start();
    go(&mut vt, Motion::Top);
    go_n(&mut vt, Motion::WordForward { big: false }, 2);
    assert_eq!(vt.snapshot().cursor.position, Some((8, 0)));
    go(&mut vt, Motion::WordEnd { big: false });
    assert_eq!(
        vt.snapshot().cursor.position,
        Some((2, 1)),
        "across the wrap"
    );
    go(&mut vt, Motion::Down);
    go(&mut vt, Motion::LineStart);
    go(&mut vt, Motion::Right);
    go(&mut vt, Motion::Right);
    assert_eq!(
        vt.snapshot().cursor.position,
        Some((3, 2)),
        "over the wide character"
    );
    let text = vt.copy_run(&Command::YankMotion {
        motion: Motion::LineEnd,
        count: None,
    });
    assert_eq!(text.as_deref(), Some("本b c"));
}
