//! Scrollback, find and links on a fake core, through Rune's real runtime with no window: the
//! wheel at Eli's zoom and the scroll speed, the trackpad's whole lines, the scrollbar, the find
//! bar (`cmd-f`), Cmd-hover underlines and Cmd-click opening links, select all and copy on
//! select (`docs/research/feature-inventory.md` §17).

use base64::Engine as _;
use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::demo_core;
use herdr_frontend::link::{Fake, Out};
use herdr_frontend::{SIZE, State, app};
use herdr_proto::methods::ExecResult;
use model::{Action, TerminalAction};
use rune::clipboard::{MemoryClipboard, memory};
use rune::{ElementInfo, Env, Key, KeyEvent, Mods, PhysicalInput, Runtime, WheelEvent};

fn runtime_with(settings: Settings) -> (Runtime<State>, Fake, MemoryClipboard) {
    let fake = demo_core();
    let board = memory();
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.use_clipboard(&board);
    rt.state.with_settings(settings);
    rt.state.connect(Box::new(fake.clone()));
    rt.state.settle();
    frames(&mut rt, 3);
    (rt, fake, board)
}

fn runtime() -> (Runtime<State>, Fake, MemoryClipboard) {
    runtime_with(Settings::fixture())
}

fn frames(rt: &mut Runtime<State>, n: usize) {
    for _ in 0..n {
        rt.invalidate();
        rt.frame(1.0 / 60.0);
    }
}

/// `t1` reset to a shell on the primary screen showing `text`.
fn shell(rt: &mut Runtime<State>, fake: &Fake, text: &str) {
    let mut bytes = b"\x1bc".to_vec();
    bytes.extend_from_slice(text.as_bytes());
    fake.output("t1", &bytes);
    frames(rt, 3);
}

fn hundred_lines(rt: &mut Runtime<State>, fake: &Fake) {
    let lines: String = (1..=100).map(|i| format!("line {i}\r\n")).collect();
    shell(rt, fake, &lines);
}

fn shown(rt: &Runtime<State>) -> terminal::Snapshot {
    rt.state.viewers["t1"]
        .pane
        .as_ref()
        .unwrap()
        .snapshot
        .clone()
}

fn element(rt: &Runtime<State>, key: &str) -> Option<ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

/// `key` in pn-1's slot: the other panes' terminals, resized to their panes, have scrollbacks
/// and scrollbars too.
fn in_pn1(rt: &Runtime<State>, key: &str) -> Option<ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting && e.path.contains("slot-pn-1/"))
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused)
        .and_then(|e| e.key)
}

