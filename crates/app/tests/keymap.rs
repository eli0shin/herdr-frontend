//! The keymap through Rune's real runtime with no window: keys reach the one keymap from the
//! root's preview while a pane has focus and from `AppSpec::on_key` while nothing has, never
//! both; sequences wait with an indicator and time out; unbuilt actions say "not yet"; the keys
//! help lists the bindings and filters; the key file in the home applies live.

use std::path::PathBuf;
use std::time::Duration;

use herdr_frontend::fixtures::demo_core;
use herdr_frontend::home::Home;
use herdr_frontend::link::{Fake, Out};
use herdr_frontend::window::Keeper;
use herdr_frontend::{SIZE, State, app};
use model::{Mode, Popup, PopupState};
use rune::{ElementInfo, Env, Key, KeyEvent, Mods, PhysicalKeyCode, Runtime};

fn runtime_with(home: Option<Home>) -> (Runtime<State>, Fake) {
    let fake = demo_core();
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    if home.is_some() {
        let mut s = State::new(
            Box::new(herdr_frontend::link::Offline),
            home,
            Keeper::default(),
        );
        s.keys_every = Duration::from_millis(10);
        rt.state = s;
        State::setup(&mut rt.state, std::sync::Arc::new(|| {}));
    }
    rt.state.connect(Box::new(fake.clone()));
    rt.state.settle();
    frames(&mut rt, 3);
    (rt, fake)
}

fn runtime() -> (Runtime<State>, Fake) {
    runtime_with(None)
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

/// A key as macOS delivers it: ⌘ chords as `Command`, the rest as text or named keys.
fn press(rt: &mut Runtime<State>, held: &str, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: mods(held),
    });
    rt.frame(1.0 / 60.0);
}

fn ch(rt: &mut Runtime<State>, held: &str, c: char) {
    let key = if held.contains('⌘') {
        Key::Command(c)
    } else {
        Key::Text(c.to_string())
    };
    press(rt, held, key);
}

/// What the app typed into a terminal.
fn typed(fake: &Fake, address: &str) -> Vec<Vec<u8>> {
    fake.sent_to(address)
        .into_iter()
        .filter_map(|o| match o {
            Out::Input(bytes) => Some(bytes),
            _ => None,
        })
        .collect()
}

/// How many `terminal.run` calls the app made.
fn runs(fake: &Fake) -> usize {
    fake.core()
        .calls
        .iter()
        .filter(|c| matches!(c, model::Request::Run(_)))
        .count()
}

fn toast_titles(rt: &Runtime<State>) -> Vec<String> {
    rt.state
        .toasts
        .shown
        .iter()
        .map(|t| format!("{}: {}", t.toast.title, t.toast.body))
        .collect()
}

#[test]
fn a_prefix_sequence_runs_from_the_focused_pane_and_never_reaches_it() {
    let (mut rt, fake) = runtime();
    assert_eq!(focused(&rt).as_deref(), Some("pane-pn-1"));
    ch(&mut rt, "⌃", 'g');
    // Read once: a second read of ctrl-g would have dropped the sequence.
    let Mode::Prefix { keys, .. } = rt.state.model.mode() else {
        panic!("{:?}", rt.state.model.mode())
    };
    assert_eq!(keys.len(), 1);
    frames(&mut rt, 1);
    let indicator = element(&rt, "prefix-keys").expect("the prefix indicator");
    assert_eq!(indicator.text.as_deref(), Some("ctrl-g \u{2026}"));

    let before = runs(&fake);
    ch(&mut rt, "", 't');
    assert_eq!(runs(&fake), before + 1, "ctrl-g t: tab.new");
    assert!(
        typed(&fake, "t1").is_empty(),
        "neither key reached the program"
    );
    frames(&mut rt, 1);
    assert!(element(&rt, "prefix").is_none());
}

#[test]
fn a_prefix_times_out_with_its_indicator_and_plain_keys_type_again() {
    let (mut rt, fake) = runtime();
    ch(&mut rt, "⌃", 'g');
    assert!(element(&rt, "prefix").is_some());
    for _ in 0..8 {
        rt.invalidate();
        rt.frame(0.2);
    }
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
    assert!(element(&rt, "prefix").is_none(), "the indicator went");
    ch(&mut rt, "", 't');
    assert_eq!(typed(&fake, "t1"), [b"t".to_vec()]);
}

