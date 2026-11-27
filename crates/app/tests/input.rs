//! Terminal input on a fake core, through Rune's real runtime with no window: keys with their
//! physical key and releases, the `terminal` key sends and copy and paste through the keymap, the input method, the mouse (reports,
//! selection, the wheel, right-clicks), pastes, dropped files and focus. Each asserts the bytes
//! the core was sent (`docs/research/terminal-rendering.md` §3).

use std::path::PathBuf;

use herdr_frontend::fixtures::demo_core;
use herdr_frontend::link::{Fake, Out, Upload};
use herdr_frontend::{SIZE, State, app};
use model::{Action, TerminalAction};
use rune::clipboard::{MemoryClipboard, memory};
use rune::{
    ClipboardContent, ClipboardImage, Env, Ime, Key, KeyEvent, Mods, PhysicalInput,
    PhysicalKeyCode, PointerButton, Runtime,
};

/// A runtime on the demo core, settled and laid out, on an in-memory clipboard. `pn-1` (`t1`)
/// has the keyboard.
fn runtime() -> (Runtime<State>, Fake, MemoryClipboard) {
    runtime_on(demo_core())
}

fn runtime_on(fake: Fake) -> (Runtime<State>, Fake, MemoryClipboard) {
    let board = memory();
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.use_clipboard(&board);
    rt.state.connect(Box::new(fake.clone()));
    rt.state.settle();
    frames(&mut rt, 3);
    (rt, fake, board)
}

fn frames(rt: &mut Runtime<State>, n: usize) {
    for _ in 0..n {
        rt.invalidate();
        rt.frame(1.0 / 60.0);
    }
}

/// The program in `t1` writes `bytes`, and the app draws them.
fn program(rt: &mut Runtime<State>, fake: &Fake, bytes: &[u8]) {
    fake.output("t1", bytes);
    frames(rt, 3);
}

/// `t1` reset (RIS) to a shell on the primary screen showing `text`: none of the Neovim
/// fixture's modes.
fn shell(rt: &mut Runtime<State>, fake: &Fake, text: &str) {
    let mut bytes = b"\x1bc".to_vec();
    bytes.extend_from_slice(text.as_bytes());
    program(rt, fake, &bytes);
}

/// The DATA frames `t1` was sent since `from`, joined.
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

fn press(rt: &mut Runtime<State>, key: Key, mods: Mods, code: PhysicalKeyCode, base: Option<char>) {
    rt.key_with(KeyEvent { key, mods }, code, base);
    rt.frame(1.0 / 60.0);
}

fn release(
    rt: &mut Runtime<State>,
    key: Key,
    mods: Mods,
    code: PhysicalKeyCode,
    base: Option<char>,
) {
    rt.physical_key_input(
        PhysicalInput::Key {
            key: code,
            mods,
            pressed: false,
            repeat: false,
            text: None,
            consumed_mods: Default::default(),
        },
        KeyEvent { key, mods },
        base,
    );
    rt.frame(1.0 / 60.0);
}

const NONE: Mods = Mods {
    shift: false,
    cmd: false,
    alt: false,
    ctrl: false,
    logo: false,
};

