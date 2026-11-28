//! The tab strip through Rune's real runtime with no window, on a fake core
//! (`fixtures::tab_strip_core`): its pills, and what clicks, double-clicks, drags, right-clicks,
//! the × and the wheel do to the shared document and the shown tab.

use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::tab_strip_core;
use herdr_frontend::link::Fake;
use herdr_frontend::{SIZE, State, app};
use rune::{ElementInfo, Env, Runtime, WheelEvent};

fn runtime() -> (Runtime<State>, Fake) {
    let fake = tab_strip_core();
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.with_settings(Settings::fixture());
    rt.state.connect(Box::new(fake.clone()));
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

/// An element inside another, by the end of its path.
fn inside(rt: &Runtime<State>, path_end: &str) -> Option<ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.path.ends_with(path_end) && !e.exiting)
}

fn shown(rt: &Runtime<State>) -> String {
    rt.state.model.shown_tab().expect("a shown tab").id.clone()
}

/// The space's tabs in the core's document.
fn order(fake: &Fake) -> Vec<String> {
    let doc = fake.core().docs[&None].0.clone();
    doc["spaces"][0]["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_owned())
        .collect()
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no {key}"));
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, y);
    frames(rt, 2);
}

fn toasts(rt: &Runtime<State>) -> Vec<String> {
    rt.state
        .toasts
        .shown
        .iter()
        .map(|t| t.toast.body.clone())
        .collect()
}

#[test]
fn the_strip_shows_the_shown_tabs_as_eli_tabs_draws_them() {
    let (rt, _) = runtime();
    for key in ["tab-tb-work", "tab-tb-pi", "tab-tb-shell", "tab-tb-long"] {
        let pill = element(&rt, key).unwrap_or_else(|| panic!("no {key}"));
        // 10 pt in, 6 pt below the top: 29 pt pills in the 38 pt strip.
        assert_eq!(pill.rect[1], 6.0, "{key}");
        assert_eq!(pill.rect[3], 29.0, "{key}");
    }
    assert!(
        element(&rt, "tab-tb-agent").is_none(),
        "agents' tabs stay out"
    );
    assert_eq!(element(&rt, "tab-tb-work").unwrap().rect[0], 366.0 + 10.0);
    let name = |tab: &str| {
        inside(&rt, &format!("tab-{tab}/tab-name"))
            .and_then(|e| e.text)
            .unwrap_or_default()
    };
    assert_eq!(name("tb-work"), "work");
    assert_eq!(name("tb-pi"), "tab strip", "the Pi session's name");
    assert_eq!(name("tb-shell"), "3", "numbered among the shown tabs");
    let long = inside(&rt, "tab-tb-long/tab-name").unwrap();
    assert!(long.rect[2] <= 170.0, "cut short: {:?}", long.rect);
    // Only the selected tab has a ×, and it is raised with a border.
    assert!(inside(&rt, "tab-tb-work/tab-close").is_some());
    for tab in ["tb-pi", "tb-shell", "tb-long"] {
        assert!(inside(&rt, &format!("tab-{tab}/tab-close")).is_none());
    }
    let (on, off) = (
        element(&rt, "tab-tb-work").unwrap(),
        element(&rt, "tab-tb-pi").unwrap(),
    );
    assert_eq!(on.fill, rt.state.chrome.raised);
    assert_eq!(on.border_color, Some(rt.state.chrome.border));
    assert_eq!(off.fill, rt.state.chrome.chrome);
    // Where the space runs, at the strip's end.
    let machine = element(&rt, "tab-machine-name").unwrap();
    assert_eq!(machine.text.as_deref(), Some("This computer"));
    let strip = element(&rt, "tabs").unwrap();
    let end = element(&rt, "tab-machine").unwrap();
    assert_eq!(
        end.rect[0] + end.rect[2],
        strip.rect[0] + strip.rect[2] - 10.0
    );
}

#[test]
fn a_lone_tab_is_a_pill_as_wide_as_its_contents() {
    use herdr_frontend::fixtures;
    use herdr_frontend::link::Fake;
    let doc = serde_json::json!({"spaces": [{
        "id": "sp-1", "name": "solo", "folder": "/tmp",
        "tabs": [fixtures::tab("tb-1", "", fixtures::split("tb-1", "row", vec![fixtures::pane("pn-1", "t1")]))],
    }]});
    let fake = Fake::new(doc, vec![fixtures::terminal("t1", 80, 24)]);
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.with_settings(Settings::fixture());
    rt.state.connect(Box::new(fake));
    rt.state.settle();
    frames(&mut rt, 3);
    let pill = element(&rt, "tab-tb-1").expect("the pill");
    let close = inside(&rt, "tab-tb-1/tab-close").expect("its ×");
    // The pill ends 11 pt past its ×, as eli-tabs' pill does (10 pt of padding and its 1 pt
    // border, which GPUI lays out inside the pill), not at the strip's end.
    assert_eq!(
        pill.rect[0] + pill.rect[2],
        close.rect[0] + close.rect[2] + 11.0,
        "{:?}",
        pill.rect
    );
    assert!(pill.rect[2] < 100.0, "{:?}", pill.rect);
}

#[test]
fn a_zoomed_tab_reads_z() {
    let (mut rt, _) = runtime();
    rt.state.step(0.0, model::Action::PaneZoom);
    frames(&mut rt, 2);
    let name = inside(&rt, "tab-tb-work/tab-name").unwrap();
    assert_eq!(name.text.as_deref(), Some("work Z"));
}

