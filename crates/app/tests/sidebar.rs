//! The Agents sidebar (`feature-inventory.md` §1 and §6) on the agents core, through Rune's real
//! runtime with no window: its rows, clicks, folds, the sort menu, the toggle and the edge drag,
//! the order it publishes, and the shared state it keeps in the home.

use std::path::{Path, PathBuf};

use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::{self, agents_core};
use herdr_frontend::home::Home;
use herdr_frontend::link::{Fake, Offline};
use herdr_frontend::shared::{SHARED, TIMES};
use herdr_frontend::window::Keeper;
use herdr_frontend::{SIZE, State, app};
use herdr_proto::StatusState;
use model::agents::{Row, Sort};
use rune::{ElementInfo, Env, Key, KeyEvent, Mods, Runtime};
use serde_json::Value;

/// Herdr app's home for this test binary: Eli-like values for a home's first run to import.
fn herdr_app() -> PathBuf {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let dir =
        std::env::temp_dir().join(format!("herdr-frontend-sidebar-app-{}", std::process::id()));
    ONCE.call_once(|| {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("layouts/.state")).unwrap();
        std::fs::write(
            dir.join(SHARED),
            r#"{"layout": "eli", "agents.sort": "grouped", "sidebar.width": 268,
                "space.colors": {"sp-2": "ansi1"}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("layouts/.state/eli.json"),
            r#"{"nodes": {"sidebar": {"size": {"px": 420.0}, "hidden": false}}}"#,
        )
        .unwrap();
        // SAFETY: set once, before any thread of this test binary reads it.
        unsafe { std::env::set_var("HERDR_APP_HOME", &dir) };
    });
    dir
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "herdr-frontend-sidebar-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A runtime on the agents core, settled and laid out; with a home when given one.
fn runtime_with(home: Option<&Path>) -> (Runtime<State>, Fake) {
    herdr_app();
    let fake = agents_core();
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    if let Some(dir) = home {
        rt.state = State::new(Box::new(Offline), Some(Home::at(dir)), Keeper::default());
        State::setup(&mut rt.state, std::sync::Arc::new(|| {}));
    }
    rt.state.with_settings(Settings::fixture());
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

fn has(rt: &Runtime<State>, key: &str) -> bool {
    element(rt, key).is_some()
}

fn click(rt: &mut Runtime<State>, path: &str) {
    let (x, y) = rt.find(path).unwrap_or_else(|| panic!("no {path}"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.state.settle();
    frames(rt, 2);
}

fn press(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.state.settle();
    frames(rt, 2);
}

/// The headers' labels, in order.
fn headers(rt: &Runtime<State>) -> Vec<String> {
    rt.state
        .list
        .rows
        .iter()
        .filter_map(|r| match r {
            Row::Header(h) => Some(h.label.clone()),
            _ => None,
        })
        .collect()
}

/// Whether an element is on screen at a key path (`ag-t10/edge`).
fn at(rt: &Runtime<State>, path: &str) -> bool {
    rt.elements()
        .iter()
        .any(|e| e.path.ends_with(&format!("/{path}")) && !e.exiting)
}

/// A text element's text, by its key path.
fn text_of(rt: &Runtime<State>, path_end: &str) -> String {
    rt.elements()
        .into_iter()
        .find(|e| e.path.ends_with(&format!("/{path_end}")) && !e.exiting)
        .and_then(|e| e.text)
        .unwrap_or_else(|| panic!("no text at {path_end}"))
}

/// Every text under a key path, in tree order.
fn texts_under(rt: &Runtime<State>, prefix: &str) -> Vec<String> {
    rt.elements()
        .into_iter()
        .filter(|e| e.path.contains(&format!("/{prefix}")) && !e.exiting)
        .filter_map(|e| e.text)
        .collect()
}

fn shared_file(dir: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join(SHARED)).unwrap()).unwrap()
}

/// Waits for the saver thread to write `name` so that `check` holds.
fn eventually(dir: &Path, name: &str, check: impl Fn(&Value) -> bool) -> Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(v) = std::fs::read_to_string(dir.join(name))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            && check(&v)
        {
            return v;
        }
        assert!(std::time::Instant::now() < deadline, "{name} never written");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn the_sidebar_lists_agents_under_its_top_row() {
    let (rt, _) = runtime();
    let brand = element(&rt, "brand").unwrap();
    assert_eq!(brand.rect, [0.0, 0.0, 366.0, 38.0]);
    assert!(has(&rt, "brand-title") && has(&rt, "ag-machines") && has(&rt, "ag-sort"));
    assert_eq!(
        headers(&rt),
        ["Pinned", "Needs you", "Working"],
        "the worker t13 lifts its orchestrator to Working; t10's blocked sub-agent lifts it"
    );
    for key in [
        "ag-t15", "ag-t11", "ag-t10", "ag-t12", "sub-t20", "sub-t21", "sub-t22",
    ] {
        assert!(has(&rt, key), "{key}");
    }
    assert!(
        !has(&rt, "ag-t13"),
        "workers stay folded until their orchestration has focus"
    );
    // The first header sits right under the top row.
    let first = element(&rt, "hdr-pinned").unwrap();
    assert_eq!(first.rect[1], 38.0);
    assert_eq!(first.rect[3], 20.0);
    // Row titles: the named tab, the agent's live title, the Pi session's.
    assert_eq!(text_of(&rt, "ag-t11/body/line1/title"), "review");
    assert_eq!(
        text_of(&rt, "ag-t10/body/line1/title"),
        "Port the agents sidebar"
    );
    assert_eq!(text_of(&rt, "ag-t15/body/line1/jump"), "1");
    let orchestrator = texts_under(&rt, "ag-t12/");
    for want in [
        "orchestrate",
        "dotfiles",
        " · HF-23",
        "2 workers",
        "#21",
        "4",
    ] {
        assert!(
            orchestrator.iter().any(|t| t == want),
            "{want} in {orchestrator:?}"
        );
    }
}

#[test]
fn the_published_order_follows_the_rows() {
    let (mut rt, _) = runtime();
    assert_eq!(rt.state.list.rows_order, ["t15", "t11", "t10", "t12"]);
    assert_eq!(rt.state.model.agent_rows(), rt.state.list.rows_order);
    assert_eq!(
        rt.state.model.agent_order(),
        ["t15", "t11", "t10", "t12", "t13", "t14"],
        "navigation keeps the folded workers"
    );
    // Focusing a worker opens its orchestration, and the rows and numbers follow.
    rt.state.step(0.0, model::Action::FocusAgent("t13".into()));
    rt.state.settle();
    frames(&mut rt, 2);
    assert_eq!(
        rt.state.list.rows_order,
        ["t15", "t11", "t10", "t12", "t13", "t14"]
    );
    assert_eq!(rt.state.model.agent_rows(), rt.state.list.rows_order);
    assert!(has(&rt, "ag-t13") && has(&rt, "ag-t14"));
    assert_eq!(text_of(&rt, "ag-t14/body/line1/jump"), "6");
}

#[test]
fn a_row_click_focuses_its_agent_and_a_sub_agent_click_peeks() {
    let (mut rt, _) = runtime();
    click(&mut rt, "ag-t11");
    assert_eq!(rt.state.model.focused_terminal(), Some("t11"));
    assert_eq!(rt.state.model.active_space().unwrap().id, "sp-1");
    let row = rt.state.list.rows.iter().find_map(|r| match r {
        Row::Agent(a) if a.address == "t11" => Some(a.focused),
        _ => None,
    });
    assert_eq!(row, Some(true), "the focused row shows it");

    click(&mut rt, "ag-t15");
    assert_eq!(rt.state.model.active_space().unwrap().id, "sp-3");

    // A sub-agent has no pane: its click asks for a peek, which comes with ticket 029.
    click(&mut rt, "sub-t20");
    assert_eq!(
        rt.state.model.focused_terminal(),
        Some("t15"),
        "focus stays"
    );
    let last = rt.state.toasts.shown.last().expect("a toast");
    assert_eq!(last.toast.title, "Not yet");
    assert!(last.toast.body.contains("peek"), "{}", last.toast.body);
}

#[test]
fn the_icon_or_chevron_folds_sub_agents_and_headers_never_fold() {
    let (mut rt, _) = runtime();
    click(&mut rt, "ag-t10/body/line1/fold");
    assert!(!has(&rt, "sub-t20") && !has(&rt, "sub-t22"));
    assert_eq!(
        texts_under(&rt, "ag-t10/body/line1/fold/"),
        ["3"],
        "every sub-agent below it"
    );
    click(&mut rt, "ag-t10/body/line1/icon");
    assert!(has(&rt, "sub-t20") && has(&rt, "sub-t22"));

    // A header is not a control: a click leaves every row where it was.
    let before = rt.state.list.clone();
    click(&mut rt, "hdr-sec-4");
    assert_eq!(rt.state.list.rows, before.rows);
}

#[test]
fn a_status_change_moves_the_row() {
    let (mut rt, fake) = runtime();
    fake.update(fixtures::agent(
        "t11",
        "codex",
        StatusState::Working,
        "codex",
    ));
    rt.state.settle();
    frames(&mut rt, 2);
    assert_eq!(rt.state.list.rows_order, ["t15", "t10", "t11", "t12"]);
    assert!(at(&rt, "ag-t11/edge"), "live rows keep their edge");
}

#[test]
fn the_sort_menu_picks_by_click_or_keys() {
    let (mut rt, _) = runtime();
    click(&mut rt, "ag-sort");
    assert!(has(&rt, "sort-menu"));
    let menu = element(&rt, "sort-menu").unwrap();
    let button = element(&rt, "ag-sort").unwrap();
    assert!(
        menu.rect[1] >= button.rect[1] + button.rect[3],
        "under the button"
    );
    assert_eq!(
        texts_under(&rt, "sort-menu/"),
        ["Sort agents", "Priority", "Grouped by space"]
    );
    click(&mut rt, "sort-menu/sort-grouped");
    assert!(!has(&rt, "sort-menu"), "a pick closes it");
    assert_eq!(rt.state.shared.sort(), Sort::Grouped);
    assert_eq!(headers(&rt), ["Pinned", "herdr-frontend", "dotfiles"]);

    // The keys: it opens on the current sort; up, then Enter.
    click(&mut rt, "ag-sort");
    let model::Mode::Popup(model::PopupState::List(list)) = rt.state.model.mode() else {
        panic!("{:?}", rt.state.model.mode())
    };
    assert_eq!(list.cursor, 1, "on Grouped");
    press(&mut rt, Key::Up, Mods::default());
    press(&mut rt, Key::Enter, Mods::default());
    assert_eq!(rt.state.shared.sort(), Sort::Priority);
    assert!(!has(&rt, "sort-menu"));
    // Esc closes it with nothing picked.
    click(&mut rt, "ag-sort");
    press(&mut rt, Key::Escape, Mods::default());
    assert!(!has(&rt, "sort-menu"));
    assert_eq!(rt.state.shared.sort(), Sort::Priority);
}

#[test]
fn cmd_b_and_ctrl_g_b_toggle_the_sidebar() {
    let (mut rt, _) = runtime();
    let cmd = Mods {
        cmd: true,
        ..Mods::default()
    };
    press(&mut rt, Key::Text("b".into()), cmd);
    frames(&mut rt, 60);
    assert!(!has(&rt, "sidebar"));
    assert_eq!(element(&rt, "main").unwrap().rect[0], 0.0);
    let ctrl = Mods {
        ctrl: true,
        ..Mods::default()
    };
    press(&mut rt, Key::Text("g".into()), ctrl);
    press(&mut rt, Key::Text("b".into()), Mods::default());
    frames(&mut rt, 60);
    assert!(has(&rt, "sidebar"));
    assert_eq!(element(&rt, "main").unwrap().rect[0], 366.0);
}

#[test]
fn the_edge_drags_the_width_between_200_and_600() {
    let (mut rt, _) = runtime();
    let drag = |rt: &mut Runtime<State>, by: f32| {
        let (x, y) = rt.find("sidebar-edge").unwrap();
        rt.pointer_down(x, y);
        for step in 1..=4 {
            rt.pointer_move(x + by * step as f32 / 4.0, y);
            rt.frame(1.0 / 60.0);
        }
        rt.pointer_up(x + by, y);
        rt.state.settle();
        frames(rt, 2);
    };
    drag(&mut rt, 100.0);
    assert_eq!(rt.state.sidebar.width, 466.0);
    assert_eq!(element(&rt, "sidebar").unwrap().rect[2], 466.0);
    assert_eq!(element(&rt, "main").unwrap().rect[0], 466.0);
    drag(&mut rt, 400.0);
    assert_eq!(rt.state.sidebar.width, 600.0);
    drag(&mut rt, -700.0);
    assert_eq!(rt.state.sidebar.width, 200.0);
}

#[test]
fn a_homes_first_run_imports_herdr_apps_values_and_keeps_its_own_after() {
    let dir = scratch("import");
    let (mut rt, _) = runtime_with(Some(&dir));
    // From Herdr app: grouped, its layout's 420 pt sidebar (not the rail's 268), sp-2's color.
    assert_eq!(rt.state.shared.sort(), Sort::Grouped);
    assert_eq!(element(&rt, "sidebar").unwrap().rect[2], 420.0);
    assert_eq!(rt.state.shared.space_colors()["sp-2"], "ansi1");
    // Written to the home, with the colors the other spaces got and the published order.
    let file = eventually(&dir, SHARED, |v| v.get("dotfiles.agents.order").is_some());
    assert_eq!(file["space.colors"]["sp-2"], "ansi1");
    assert_eq!(file["space.colors"]["sp-1"], "ansi6");
    assert_eq!(file["dotfiles.agents.order"]["addresses"][0], "t15");
    eventually(&dir, TIMES, |v| v["changed"]["t10"]["s"] == "working");

    // Changes go to the home; Herdr app's file is left as it was.
    click(&mut rt, "ag-sort");
    click(&mut rt, "sort-menu/sort-priority");
    let cmd = Mods {
        cmd: true,
        ..Mods::default()
    };
    press(&mut rt, Key::Text("b".into()), cmd);
    let file = eventually(&dir, SHARED, |v| v["sidebar.hidden"] == true);
    assert_eq!(file["agents.sort"], "priority");
    let app_file: Value =
        serde_json::from_str(&std::fs::read_to_string(herdr_app().join(SHARED)).unwrap()).unwrap();
    assert_eq!(app_file["agents.sort"], "grouped");
    assert!(app_file.get("dotfiles.agents.order").is_none());

    // The next run reads the home's own file.
    drop(rt);
    let (rt, _) = runtime_with(Some(&dir));
    assert_eq!(rt.state.shared.sort(), Sort::Priority);
    assert!(!has(&rt, "sidebar"), "still hidden");
    assert_eq!(shared_file(&dir)["sidebar.px"], 420.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn with_no_agents_the_list_says_how_they_show_up() {
    herdr_app();
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.with_settings(Settings::fixture());
    rt.state.connect(Box::new(fixtures::demo_core()));
    rt.state.settle();
    frames(&mut rt, 3);
    let empty = element(&rt, "agents-empty").expect("the empty note");
    assert!(empty.text.unwrap().starts_with("No agent status yet."));
    assert!(empty.rect[2] <= 366.0, "it wraps inside the sidebar");
}

#[test]
fn the_edge_follows_the_pointer_at_any_zoom() {
    herdr_app();
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.with_settings(Settings {
        zoom: 2.0,
        ..Settings::fixture()
    });
    rt.state.connect(Box::new(agents_core()));
    rt.state.settle();
    frames(&mut rt, 3);
    let (x, y) = rt.find("sidebar-edge").unwrap();
    rt.pointer_down(x, y);
    for step in 1..=4 {
        rt.pointer_move(x + 10.0 * step as f32, y);
        rt.frame(1.0 / 60.0);
    }
    rt.pointer_up(x + 40.0, y);
    rt.state.settle();
    frames(&mut rt, 2);
    assert_eq!(rt.state.sidebar.width, 406.0);
    let (ex, _) = rt.find("sidebar-edge").unwrap();
    assert_eq!(ex, x + 40.0, "the edge is where the pointer let go");
}