fn with(f: impl FnOnce(&mut Mods)) -> Mods {
    let mut m = NONE;
    f(&mut m);
    m
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

#[test]
fn keys_go_with_their_physical_key_and_mods() {
    let (mut rt, fake, _) = runtime();
    shell(&mut rt, &fake, "$ ");
    let from = mark(&fake);
    press(
        &mut rt,
        Key::Text("l".into()),
        NONE,
        PhysicalKeyCode::KeyL,
        Some('l'),
    );
    press(
        &mut rt,
        Key::Text("s".into()),
        NONE,
        PhysicalKeyCode::KeyS,
        Some('s'),
    );
    press(&mut rt, Key::Enter, NONE, PhysicalKeyCode::Enter, None);
    let ctrl = with(|m| m.ctrl = true);
    press(
        &mut rt,
        Key::Text("c".into()),
        ctrl,
        PhysicalKeyCode::KeyC,
        Some('c'),
    );
    press(
        &mut rt,
        Key::Text("[".into()),
        ctrl,
        PhysicalKeyCode::BracketLeft,
        Some('['),
    );
    // Option is Alt: Rune hands it over as the key it's on.
    let alt = with(|m| m.alt = true);
    press(
        &mut rt,
        Key::Text("b".into()),
        alt,
        PhysicalKeyCode::KeyB,
        Some('b'),
    );
    press(&mut rt, Key::Left, ctrl, PhysicalKeyCode::ArrowLeft, None);
    assert_eq!(typed(&fake, from), b"ls\r\x03\x1b\x1bb\x1b[1;5D");

    // Cmd chords are the app's.
    let from = mark(&fake);
    let cmd = with(|m| m.cmd = true);
    press(
        &mut rt,
        Key::Text("t".into()),
        cmd,
        PhysicalKeyCode::KeyT,
        Some('t'),
    );
    assert_eq!(typed(&fake, from), b"");
}

#[test]
fn the_kitty_protocol_hears_consumed_shift_repeats_and_releases() {
    let (mut rt, fake, _) = runtime();
    shell(&mut rt, &fake, "$ ");
    // Disambiguate, event types, every key as an escape code.
    program(&mut rt, &fake, b"\x1b[>11u");
    let from = mark(&fake);
    let shift = with(|m| m.shift = true);
    press(
        &mut rt,
        Key::Text(":".into()),
        shift,
        PhysicalKeyCode::Semicolon,
        Some(';'),
    );
    release(
        &mut rt,
        Key::Text(":".into()),
        shift,
        PhysicalKeyCode::Semicolon,
        Some(';'),
    );
    assert_eq!(typed(&fake, from), b"\x1b[59;2u\x1b[59;2:3u");

    // A release whose press the terminal never heard isn't sent.
    let from = mark(&fake);
    release(
        &mut rt,
        Key::Text("x".into()),
        NONE,
        PhysicalKeyCode::KeyX,
        Some('x'),
    );
    assert_eq!(typed(&fake, from), b"");

    // Under flags 1|2 only, a text key types its text and its release is a code; Escape is
    // unambiguous.
    program(&mut rt, &fake, b"\x1b[<u\x1b[>3u");
    let from = mark(&fake);
    press(
        &mut rt,
        Key::Text("a".into()),
        NONE,
        PhysicalKeyCode::KeyA,
        Some('a'),
    );
    release(
        &mut rt,
        Key::Text("a".into()),
        NONE,
        PhysicalKeyCode::KeyA,
        Some('a'),
    );
    press(&mut rt, Key::Escape, NONE, PhysicalKeyCode::Escape, None);
    release(&mut rt, Key::Escape, NONE, PhysicalKeyCode::Escape, None);
    assert_eq!(typed(&fake, from), b"a\x1b[97;1:3u\x1b[27u\x1b[27;1:3u");
}

#[test]
fn the_terminal_key_sends_go_as_their_bytes() {
    let (mut rt, fake, _) = runtime();
    shell(&mut rt, &fake, "$ ");
    // As macOS delivers them; the keymap takes each before the pane hears it.
    let alt = with(|m| m.alt = true);
    let cmd = with(|m| {
        m.cmd = true;
        m.logo = true;
    });
    let ctrl = with(|m| m.ctrl = true);
    for (what, keys, bytes) in [
        ("alt-left", vec![(Key::Left, alt)], b"\x1bb".as_slice()),
        ("alt-right", vec![(Key::Right, alt)], b"\x1bf"),
        ("alt-backspace", vec![(Key::Backspace, alt)], b"\x17"),
        ("cmd-left", vec![(Key::Left, cmd)], b"\x01"),
        ("cmd-right", vec![(Key::Right, cmd)], b"\x05"),
        ("cmd-backspace", vec![(Key::Backspace, cmd)], b"\x15"),
        (
            "ctrl-g ctrl-l",
            vec![(Key::Text("g".into()), ctrl), (Key::Text("l".into()), ctrl)],
            b"\x0c",
        ),
    ] {
        let from = mark(&fake);
        for (key, mods) in keys {
            rt.key(KeyEvent { key, mods });
            rt.frame(1.0 / 60.0);
        }
        frames(&mut rt, 2);
        assert_eq!(typed(&fake, from), bytes, "{what}");
    }
}

#[test]
fn cmd_v_pastes_and_cmd_c_copies_the_selection() {
    let (mut rt, fake, board) = runtime();
    shell(&mut rt, &fake, "hello world\r\n$ ");
    let cmd = with(|m| {
        m.cmd = true;
        m.logo = true;
    });
    board.set_text("pwd");
    let from = mark(&fake);
    rt.key(KeyEvent {
        key: Key::Command('v'),
        mods: cmd,
    });
    frames(&mut rt, 3);
    assert_eq!(typed(&fake, from), b"pwd");

    // Select "world" with a double click, put something else on the clipboard, then copy.
    let (x, y) = cell(&rt, 7, 0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(0.05);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, 3);
    board.set_text("other");
    rt.key(KeyEvent {
        key: Key::Command('c'),
        mods: cmd,
    });
    frames(&mut rt, 3);
    assert_eq!(board.text().as_deref().map(String::as_str), Some("world"));
}

#[test]
fn the_input_method_composes_at_the_cursor_and_commits_its_text() {
    let (mut rt, fake, _) = runtime();
    shell(&mut rt, &fake, "$ ");
    assert!(
        rt.elements()
            .iter()
            .any(|e| e.key.as_deref() == Some("ime")),
        "the focused pane marks the caret"
    );
    rt.ime(Ime::Preedit("にほ".into(), Some((6, 6)), Vec::new()));
    frames(&mut rt, 1);
    let preedit = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("preedit-text"))
        .expect("the composition shows");
    assert_eq!(preedit.text.as_deref(), Some("にほ"));
    let from = mark(&fake);
    // A key while composing is the input method's.
    press(
        &mut rt,
        Key::Text("n".into()),
        NONE,
        PhysicalKeyCode::KeyN,
        Some('n'),
    );
    assert_eq!(typed(&fake, from), b"");

    rt.ime(Ime::Commit("日本".into()));
    frames(&mut rt, 2);
    assert_eq!(typed(&fake, from), "日本".as_bytes());
    assert!(
        !rt.elements()
            .iter()
            .any(|e| e.key.as_deref() == Some("preedit")),
        "the composition is gone"
    );
}

