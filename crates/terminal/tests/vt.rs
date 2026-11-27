//! A [`Vt`] driven directly: replies, effects, render holds, scrollback, colors and the input
//! encoders.

use std::time::{Duration, Instant};

use ghostty_vt_sys::{
    GHOSTTY_KEY_ARROW_UP, GHOSTTY_KEY_C, GHOSTTY_KEY_ESCAPE, GHOSTTY_KEY_SEMICOLON,
};
use terminal::{
    ClipboardLocation, ColorScheme, Config, Dirty, Event, Focus, KeyEvent, Mods, MouseAction,
    MouseButton, MouseEvent, PasteRejected, Progress, Rgb, Scroll, Scrollback, SelectAction,
    SelectEvent, Theme, Vt, encode_focus,
};

fn config() -> Config {
    Config {
        cell_width: 10,
        cell_height: 22,
        ..Config::default()
    }
}

fn replies(vt: &mut Vt) -> String {
    String::from_utf8(vt.take_replies()).expect("utf-8 replies")
}

#[test]
fn answers_queries_from_its_config() {
    let mut vt = Vt::new(&Config {
        theme: Theme {
            foreground: Some(Rgb::new(0xee, 0xee, 0xee)),
            background: Some(Rgb::new(0x11, 0x22, 0x33)),
            ..Theme::default()
        },
        color_scheme: Some(ColorScheme::Dark),
        ..config()
    });
    // Colors, pixel and cell sizes, grid size, color scheme, cursor position.
    vt.write(b"\x1b]11;?\x07\x1b]10;?\x1b\\\x1b[14t\x1b[16t\x1b[18t\x1b[?996n\x1b[3;5H\x1b[6n");
    assert_eq!(
        replies(&mut vt),
        [
            "\x1b]11;rgb:1111/2222/3333\x07",
            "\x1b]10;rgb:eeee/eeee/eeee\x1b\\",
            "\x1b[4;528;800t",
            "\x1b[6;22;10t",
            "\x1b[8;24;80t",
            "\x1b[?997;1n",
            "\x1b[3;5R",
        ]
        .concat()
    );
    assert_eq!(replies(&mut vt), "", "taking the replies empties them");
}

#[test]
fn leaves_queries_it_cannot_answer_unanswered() {
    // No pixel size before the metrics are known, no theme background, no color scheme.
    let mut vt = Vt::new(&Config::default());
    vt.write(b"\x1b[14t\x1b[16t\x1b]11;?\x07\x1b[?996n");
    assert_eq!(replies(&mut vt), "");
}

#[test]
fn reports_its_size_under_mode_2048() {
    let mut vt = Vt::new(&config());
    vt.write(b"\x1b[?2048h");
    assert_eq!(replies(&mut vt), "\x1b[48;24;80;528;800t");
    // The report's pixels are rows and columns of the whole-pixel cell.
    vt.resize(100, 30, 21, 45);
    assert_eq!(replies(&mut vt), "\x1b[48;30;100;1350;2100t");
    let s = vt.snapshot();
    assert_eq!(
        (s.cols, s.rows, s.cell_width, s.cell_height),
        (100, 30, 21, 45)
    );
    assert_eq!(s.screen.len(), 30);
    assert_eq!(s.dirty, Dirty::Full);
}

#[test]
fn turns_effects_into_events() {
    let mut vt = Vt::new(&config());
    vt.write(b"\x1b]0;hello title\x07\x1b]7;file://host/tmp\x07\x07");
    vt.write(b"\x1b]9;note body\x07\x1b]777;notify;T;B\x07");
    vt.write(b"\x1b]52;c;aGVsbG8=\x07\x1b]9;4;1;50\x07\x1b]9;4;0\x07\x1bc");
    assert_eq!(
        vt.take_events(),
        [
            Event::Title("hello title".into()),
            Event::Pwd("file://host/tmp".into()),
            Event::Bell,
            Event::Notification {
                title: String::new(),
                body: "note body".into(),
            },
            Event::Notification {
                title: "T".into(),
                body: "B".into(),
            },
            Event::Clipboard {
                location: ClipboardLocation::Standard,
                text: Some("hello".into()),
            },
            Event::Progress(Progress::Set(50)),
            Event::Progress(Progress::Remove),
            // A full reset (RIS) clears the progress report too.
            Event::Progress(Progress::Remove),
            Event::Reset,
        ]
    );
    assert_eq!(vt.take_events(), []);
}