fn typed(fake: &Fake, from: usize) -> Vec<u8> {
    fake.sent_to("t1")[from..]
        .iter()
        .filter_map(|o| match o {
            Out::Input(bytes) => Some(bytes.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn mark(fake: &Fake) -> usize {
    fake.sent_to("t1").len()
}

const NONE: Mods = Mods {
    shift: false,
    cmd: false,
    alt: false,
    ctrl: false,
    logo: false,
};

const CMD: Mods = Mods {
    cmd: true,
    logo: true,
    ..NONE
};

fn press(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.frame(1.0 / 60.0);
}

fn hold(rt: &mut Runtime<State>, mods: Mods) {
    rt.physical_input(PhysicalInput::Modifiers(mods));
    frames(rt, 2);
}

/// Where `t1`'s cell `(col, row)` is in the window, a little inside its left half.
fn cell(rt: &Runtime<State>, col: u16, row: u16) -> (f32, f32) {
    let grid = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("screen") && e.path.contains("pane-pn-1/"))
        .expect("t1's grid");
    let face = rt.state.face(2.0);
    let (w, h) = face.cell_pt();
    (
        grid.rect[0] + f32::from(col) * w + 1.0,
        grid.rect[1] + f32::from(row) * h + h / 2.0,
    )
}

/// The wheel buttons a mouse-tracking `t1` heard since `from`: up is positive.
fn wheel_reports(fake: &Fake, from: usize) -> i32 {
    let sent = String::from_utf8(typed(fake, from)).unwrap();
    sent.matches("\x1b[<64;").count() as i32 - sent.matches("\x1b[<65;").count() as i32
}

/// Nine lines is three notches at any zoom: Rune's division by the zoom and the wheel's
/// multiplication back can't lose one.
#[test]
fn notches_scroll_whole_lines_at_zooms_that_dont_divide_evenly() {
    for zoom in [1.1, 1.2] {
        let (mut rt, fake, _) = runtime_with(Settings {
            zoom,
            ..Settings::fixture()
        });
        shell(&mut rt, &fake, "$ ");
        fake.output("t1", b"\x1b[?1000h\x1b[?1006h");
        frames(&mut rt, 3);
        let (x, y) = cell(&rt, 2, 1);
        for notches in [3.0, 9.0] {
            let from = mark(&fake);
            rt.wheel_xy(x, y, 0.0, notches * 40.0 / zoom);
            frames(&mut rt, 2);
            assert_eq!(
                wheel_reports(&fake, from),
                notches as i32 * 3,
                "{notches} notches at zoom {zoom}"
            );
        }
    }
}

/// A slow notch (macOS hands over part of one) still scrolls a notch's three lines, as
/// Ghostty's `scrollCallback` rounds it up to one.
#[test]
fn a_fractional_notch_is_a_whole_notch() {
    let (mut rt, fake, _) = runtime();
    shell(&mut rt, &fake, "$ ");
    fake.output("t1", b"\x1b[?1000h\x1b[?1006h");
    frames(&mut rt, 3);
    let (x, y) = cell(&rt, 2, 1);
    let from = mark(&fake);
    rt.wheel_xy(x, y, 0.0, 0.4 * 40.0);
    frames(&mut rt, 2);
    assert_eq!(wheel_reports(&fake, from), 3);
}

#[test]
fn the_scroll_speed_multiplies_the_wheel() {
    let (mut rt, fake, _) = runtime_with(Settings {
        terminal_scroll_speed: 2.0,
        ..Settings::fixture()
    });
    shell(&mut rt, &fake, "$ ");
    fake.output("t1", b"\x1b[?1000h\x1b[?1006h");
    frames(&mut rt, 3);
    let (x, y) = cell(&rt, 2, 1);
    let from = mark(&fake);
    rt.wheel_xy(x, y, 0.0, 40.0);
    frames(&mut rt, 2);
    assert_eq!(wheel_reports(&fake, from), 6, "a notch is six lines at 2");

    // At its slowest a notch still moves a line.
    rt.state.with_settings(Settings {
        terminal_scroll_speed: 0.1,
        ..Settings::fixture()
    });
    let from = mark(&fake);
    rt.wheel_xy(x, y, 0.0, -40.0);
    frames(&mut rt, 2);
    assert_eq!(wheel_reports(&fake, from), -1);

    // A trackpad over the scrollback: twice the travel.
    rt.state.with_settings(Settings {
        terminal_scroll_speed: 2.0,
        ..Settings::fixture()
    });
    hundred_lines(&mut rt, &fake);
    let line = rt.state.face(2.0).cell_pt().1;
    rt.wheel_event(WheelEvent::new(0.0, line * 1.5).at(x, y).precise(true));
    frames(&mut rt, 3);
    let s = shown(&rt);
    assert_eq!(s.scrollbar.offset, s.scrollbar.total - s.scrollbar.len - 3);
}

/// A trackpad scrolls whole lines, as Ghostty does: its travel adds up to a line per cell
/// height, and what's left over waits for more.
#[test]
fn a_trackpad_scrolls_the_scrollback_by_whole_lines() {
    let (mut rt, fake, _) = runtime();
    hundred_lines(&mut rt, &fake);
    let (x, y) = cell(&rt, 2, 2);
    let line = rt.state.face(2.0).cell_pt().1;
    let bottom = shown(&rt).scrollbar.offset;
    let from = mark(&fake);
    let up = |rt: &mut Runtime<State>, lines: f32| {
        rt.wheel_event(WheelEvent::new(0.0, line * lines).at(x, y).precise(true));
        frames(rt, 3);
    };
    up(&mut rt, 1.5);
    assert_eq!(typed(&fake, from), b"", "the scrollback is the terminal's");
    assert_eq!(shown(&rt).scrollbar.offset, bottom - 1);
    up(&mut rt, 0.5);
    assert_eq!(
        shown(&rt).scrollbar.offset,
        bottom - 2,
        "the half left over"
    );
    let grid = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("screen") && e.path.contains("pane-pn-1/"))
        .unwrap();
    let pane = element(&rt, "pane-pn-1").unwrap();
    assert!(
        (grid.rect[1] + grid.rect[3] - (pane.rect[1] + pane.rect[3])).abs() <= 0.5,
        "on its rows at the pane's bottom, never part of one: {:?} in {:?}",
        grid.rect,
        pane.rect
    );

    // Back down past the bottom: following output again.
    up(&mut rt, -4.0);
    assert!(shown(&rt).scrollbar.at_bottom);
}

#[test]
fn the_scrollbar_shows_where_the_viewport_is_and_drags_it() {
    let (mut rt, fake, _) = runtime();
    hundred_lines(&mut rt, &fake);
    assert!(in_pn1(&rt, "scrollbar").is_none(), "off, as Eli has it");
    rt.state.with_settings(Settings {
        pane_scrollbars: true,
        ..Settings::fixture()
    });
    frames(&mut rt, 2);
    let lane = in_pn1(&rt, "scrollbar").expect("the scrollbar");
    let thumb = in_pn1(&rt, "scrollbar-thumb").unwrap();
    // At the bottom, the thumb sits at the lane's end.
    assert!((thumb.rect[1] + thumb.rect[3] - (lane.rect[1] + lane.rect[3])).abs() < 0.5);

    // Dragged to the top, the viewport goes to the top of the scrollback.
    let (x, y) = (thumb.rect[0] + 1.0, thumb.rect[1] + 2.0);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x, lane.rect[1] - 50.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, lane.rect[1] - 50.0);
    frames(&mut rt, 3);
    assert_eq!(shown(&rt).scrollbar.offset, 0);
    assert_eq!(shown(&rt).line(0), "line 1");

    // None on the alternate screen.
    fake.output("t1", b"\x1b[?1049h");
    frames(&mut rt, 3);
    assert!(in_pn1(&rt, "scrollbar").is_none());
}

fn type_text(rt: &mut Runtime<State>, text: &str) {
    for c in text.chars() {
        press(rt, Key::Text(c.to_string()), NONE);
    }
    frames(rt, 2);
}

fn count(rt: &Runtime<State>) -> String {
    element(rt, "find-count")
        .and_then(|e| e.text)
        .unwrap_or_default()
}

#[test]
fn cmd_f_finds_in_the_screen_and_scrollback() {
    let (mut rt, fake, _) = runtime();
    hundred_lines(&mut rt, &fake);
    let from = mark(&fake);
    press(&mut rt, Key::Command('f'), CMD);
    frames(&mut rt, 2);
    assert!(element(&rt, "find-bar-pn-1").is_some());
    assert_eq!(focused(&rt).as_deref(), Some("find-pn-1"));

    // Typing searches as it goes and never reaches the program.
    type_text(&mut rt, "line 9");
    assert_eq!(typed(&fake, from), b"");
    assert_eq!(count(&rt), "11 matches");
    let s = shown(&rt);
    assert!(s.screen.iter().any(|r| !r.matches.is_empty()));

    // Enter goes up from the bottom; Shift+Enter back down, wrapping to the oldest.
    press(&mut rt, Key::Enter, NONE);
    frames(&mut rt, 2);
    assert_eq!(count(&rt), "1 / 11");
    press(&mut rt, Key::Enter, NONE);
    frames(&mut rt, 2);
    assert_eq!(count(&rt), "2 / 11");
    press(
        &mut rt,
        Key::Enter,
        Mods {
            shift: true,
            ..NONE
        },
    );
    press(
        &mut rt,
        Key::Enter,
        Mods {
            shift: true,
            ..NONE
        },
    );
    frames(&mut rt, 2);
    assert_eq!(count(&rt), "11 / 11");
    assert_eq!(shown(&rt).line(0), "line 9", "scrolled up to it");

    // Clicking the terminal gives typing back to the program; the bar and hits stay.
    let (x, y) = cell(&rt, 0, 3);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, 2);
    assert_eq!(focused(&rt).as_deref(), Some("pane-pn-1"));
    assert!(element(&rt, "find-bar-pn-1").is_some());
    let from = mark(&fake);
    press(&mut rt, Key::Text("x".into()), NONE);
    frames(&mut rt, 2);
    assert_eq!(typed(&fake, from), b"x");

    // Escape in the field closes the bar and its hits; the query stays for next time.
    press(&mut rt, Key::Command('f'), CMD);
    frames(&mut rt, 2);
    assert_eq!(focused(&rt).as_deref(), Some("find-pn-1"));
    let edit = &rt.state.viewers["t1"].find.as_ref().unwrap().edit;
    assert_eq!(edit.text, "line 9");
    press(&mut rt, Key::Escape, NONE);
    frames(&mut rt, 3);
    assert!(element(&rt, "find-bar-pn-1").is_none());
    assert_eq!(focused(&rt).as_deref(), Some("pane-pn-1"));
    assert!(shown(&rt).screen.iter().all(|r| r.matches.is_empty()));
    assert_eq!(shown(&rt).find, None);

    // Reopening brings the query back, selected, so typing replaces it.
    press(&mut rt, Key::Command('f'), CMD);
    frames(&mut rt, 2);
    let edit = &rt.state.viewers["t1"].find.as_ref().unwrap().edit;
    assert_eq!(
        (edit.text.as_str(), edit.anchor, edit.focus),
        ("line 9", 0, 6)
    );
    assert_eq!(count(&rt), "11 matches");
}

