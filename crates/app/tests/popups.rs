//! The palette, the pickers, the prompt and confirm dialogs, the menus and the settings dialog
//! through Rune's real runtime with no window (`feature-inventory.md` §3, §10): what they draw,
//! where the keyboard goes, and what picking runs on the fake core.

use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::busy_core;
use herdr_frontend::link::Fake;
use herdr_frontend::{SIZE, State, app};
use model::mode::{ListPopup, PopupState};
use model::{Mode, Popup, Request};
use rune::{ElementInfo, Env, Key, KeyEvent, Mods, Runtime};

fn runtime() -> (Runtime<State>, Fake) {
    let fake = busy_core();
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

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused)
        .and_then(|e| e.key)
}

fn mods(spec: &str) -> Mods {
    Mods {
        cmd: spec.contains('⌘'),
        logo: spec.contains('⌘'),
        ctrl: spec.contains('⌃'),
        alt: spec.contains('⌥'),
        shift: spec.contains('⇧'),
    }
}

fn press(rt: &mut Runtime<State>, held: &str, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: mods(held),
    });
    frames(rt, 1);
}

fn ch(rt: &mut Runtime<State>, held: &str, c: char) {
    let key = if held.contains('⌘') {
        Key::Command(c)
    } else {
        Key::Text(c.to_string())
    };
    press(rt, held, key);
}

fn type_text(rt: &mut Runtime<State>, text: &str) {
    for c in text.chars() {
        ch(rt, "", c);
    }
    frames(rt, 1);
}

fn list(rt: &Runtime<State>) -> ListPopup {
    match rt.state.model.mode() {
        Mode::Popup(PopupState::List(l)) => l.clone(),
        other => panic!("{other:?}"),
    }
}