#[test]
fn a_program_tracking_the_mouse_hears_clicks_drags_and_the_wheel() {
    let (mut rt, fake, _) = runtime();
    // Button-event tracking in SGR form.
    shell(&mut rt, &fake, "$ ");
    program(&mut rt, &fake, b"\x1b[?1002h\x1b[?1006h");
    let from = mark(&fake);
    let (x, y) = cell(&rt, 2, 1);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    let (x2, y2) = cell(&rt, 5, 1);
    rt.pointer_move(x2, y2);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x2, y2);
    frames(&mut rt, 2);
    assert_eq!(
        String::from_utf8(typed(&fake, from)).unwrap(),
        "\x1b[<0;3;2M\x1b[<32;6;2M\x1b[<0;6;2m"
    );

    // A notch of the wheel is three lines; up is button 64.
    let from = mark(&fake);
    rt.wheel_xy(x, y, 0.0, 40.0);
    frames(&mut rt, 2);
    assert_eq!(
        String::from_utf8(typed(&fake, from)).unwrap(),
        "\x1b[<64;3;2M".repeat(3)
    );
}

#[test]
fn dragging_selects_and_letting_go_copies_while_the_program_leaves_the_mouse() {
    let (mut rt, fake, board) = runtime();
    shell(&mut rt, &fake, "hello world\r\n$ ");
    let from = mark(&fake);
    let (x, y) = cell(&rt, 0, 0);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    let (x2, y2) = cell(&rt, 4, 0);
    rt.pointer_move(x2 + 6.0, y2);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x2 + 6.0, y2);
    frames(&mut rt, 3);
    assert_eq!(typed(&fake, from), b"", "nothing for the program");
    assert_eq!(board.text().as_deref().map(String::as_str), Some("hello"));
    let shown = rt.state.viewers["t1"]
        .pane
        .as_ref()
        .unwrap()
        .snapshot
        .clone();
    assert_eq!(shown.screen[0].selection, Some((0, 4)));

    // Typing drops it.
    press(
        &mut rt,
        Key::Text("x".into()),
        NONE,
        PhysicalKeyCode::KeyX,
        Some('x'),
    );
    frames(&mut rt, 2);
    let shown = rt.state.viewers["t1"]
        .pane
        .as_ref()
        .unwrap()
        .snapshot
        .clone();
    assert!(shown.screen.iter().all(|r| r.selection.is_none()));
}