#[test]
fn each_terminal_keeps_its_own_query() {
    let (mut rt, fake, _) = runtime();
    hundred_lines(&mut rt, &fake);
    press(&mut rt, Key::Command('f'), CMD);
    type_text(&mut rt, "line 5");
    // The other pane's terminal gets its own bar.
    let other = element(&rt, "pane-pn-2").expect("pn-2");
    let (x, y) = (other.rect[0] + 20.0, other.rect[1] + 20.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, 2);
    press(&mut rt, Key::Command('f'), CMD);
    frames(&mut rt, 2);
    assert_eq!(focused(&rt).as_deref(), Some("find-pn-2"));
    let query = |rt: &Runtime<State>, a: &str| {
        rt.state.viewers[a]
            .find
            .as_ref()
            .map(|b| b.edit.text.clone())
    };
    assert_eq!(query(&rt, "t2").as_deref(), Some(""));
    assert_eq!(query(&rt, "t1").as_deref(), Some("line 5"));
    assert!(element(&rt, "find-bar-pn-1").is_some(), "t1's bar stays");
}

/// The columns of `t1`'s first row its rows underline as the hovered link.
fn underlined(rt: &Runtime<State>) -> Option<(u16, u16)> {
    shown(rt).screen[0].link
}

#[test]
fn cmd_hover_underlines_links_and_cmd_click_opens_urls() {
    let (mut rt, fake, _) = runtime();
    shell(&mut rt, &fake, "see https://example.com/x ok\r\n$ ");
    let (x, y) = cell(&rt, 6, 0);
    rt.pointer_move(x, y);
    frames(&mut rt, 2);
    assert!(underlined(&rt).is_none(), "no underline without Cmd");
    hold(&mut rt, CMD);
    assert_eq!(underlined(&rt), Some((4, 24)), "the underline");
    assert_eq!(
        shown(&rt).link,
        [terminal::Span {
            row: 0,
            start: 4,
            end: 24
        }]
    );
    hold(&mut rt, NONE);
    assert!(underlined(&rt).is_none(), "gone with Cmd");

    // Out of the pane, the link isn't under the pointer: Cmd held or pressed there shows none.
    hold(&mut rt, CMD);
    assert!(underlined(&rt).is_some());
    let sidebar = element(&rt, "sidebar").expect("the sidebar");
    rt.pointer_move(sidebar.rect[0] + 20.0, sidebar.rect[1] + 200.0);
    frames(&mut rt, 2);
    assert!(underlined(&rt).is_none(), "gone with the pointer");
    hold(&mut rt, NONE);
    hold(&mut rt, CMD);
    assert!(underlined(&rt).is_none(), "Cmd outside underlines nothing");
    hold(&mut rt, NONE);
    rt.pointer_move(x, y);
    frames(&mut rt, 2);

    // Cmd-click opens it, and nothing reaches the program or the selection.
    let from = mark(&fake);
    hold(&mut rt, CMD);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, 3);
    assert_eq!(rt.take_url_requests(), ["https://example.com/x"]);
    assert_eq!(typed(&fake, from), b"");
    assert!(shown(&rt).screen.iter().all(|r| r.selection.is_none()));
}

