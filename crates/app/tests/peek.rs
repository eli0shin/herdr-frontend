//! The peek (`feature-inventory.md` §15) in the window: a sub-agent's row or a Cmd-click on a
//! terminal address opens it over the window; its terminal takes the keyboard (Esc too) and
//! the size; ✕, a click outside and `cmd-w` close it; "↗ keep" gives it a tab.

use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::agents_core;
use herdr_frontend::link::{Fake, Out};
use herdr_frontend::state::PEEK_KEY;
use herdr_frontend::{SIZE, State, app};
use rune::{ElementInfo, Env, Key, KeyEvent, Mods, PhysicalInput, Runtime};

/// A runtime on the agents core, settled and laid out. `t1` shows `see t22 now`.
fn runtime() -> (Runtime<State>, Fake) {
    let fake = agents_core();
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.with_settings(Settings::fixture());
    rt.state.connect(Box::new(fake.clone()));
    rt.state.settle();
    frames(&mut rt, 3);
    fake.output("t1", b"\x1bcsee t22 now");
    rt.state.settle();
    frames(&mut rt, 3);
    (rt, fake)
}

fn frames(rt: &mut Runtime<State>, n: usize) {
    for _ in 0..n {
        rt.invalidate();
        rt.frame(1.0 / 60.0);
    }
}