#[test]
fn the_wheel_scrolls_the_scrollback_or_sends_arrows_on_the_alternate_screen() {
    let (mut rt, fake, _) = runtime();
    let lines: String = (1..=100).map(|i| format!("line {i}\r\n")).collect();
    shell(&mut rt, &fake, &lines);
    let (x, y) = cell(&rt, 2, 2);
    let from = mark(&fake);
    rt.wheel_xy(x, y, 0.0, 40.0);
    frames(&mut rt, 3);
    assert_eq!(typed(&fake, from), b"", "the scrollback is the terminal's");
    let shown = rt.state.viewers["t1"]
        .pane
        .as_ref()
        .unwrap()
        .snapshot
        .clone();
    assert!(!shown.scrollbar.at_bottom, "scrolled back");

    // Typing returns to the bottom.
    press(
        &mut rt,
        Key::Text("x".into()),
        NONE,
        PhysicalKeyCode::KeyX,
        Some('x'),
    );
    frames(&mut rt, 3);
    let shown = rt.state.viewers["t1"]
        .pane
        .as_ref()
        .unwrap()
        .snapshot
        .clone();
    assert!(shown.scrollbar.at_bottom);

    // A pager on the alternate screen that doesn't track the mouse gets arrows.
    program(&mut rt, &fake, b"\x1b[?1049h");
    let from = mark(&fake);
    rt.wheel_xy(x, y, 0.0, -40.0);
    frames(&mut rt, 3);
    assert_eq!(typed(&fake, from), b"\x1b[B\x1b[B\x1b[B");
}

#[test]
fn pastes_follow_bracketed_paste_and_files_paste_as_their_paths() {
    let (mut rt, fake, board) = runtime();
    shell(&mut rt, &fake, "$ ");
    let paste = |rt: &mut Runtime<State>| {
        rt.update(|s: &mut State, tx| {
            s.step(tx.clock(), Action::Terminal(TerminalAction::Paste));
        });
        frames(rt, 3);
    };
    board.set_text("echo one\necho two");
    let from = mark(&fake);
    paste(&mut rt);
    assert_eq!(
        typed(&fake, from),
        b"echo one\recho two",
        "no confirmation, as Herdr app"
    );

    program(&mut rt, &fake, b"\x1b[?2004h");
    let from = mark(&fake);
    paste(&mut rt);
    assert_eq!(typed(&fake, from), b"\x1b[200~echo one\necho two\x1b[201~");

    // Copied files paste as if dropped: quoted where the shell needs it.
    board.set_content(ClipboardContent::default().with_files([
        PathBuf::from("/tmp/shot.png"),
        PathBuf::from("/tmp/a file's.jpg"),
    ]));
    let from = mark(&fake);
    paste(&mut rt);
    assert_eq!(
        String::from_utf8(typed(&fake, from)).unwrap(),
        "\x1b[200~/tmp/shot.png '/tmp/a file'\\''s.jpg' \x1b[201~"
    );

    // A paste Rune delivers itself (its Edit menu, a script) goes to the focused terminal.
    let from = mark(&fake);
    rt.paste("pwd");
    frames(&mut rt, 3);
    assert_eq!(typed(&fake, from), b"\x1b[200~pwd\x1b[201~");
}

