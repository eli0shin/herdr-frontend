//! Copy mode (`ctrl-g [`, `docs/research/feature-inventory.md` §16) in the window, on a fake
//! core through Rune's real runtime with no window: the bar, Vim's motions over the scrollback,
//! search, the visual modes, yanking to the clipboard and every way out.

use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::demo_core;
use herdr_frontend::link::{Fake, Out};
use herdr_frontend::{SIZE, State, app};
use model::Mode;
use model::copy_mode::Visual;
use rune::clipboard::{MemoryClipboard, memory};
use rune::{ElementInfo, Env, Key, KeyEvent, Mods, Runtime};

fn runtime() -> (Runtime<State>, Fake, MemoryClipboard) {
    let fake = demo_core();
    let board = memory();
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.use_clipboard(&board);
    rt.state.with_settings(Settings::fixture());
    rt.state.connect(Box::new(fake.clone()));
    rt.state.settle();
    frames(&mut rt, 3);
    let mut bytes = b"\x1bc".to_vec();
    bytes.extend((1..=100).flat_map(|i| format!("line {i}\r\n").into_bytes()));
    bytes.extend_from_slice(b"$ ");
    fake.output("t1", &bytes);
    frames(&mut rt, 3);
    (rt, fake, board)
}

fn frames(rt: &mut Runtime<State>, n: usize) {
    for _ in 0..n {
        rt.invalidate();
        rt.frame(1.0 / 60.0);
    }
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

fn text(rt: &Runtime<State>, key: &str) -> String {
    element(rt, key)
        .and_then(|e| e.text)
        .unwrap_or_else(|| panic!("no text at {key}"))
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused)
        .and_then(|e| e.key)
}