/// Off a link, Cmd-click is a click: it selects, or reaches a program tracking the mouse
/// (Ghostty's `mouseButtonCallback` takes it only over a link).
#[test]
fn cmd_click_off_a_link_selects_or_reaches_the_program() {
    let (mut rt, fake, board) = runtime();
    shell(&mut rt, &fake, "hello world https://example.com\r\n$ ");
    let (x, y) = cell(&rt, 0, 0);
    rt.pointer_move(x, y);
    frames(&mut rt, 2);
    hold(&mut rt, CMD);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    let (x2, y2) = cell(&rt, 4, 0);
    rt.pointer_move(x2 + 6.0, y2);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x2 + 6.0, y2);
    frames(&mut rt, 3);
    assert!(rt.take_url_requests().is_empty());
    assert_eq!(shown(&rt).screen[0].selection, Some((0, 4)));
    assert_eq!(board.text().as_deref().map(String::as_str), Some("hello"));
    hold(&mut rt, NONE);

    // A program tracking the mouse hears it.
    fake.output("t1", b"\x1b[?1000h\x1b[?1006h");
    frames(&mut rt, 3);
    let from = mark(&fake);
    hold(&mut rt, CMD);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, 3);
    let sent = String::from_utf8(typed(&fake, from)).unwrap();
    // SGR reports have no bit for Cmd: a plain press and release.
    assert_eq!(sent, "\x1b[<0;1;1M\x1b[<0;1;1m");
}

