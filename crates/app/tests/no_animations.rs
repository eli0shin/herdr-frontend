//! No animations, anywhere (`AGENTS.md`): every state change lands in one frame. Frame by frame
//! through the real renderer, headless (`frames`), across a sidebar toggle, the palette opening,
//! paging and closing, a menu opening and closing, a split, a pane resize and a toast coming
//! and going: every frame of the whole window is the look before the input or the look after
//! it, never one between. A wheel's notch scrolls a popup list or the keys help in its frame.
//! The working spinners, the one motion allowed, are masked out of the comparison: they keep
//! turning, all pointing the same way at once.

mod frames;

use frames::{AT, HOLD, assert_swaps, run, run_from};
use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::busy_core;
use herdr_frontend::{SIZE, State, app};
use rune::{ElementInfo, Env, Key, KeyEvent, Mods, Runtime, WheelEvent};
use serde_json::json;

/// A prefix chord (`ctrl-g` then `key`) that lands at [`AT`]: the prefix comes well before, so
/// its indicator is already showing when the dump starts.
fn prefixed(key: &str) -> serde_json::Value {
    json!([
        { "t": 0.2, "type": "key", "name": "g", "ctrl": true },
        { "t": AT, "type": "key", "name": key },
    ])
}

#[test]
fn hiding_the_sidebar_lands_in_one_frame() {
    // `ctrl-g b`: the sidebar goes and the tab strip and panes take its room, in one frame.
    let run = run("sidebar-hide", "shell", prefixed("b"), "window");
    assert_swaps("hiding the sidebar", &run, AT);
}

#[test]
fn showing_the_sidebar_lands_in_one_frame() {
    // Hidden well before, then `ctrl-g b` again brings it back in one frame.
    let script = json!([
        { "t": 0.05, "type": "key", "name": "g", "ctrl": true },
        { "t": 0.1, "type": "key", "name": "b" },
        { "t": 0.2, "type": "key", "name": "g", "ctrl": true },
        { "t": AT, "type": "key", "name": "b" },
    ]);
    let run = run("sidebar-show", "shell", script, "window");
    assert_swaps("showing the sidebar", &run, AT);
}

/// `cmd-shift-p` is ⌘⇧P only on macOS: elsewhere Rune's scripts hold Ctrl with `cmd`, which the
/// keymap reads as another chord.
#[cfg(target_os = "macos")]
#[test]
fn opening_the_palette_lands_in_one_frame() {
    let script = json!([{ "t": AT, "type": "key", "name": "p", "cmd": true, "shift": true }]);
    let run = run("palette-open", "shell", script, "window");
    assert_swaps("opening the palette", &run, AT);
}

#[test]
fn closing_the_palette_lands_in_one_frame() {
    let script = json!([{ "t": AT, "type": "key", "name": "Escape" }]);
    let run = run("palette-close", "shell-palette", script, "window");
    assert_swaps("closing the palette", &run, AT);
}

#[test]
fn paging_the_palette_moves_its_rows_in_one_frame() {
    // A page down moves the list's window: the rows it scrolls past are gone and the ones it
    // brings are there in the same frame, with no rows growing, shrinking or fading.
    let script = json!([{ "t": AT, "type": "key", "name": "PageDown" }]);
    let run = run("palette-page", "shell-palette", script, "window");
    assert_swaps("paging the palette", &run, AT);
}

#[test]
fn opening_a_menu_lands_in_one_frame() {
    // A click on the sidebar's sort button opens the sort menu under it. The pointer is on the
    // button well before, so its hover is part of the look before, and stays: the menu opens
    // below the button, not over it.
    let script = json!([
        { "t": 0.2, "type": "move", "key": "ag-sort" },
        { "t": AT, "type": "tap", "key": "ag-sort" },
    ]);
    let run = run("menu-open", "shell-agents", script, "window");
    assert_swaps("opening the sort menu", &run, AT + HOLD);
}

#[test]
fn closing_a_menu_lands_in_one_frame() {
    let script = json!([{ "t": AT, "type": "key", "name": "Escape" }]);
    let run = run("menu-close", "shell-sort-menu", script, "window");
    assert_swaps("closing the sort menu", &run, AT);
}

#[test]
fn splitting_a_pane_lands_in_one_frame() {
    // `ctrl-g v` splits the focused pane: both halves are at their new size in one frame.
    let run = run("split", "shell", prefixed("v"), "window");
    assert_swaps("splitting a pane", &run, AT);
}

#[test]
fn resizing_a_pane_lands_in_one_frame() {
    // Alt-Shift-L grows the left pane: the panes are at their new size in one frame.
    let script = json!([{ "t": AT, "type": "key", "name": "l", "alt": true, "shift": true }]);
    let run = run("resize", "shell", script, "window");
    assert_swaps("resizing a pane", &run, AT);
}

#[test]
fn a_toast_shows_in_one_frame() {
    // `ctrl-g [` with no terminal to copy from: a warning toast.
    let run = run("toast-show", "shell-no-spaces", prefixed("["), "window");
    assert_swaps("a toast showing", &run, AT);
}

#[test]
fn dismissing_a_toast_lands_in_one_frame() {
    // A click on the "Not yet" toast dismisses it. The pointer is on it well before.
    let script = json!([
        { "t": 0.2, "type": "move", "key": "toast-1" },
        { "t": AT, "type": "tap", "key": "toast-1" },
    ]);
    let run = run("toast-dismiss", "shell-not-yet", script, "window");
    assert_swaps("dismissing a toast", &run, AT + HOLD);
}