#[test]
fn shifted_keys_match_by_their_key_or_the_character_they_type() {
    let (mut rt, _) = runtime();
    // ⇧⌘] on a US layout: Command('}') with the key's base `]`.
    rt.key_with(
        KeyEvent {
            key: Key::Command('}'),
            mods: mods("⌘⇧"),
        },
        PhysicalKeyCode::BracketRight,
        Some(']'),
    );
    frames(&mut rt, 1);
    assert_eq!(
        rt.state.model.shown_tab().map(|t| t.id.as_str()),
        Some("tb-logs"),
        "cmd-shift-]: the next tab"
    );
}

#[test]
fn with_nothing_focused_the_window_reads_the_keymap_once_then_types() {
    let (mut rt, fake) = runtime();
    let (x, y) = rt.find("brand-title").expect("the sidebar's label");
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, 1);
    assert_eq!(focused(&rt), None, "a click on plain content blurs");

    ch(&mut rt, "⌃", 'g');
    let Mode::Prefix { keys, .. } = rt.state.model.mode() else {
        panic!("{:?}", rt.state.model.mode())
    };
    assert_eq!(keys.len(), 1, "AppSpec::on_key read it once");
    let before = runs(&fake);
    ch(&mut rt, "", 't');
    assert_eq!(runs(&fake), before + 1);

    // An unbound key types into the model's focused pane, which takes the keyboard back.
    rt.state.settle();
    let pane = rt.state.model.focused_pane().unwrap();
    let address = rt.state.model.world().address_of(&pane).unwrap().to_owned();
    let (x, y) = rt.find("brand-title").unwrap();
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, 1);
    ch(&mut rt, "", 'x');
    frames(&mut rt, 1);
    assert_eq!(typed(&fake, &address), [b"x".to_vec()]);
    assert_eq!(focused(&rt), Some(herdr_frontend::state::pane_key(&pane)));
}

#[test]
fn unbuilt_actions_show_a_not_yet_toast_that_goes() {
    let (mut rt, _) = runtime();
    ch(&mut rt, "⌘", 'r');
    assert_eq!(toast_titles(&rt), ["Not yet: review isn't built yet"]);
    frames(&mut rt, 1);
    let shown = &rt.state.toasts.shown[0];
    let card = element(&rt, &format!("toast-{}", shown.id)).expect("the toast card");
    assert_eq!(card.rect[2..], [340.0, 72.0]);
    assert_eq!(card.rect[0] + card.rect[2], SIZE.0 - 16.0);
    assert_eq!(card.rect[1] + card.rect[3], SIZE.1 - 16.0);
    for _ in 0..6 {
        rt.invalidate();
        rt.frame(1.0);
    }
    assert!(rt.state.toasts.shown.is_empty());
    assert!(element(&rt, "toasts").is_none());
}

#[test]
fn a_popup_the_window_cant_draw_yet_lets_go_of_the_keyboard() {
    let (mut rt, fake) = runtime();
    ch(&mut rt, "⌘", 'p');
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
    assert_eq!(toast_titles(&rt), ["Not yet: popup.picker isn't built yet"]);
    frames(&mut rt, 2);
    ch(&mut rt, "", 'a');
    assert_eq!(typed(&fake, "t1"), [b"a".to_vec()], "typing still works");
}

#[test]
fn the_keys_help_lists_the_bindings_filters_and_closes() {
    let (mut rt, fake) = runtime();
    ch(&mut rt, "⌘", '/');
    frames(&mut rt, 2);
    let Mode::Popup(PopupState::List(list)) = rt.state.model.mode() else {
        panic!("{:?}", rt.state.model.mode())
    };
    assert_eq!(list.popup, Popup::Help);
    let help = element(&rt, "help").expect("the help dialog");
    assert_eq!(help.rect[2..], [640.0, 520.0]);
    assert_eq!(focused(&rt).as_deref(), Some("popup-filter"));
    assert!(element(&rt, "help-group-tabs").is_some());
    let all = list.rows;
    assert!(all > 50, "{all} rows");

    // Typing goes to the filter, never to the terminal behind.
    for c in "split".chars() {
        ch(&mut rt, "", c);
    }
    frames(&mut rt, 1);
    assert!(typed(&fake, "t1").is_empty());
    let Mode::Popup(PopupState::List(list)) = rt.state.model.mode() else {
        panic!()
    };
    assert_eq!(list.query, "split");
    assert_eq!(list.rows, 2);
    let titles: Vec<String> = (0..2)
        .filter_map(|i| element(&rt, &format!("help-row-{i}")))
        .map(|e| e.label.or(e.text).unwrap_or_default())
        .collect();
    assert_eq!(titles.len(), 2);

    press(&mut rt, "", Key::Escape);
    frames(&mut rt, 2);
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
    assert!(element(&rt, "help").is_none());
    assert_eq!(
        focused(&rt).as_deref(),
        Some("pane-pn-1"),
        "focus went back"
    );
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "herdr-frontend-keymap-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn save(path: &std::path::Path, text: &str) {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text).unwrap();
    std::fs::rename(&tmp, path).unwrap();
}

