//! The agent-jump bar (`ctrl-;`, `feature-inventory.md` §7) on the agents core, through Rune's
//! real runtime with no window: where it hangs and what it says, a label's jump, the labels
//! brightening, the ways out, and a hidden sidebar shown just for the jump.

use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::agents_core;
use herdr_frontend::view::jump;
use herdr_frontend::{SIZE, State, app};
use model::Mode;
use model::mode::PopupState;
use rune::{ElementInfo, Env, Key, KeyEvent, Mods, Runtime};

fn runtime() -> Runtime<State> {
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.with_settings(Settings::fixture());
    rt.state.connect(Box::new(agents_core()));
    rt.state.settle();
    frames(&mut rt, 3);
    rt
}

fn frames(rt: &mut Runtime<State>, n: usize) {
    for _ in 0..n {
        rt.invalidate();
        rt.frame(1.0 / 60.0);
    }
}

/// An element on screen, not one fading out after it left the view.
fn element(rt: &Runtime<State>, key: &str) -> Option<ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn has(rt: &Runtime<State>, key: &str) -> bool {
    element(rt, key).is_some()
}

fn text(rt: &Runtime<State>, key: &str) -> String {
    element(rt, key)
        .and_then(|e| e.text)
        .unwrap_or_else(|| panic!("no text at {key}"))
}

fn label_color(rt: &Runtime<State>, row: &str) -> Option<rune::Color> {
    rt.elements()
        .into_iter()
        .find(|e| e.path.ends_with(&format!("/{row}/body/line1/jump")) && !e.exiting)
        .and_then(|e| e.text_color)
}

fn press(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.state.settle();
    frames(rt, 2);
}

fn typed(rt: &mut Runtime<State>, s: &str) {
    press(rt, Key::Text(s.into()), Mods::default());
}

fn ctrl_semicolon(rt: &mut Runtime<State>) {
    let ctrl = Mods {
        ctrl: true,
        ..Mods::default()
    };
    press(rt, Key::Text(";".into()), ctrl);
}

fn jumping(rt: &Runtime<State>) -> bool {
    matches!(rt.state.model.mode(), Mode::Popup(PopupState::Jump(_)))
}

#[test]
fn the_bar_hangs_at_the_top_and_a_label_focuses_its_row() {
    let mut rt = runtime();
    assert_eq!(rt.state.model.agent_rows(), ["t15", "t11", "t10", "t12"]);
    let dim = label_color(&rt, "ag-t11");
    ctrl_semicolon(&mut rt);
    assert!(jumping(&rt), "{:?}", rt.state.model.mode());
    let bar = element(&rt, "agent-jump").expect("the bar");
    let zoom = rt.state.zoom();
    assert_eq!(
        bar.rect,
        [(SIZE.0 / zoom - 360.0) / 2.0, 8.0 / zoom, 360.0, 32.0],
        "360 wide, centered, 8 window points below the top at this zoom"
    );
    assert_eq!(jump::top(1.0), 8.0);
    assert_eq!(jump::top(2.0), 6.0, "6 zoomed, once that's past 8");
    assert_eq!(text(&rt, "jump-title"), "JUMP");
    assert_eq!(text(&rt, "jump-hint"), "1–4 · Esc");
    assert!(
        rt.state.toasts.shown.is_empty(),
        "it opens, with no not-yet toast"
    );
    frames(&mut rt, 60);
    let bright = label_color(&rt, "ag-t11");
    assert_ne!(bright, dim, "the rows' labels brighten");
    assert_eq!(bright, Some(rt.state.chrome.working));

    // A key no label has is dropped; a label closes the bar and focuses its row.
    typed(&mut rt, "7");
    assert!(jumping(&rt));
    assert!(!has(&rt, "jump-typed"));
    typed(&mut rt, "2");
    assert!(!jumping(&rt));
    assert!(!has(&rt, "agent-jump"));
    assert_eq!(rt.state.model.focused_terminal(), Some("t11"));
    frames(&mut rt, 60);
    assert_eq!(label_color(&rt, "ag-t11"), dim, "labels dim again");
}

#[test]
fn esc_ctrl_semicolon_and_a_press_outside_leave_without_a_jump() {
    let mut rt = runtime();
    let focused = rt.state.model.focused_terminal().map(str::to_owned);
    ctrl_semicolon(&mut rt);
    press(&mut rt, Key::Escape, Mods::default());
    assert!(!jumping(&rt));
    ctrl_semicolon(&mut rt);
    ctrl_semicolon(&mut rt);
    assert!(!jumping(&rt), "Ctrl+; again");

    // A press on the bar keeps it; one beside it, either button, leaves.
    ctrl_semicolon(&mut rt);
    let (x, y) = rt.find("agent-jump").unwrap();
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, 2);
    assert!(jumping(&rt));
    // Over a sidebar row: the row isn't clicked.
    let (x, y) = rt.find("ag-t10").unwrap();
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.state.settle();
    frames(&mut rt, 2);
    assert!(!jumping(&rt));
    assert_eq!(
        rt.state.model.focused_terminal().map(str::to_owned),
        focused
    );
    ctrl_semicolon(&mut rt);
    rt.secondary_down(SIZE.0 / 2.0, SIZE.1 / 2.0);
    rt.secondary_up(SIZE.0 / 2.0, SIZE.1 / 2.0);
    rt.state.settle();
    frames(&mut rt, 2);
    assert!(!jumping(&rt));
    assert_eq!(
        rt.state.model.focused_terminal().map(str::to_owned),
        focused
    );
}

#[test]
fn a_hidden_sidebar_shows_for_the_jump_and_hides_after() {
    let mut rt = runtime();
    let cmd = Mods {
        cmd: true,
        ..Mods::default()
    };
    press(&mut rt, Key::Text("b".into()), cmd);
    frames(&mut rt, 60);
    assert!(!has(&rt, "sidebar"));

    ctrl_semicolon(&mut rt);
    frames(&mut rt, 60);
    assert!(has(&rt, "sidebar"), "shown for its labels");
    assert!(rt.state.shared.sidebar_hidden(), "and not saved as shown");
    typed(&mut rt, "1");
    frames(&mut rt, 60);
    assert_eq!(rt.state.model.focused_terminal(), Some("t15"));
    assert!(!has(&rt, "sidebar"), "hidden again after the jump");

    ctrl_semicolon(&mut rt);
    press(&mut rt, Key::Escape, Mods::default());
    frames(&mut rt, 60);
    assert!(!has(&rt, "sidebar"), "and after leaving");

    // A sidebar that was showing stays.
    press(&mut rt, Key::Text("b".into()), cmd);
    ctrl_semicolon(&mut rt);
    press(&mut rt, Key::Escape, Mods::default());
    frames(&mut rt, 60);
    assert!(has(&rt, "sidebar"));
}