#[test]
fn a_click_shows_the_tab_and_its_panes_take_the_keyboard() {
    let (mut rt, _) = runtime();
    click(&mut rt, "tab-tb-shell");
    assert_eq!(shown(&rt), "tb-shell");
    assert!(element(&rt, "pane-pn-5").is_some());
    assert!(inside(&rt, "tab-tb-shell/tab-close").is_some());
    assert_eq!(rt.state.model.focused_pane().as_deref(), Some("pn-5"));
    let focused = rt
        .elements()
        .into_iter()
        .find(|e| e.focused)
        .and_then(|e| e.key);
    assert_eq!(
        focused.as_deref(),
        Some("pane-pn-5"),
        "the pill doesn't keep it"
    );
    // Clicking the selected tab again changes nothing.
    click(&mut rt, "tab-tb-shell");
    assert_eq!(shown(&rt), "tb-shell");
}

#[test]
fn a_press_released_on_another_pill_moves_the_tab_there() {
    let (mut rt, fake) = runtime();
    let (x0, y0) = rt.find("tab-tb-work").unwrap();
    let (x1, y1) = rt.find("tab-tb-long").unwrap();
    rt.pointer_down(x0, y0);
    rt.frame(1.0 / 60.0);
    for i in 1..=8 {
        let t = i as f32 / 8.0;
        rt.pointer_move(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
        rt.frame(1.0 / 60.0);
    }
    rt.pointer_up(x1, y1);
    frames(&mut rt, 3);
    rt.state.settle();
    // tb-long's place among all the tabs, the agent's included.
    assert_eq!(
        order(&fake),
        ["tb-pi", "tb-agent", "tb-shell", "tb-long", "tb-work"]
    );
    assert_eq!(shown(&rt), "tb-work", "moving shows nothing new");
    // Released on nothing, a press does nothing.
    let (x, y) = rt.find("tab-tb-pi").unwrap();
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x, y + 200.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, y + 200.0);
    frames(&mut rt, 2);
    assert_eq!(shown(&rt), "tb-work");
    assert_eq!(
        order(&fake),
        ["tb-pi", "tb-agent", "tb-shell", "tb-long", "tb-work"]
    );
}

#[test]
fn a_double_click_renames_and_a_right_click_opens_the_tab_menu() {
    let (mut rt, _) = runtime();
    let (x, y) = rt.find("tab-tb-pi").unwrap();
    for _ in 0..2 {
        rt.pointer_down(x, y);
        rt.frame(0.05);
        rt.pointer_up(x, y);
        rt.frame(0.05);
    }
    frames(&mut rt, 1);
    assert_eq!(shown(&rt), "tb-pi");
    // The rename prompt opens for that tab.
    assert!(matches!(
        rt.state.model.mode(),
        model::Mode::Popup(model::PopupState::Prompt(p))
            if p.rename == model::mode::Rename::Tab("tb-pi".into())
    ));
    assert!(toasts(&rt).is_empty());
    rt.state
        .step(0.0, model::Action::Popup(model::PopupAction::Close));
    frames(&mut rt, 1);
    let (x, y) = rt.find("tab-tb-shell").unwrap();
    rt.secondary_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.secondary_up(x, y);
    frames(&mut rt, 2);
    // The tab menu opens where the right-click was.
    assert!(matches!(
        rt.state.model.mode(),
        model::Mode::Popup(model::PopupState::Menu(m))
            if m.target == model::MenuTarget::Tab("tb-shell".into())
    ));
    assert!(rt.find("menu").is_some());
    assert!(toasts(&rt).is_empty());
    assert_eq!(shown(&rt), "tb-pi", "a right-click selects nothing");
}

#[test]
fn the_close_button_closes_its_tab_and_eli_shows_the_one_to_its_left() {
    let (mut rt, fake) = runtime();
    click(&mut rt, "tab-tb-shell");
    let (x, y) = {
        let close = inside(&rt, "tab-tb-shell/tab-close").expect("the ×");
        (
            close.rect[0] + close.rect[2] / 2.0,
            close.rect[1] + close.rect[3] / 2.0,
        )
    };
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, y);
    frames(&mut rt, 2);
    rt.state.settle();
    assert_eq!(order(&fake), ["tb-work", "tb-pi", "tb-agent", "tb-long"]);
    let killed: Vec<String> = fake
        .core()
        .calls
        .iter()
        .filter_map(|c| match c {
            model::Request::Kill(k) => Some(k.terminal.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(killed, ["t5"]);
    // Left of it is the agent's tab, hidden: the Pi tab before it.
    assert_eq!(shown(&rt), "tb-pi");
}

#[test]
fn the_wheel_steps_through_the_shown_tabs() {
    let (mut rt, _) = runtime();
    let (x, y) = rt.find("tab-tb-pi").unwrap();
    let wheel = |rt: &mut Runtime<State>, dy: f32| {
        rt.wheel_event(WheelEvent::new(0.0, dy).at(x, y));
        frames(rt, 2);
    };
    wheel(&mut rt, -10.0);
    assert_eq!(shown(&rt), "tb-pi", "down: the next tab");
    wheel(&mut rt, -10.0);
    assert_eq!(shown(&rt), "tb-shell", "over the agent's tab");
    wheel(&mut rt, 10.0);
    assert_eq!(shown(&rt), "tb-pi", "up: the previous");
    wheel(&mut rt, 0.5);
    assert_eq!(shown(&rt), "tb-pi", "less than a point moves nothing");
}