#[test]
fn a_file_dropped_on_a_pane_types_its_path() {
    let (mut rt, fake, _) = runtime();
    shell(&mut rt, &fake, "$ ");
    let from = mark(&fake);
    let (x, y) = cell(&rt, 3, 3);
    rt.drop_files(
        vec![PathBuf::from("/Users/me/Desktop/Screen Shot.png")],
        Some((x, y)),
    );
    frames(&mut rt, 3);
    assert_eq!(typed(&fake, from), b"'/Users/me/Desktop/Screen Shot.png' ");
}

#[test]
fn a_remote_terminal_gets_files_copied_beside_it_first() {
    let (mut rt, fake, _) = runtime();
    rt.update(|s: &mut State, _| {
        s.paste_content(
            "box/t9",
            &ClipboardContent::default().with_files([PathBuf::from("/tmp/shot.png")]),
        )
    });
    frames(&mut rt, 2);
    // An image alone goes there as a PNG of its own.
    let png = ClipboardImage::from_rgba(1, 1, &[255, 0, 0, 255]).unwrap();
    rt.update(|s: &mut State, _| {
        s.paste_content(
            "box/t9",
            &ClipboardContent::default().with_image(png.clone()),
        )
    });
    frames(&mut rt, 2);
    let core = fake.core();
    assert_eq!(core.puts.len(), 2);
    assert_eq!(
        core.puts[0],
        (
            "box/t9".to_owned(),
            vec![Upload::File(PathBuf::from("/tmp/shot.png"))]
        )
    );
    let (address, uploads) = &core.puts[1];
    assert_eq!(address, "box/t9");
    match uploads.as_slice() {
        [Upload::Bytes { name, data }] => {
            assert!(
                name.starts_with("herdr-paste-") && name.ends_with(".png"),
                "{name}"
            );
            assert_eq!(&data[..], png.png());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_image_alone_is_ctrl_v_for_a_local_program() {
    let (mut rt, fake, board) = runtime();
    shell(&mut rt, &fake, "$ ");
    let png = ClipboardImage::from_rgba(1, 1, &[255, 0, 0, 255]).unwrap();
    board.set_content(ClipboardContent::default().with_image(png));
    let from = mark(&fake);
    rt.update(|s: &mut State, tx| {
        s.step(tx.clock(), Action::Terminal(TerminalAction::PasteImage));
    });
    frames(&mut rt, 3);
    assert_eq!(typed(&fake, from), b"\x16");
}

#[test]
fn only_the_pane_in_use_sends_the_program_pointer_input() {
    let (mut rt, fake, _) = runtime();
    // t2 tracks the mouse, but pn-1 (t1) has the keyboard.
    fake.output("t2", b"\x1bc\x1b[?1003h\x1b[?1006h");
    frames(&mut rt, 3);
    let (x, y) = rt.find("pane-pn-2").expect("pane 2");
    rt.wheel_xy(x, y, 0.0, 40.0);
    rt.pointer_move(x + 20.0, y + 20.0);
    frames(&mut rt, 3);
    assert_eq!(fake.sent_to("t2"), [], "no DATA, so no stale size taken");
}

#[test]
fn a_composition_left_behind_by_a_pane_switch_is_dropped() {
    let (mut rt, fake, _) = runtime();
    shell(&mut rt, &fake, "$ ");
    rt.ime(Ime::Preedit("に".into(), None, Vec::new()));
    frames(&mut rt, 1);
    assert!(rt.state.viewers["t1"].preedit.is_some());
    let (x, y) = rt.find("pane-pn-2").expect("pane 2");
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, 2);
    assert_eq!(rt.state.viewers["t1"].preedit, None);

    // Back in pn-1, keys type again.
    let (x, y) = cell(&rt, 1, 1);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, 2);
    let from = mark(&fake);
    press(
        &mut rt,
        Key::Text("a".into()),
        NONE,
        PhysicalKeyCode::KeyA,
        Some('a'),
    );
    assert_eq!(typed(&fake, from), b"a");
}

#[test]
fn right_clicks_go_to_the_program_only_when_its_terminal_says_so() {
    let fake = demo_core();
    fake.core()
        .terminals
        .get_mut("t1")
        .unwrap()
        .labels
        .insert("herdr.right-click".into(), "program".into());
    let (mut rt, fake, _) = runtime_on(fake);
    shell(&mut rt, &fake, "$ ");
    program(&mut rt, &fake, b"\x1b[?1000h\x1b[?1006h");
    // From another pane: the label decides, and the pane takes the size first.
    let (x, y) = rt.find("pane-pn-2").expect("pane 2");
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, 2);
    assert_eq!(rt.state.in_use(), Some("t2"));
    let from = mark(&fake);
    let (x, y) = cell(&rt, 1, 0);
    rt.secondary_down(x, y);
    rt.button_up(x, y, PointerButton::Secondary);
    frames(&mut rt, 3);
    assert_eq!(
        String::from_utf8(typed(&fake, from)).unwrap(),
        "\x1b[<2;2;1M\x1b[<2;2;1m"
    );
    assert_eq!(rt.state.in_use(), Some("t1"));
    let sent = &fake.sent_to("t1")[from..];
    let resize = sent.iter().position(|o| matches!(o, Out::Resize { .. }));
    let data = sent.iter().position(|o| matches!(o, Out::Input(_)));
    assert!(
        resize.is_some() && resize < data,
        "RESIZE before the click: {sent:?}"
    );
}