/// Frames until `done`, for news from the key file's thread.
fn until(rt: &mut Runtime<State>, done: impl Fn(&State) -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !done(&rt.state) {
        assert!(std::time::Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(5));
        rt.invalidate();
        rt.frame(1.0 / 60.0);
    }
}

#[test]
fn the_key_file_in_the_home_applies_live_and_a_bad_one_keeps_the_keys() {
    let dir = scratch("live");
    let file = dir.join(model::KEY_FILE);
    save(&file, r#"{"global": {"cmd-k": "mod.open:picker"}}"#);
    let (mut rt, _) = runtime_with(Some(Home::at(&dir)));
    // The file is the whole keymap: cmd-t isn't in it.
    assert_eq!(
        rt.state
            .model
            .keymap()
            .keys_for(&model::Action::TabNew)
            .len(),
        0
    );

    save(&file, r#"{"global": {"cmd-k": "app.help"}}"#);
    until(&mut rt, |s| {
        !s.model
            .keymap()
            .keys_for(&model::Action::Open(Popup::Help))
            .is_empty()
    });
    ch(&mut rt, "⌘", 'k');
    assert!(matches!(
        rt.state.model.mode(),
        Mode::Popup(PopupState::List(l)) if l.popup == Popup::Help
    ));
    press(&mut rt, "", Key::Escape);

    save(&file, r#"{"global": {"cmd-k": "tab.nope"}}"#);
    until(&mut rt, |s| !s.toasts.shown.is_empty());
    assert_eq!(
        toast_titles(&rt),
        [r#"keys.json can't be used: global "cmd-k": unknown action "tab.nope""#]
    );
    assert!(dir.join("keys.error").exists());
    ch(&mut rt, "⌘", 'k');
    assert!(
        matches!(rt.state.model.mode(), Mode::Popup(_)),
        "the last good keys still run"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_escape_hatch_brings_elis_keys_back_and_sets_the_file_aside() {
    let dir = scratch("reset");
    let file = dir.join(model::KEY_FILE);
    save(&file, r#"{"global": {"cmd-k": "tab.new"}}"#);
    let (mut rt, _) = runtime_with(Some(Home::at(&dir)));
    press(&mut rt, "⌃⌥⇧", Key::Escape);
    assert_eq!(*rt.state.model.keymap(), model::Keymap::eli());
    until(&mut rt, |s| !s.toasts.shown.is_empty());
    let shown = toast_titles(&rt);
    assert!(
        shown[0].starts_with("Keys reset: Eli's keys are back; yours are in keys.json.reset-"),
        "{shown:?}"
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), model::ELI_KEYS);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_open_help_keeps_the_keyboard_even_when_tab_tries_to_leave_it() {
    let (mut rt, fake) = runtime();
    ch(&mut rt, "⌘", '/');
    frames(&mut rt, 2);
    assert_eq!(focused(&rt).as_deref(), Some("popup-filter"));
    press(&mut rt, "", Key::Tab);
    frames(&mut rt, 1);
    assert_eq!(focused(&rt).as_deref(), Some("popup-filter"), "Tab stays");
    ch(&mut rt, "", 'a');
    frames(&mut rt, 1);
    assert!(typed(&fake, "t1").is_empty(), "typing went to the filter");
    let Mode::Popup(PopupState::List(list)) = rt.state.model.mode() else {
        panic!("{:?}", rt.state.model.mode())
    };
    assert_eq!(list.query, "a");
}

#[test]
fn copy_mode_says_not_yet_once_and_lets_go() {
    let (mut rt, fake) = runtime();
    ch(&mut rt, "⌃", 'g');
    ch(&mut rt, "", '[');
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
    assert_eq!(toast_titles(&rt), ["Not yet: copy_mode isn't built yet"]);
    ch(&mut rt, "", 'j');
    assert_eq!(typed(&fake, "t1"), [b"j".to_vec()]);
}