/// The arc each working ring on screen shows (`view::spinner::ring`): the sidebar's, the tab
/// pill's and the picker's, by key.
fn rings(rt: &Runtime<State>) -> Vec<(String, usize)> {
    let elements = rt.elements();
    let ring = |e: &ElementInfo| {
        let key = e.key.as_deref()?;
        (key == "spinner" || key == "tab-working" || key.starts_with("picker-working-"))
            .then(|| key.to_owned())
    };
    let shown = |ring: &ElementInfo| -> Vec<usize> {
        elements
            .iter()
            .filter(|e| e.path.starts_with(&format!("{}/", ring.path)) && e.opacity > 0.5)
            .filter_map(|e| e.key.as_deref()?.strip_prefix("arc-")?.parse().ok())
            .collect()
    };
    elements
        .iter()
        .filter(|e| !e.exiting)
        .filter_map(|e| {
            let key = ring(e)?;
            let arcs = shown(e);
            assert_eq!(arcs.len(), 1, "{key} shows one arc at a time: {arcs:?}");
            Some((key, arcs[0]))
        })
        .collect()
}

#[test]
fn spinners_that_came_at_different_times_point_the_same_way() {
    // The sidebar's and the tab strip's spinners come with the first frame; the picker's come
    // when it opens, an odd fraction of a turn later.
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 1.0));
    rt.state.with_settings(Settings::fixture());
    rt.state.connect(Box::new(busy_core()));
    rt.state.settle();
    let step = |rt: &mut Runtime<State>| {
        rt.invalidate();
        rt.frame(0.0137);
    };
    for _ in 0..23 {
        step(&mut rt);
    }
    let before = rings(&rt);
    assert!(!before.is_empty(), "a working agent's spinner");
    rt.key(KeyEvent {
        key: Key::Command('p'),
        mods: Mods {
            cmd: true,
            logo: true,
            ..Mods::default()
        },
    });
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..80 {
        step(&mut rt);
        let now = rings(&rt);
        assert!(
            now.iter()
                .any(|(key, _)| key.starts_with("picker-working-")),
            "the picker's spinner: {now:?}"
        );
        assert!(now.len() > before.len(), "both kinds of spinner: {now:?}");
        let arc = now[0].1;
        assert!(
            now.iter().all(|(_, a)| *a == arc),
            "every spinner points the same way: {now:?}"
        );
        seen.insert(arc);
    }
    assert!(seen.len() > 20, "they turn: {seen:?}");
}

/// The app on the busy core, headless, a frame drawn.
fn busy() -> Runtime<State> {
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 1.0));
    rt.state.with_settings(Settings::fixture());
    rt.state.connect(Box::new(busy_core()));
    rt.state.settle();
    rt.frame(1.0 / 60.0);
    rt
}

/// How far the viewport keyed `key` has scrolled its content, by its first child's offset.
fn scrolled(rt: &Runtime<State>, key: &str) -> f32 {
    let elements = rt.elements();
    let viewport = elements
        .iter()
        .find(|e| e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("no {key}"));
    let inside = format!("{}/", viewport.path);
    elements
        .iter()
        .find(|e| e.path.starts_with(&inside))
        .map_or(0.0, |e| e.scrolled[1])
}

/// A notch over the viewport keyed `key`, after `opens` (⌘, Shift when `shift`, and a
/// character) brings it up: the whole notch lands in its frame, stays, and two in one frame
/// both count.
fn notch_lands_at_once(opens: char, shift: bool, key: &str) {
    // Rune eases a notch over several frames; the viewport takes it and lands it at once.
    let mut rt = busy();
    rt.key(KeyEvent {
        key: Key::Command(opens),
        mods: Mods {
            cmd: true,
            logo: true,
            shift,
            ..Mods::default()
        },
    });
    for _ in 0..3 {
        rt.frame(1.0 / 60.0);
    }
    let rows = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .unwrap_or_else(|| panic!("no {key}"));
    let (x, y) = (rows.rect[0] + 20.0, rows.rect[1] + 20.0);
    let notch = || WheelEvent::new(0.0, -20.0).notch(true).at(x, y);
    assert_eq!(scrolled(&rt, key), 0.0);
    rt.wheel_event(notch());
    rt.frame(1.0 / 60.0);
    assert_eq!(
        scrolled(&rt, key),
        20.0,
        "{key}: the whole notch, in its frame"
    );
    for _ in 0..20 {
        rt.frame(1.0 / 60.0);
        assert_eq!(scrolled(&rt, key), 20.0, "{key}: and no further");
    }
    rt.wheel_event(notch());
    rt.wheel_event(notch());
    rt.frame(1.0 / 60.0);
    assert_eq!(
        scrolled(&rt, key),
        60.0,
        "{key}: two notches in one frame both count"
    );
}

#[test]
fn a_wheel_notch_scrolls_the_keys_help_in_one_frame() {
    notch_lands_at_once('/', false, "help-rows");
}

#[test]
fn a_wheel_notch_scrolls_a_popup_list_in_one_frame() {
    // The palette, whose commands don't fit.
    notch_lands_at_once('p', true, "palette-rows");
}

#[test]
fn a_toast_that_times_out_goes_in_one_frame() {
    // The "Not yet" toast goes 5 s after it came.
    let due = 5.0;
    let run = run_from(
        "toast-expire",
        "shell-not-yet",
        json!([]),
        "window",
        due - 0.05,
    );
    assert_swaps("a toast timing out", &run, due);
}