#[test]
fn a_drop_in_a_window_in_the_background_takes_the_panes_size_first() {
    let (mut rt, fake, _) = runtime();
    rt.window_focus(false);
    frames(&mut rt, 2);
    let (x, y) = rt.find("pane-pn-3").expect("pane 3");
    rt.drop_files(vec![PathBuf::from("/tmp/a.png")], Some((x, y)));
    frames(&mut rt, 3);
    let sent = fake.sent_to("t3");
    assert!(matches!(sent.first(), Some(Out::Resize { .. })), "{sent:?}");
    assert!(sent.contains(&Out::Input(b"/tmp/a.png ".to_vec())));
}

#[test]
fn focus_reaches_the_program_through_the_core_only() {
    let (mut rt, fake, _) = runtime();
    // The program asks for focus events; the app still sends FOCUS frames, never CSI I/O.
    program(&mut rt, &fake, b"\x1b[?1004h");
    let from = mark(&fake);
    let (x, y) = rt.find("pane-pn-2").expect("pane 2");
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, 2);
    rt.window_focus(false);
    frames(&mut rt, 2);
    let sent = &fake.sent_to("t1")[from..];
    assert_eq!(sent, [Out::Focus(false)]);
    assert!(fake.sent_to("t2").contains(&Out::Focus(true)));
    for address in ["t1", "t2"] {
        assert!(
            !fake.sent_to(address).iter().any(|o| matches!(
                o,
                Out::Input(b) | Out::Automatic(b) if b.windows(3).any(|w| w == b"\x1b[I" || w == b"\x1b[O")
            )),
            "{address} was sent focus bytes"
        );
    }
}