fn typed_into_t1(fake: &Fake) -> Vec<u8> {
    fake.sent_to("t1")
        .iter()
        .filter_map(|o| match o {
            Out::Input(bytes) => Some(bytes.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

const NONE: Mods = Mods {
    shift: false,
    cmd: false,
    alt: false,
    ctrl: false,
    logo: false,
};

fn press(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.state.settle();
    frames(rt, 2);
}

fn ctrl(rt: &mut Runtime<State>, c: char) {
    press(rt, Key::Text(c.to_string()), Mods { ctrl: true, ..NONE });
}

/// Types each character as a key, with Shift for capitals and shifted symbols.
fn keys(rt: &mut Runtime<State>, text: &str) {
    for c in text.chars() {
        let shift = c.is_ascii_uppercase() || "~!@#$%^&*()_+{}|:\"<>?".contains(c);
        press(rt, Key::Text(c.to_string()), Mods { shift, ..NONE });
    }
}

fn copy_mode(rt: &mut Runtime<State>) {
    ctrl(rt, 'g');
    keys(rt, "[");
}

fn cursor(rt: &Runtime<State>) -> (u16, u16) {
    shown(rt).cursor.position.expect("the cursor is in view")
}

fn cursor_line(rt: &Runtime<State>) -> String {
    let s = shown(rt);
    s.line(usize::from(s.cursor.position.unwrap().1))
}

#[test]
fn ctrl_g_bracket_opens_the_bar_at_the_programs_cursor() {
    let (mut rt, fake, _) = runtime();
    let before = typed_into_t1(&fake).len();
    copy_mode(&mut rt);
    assert!(matches!(rt.state.model.mode(), Mode::Copy(_)));
    assert!(rt.state.toasts.shown.is_empty(), "no not-yet toast");
    let bar = element(&rt, "copy-mode").expect("the bar");
    let zoom = rt.state.zoom();
    assert_eq!(
        bar.rect,
        [(SIZE.0 / zoom - 420.0) / 2.0, 8.0 / zoom, 420.0, 32.0],
        "420 × 32 at the top, as the jump bar hangs"
    );
    assert_eq!(focused(&rt).as_deref(), Some("copy-mode"));
    assert_eq!(text(&rt, "copy-mode-name"), "COPY");
    let total = shown(&rt).scrollbar.total;
    assert_eq!(text(&rt, "copy-row"), format!("101/{total}"));
    assert_eq!(cursor_line(&rt), "$");
    assert_eq!(cursor(&rt).0, 2, "at the prompt's cursor");

    // Motions with counts; the view follows into the history.
    keys(&mut rt, "3k");
    assert_eq!(cursor_line(&rt), "line 98");
    keys(&mut rt, "gg");
    assert_eq!(shown(&rt).line(0), "line 1");
    assert_eq!(cursor(&rt), (0, 0));
    assert_eq!(text(&rt, "copy-row"), format!("1/{total}"));
    keys(&mut rt, "w");
    assert_eq!(cursor(&rt), (5, 0));
    ctrl(&mut rt, 'd');
    assert!(shown(&rt).scrollbar.offset > 0, "Ctrl-D scrolls the view");
    keys(&mut rt, "G");
    assert_eq!(cursor_line(&rt), "$");
    assert_eq!(
        typed_into_t1(&fake).len(),
        before,
        "no key reached the program"
    );
}

#[test]
fn search_finds_in_the_scrollback_and_v_and_y_copy_to_the_clipboard() {
    let (mut rt, _fake, board) = runtime();
    copy_mode(&mut rt);
    keys(&mut rt, "/line 5");
    assert_eq!(text(&rt, "copy-hint"), "/line 5\u{258f}");
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(
        cursor_line(&rt),
        "line 5",
        "down from the prompt wraps to the top"
    );
    keys(&mut rt, "?line 50");
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(cursor_line(&rt), "line 50");
    let s = shown(&rt);
    let y = usize::from(cursor(&rt).1);
    assert!(
        s.screen[y].matches[0].selected,
        "the match under the cursor"
    );
    keys(&mut rt, "/nowhere");
    press(&mut rt, Key::Enter, NONE);
    assert_eq!(text(&rt, "copy-hint"), "Pattern not found: nowhere");

    // V selects rows; y copies them and leaves; the view goes back to the bottom.
    keys(&mut rt, "Vj");
    assert_eq!(text(&rt, "copy-mode-name"), "VISUAL LINE");
    assert!(shown(&rt).screen[y].selection.is_some());
    keys(&mut rt, "y");
    frames(&mut rt, 3);
    assert_eq!(
        board.text().as_deref().map(String::as_str),
        Some("line 50\nline 51")
    );
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
    assert!(element(&rt, "copy-mode").is_none());
    let s = shown(&rt);
    assert!(s.scrollbar.at_bottom);
    assert!(
        s.screen
            .iter()
            .all(|r| r.selection.is_none() && r.matches.is_empty())
    );
    assert_eq!(s.copy, None);
}

#[test]
fn text_objects_blocks_and_yanks_over_motions() {
    let (mut rt, fake, board) = runtime();
    fake.output("t1", b"\x1bcecho \"hello world\" (one two)\r\n$ ");
    frames(&mut rt, 3);
    copy_mode(&mut rt);
    keys(&mut rt, "kfh");
    keys(&mut rt, "vi\"");
    assert_eq!(text(&rt, "copy-mode-name"), "VISUAL");
    press(&mut rt, Key::Enter, NONE);
    frames(&mut rt, 3);
    assert_eq!(
        board.text().as_deref().map(String::as_str),
        Some("hello world")
    );

    copy_mode(&mut rt);
    keys(&mut rt, "k$");
    keys(&mut rt, "ya(");
    frames(&mut rt, 3);
    assert_eq!(
        board.text().as_deref().map(String::as_str),
        Some("(one two)")
    );

    copy_mode(&mut rt);
    keys(&mut rt, "k0yiW");
    frames(&mut rt, 3);
    assert_eq!(board.text().as_deref().map(String::as_str), Some("echo"));

    copy_mode(&mut rt);
    keys(&mut rt, "0");
    ctrl(&mut rt, 'v');
    let Mode::Copy(copy) = rt.state.model.mode() else {
        panic!()
    };
    assert_eq!(
        copy.visual(),
        Some(Visual::Block),
        "Ctrl-V is a block, not a paste"
    );
    assert_eq!(text(&rt, "copy-mode-name"), "VISUAL BLOCK");
    keys(&mut rt, "kl");
    keys(&mut rt, "y");
    frames(&mut rt, 3);
    assert_eq!(board.text().as_deref().map(String::as_str), Some("ec\n$"));
}

#[test]
fn escape_drops_the_selection_then_leaves_and_q_ctrl_c_and_a_press_outside_leave() {
    let (mut rt, _fake, board) = runtime();
    board.set_text("before");
    copy_mode(&mut rt);
    keys(&mut rt, "vk");
    assert_eq!(text(&rt, "copy-mode-name"), "VISUAL");
    press(&mut rt, Key::Escape, NONE);
    assert_eq!(text(&rt, "copy-mode-name"), "COPY");
    assert!(shown(&rt).screen.iter().all(|r| r.selection.is_none()));
    press(&mut rt, Key::Escape, NONE);
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
    assert_eq!(
        focused(&rt).as_deref(),
        Some("pane-pn-1"),
        "the pane has the keyboard"
    );

    copy_mode(&mut rt);
    keys(&mut rt, "vkq");
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
    copy_mode(&mut rt);
    keys(&mut rt, "v");
    ctrl(&mut rt, 'c');
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
    copy_mode(&mut rt);
    keys(&mut rt, "vk");
    let (x, y) = rt.find("copy-mode").unwrap();
    rt.pointer_down(x, y + 200.0);
    rt.pointer_up(x, y + 200.0);
    rt.state.settle();
    frames(&mut rt, 3);
    assert_eq!(
        *rt.state.model.mode(),
        Mode::Normal,
        "a press outside leaves"
    );
    assert_eq!(
        board.text().as_deref().map(String::as_str),
        Some("before"),
        "none of them copied"
    );
    assert!(shown(&rt).screen.iter().all(|r| r.selection.is_none()));
}