#[test]
fn snapshots_carry_title_and_pwd() {
    let mut vt = Vt::new(&config());
    vt.write(b"\x1b]2;editor\x07\x1b]7;file://host/src\x07");
    let s = vt.snapshot();
    assert_eq!(
        (s.title.as_str(), s.pwd.as_str()),
        ("editor", "file://host/src")
    );
}

#[test]
fn clipboard_writes_can_be_denied() {
    let mut vt = Vt::new(&Config {
        clipboard_write: false,
        ..config()
    });
    vt.write(b"\x1b]52;c;aGVsbG8=\x07");
    assert_eq!(vt.take_events(), []);
}

/// Standard base64 with padding, for OSC 52 payloads.
fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let bits = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, &b)| acc | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(bits >> (18 - 6 * i) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[test]
fn clipboard_writes_are_capped() {
    // Herdr app's limit: exactly 1 MiB of text goes through, one byte more is refused whole.
    let limit = Config::default().clipboard_max_bytes;
    assert_eq!(limit, 1 << 20);
    let mut vt = Vt::new(&config());
    let osc52 = |text: &[u8]| format!("\x1b]52;c;{}\x07", base64(text)).into_bytes();

    vt.write(&osc52(&vec![b'x'; limit]));
    match vt.take_events().as_slice() {
        [
            Event::Clipboard {
                text: Some(text), ..
            },
        ] => assert_eq!(text.len(), limit),
        other => panic!("expected one clipboard write, got {} events", other.len()),
    }

    vt.write(&osc52(&vec![b'x'; limit + 1]));
    vt.write(&osc52(&vec![b'x'; 4 << 20]));
    assert_eq!(vt.take_events(), []);
    assert_eq!(replies(&mut vt), "");

    // Writes under the limit still go through afterwards.
    vt.write(&osc52(b"ok"));
    assert_eq!(
        vt.take_events(),
        [Event::Clipboard {
            location: ClipboardLocation::Standard,
            text: Some("ok".into()),
        }]
    );
}

#[test]
fn a_render_hold_shows_the_frame_from_when_it_began() {
    let mut vt = Vt::new(&config());
    vt.write(b"before");
    assert_eq!(vt.snapshot().line(0), "before");

    // The program begins a synchronized update and starts redrawing.
    vt.write(b"\x1b[?2026h\r\x1b[Kduring");
    assert!(vt.hold_started().is_some());
    assert!(
        vt.can_show(),
        "the frame captured when the hold began is new"
    );
    let held = vt.snapshot();
    assert_eq!(held.line(0), "before");
    assert!(held.modes.held);
    assert!(!vt.can_show(), "nothing new until the hold ends");
    vt.write(b" more");
    assert!(!vt.can_show());
    assert_eq!(vt.snapshot().line(0), "before");

    // It ends the update: the whole redraw shows at once.
    vt.write(b"\x1b[?2026l");
    assert!(vt.can_show());
    let done = vt.snapshot();
    assert_eq!(done.line(0), "during more");
    assert!(!done.modes.held);
}

#[test]
fn a_render_hold_expires() {
    let mut vt = Vt::new(&Config {
        render_hold_timeout: Duration::from_millis(50),
        ..config()
    });
    vt.write(b"\x1b[?2026hstuck");
    let began = vt.hold_started().expect("held");
    assert_eq!(vt.hold_deadline(), Some(began + Duration::from_millis(50)));
    vt.snapshot();
    assert!(!vt.expire_hold(began + Duration::from_millis(49)));
    assert!(!vt.can_show());

    assert!(vt.expire_hold(began + Duration::from_millis(50)));
    assert_eq!(vt.hold_started(), None);
    assert!(vt.can_show());
    let s = vt.snapshot();
    assert_eq!(s.line(0), "stuck");
    assert!(!s.modes.held);
    // The program sees the mode as off.
    vt.write(b"\x1b[?2026$p");
    assert_eq!(replies(&mut vt), "\x1b[?2026;2$y");
    assert!(!vt.expire_hold(Instant::now() + Duration::from_secs(5)));
}

fn lines(n: usize) -> Vec<u8> {
    (1..=n)
        .flat_map(|i| format!("line {i}\r\n").into_bytes())
        .collect()
}

#[test]
fn scrollback_keeps_history_and_scrolls() {
    let mut vt = Vt::new(&config());
    vt.write(&lines(100));
    let s = vt.snapshot();
    assert_eq!(s.line(22), "line 100");
    assert_eq!(s.scrollbar.total, 101);
    assert_eq!((s.scrollbar.offset, s.scrollbar.len), (77, 24));
    assert!(s.scrollbar.at_bottom);

    vt.scroll(Scroll::Top);
    let s = vt.snapshot();
    assert_eq!(s.line(0), "line 1");
    assert_eq!(s.scrollbar.offset, 0);
    assert!(!s.scrollbar.at_bottom);
    assert_eq!(s.cursor.position, None, "the cursor is below the viewport");

    vt.scroll(Scroll::Delta(10));
    assert_eq!(vt.snapshot().line(0), "line 11");
    vt.scroll(Scroll::Row(50));
    assert_eq!(vt.snapshot().line(0), "line 51");
    vt.scroll(Scroll::Bottom);
    let s = vt.snapshot();
    assert_eq!(s.line(0), "line 78");
    assert!(s.scrollbar.at_bottom);
}

#[test]
fn scrollback_can_be_turned_off() {
    let mut vt = Vt::new(&Config {
        scrollback: Scrollback {
            max_lines: None,
            max_bytes: Some(0),
        },
        ..config()
    });
    vt.write(&lines(100));
    let s = vt.snapshot();
    assert_eq!(s.scrollbar.total, 24);
    vt.scroll(Scroll::Top);
    assert_eq!(vt.snapshot().line(0), "line 78");
}

#[test]
fn scrollback_limits_prune_old_history() {
    // Pruning works in whole pages, so the limit is approximate in both directions.
    let mut vt = Vt::new(&Config {
        scrollback: Scrollback {
            max_lines: Some(1_000),
            max_bytes: None,
        },
        ..config()
    });
    vt.write(&lines(50_000));
    let total = vt.snapshot().scrollbar.total;
    assert!((500..=1_500).contains(&total), "kept {total} rows");
}

#[test]
fn themes_set_the_default_colors() {
    let theme = Theme {
        foreground: Some(Rgb::new(1, 2, 3)),
        background: Some(Rgb::new(4, 5, 6)),
        cursor: Some(Rgb::new(7, 8, 9)),
        palette: vec![(1, Rgb::new(0xaa, 0, 0))],
    };
    let mut vt = Vt::new(&Config {
        theme: theme.clone(),
        ..config()
    });
    vt.write(b"\x1b[31mred");
    let s = vt.snapshot();
    assert_eq!(s.colors.foreground, Rgb::new(1, 2, 3));
    assert_eq!(s.colors.background, Rgb::new(4, 5, 6));
    assert_eq!(s.colors.cursor, Some(Rgb::new(7, 8, 9)));
    assert_eq!(s.screen[0].cells[0].fg, Some(Rgb::new(0xaa, 0, 0)));
    let built_in = s.colors.palette[2];

    // A new theme rebuilds every row, and colors the program set itself (OSC 4) still win.
    vt.write(b"\x1b]4;2;rgb:00/bb/00\x07");
    vt.set_theme(&Theme {
        palette: vec![(1, Rgb::new(0xcc, 0, 0)), (2, Rgb::new(0, 0x11, 0))],
        ..theme
    });
    let s = vt.snapshot();
    assert_eq!(s.dirty, Dirty::Full);
    assert_eq!(s.screen[0].cells[0].fg, Some(Rgb::new(0xcc, 0, 0)));
    assert_eq!(s.colors.palette[2], Rgb::new(0, 0xbb, 0));
    assert_ne!(s.colors.palette[2], built_in);

    // Back to the library's defaults.
    vt.set_theme(&Theme::default());
    vt.write(b"\x1b]104\x07");
    let s = vt.snapshot();
    assert_eq!(s.colors.palette[2], built_in);
    assert_eq!(s.colors.cursor, None);
}

#[test]
fn keys_encode_for_the_programs_modes() {
    let mut vt = Vt::new(&config());
    let up = KeyEvent::press(GHOSTTY_KEY_ARROW_UP, Mods::NONE);
    assert_eq!(vt.encode_key(&up), b"\x1b[A");
    // Application cursor keys (DECCKM).
    vt.write(b"\x1b[?1h");
    assert_eq!(vt.encode_key(&up), b"\x1bOA");

    assert_eq!(vt.encode_key(&KeyEvent::text(GHOSTTY_KEY_C, "c")), b"c");
    let ctrl_c = KeyEvent {
        text: Some("\x03".into()),
        ..KeyEvent::press(GHOSTTY_KEY_C, Mods::CTRL)
    };
    assert_eq!(vt.encode_key(&ctrl_c), b"\x03");

    // Under the kitty keyboard protocol, Esc is unambiguous, and the Shift the layout used to
    // type `:` is consumed, so the program sees `:` rather than shift+;.
    vt.write(b"\x1b[>1u");
    assert_eq!(vt.modes().kitty_keyboard, 1);
    assert_eq!(
        vt.encode_key(&KeyEvent::press(GHOSTTY_KEY_ESCAPE, Mods::NONE)),
        b"\x1b[27u"
    );
    let colon = KeyEvent {
        mods: Mods::SHIFT,
        consumed_mods: Mods::SHIFT,
        text: Some(":".into()),
        unshifted: Some(';'),
        ..KeyEvent::default()
    };
    let colon = KeyEvent {
        key: GHOSTTY_KEY_SEMICOLON,
        ..colon
    };
    assert_eq!(vt.encode_key(&colon), b":");
}

#[test]
fn the_mouse_encodes_only_while_tracked() {
    let mut vt = Vt::new(&config());
    // Column 3, row 3 of a 10x22 cell grid.
    let click = MouseEvent {
        action: MouseAction::Press,
        button: Some(MouseButton::Left),
        mods: Mods::NONE,
        x: 25.0,
        y: 50.0,
        any_button_pressed: false,
    };
    assert_eq!(vt.encode_mouse(&click), b"");
    assert!(!vt.modes().mouse_tracking);

    vt.write(b"\x1b[?1002h\x1b[?1006h");
    assert!(vt.modes().mouse_tracking);
    assert_eq!(vt.encode_mouse(&click), b"\x1b[<0;3;3M");
    let drag = MouseEvent {
        action: MouseAction::Motion,
        x: 26.0,
        any_button_pressed: true,
        ..click
    };
    assert_eq!(
        vt.encode_mouse(&drag),
        b"",
        "motion within the last reported cell"
    );
    let drag = MouseEvent { x: 35.0, ..drag };
    assert_eq!(vt.encode_mouse(&drag), b"\x1b[<32;4;3M");
    let release = MouseEvent {
        action: MouseAction::Release,
        x: 35.0,
        ..click
    };
    assert_eq!(vt.encode_mouse(&release), b"\x1b[<0;4;3m");
}

#[test]
fn pastes_follow_bracketed_paste_and_refuse_unsafe_text() {
    let mut vt = Vt::new(&config());
    assert_eq!(vt.paste("hello", false), Ok(b"hello".to_vec()));
    assert_eq!(
        vt.paste("ls\nrm -rf x", false),
        Err(PasteRejected {
            text: "ls\nrm -rf x".into(),
        })
    );
    assert!(vt.paste("ls\nrm -rf x", true).is_ok_and(|b| !b.is_empty()));
    assert_eq!(vt.paste("", false), Ok(Vec::new()));

    vt.write(b"\x1b[?2004h");
    assert!(vt.modes().bracketed_paste);
    assert_eq!(
        vt.paste("ls\nrm", false),
        Ok(b"\x1b[200~ls\nrm\x1b[201~".to_vec())
    );
    // A paste never mixes with replies waiting to go out.
    vt.write(b"\x1b[5n");
    assert_eq!(vt.paste("x", false), Ok(b"\x1b[200~x\x1b[201~".to_vec()));
    assert_eq!(replies(&mut vt), "\x1b[0n");
}

#[test]
fn focus_encodes_as_csi_i_and_o() {
    assert_eq!(encode_focus(Focus::Gained), b"\x1b[I");
    assert_eq!(encode_focus(Focus::Lost), b"\x1b[O");
}

/// A pixel in the left half of a cell in `config()`'s 10x22 cells. Ghostty takes a cell into a
/// selection when the pointer crosses its middle, so presses go here...
fn at(col: u16, row: u16) -> (f32, f32) {
    (f32::from(col) * 10.0 + 2.0, f32::from(row) * 22.0 + 11.0)
}

/// ...and drags that end on a cell go past its middle.
fn past(col: u16, row: u16) -> (f32, f32) {
    (f32::from(col) * 10.0 + 8.0, f32::from(row) * 22.0 + 11.0)
}

fn select(vt: &mut Vt, action: SelectAction, (x, y): (f32, f32), clicks: u32) {
    let event = SelectEvent {
        clicks,
        ..SelectEvent::new(action, x, y)
    };
    vt.select(&event, Instant::now());
}

#[test]
fn dragging_selects_cells_and_a_click_clears_them() {
    let mut vt = Vt::new(&config());
    vt.write(b"hello world\r\nsecond line");
    select(&mut vt, SelectAction::Press, at(0, 0), 1);
    assert!(!vt.has_selection(), "a press alone selects nothing");
    select(&mut vt, SelectAction::Drag, past(4, 0), 1);
    select(&mut vt, SelectAction::Release, past(4, 0), 1);
    assert_eq!(vt.selection_text().as_deref(), Some("hello"));
    let s = vt.snapshot();
    assert_eq!(s.screen[0].selection, Some((0, 4)), "the frame shows it");

    // Across rows, the way text flows.
    select(&mut vt, SelectAction::Press, at(6, 0), 1);
    select(&mut vt, SelectAction::Drag, past(5, 1), 1);
    select(&mut vt, SelectAction::Release, past(5, 1), 1);
    assert_eq!(vt.selection_text().as_deref(), Some("world\nsecond"));

    select(&mut vt, SelectAction::Press, at(2, 1), 1);
    select(&mut vt, SelectAction::Release, at(2, 1), 1);
    assert_eq!(vt.selection_text(), None, "a plain click clears it");
}

#[test]
fn double_and_triple_clicks_select_words_and_lines() {
    let mut vt = Vt::new(&config());
    vt.write(b"cargo test --all\r\nnext");
    select(&mut vt, SelectAction::Press, at(7, 0), 2);
    assert_eq!(vt.selection_text().as_deref(), Some("test"));
    select(&mut vt, SelectAction::Release, at(7, 0), 2);
    select(&mut vt, SelectAction::Press, at(7, 0), 3);
    assert_eq!(vt.selection_text().as_deref(), Some("cargo test --all"));
}

#[test]
fn shift_click_moves_the_end_of_the_selection() {
    let mut vt = Vt::new(&config());
    vt.write(b"one two three");
    select(&mut vt, SelectAction::Press, at(0, 0), 1);
    select(&mut vt, SelectAction::Drag, past(2, 0), 1);
    select(&mut vt, SelectAction::Release, past(2, 0), 1);
    let (x, y) = past(6, 0);
    let extend = SelectEvent {
        extend: true,
        ..SelectEvent::new(SelectAction::Press, x, y)
    };
    vt.select(&extend, Instant::now());
    assert_eq!(vt.selection_text().as_deref(), Some("one two"));
    let (x, y) = past(12, 0);
    vt.select(
        &SelectEvent {
            extend: true,
            ..SelectEvent::new(SelectAction::Drag, x, y)
        },
        Instant::now(),
    );
    assert_eq!(vt.selection_text().as_deref(), Some("one two three"));
}

#[test]
fn a_drag_held_past_the_top_scrolls_back_and_keeps_selecting() {
    let mut vt = Vt::new(&Config {
        rows: 5,
        ..config()
    });
    vt.write(&lines(20));
    let top = vt.snapshot().scrollbar.offset;
    select(&mut vt, SelectAction::Press, at(3, 2), 1);
    select(&mut vt, SelectAction::Drag, (5.0, -30.0), 1);
    let due = vt
        .autoscroll_deadline()
        .expect("autoscroll while past the edge");
    assert!(
        !vt.autoscroll(due - Duration::from_millis(1)),
        "not before it's due"
    );
    assert!(vt.autoscroll(due));
    assert!(vt.autoscroll(due + terminal::AUTOSCROLL_EVERY));
    assert_eq!(vt.snapshot().scrollbar.offset, top - 2, "a row a tick");
    let text = vt.selection_text().expect("still selecting");
    assert!(text.starts_with("line 15\nline 16"), "{text:?}");

    // Back inside, the ticks stop; letting go keeps the selection.
    select(&mut vt, SelectAction::Drag, past(3, 1), 1);
    assert_eq!(vt.autoscroll_deadline(), None);
    select(&mut vt, SelectAction::Release, at(3, 1), 1);
    assert!(vt.has_selection());
}
