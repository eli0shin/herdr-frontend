//! The peek (`feature-inventory.md` §15) in the window: a sub-agent's row or a Cmd-click on a
//! terminal address opens it over the window; its terminal takes the keyboard (Esc too) and
//! the size; ✕, a click outside and `cmd-w` close it; "↗ keep" gives it a tab. In it, `cmd-f`
//! opens the find bar, a right-click the terminal menu without pane items, and Cmd-hover
//! underlines terminal addresses as it does in a pane.

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

/// `cmd-f` in the peek opens the peeked terminal's find bar over the peek, with the keyboard in
/// its field; Escape closes it and gives the keyboard back to the peek, which stays open.
#[test]
fn cmd_f_in_the_peek_opens_its_find_bar() {
    let (mut rt, fake) = runtime();
    click(&mut rt, "sub-t20");
    fake.output("t20", b"\x1bcone match here");
    rt.state.settle();
    frames(&mut rt, 3);
    key(&mut rt, Key::Command('f'), CMD);
    assert_eq!(rt.state.model.peeked(), Some("t20"), "the peek stays");
    assert_eq!(focused(&rt).as_deref(), Some("find-@peek"));
    let bar = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("find-bar-@peek") && !e.exiting)
        .expect("the bar");
    assert!(
        bar.path.contains("peek-area/"),
        "over the peek's terminal: {}",
        bar.path
    );
    let area = element(&rt, "peek-area").unwrap().rect;
    assert!(
        (bar.rect[0] + bar.rect[2] - (area[0] + area[2] - 8.0)).abs() < 1.0,
        "at its top right: {:?} in {area:?}",
        bar.rect
    );
    assert!(rt.state.toasts.shown.is_empty(), "no \"not yet\"");
    for c in "match".chars() {
        key(&mut rt, Key::Text(c.to_string()), Mods::default());
    }
    rt.state.settle();
    frames(&mut rt, 3);
    let t20 = &rt.state.viewers["t20"];
    assert_eq!(t20.find.as_ref().unwrap().edit.text, "match");
    let found = t20.pane.as_ref().unwrap().snapshot.find.expect("searched");
    assert_eq!(found.total, 1);
    assert_eq!(text_of(&rt, "find-count"), "1 matches");
    assert!(typed(&fake, "t20").is_empty(), "the field took the typing");
    key(&mut rt, Key::Escape, Mods::default());
    assert!(element(&rt, "find-bar-@peek").is_none());
    assert_eq!(rt.state.model.peeked(), Some("t20"), "Escape left the peek");
    assert_eq!(focused(&rt).as_deref(), Some(PEEK_KEY));
}

/// A peeked terminal with a pane on screen behind the peek shows its find bar over the peek
/// only.
#[test]
fn a_peeked_terminals_find_bar_is_over_the_peek_not_its_pane() {
    let (mut rt, _) = runtime();
    let now = rt.state.model.now();
    rt.state.step(now, model::Action::Peek("t1".into()));
    rt.state.settle();
    frames(&mut rt, 2);
    key(&mut rt, Key::Command('f'), CMD);
    assert_eq!(focused(&rt).as_deref(), Some("find-@peek"));
    assert!(element(&rt, "find-bar-@peek").is_some());
    assert!(element(&rt, "find-bar-pn-1").is_none(), "not behind it too");
}

/// A right-click in the peek opens the terminal menu for the peeked terminal, without the pane
/// items (it has no pane to split, zoom or close); closing it gives the keyboard back to the
/// peek.
#[test]
fn a_right_click_in_the_peek_opens_the_terminal_menu_without_pane_items() {
    let (mut rt, fake) = runtime();
    click(&mut rt, "sub-t20");
    let area = element(&rt, "peek-area").unwrap().rect;
    let (x, y) = (area[0] + area[2] / 2.0, area[1] + area[3] / 2.0);
    rt.secondary_down(x, y);
    rt.secondary_up(x, y);
    rt.state.settle();
    frames(&mut rt, 2);
    let model::Mode::Popup(model::PopupState::Menu(menu)) = rt.state.model.mode() else {
        panic!("{:?}", rt.state.model.mode());
    };
    assert_eq!(menu.target, model::MenuTarget::Terminal("t20".into()));
    let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["Send right-clicks to pane", "Send back to t10"]);
    assert_eq!(rt.state.model.peeked(), Some("t20"), "over the peek");
    let at = element(&rt, "menu-item-0").expect("drawn").rect;
    assert!(
        (at[0] - x).abs() < 12.0 && (at[1] - y).abs() < 12.0,
        "at the pointer: {at:?}"
    );
    key(&mut rt, Key::Escape, Mods::default());
    assert_eq!(*rt.state.model.mode(), model::Mode::Normal);
    assert_eq!(rt.state.model.peeked(), Some("t20"));
    assert_eq!(focused(&rt).as_deref(), Some(PEEK_KEY));
    assert!(
        typed(&fake, "t20").is_empty(),
        "Escape closed the menu only"
    );
    // Its first item sends the peeked terminal's right-clicks to its program.
    rt.secondary_down(x, y);
    rt.secondary_up(x, y);
    rt.state.settle();
    frames(&mut rt, 2);
    click(&mut rt, "menu-item-0");
    let t20 = &fake.core().terminals["t20"];
    assert_eq!(
        t20.labels.get("herdr.right-click").map(String::as_str),
        Some("program")
    );
    assert_eq!(rt.state.model.peeked(), Some("t20"));
}