#[test]
fn cmd_click_on_a_source_path_opens_it_in_neovim_beside_the_pane() {
    let (mut rt, fake, _) = runtime();
    shell(&mut rt, &fake, "error at src/main.rs:12:3\r\n$ ");
    fake.core().exec = Some(Ok(ExecResult {
        code: Some(0),
        stdout: base64::engine::general_purpose::STANDARD.encode("1\tf\t/tmp/src/main.rs\n"),
        ..Default::default()
    }));
    let (x, y) = cell(&rt, 12, 0);
    rt.pointer_move(x, y);
    frames(&mut rt, 2);
    hold(&mut rt, CMD);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, 4);
    let calls = fake.core().calls.clone();
    let exec = calls
        .iter()
        .find_map(|c| match c {
            model::Request::Exec(e) => Some(e.clone()),
            _ => None,
        })
        .expect("a lookup");
    assert_eq!(exec.on, None, "on this machine");
    assert_eq!(exec.argv[5..], ["src/main.rs:12:3", "src/main.rs"]);
    let run = calls
        .iter()
        .find_map(|c| match c {
            model::Request::Run(r) => Some(r.clone()),
            _ => None,
        })
        .expect("Neovim in a split");
    assert_eq!(
        run.argv,
        ["nvim", "+call cursor(12, 3)", "--", "/tmp/src/main.rs"]
    );
    assert!(rt.take_path_requests().is_empty());

    // A folder goes to the opener.
    fake.core().exec = Some(Ok(ExecResult {
        code: Some(0),
        stdout: base64::engine::general_purpose::STANDARD.encode("0\td\t/tmp/src\n"),
        ..Default::default()
    }));
    shell(&mut rt, &fake, "in ./src/ now\r\n$ ");
    hold(&mut rt, NONE);
    let (x, y) = cell(&rt, 4, 0);
    rt.pointer_move(x, y);
    frames(&mut rt, 2);
    hold(&mut rt, CMD);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, 4);
    let opened: Vec<_> = rt
        .take_path_requests()
        .into_iter()
        .map(|r| r.path)
        .collect();
    assert_eq!(opened, [std::path::PathBuf::from("/tmp/src")]);
}

#[test]
fn select_all_takes_the_scrollback_and_copy_on_select_can_be_off() {
    let (mut rt, fake, board) = runtime();
    hundred_lines(&mut rt, &fake);
    rt.state
        .step(0.0, Action::Terminal(TerminalAction::SelectAll));
    frames(&mut rt, 2);
    press(&mut rt, Key::Command('c'), CMD);
    frames(&mut rt, 3);
    let text = board.text().map(|t| t.to_string()).unwrap_or_default();
    assert!(text.starts_with("line 1\n"), "{text:?}");
    assert!(text.trim_end().ends_with("line 100"));

    rt.state.with_settings(Settings {
        copy_on_select: false,
        ..Settings::fixture()
    });
    board.set_text("before");
    shell(&mut rt, &fake, "hello world\r\n$ ");
    let (x, y) = cell(&rt, 0, 0);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    let (x2, y2) = cell(&rt, 4, 0);
    rt.pointer_move(x2 + 6.0, y2);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x2 + 6.0, y2);
    frames(&mut rt, 3);
    assert_eq!(board.text().as_deref().map(String::as_str), Some("before"));
    assert_eq!(
        shown(&rt).screen[0].selection,
        Some((0, 4)),
        "still selected"
    );
}