fn element(rt: &Runtime<State>, key: &str) -> Option<ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn text_of(rt: &Runtime<State>, key: &str) -> String {
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

fn click(rt: &mut Runtime<State>, path: &str) {
    let (x, y) = rt.find(path).unwrap_or_else(|| panic!("no {path}"));
    click_at(rt, x, y);
}

fn click_at(rt: &mut Runtime<State>, x: f32, y: f32) {
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.state.settle();
    frames(rt, 2);
}

fn key(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.state.settle();
    frames(rt, 2);
}

fn typed(fake: &Fake, address: &str) -> Vec<u8> {
    fake.sent_to(address)
        .into_iter()
        .filter_map(|o| match o {
            Out::Input(bytes) => Some(bytes),
            _ => None,
        })
        .flatten()
        .collect()
}

const CMD: Mods = Mods {
    shift: false,
    cmd: true,
    alt: false,
    ctrl: false,
    logo: true,
};

#[test]
fn a_sub_agents_row_opens_the_peek_with_its_bar_and_terminal() {
    let (mut rt, fake) = runtime();
    click(&mut rt, "sub-t20");
    assert_eq!(rt.state.model.peeked(), Some("t20"));
    assert_eq!(rt.state.model.focused_terminal(), Some("t1"), "focus stays");
    let dialog = element(&rt, "peek").expect("the peek");
    assert!(
        (dialog.rect[2] - SIZE.0 * 0.8).abs() < 1.0,
        "{:?}",
        dialog.rect
    );
    assert!(
        (dialog.rect[3] - SIZE.1 * 0.8).abs() < 1.0,
        "{:?}",
        dialog.rect
    );
    assert_eq!(element(&rt, "peek-bar").unwrap().rect[3], 34.0);
    assert_eq!(text_of(&rt, "peek-name"), "explore");
    assert_eq!(text_of(&rt, "peek-status"), "working");
    assert!(text_of(&rt, "peek-from").starts_with("from "));
    assert!(element(&rt, "peek-spinner").is_some());
    assert_eq!(text_of(&rt, "peek-keep"), "↗ keep");
    assert!(element(&rt, "peek-close").is_some());
    // Its terminal has the keyboard, and takes the size, as the pane in use does.
    assert_eq!(focused(&rt).as_deref(), Some(PEEK_KEY));
    let sent = fake.sent_to("t20");
    assert!(
        sent.iter().any(|o| matches!(o, Out::Resize { .. })),
        "{sent:?}"
    );
    assert!(sent.contains(&Out::Focus(true)));
    assert_eq!(rt.state.in_use(), Some("t20"));
}

#[test]
fn keys_go_to_the_peeked_terminal_and_esc_too() {
    let (mut rt, fake) = runtime();
    click(&mut rt, "sub-t20");
    key(&mut rt, Key::Text("x".into()), Mods::default());
    key(&mut rt, Key::Escape, Mods::default());
    assert_eq!(typed(&fake, "t20"), b"x\x1b");
    assert!(
        typed(&fake, "t1").is_empty(),
        "nothing reaches the pane behind"
    );
    assert_eq!(rt.state.model.peeked(), Some("t20"), "Esc doesn't close it");
    // Closing a tab by key closes the peek instead; the keyboard goes back to the pane.
    key(&mut rt, Key::Command('w'), CMD);
    assert_eq!(rt.state.model.peeked(), None);
    assert!(element(&rt, "peek").is_none());
    assert_eq!(rt.state.model.shown_tab().unwrap().id, "tb-work");
    assert_eq!(focused(&rt).as_deref(), Some("pane-pn-1"));
    // Its viewer goes with it (a terminal with no pane has nothing to show), and the pane
    // takes the size back.
    assert!(!rt.state.viewers.contains_key("t20"));
    assert_eq!(rt.state.in_use(), Some("t1"));
}

/// The peek has no find bar: `cmd-f` there says "not yet" and starts no search.
#[test]
fn cmd_f_in_the_peek_says_not_yet() {
    let (mut rt, _) = runtime();
    click(&mut rt, "sub-t20");
    key(&mut rt, Key::Command('f'), CMD);
    assert_eq!(rt.state.model.peeked(), Some("t20"), "the peek stays");
    assert!(
        rt.state
            .toasts
            .shown
            .iter()
            .any(|t| t.toast.title == "Not yet" && t.toast.body.contains("terminal.search")),
        "{:?}",
        rt.state
            .toasts
            .shown
            .iter()
            .map(|t| &t.toast)
            .collect::<Vec<_>>()
    );
    assert!(rt.state.viewers["t20"].find.is_none(), "no search started");
    assert_eq!(
        focused(&rt).as_deref(),
        Some(PEEK_KEY),
        "the keyboard stays"
    );
}

#[test]
fn the_close_button_or_a_click_outside_closes_it() {
    let (mut rt, _) = runtime();
    click(&mut rt, "sub-t20");
    click(&mut rt, "peek-close");
    assert_eq!(rt.state.model.peeked(), None);
    click(&mut rt, "sub-t20");
    click_at(&mut rt, 4.0, SIZE.1 - 4.0);
    assert_eq!(rt.state.model.peeked(), None);
    // A click inside stays inside.
    click(&mut rt, "sub-t20");
    click(&mut rt, "peek-bar");
    assert_eq!(rt.state.model.peeked(), Some("t20"));
}

#[test]
fn keep_gives_it_a_tab_and_focuses_it() {
    let (mut rt, fake) = runtime();
    click(&mut rt, "sub-t20");
    click(&mut rt, "peek-keep");
    assert_eq!(rt.state.model.peeked(), None);
    assert_eq!(rt.state.model.focused_terminal(), Some("t20"));
    let t20 = &fake.core().terminals["t20"];
    assert_eq!(
        t20.labels.get("herdr.space").map(String::as_str),
        Some("sp-1")
    );
    assert!(t20.keep);
}

#[test]
fn go_to_shows_a_peeked_terminals_own_pane() {
    let (mut rt, _) = runtime();
    let now = rt.state.model.now();
    rt.state.step(now, model::Action::Peek("t12".into()));
    rt.state.settle();
    frames(&mut rt, 2);
    assert_eq!(text_of(&rt, "peek-keep"), "↗ go to");
    click(&mut rt, "peek-keep");
    assert_eq!(rt.state.model.peeked(), None);
    assert_eq!(rt.state.model.focused_terminal(), Some("t12"));
}

#[test]
fn cmd_click_on_a_live_terminals_address_peeks_it() {
    let (mut rt, fake) = runtime();
    let grid = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("screen") && e.path.contains("pane-pn-1/"))
        .expect("t1's grid");
    let (w, h) = rt.state.face(2.0).cell_pt();
    let at = |col: f32| (grid.rect[0] + (col + 0.5) * w, grid.rect[1] + h / 2.0);
    // A plain click on it selects, as ever.
    let (x, y) = at(5.0);
    click_at(&mut rt, x, y);
    assert_eq!(rt.state.model.peeked(), None);
    rt.physical_input(PhysicalInput::Modifiers(CMD));
    // "see" isn't a terminal.
    let (x, y) = at(1.0);
    click_at(&mut rt, x, y);
    assert_eq!(rt.state.model.peeked(), None);
    let (x, y) = at(5.0);
    click_at(&mut rt, x, y);
    assert_eq!(rt.state.model.peeked(), Some("t22"));
    assert!(
        typed(&fake, "t1").is_empty(),
        "the click never reached the program"
    );
}