/// The columns of row 0 of `address`'s snapshot underlined as the hovered link.
fn underlined(rt: &Runtime<State>, address: &str) -> Option<(u16, u16)> {
    rt.state.viewers[address]
        .pane
        .as_ref()
        .unwrap()
        .snapshot
        .screen[0]
        .link
}

fn hold(rt: &mut Runtime<State>, mods: Mods) {
    rt.physical_input(PhysicalInput::Modifiers(mods));
    rt.state.settle();
    frames(rt, 2);
}

/// Cmd-hover underlines a live terminal's address, as it does a URL or path, in a pane and in
/// the peek; a name that isn't a live terminal's address isn't underlined.
#[test]
fn cmd_hover_underlines_a_live_terminals_address() {
    let (mut rt, fake) = runtime();
    let grid = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("screen") && e.path.contains("pane-pn-1/"))
        .expect("t1's grid");
    let (w, h) = rt.state.face(2.0).cell_pt();
    let at = |col: f32| (grid.rect[0] + (col + 0.5) * w, grid.rect[1] + h / 2.0);
    // `see t22 now`: t22 is live.
    let (x, y) = at(5.0);
    rt.pointer_move(x, y);
    frames(&mut rt, 2);
    assert_eq!(underlined(&rt, "t1"), None, "not without Cmd");
    hold(&mut rt, CMD);
    assert_eq!(underlined(&rt, "t1"), Some((4, 6)));
    hold(&mut rt, Mods::default());
    assert_eq!(underlined(&rt, "t1"), None, "gone with Cmd");
    // Moving onto it with Cmd held underlines it too; "see" isn't a terminal.
    hold(&mut rt, CMD);
    let (x, y) = at(1.0);
    rt.pointer_move(x, y);
    rt.state.settle();
    frames(&mut rt, 2);
    assert_eq!(underlined(&rt, "t1"), None);
    let (x, y) = at(5.0);
    rt.pointer_move(x, y);
    rt.state.settle();
    frames(&mut rt, 2);
    assert_eq!(underlined(&rt, "t1"), Some((4, 6)));
    // A name is no address once its terminal is gone.
    fake.output("t1", b"\x1bcsee t99 now");
    rt.state.settle();
    frames(&mut rt, 2);
    rt.pointer_move(x + 1.0, y);
    rt.state.settle();
    frames(&mut rt, 2);
    assert_eq!(underlined(&rt, "t1"), None, "t99 isn't live");
    hold(&mut rt, Mods::default());

    // In the peek, the same.
    click(&mut rt, "sub-t20");
    fake.output("t20", b"\x1bcfrom t10 ok");
    rt.state.settle();
    frames(&mut rt, 3);
    let grid = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("screen") && e.path.contains("peek-terminal/"))
        .expect("the peek's grid");
    let (x, y) = (grid.rect[0] + 6.5 * w, grid.rect[1] + h / 2.0);
    rt.pointer_move(x, y);
    hold(&mut rt, CMD);
    assert_eq!(underlined(&rt, "t20"), Some((5, 7)));
    // Cmd-click there still peeks it.
    click_at(&mut rt, x, y);
    assert_eq!(rt.state.model.peeked(), Some("t10"));
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

/// Copy mode follows the keyboard: with the peek open, it runs in the peeked terminal, its
/// bar over the peek, and leaving gives the keyboard back to the peek.
#[test]
fn copy_mode_in_the_peek_runs_in_the_peeked_terminal() {
    let (mut rt, fake) = runtime();
    click(&mut rt, "sub-t20");
    fake.output("t20", b"\x1bcpeeked text");
    rt.state.settle();
    frames(&mut rt, 3);
    let ctrl = Mods {
        ctrl: true,
        ..Mods::default()
    };
    key(&mut rt, Key::Text("g".into()), ctrl);
    key(&mut rt, Key::Text("[".into()), Mods::default());
    match rt.state.model.mode() {
        model::Mode::Copy(copy) => assert_eq!(copy.address, "t20"),
        other => panic!("{other:?}"),
    }
    assert!(element(&rt, "copy-mode").is_some());
    assert_eq!(focused(&rt).as_deref(), Some("copy-mode"));
    let peeked = rt.state.viewers["t20"].pane.as_ref().unwrap();
    assert!(
        peeked.snapshot.copy.is_some(),
        "the peeked terminal's copy mode"
    );
    let pane = rt.state.viewers["t1"].pane.as_ref().unwrap();
    assert_eq!(pane.snapshot.copy, None, "not the pane's behind it");
    for c in ["0", "v", "e"] {
        key(&mut rt, Key::Text(c.into()), Mods::default());
    }
    let peeked = rt.state.viewers["t20"].pane.as_ref().unwrap();
    assert_eq!(peeked.snapshot.screen[0].selection, Some((0, 5)));
    key(&mut rt, Key::Escape, Mods::default());
    key(&mut rt, Key::Escape, Mods::default());
    assert_eq!(*rt.state.model.mode(), model::Mode::Normal);
    assert_eq!(rt.state.model.peeked(), Some("t20"), "the peek stays open");
    assert_eq!(focused(&rt).as_deref(), Some(PEEK_KEY));
    assert!(typed(&fake, "t20").is_empty(), "no key reached the program");
}