/// Clicks the middle of an element.
fn click(rt: &mut Runtime<State>, key: &str) {
    let e = element(rt, key).unwrap_or_else(|| panic!("no {key}"));
    let (x, y) = (e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(rt, 2);
}

fn text_of(rt: &Runtime<State>, key: &str) -> String {
    let e = element(rt, key).unwrap_or_else(|| panic!("no {key}"));
    e.label.or(e.text).unwrap_or_default()
}

fn runs(fake: &Fake) -> Vec<herdr_proto::methods::TerminalRun> {
    fake.core()
        .calls
        .iter()
        .filter_map(|c| match c {
            Request::Run(r) => Some((**r).clone()),
            _ => None,
        })
        .collect()
}

fn doc_tab_names(fake: &Fake, space: &str) -> Vec<String> {
    let doc = &fake.core().docs[&None].0;
    doc["spaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == space)
        .map(|s| {
            s["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["name"].as_str().unwrap_or_default().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn the_palette_filters_runs_a_command_and_remembers_it() {
    let (mut rt, _) = runtime();
    ch(&mut rt, "⌘⇧", 'p');
    frames(&mut rt, 2);
    assert_eq!(list(&rt).popup, Popup::Palette);
    let palette = element(&rt, "palette").expect("the palette");
    assert_eq!(palette.rect[2], 620.0);
    assert_eq!(palette.rect[1], 60.0, "60 from the top");
    assert_eq!(palette.rect[0], (SIZE.0 - 620.0) / 2.0);
    assert_eq!(focused(&rt).as_deref(), Some("popup-filter"));
    assert!(element(&rt, "palette-head-tab").is_some());
    assert_eq!(text_of(&rt, "palette-title-0"), "New tab");

    // ↑ stops at the top; ↓ moves and the window follows.
    press(&mut rt, "", Key::Up);
    assert_eq!(list(&rt).cursor, 0);
    for _ in 0..12 {
        press(&mut rt, "", Key::Down);
    }
    assert_eq!(list(&rt).cursor, 12);
    assert!(element(&rt, "palette-row-12").is_some(), "in view");
    assert!(element(&rt, "palette-row-0").is_none(), "scrolled out");

    type_text(&mut rt, "zoom in");
    assert_eq!(list(&rt).query, "zoom in");
    assert_eq!(list(&rt).cursor, 0);
    assert_eq!(text_of(&rt, "palette-title-0"), "Zoom in");
    let zoom = rt.state.zoom();
    press(&mut rt, "", Key::Enter);
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
    assert!(rt.state.zoom() > zoom, "it ran");
    assert_eq!(rt.state.model.view().palette, ["app.zoom_in"]);
    assert_eq!(
        focused(&rt).as_deref(),
        Some("pane-pn-1"),
        "focus went back"
    );

    // Open again: the recent command first, under "Recent".
    ch(&mut rt, "⌘⇧", 'p');
    frames(&mut rt, 2);
    assert!(element(&rt, "palette-head-recent").is_some());
    assert_eq!(text_of(&rt, "palette-title-0"), "Zoom in");
    type_text(&mut rt, "qqqq");
    assert!(element(&rt, "palette-empty").is_some());
    press(&mut rt, "", Key::Escape);
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
}

#[test]
fn a_palette_rename_asks_for_the_name_in_the_prompt() {
    let (mut rt, fake) = runtime();
    ch(&mut rt, "⌘⇧", 'p');
    type_text(&mut rt, "rename tab");
    press(&mut rt, "", Key::Enter);
    frames(&mut rt, 2);
    let Mode::Popup(PopupState::Prompt(_)) = rt.state.model.mode() else {
        panic!("{:?}", rt.state.model.mode())
    };
    assert_eq!(text_of(&rt, "prompt-title"), "Rename tab");
    assert_eq!(rt.state.popup_field.text, "work");
    assert_eq!(
        rt.state.popup_field.selected(),
        "work",
        "the name is selected"
    );
    assert_eq!(focused(&rt).as_deref(), Some("popup-filter"));
    // Typing replaces it; Enter saves.
    type_text(&mut rt, "build");
    press(&mut rt, "", Key::Enter);
    frames(&mut rt, 2);
    rt.state.settle();
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
    assert_eq!(doc_tab_names(&fake, "sp-1"), ["build", "logs"]);
}

#[test]
fn the_prompt_cancels_with_esc_and_saves_with_its_button() {
    let (mut rt, fake) = runtime();
    // shift-f2: rename the space.
    press(&mut rt, "⇧", Key::Function(2));
    frames(&mut rt, 2);
    let prompt = element(&rt, "prompt").expect("the prompt");
    assert_eq!(prompt.rect[2], 380.0);
    assert_eq!(text_of(&rt, "prompt-hint"), "empty: named after its folder");
    type_text(&mut rt, "x");
    press(&mut rt, "", Key::Escape);
    rt.state.settle();
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
    assert_eq!(
        fake.core().docs[&None].0["spaces"][0]["name"],
        "herdr-frontend"
    );

    press(&mut rt, "⇧", Key::Function(2));
    frames(&mut rt, 2);
    type_text(&mut rt, "front");
    click(&mut rt, "dialog-yes");
    rt.state.settle();
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
    assert_eq!(fake.core().docs[&None].0["spaces"][0]["name"], "front");
}

#[test]
fn closing_a_space_asks_and_cancel_or_confirm_answers() {
    let (mut rt, fake) = runtime();
    ch(&mut rt, "⌃", 'g');
    ch(&mut rt, "", 'x');
    frames(&mut rt, 2);
    assert_eq!(text_of(&rt, "confirm-title"), "Close space?");
    assert_eq!(
        text_of(&rt, "confirm-detail"),
        "herdr-frontend \u{2014} 2 tabs, 4 terminals"
    );
    assert_eq!(
        focused(&rt).as_deref(),
        Some("confirm"),
        "no field: the box"
    );
    // Typing goes nowhere.
    ch(&mut rt, "", 'a');
    assert!(matches!(
        rt.state.model.mode(),
        Mode::Popup(PopupState::Confirm(_))
    ));
    click(&mut rt, "dialog-cancel");
    assert_eq!(*rt.state.model.mode(), Mode::Normal);

    ch(&mut rt, "⌃", 'g');
    ch(&mut rt, "", 'x');
    press(&mut rt, "", Key::Enter);
    rt.state.settle();
    let spaces = fake.core().docs[&None].0["spaces"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(spaces, 3, "the space closed");
}

#[test]
fn the_picker_goes_to_tabs_closes_them_and_unfolds_quiet_spaces() {
    let (mut rt, fake) = runtime();
    ch(&mut rt, "⌘", 'p');
    frames(&mut rt, 2);
    let picker = element(&rt, "picker").expect("the picker");
    assert_eq!((picker.rect[1], picker.rect[2]), (40.0, 640.0));
    assert!(
        element(&rt, "picker-head-sp-2").is_some(),
        "needs you: first"
    );
    // dotfiles: review (0), shell (1); repos: fix (2), docs (3); earlier (4); new tab (5)…
    assert_eq!(list(&rt).rows, 7);
    assert_eq!(list(&rt).cursor, 0);

    // "earlier" unfolds the quiet spaces and stays open on it.
    for _ in 0..4 {
        press(&mut rt, "⌃", Key::Text("n".into()));
    }
    press(&mut rt, "", Key::Enter);
    frames(&mut rt, 2);
    assert!(rt.state.picker_earlier);
    assert_eq!(list(&rt).cursor, 4);
    assert_eq!(
        list(&rt).rows,
        10,
        "herdr-frontend's two tabs and scratch's one"
    );

    // Typing filters by program: the Pi tabs.
    type_text(&mut rt, "pi");
    // No quiet space matches, so no "earlier" row.
    assert_eq!(list(&rt).rows, 3 + 2, "three Pi tabs and the new rows");
    // Ctrl+D closes the first (dotfiles' review) and the picker.
    press(&mut rt, "⌃", Key::Text("d".into()));
    rt.state.settle();
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
    assert_eq!(doc_tab_names(&fake, "sp-2"), ["shell"]);

    // Enter on a tab shows it, in its space.
    ch(&mut rt, "⌘", 'p');
    type_text(&mut rt, "docs");
    press(&mut rt, "", Key::Enter);
    assert_eq!(rt.state.model.shown_tab().unwrap().id, "tb-docs");
    assert_eq!(rt.state.model.active_space().unwrap().id, "sp-3");
}

#[test]
fn a_click_on_a_picker_row_picks_it_and_a_click_outside_closes() {
    let (mut rt, _) = runtime();
    ch(&mut rt, "⌘", 'p');
    frames(&mut rt, 2);
    click(&mut rt, "picker-row-2");
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
    assert_eq!(rt.state.model.shown_tab().unwrap().id, "tb-fix");

    ch(&mut rt, "⌘", 'p');
    frames(&mut rt, 2);
    rt.pointer_down(20.0, SIZE.1 - 20.0);
    rt.pointer_up(20.0, SIZE.1 - 20.0);
    frames(&mut rt, 2);
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
}

#[test]
fn the_space_picker_ranks_and_goes_to_a_terminal_tab() {
    let (mut rt, _) = runtime();
    // On dotfiles' agent tab first.
    rt.state
        .step(0.0, model::Action::ShowTab("tb-review".into()));
    rt.state.step(0.0, model::Action::ShowTab("tb-work".into()));
    ch(&mut rt, "⌃", 'g');
    ch(&mut rt, "", 's');
    frames(&mut rt, 2);
    let picker = element(&rt, "space-picker").expect("the space picker");
    assert_eq!((picker.rect[1], picker.rect[2]), (40.0, 520.0));
    assert_eq!(list(&rt).rows, 4);
    assert_eq!(list(&rt).cursor, 0, "on the shown space");
    assert_eq!(text_of(&rt, "space-name-1"), "dotfiles");
    type_text(&mut rt, "dotf");
    assert_eq!(list(&rt).rows, 1);
    // Back to empty: back on the shown space.
    for _ in 0..4 {
        press(&mut rt, "", Key::Backspace);
    }
    frames(&mut rt, 1);
    assert_eq!(list(&rt).rows, 4);
    assert_eq!(list(&rt).cursor, 0);
    // Ctrl+N to dotfiles, Enter: its selected tab is an agent's, so its shell shows.
    rt.state
        .step(0.0, model::Action::ShowTab("tb-review".into()));
    rt.state.step(0.0, model::Action::ShowTab("tb-work".into()));
    press(&mut rt, "⌃", Key::Text("n".into()));
    press(&mut rt, "", Key::Enter);
    assert_eq!(rt.state.model.active_space().unwrap().id, "sp-2");
    assert_eq!(rt.state.model.shown_tab().unwrap().id, "tb-dot");
    ch(&mut rt, "⌃", 'g');
    ch(&mut rt, "", 's');
    type_text(&mut rt, "zzz");
    assert!(element(&rt, "space-empty").is_some());
}

#[test]
fn menus_open_where_the_right_click_was_and_run_their_items() {
    let (mut rt, fake) = runtime();
    rt.secondary_down(500.0, 30.0);
    rt.secondary_up(500.0, 30.0);
    frames(&mut rt, 1);
    rt.state.step(0.0, model::Action::TabMenu("tb-logs".into()));
    frames(&mut rt, 2);
    let menu = element(&rt, "menu").expect("the menu");
    assert_eq!(menu.rect[..3], [500.0, 30.0, 200.0]);
    assert_eq!(focused(&rt).as_deref(), Some("menu"));
    assert_eq!(text_of(&rt, "menu-label-1"), "New tab");
    // ↑ wraps to Close; Enter runs it.
    press(&mut rt, "", Key::Up);
    press(&mut rt, "", Key::Enter);
    rt.state.settle();
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
    assert_eq!(doc_tab_names(&fake, "sp-1"), ["work"]);

    // A click on an item runs it; a menu near the edge stays inside the window.
    rt.secondary_down(SIZE.0 - 10.0, SIZE.1 - 10.0);
    rt.secondary_up(SIZE.0 - 10.0, SIZE.1 - 10.0);
    rt.state
        .step(0.0, model::Action::TerminalMenu("pn-1".into()));
    frames(&mut rt, 2);
    let menu = element(&rt, "menu").expect("the terminal menu");
    assert_eq!(menu.rect[2], 220.0);
    assert!(menu.rect[0] + menu.rect[2] <= SIZE.0);
    assert!(menu.rect[1] + menu.rect[3] <= SIZE.1);
    let before = runs(&fake).len();
    click(&mut rt, "menu-item-1");
    rt.state.settle();
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
    assert_eq!(
        runs(&fake).len(),
        before + 1,
        "split right started a terminal"
    );

    // Esc closes.
    rt.state.step(0.0, model::Action::TabMenu("tb-work".into()));
    press(&mut rt, "", Key::Escape);
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
}

#[test]
fn settings_lists_the_files_and_opens_one_in_the_editor() {
    let (mut rt, fake) = runtime();
    ch(&mut rt, "⌘", ',');
    frames(&mut rt, 2);
    assert_eq!(list(&rt).popup, Popup::Settings);
    assert_eq!(list(&rt).rows, 4);
    assert_eq!(focused(&rt).as_deref(), Some("settings"));
    assert_eq!(text_of(&rt, "settings-name-1"), "keys.json");
    // keys.json, in a new tab running the editor.
    press(&mut rt, "", Key::Down);
    press(&mut rt, "", Key::Enter);
    rt.state.settle();
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
    let run = runs(&fake).pop().expect("terminal.run");
    let path = rt.state.home_dir().join("keys.json");
    assert_eq!(
        run.argv[..3],
        ["/bin/sh", "-c", "exec ${EDITOR:-nvim} \"$1\""]
    );
    assert_eq!(run.argv[4], path.to_string_lossy());
    // The last row shows the home folder (recorded here: no Finder in tests).
    ch(&mut rt, "⌘", ',');
    press(&mut rt, "", Key::Up);
    press(&mut rt, "", Key::Enter);
    assert_eq!(rt.state.revealed, [rt.state.home_dir()]);
}
