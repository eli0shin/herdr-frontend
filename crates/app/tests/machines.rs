//! Machines (`feature-inventory.md` §9, `docs/machines.md`) through Rune's real runtime with no
//! window, on the fake core with other machines ([`fixtures::machines_core`]): another machine's
//! spaces and agents, the machine filter and the hidden list it keeps, the Machines dialog and
//! its menu, and adding a machine.

use std::path::{Path, PathBuf};

use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::{machine, machines_core};
use herdr_frontend::home::Home;
use herdr_frontend::link::{Fake, Offline};
use herdr_frontend::shared::SHARED;
use herdr_frontend::window::Keeper;
use herdr_frontend::{SIZE, State, app};
use herdr_proto::types::MachineState;
use herdr_proto::{ErrorKind, RpcError};
use model::agents::Row;
use model::machines::MachineCall;
use model::mode::{ListPopup, PopupState};
use model::{Mode, Popup};
use rune::{ElementInfo, Env, Key, KeyEvent, Mods, Runtime};
use serde_json::Value;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "herdr-frontend-machines-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A runtime on the machines core with its own home, settled and laid out.
fn runtime(name: &str) -> (Runtime<State>, Fake, PathBuf) {
    let dir = scratch(name);
    let fake = machines_core();
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state = State::new(
        Box::new(Offline),
        Some(Home::at(dir.join("home"))),
        Keeper::default(),
    );
    State::setup(&mut rt.state, std::sync::Arc::new(|| {}));
    rt.state.with_settings(Settings::fixture());
    rt.state.connect(Box::new(fake.clone()));
    rt.state.settle();
    frames(&mut rt, 3);
    (rt, fake, dir)
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

fn has(rt: &Runtime<State>, key: &str) -> bool {
    element(rt, key).is_some()
}

fn text_of(rt: &Runtime<State>, key: &str) -> String {
    let e = element(rt, key).unwrap_or_else(|| panic!("no {key}"));
    e.label.or(e.text).unwrap_or_default()
}

fn focused(rt: &Runtime<State>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.focused)
        .and_then(|e| e.key)
}

fn press(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.state.settle();
    frames(rt, 2);
}

fn key(rt: &mut Runtime<State>, key: Key) {
    press(rt, key, Mods::default());
}

fn letter(rt: &mut Runtime<State>, c: char) {
    key(rt, Key::Text(c.to_string()));
}

fn type_text(rt: &mut Runtime<State>, text: &str) {
    for c in text.chars() {
        letter(rt, c);
    }
}

/// `ctrl-g` then `c`.
fn prefixed(rt: &mut Runtime<State>, c: char) {
    let ctrl = Mods {
        ctrl: true,
        ..Mods::default()
    };
    press(rt, Key::Text("g".into()), ctrl);
    letter(rt, c);
}

fn click(rt: &mut Runtime<State>, key: &str) {
    let e = element(rt, key).unwrap_or_else(|| panic!("no {key}"));
    let (x, y) = (e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.state.settle();
    frames(rt, 2);
}

fn list(rt: &Runtime<State>) -> ListPopup {
    match rt.state.model.mode() {
        Mode::Popup(PopupState::List(l)) => l.clone(),
        other => panic!("{other:?}"),
    }
}

fn open(rt: &mut Runtime<State>, popup: Popup) {
    rt.state.step(0.0, model::Action::Open(popup));
    rt.state.settle();
    frames(rt, 2);
}

/// The Agents list's agents, in order.
fn agents(rt: &Runtime<State>) -> Vec<String> {
    rt.state
        .list
        .rows
        .iter()
        .filter_map(|r| match r {
            Row::Agent(a) => Some(a.address.clone()),
            _ => None,
        })
        .collect()
}

/// Waits for the saver thread to write `want` as the hidden machines in the home's
/// `shared.json` (it writes off the UI thread, so an older value may be there first).
fn assert_hidden(dir: &Path, want: Value) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let file = dir.join("home").join(SHARED);
    loop {
        let hidden = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.get(model::agents::HIDDEN_MACHINES_KEY).cloned());
        if hidden.as_ref() == Some(&want) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "shared.json has {hidden:?}, not {want}"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn toasts(rt: &Runtime<State>) -> Vec<(String, String)> {
    rt.state
        .toasts
        .shown
        .iter()
        .map(|t| (t.toast.title.clone(), t.toast.body.clone()))
        .collect()
}

fn machine_calls(fake: &Fake) -> Vec<MachineCall> {
    fake.core().machine_calls.clone()
}

#[test]
fn another_machines_spaces_and_agents_show_beside_this_ones() {
    let (mut rt, _, _) = runtime("remote");
    let world = rt.state.model.world();
    let spaces: Vec<&str> = world.spaces().map(|s| s.id.as_str()).collect();
    assert_eq!(spaces, ["sp-1", "sp-2", "sp-3", "devbox/sp-api"]);
    assert!(
        world.machine(Some("work-mac")).is_none(),
        "temporary machines are left out"
    );
    // devbox's Claude is listed, with its machine on line 2.
    assert!(agents(&rt).contains(&"devbox/t1".to_owned()));
    let row = rt
        .state
        .list
        .rows
        .iter()
        .find_map(|r| match r {
            Row::Agent(a) if a.address == "devbox/t1" => Some(a.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(row.machine.as_deref(), Some("devbox"));
    // Its sub-agent names devbox by its host (`host-devbox/t1`) and still nests under it.
    let rows = &rt.state.list.rows;
    let at = rows
        .iter()
        .position(|r| matches!(r, Row::Agent(a) if a.address == "devbox/t1"))
        .unwrap();
    assert!(
        matches!(&rows[at + 1], Row::Sub(s) if s.address == "devbox/t3"),
        "{:?}",
        rows.get(at + 1)
    );
    // Showing devbox's space names it at the tab bar's end.
    rt.state
        .step(0.0, model::Action::ShowSpace("devbox/sp-api".into()));
    rt.state.settle();
    frames(&mut rt, 2);
    assert_eq!(text_of(&rt, "tab-machine-name"), "devbox");
}

#[test]
fn the_machine_filter_lists_machines_and_space_hides_one_in_the_shared_state() {
    let (mut rt, _, dir) = runtime("filter");
    prefixed(&mut rt, '/');
    assert_eq!(list(&rt).popup, Popup::AgentMachines);
    assert_eq!(focused(&rt).as_deref(), Some("agent-machines"));
    assert_eq!(text_of(&rt, "am-title"), "Agents from");
    let labels: Vec<String> = (0..4)
        .map(|i| text_of(&rt, &format!("am-label-{i}")))
        .collect();
    assert_eq!(
        labels,
        ["All machines", "This computer", "devbox", "omarchy"]
    );
    assert!(!has(&rt, "am-label-4"), "the sandbox is left out");
    assert_eq!(text_of(&rt, "am-count-2"), "1 agent");
    assert_eq!(text_of(&rt, "am-count-1"), "6 agents");
    assert!(has(&rt, "am-offline"), "omarchy needs a login");
    assert!(!has(&rt, "am-only"), "not on \"All machines\"");

    // j, j: devbox, "⏎ only" on it; Space hides it and the filter stays.
    letter(&mut rt, 'j');
    letter(&mut rt, 'j');
    assert_eq!(list(&rt).cursor, 2);
    assert!(has(&rt, "am-only"));
    key(&mut rt, Key::Text(" ".into()));
    assert_eq!(list(&rt).popup, Popup::AgentMachines);
    assert_hidden(&dir, serde_json::json!(["devbox"]));
    assert!(!agents(&rt).contains(&"devbox/t1".to_owned()));
    // Its agents are still tracked: a remote agent keeps its time while hidden.
    assert!(rt.state.agents.changed.contains_key("devbox/t1"));
    // The header button says so.
    assert_eq!(text_of(&rt, "ag-machines-hidden"), "1 hidden");

    // k up to this computer, Enter: only it shows, and the filter closes.
    letter(&mut rt, 'k');
    key(&mut rt, Key::Enter);
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
    assert_hidden(&dir, serde_json::json!(["devbox", "omarchy"]));

    // "All machines" shows every one again: the key is null, as the mod stores it.
    prefixed(&mut rt, '/');
    key(&mut rt, Key::Enter);
    assert_hidden(&dir, Value::Null);
    assert!(agents(&rt).contains(&"devbox/t1".to_owned()));

    // q closes.
    prefixed(&mut rt, '/');
    letter(&mut rt, 'q');
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
}

#[test]
fn the_header_button_opens_the_filter_and_a_click_toggles_a_row() {
    let (mut rt, _, dir) = runtime("click");
    click(&mut rt, "ag-machines");
    assert_eq!(list(&rt).popup, Popup::AgentMachines);
    click(&mut rt, "am-1");
    assert_eq!(
        list(&rt).popup,
        Popup::AgentMachines,
        "a click keeps it open"
    );
    assert_eq!(list(&rt).cursor, 1);
    assert_hidden(&dir, serde_json::json!([""]));
    let every_local_agent_hidden = agents(&rt).iter().all(|a| a.starts_with("devbox/"));
    assert!(every_local_agent_hidden, "{:?}", agents(&rt));
    click(&mut rt, "am-1");
    assert_hidden(&dir, Value::Null);
    key(&mut rt, Key::Escape);
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
}

#[test]
fn a_hidden_machine_known_only_from_the_list_stays_until_shown() {
    let (mut rt, fake, _) = runtime("gone");
    rt.state.set_hidden_machines(["old-box".to_owned()].into());
    // omarchy is forgotten elsewhere; devbox drops.
    fake.set_machines(vec![machine("devbox", "devbox", MachineState::Unreachable)]);
    rt.state.settle();
    open(&mut rt, Popup::AgentMachines);
    let labels: Vec<String> = (0..4)
        .map(|i| text_of(&rt, &format!("am-label-{i}")))
        .collect();
    assert_eq!(
        labels,
        ["All machines", "This computer", "devbox", "old-box"]
    );
    // Both are offline: devbox is unreachable, old-box isn't added.
    let offline = rt
        .elements()
        .into_iter()
        .filter(|e| e.key.as_deref() == Some("am-offline") && !e.exiting)
        .count();
    assert_eq!(offline, 2);
}

#[test]
fn the_machines_dialog_shows_each_line_and_what_a_machine_needs() {
    let (mut rt, _, _) = runtime("dialog");
    open(&mut rt, Popup::Machines);
    assert_eq!(focused(&rt).as_deref(), Some("machines"));
    assert_eq!(text_of(&rt, "machine-name-0"), "This computer");
    assert_eq!(
        text_of(&rt, "machine-detail-0"),
        "Connected · hc version unknown"
    );
    assert_eq!(text_of(&rt, "machine-name-1"), "devbox");
    assert_eq!(
        text_of(&rt, "machine-detail-1"),
        format!("devbox · connected · hc {}", herdr_proto::MIN_CORE)
    );
    assert_eq!(text_of(&rt, "machine-name-2"), "omarchy");
    assert_eq!(text_of(&rt, "machine-need-2-0"), "hc machine login omarchy");
    assert_eq!(
        text_of(&rt, "machine-need-2-1"),
        "sudo loginctl enable-linger -- eli"
    );
    assert_eq!(text_of(&rt, "machine-name-3"), "Add a machine");
    assert_eq!(list(&rt).rows, 4);
}

#[test]
fn a_machines_menu_connects_copies_renames_and_removes() {
    let (mut rt, fake, _) = runtime("menu");
    open(&mut rt, Popup::Machines);
    // omarchy's menu.
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Enter);
    let Mode::Popup(PopupState::Menu(menu)) = rt.state.model.mode().clone() else {
        panic!("{:?}", rt.state.model.mode());
    };
    let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Connect now",
            "Copy hc machine login omarchy",
            "Copy sudo loginctl enable-linger -- eli",
            "Rename",
            "Remove"
        ]
    );
    // Connect now: dialed, and back to the dialog, where it now reads connected.
    key(&mut rt, Key::Enter);
    assert_eq!(
        machine_calls(&fake),
        [MachineCall::Connect("omarchy".into())]
    );
    assert_eq!(list(&rt).popup, Popup::Machines);
    assert!(text_of(&rt, "machine-detail-2").contains("connected"));

    // The dialog came back on omarchy. Copy the linger command: on the clipboard, and a
    // toast says so. Connected now, it needs no login.
    assert_eq!(list(&rt).cursor, 2);
    key(&mut rt, Key::Enter);
    assert_eq!(menu_item(&rt, "Copy sudo loginctl enable-linger -- eli"), 0);
    key(&mut rt, Key::Enter);
    assert!(
        toasts(&rt).contains(&("Copied".into(), "sudo loginctl enable-linger -- eli".into())),
        "{:?}",
        toasts(&rt)
    );
    assert_eq!(list(&rt).popup, Popup::Machines);

    // Rename: the prompt holds its name; a new one is sent.
    key(&mut rt, Key::Enter);
    let rename = menu_item(&rt, "Rename");
    move_to(&mut rt, rename);
    key(&mut rt, Key::Enter);
    assert_eq!(text_of(&rt, "prompt-title"), "Rename machine");
    // Select-all then type over it.
    press(
        &mut rt,
        Key::Command('a'),
        Mods {
            cmd: true,
            logo: true,
            ..Mods::default()
        },
    );
    type_text(&mut rt, "arch");
    key(&mut rt, Key::Enter);
    assert_eq!(
        machine_calls(&fake).last(),
        Some(&MachineCall::Rename {
            machine: "omarchy".into(),
            to: "arch".into()
        })
    );
    assert!(toasts(&rt).contains(&("Renamed omarchy to arch".into(), String::new())));
    assert!(rt.state.model.world().machine(Some("arch")).is_some());

    // Remove asks first; confirming forgets it. The dialog opens on it under its new name,
    // which now sorts first.
    open(&mut rt, Popup::Machines);
    assert_eq!(text_of(&rt, "machine-name-1"), "arch");
    assert_eq!(list(&rt).cursor, 1);
    key(&mut rt, Key::Enter);
    let remove = menu_item(&rt, "Remove");
    move_to(&mut rt, remove);
    key(&mut rt, Key::Enter);
    let Mode::Popup(PopupState::Confirm(confirm)) = rt.state.model.mode().clone() else {
        panic!("{:?}", rt.state.model.mode());
    };
    assert_eq!(confirm.title, "Remove machine?");
    key(&mut rt, Key::Enter);
    assert_eq!(
        machine_calls(&fake).last(),
        Some(&MachineCall::Remove("arch".into()))
    );
    assert!(rt.state.model.world().machine(Some("arch")).is_none());
}

fn menu_item(rt: &Runtime<State>, label: &str) -> usize {
    match rt.state.model.mode() {
        Mode::Popup(PopupState::Menu(m)) => m.items.iter().position(|i| i.label == label).unwrap(),
        other => panic!("{other:?}"),
    }
}

fn move_to(rt: &mut Runtime<State>, item: usize) {
    for _ in 0..item {
        key(rt, Key::Down);
    }
}

#[test]
fn a_failed_call_says_so_in_a_toast() {
    let (mut rt, fake, _) = runtime("fails");
    fake.core().machine_fails.insert(
        "omarchy".into(),
        RpcError {
            code: -32000,
            message: "it said no".into(),
            kind: ErrorKind::Unreachable,
            hint: None,
            data: Default::default(),
        },
    );
    rt.state.step(
        0.0,
        model::Action::Machine(model::machines::MachineAction::Connect("omarchy".into())),
    );
    rt.state.settle();
    assert!(toasts(&rt).contains(&("Couldn't connect to omarchy".into(), "it said no".into())));
    assert!(rt.state.machines.busy.is_empty());
}

fn ssh_config(dir: &Path) -> PathBuf {
    let config = dir.join("ssh_config");
    std::fs::write(
        &config,
        "Host devbox\n  HostName 10.0.0.7\nHost box\n  User eli\n  HostName box.local\nHost vps\n",
    )
    .unwrap();
    config
}

#[test]
fn adding_a_machine_lists_the_ssh_hosts_not_added_yet_and_adds_the_one_picked() {
    let (mut rt, fake, dir) = runtime("add");
    rt.state.machines.ssh_config = Some(ssh_config(&dir));
    open(&mut rt, Popup::MachineAdd);
    assert_eq!(focused(&rt).as_deref(), Some("popup-filter"));
    assert_eq!(text_of(&rt, "add-heading"), "From your ssh config");
    // devbox is added already.
    assert_eq!(text_of(&rt, "add-name-0"), "box");
    assert_eq!(text_of(&rt, "add-name-1"), "vps");
    assert_eq!(list(&rt).rows, 2);
    key(&mut rt, Key::Enter);
    assert_eq!(machine_calls(&fake), [MachineCall::Add("box".into())]);
    assert_eq!(list(&rt).popup, Popup::MachineAdd, "the dialog stays");
    assert!(toasts(&rt).contains(&("Added box".into(), String::new())));
    assert!(rt.state.model.world().machine(Some("box")).is_some());
    assert_eq!(text_of(&rt, "add-name-0"), "vps", "box is added now");

    // A target typed in: "Add can@10.0.0.9", added, the field emptied.
    type_text(&mut rt, "can@10.0.0.9");
    assert_eq!(text_of(&rt, "add-name-0"), "Add can@10.0.0.9");
    key(&mut rt, Key::Enter);
    assert_eq!(
        machine_calls(&fake).last(),
        Some(&MachineCall::Add("can@10.0.0.9".into()))
    );
    assert_eq!(rt.state.popup_field.text, "");
    assert_eq!(list(&rt).query, "");
    key(&mut rt, Key::Escape);
    assert_eq!(*rt.state.model.mode(), Mode::Normal);
}

#[test]
fn an_add_that_needs_answers_says_what_to_run_and_enter_copies_it() {
    let (mut rt, fake, dir) = runtime("add-fails");
    rt.state.machines.ssh_config = Some(ssh_config(&dir));
    fake.core().machine_fails.insert(
        "vps".into(),
        RpcError {
            code: -32000,
            message: "no herdr-core there".into(),
            kind: ErrorKind::HcMissing,
            hint: None,
            data: Default::default(),
        },
    );
    open(&mut rt, Popup::MachineAdd);
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Enter);
    assert_eq!(machine_calls(&fake), [MachineCall::Add("vps".into())]);
    assert!(
        text_of(&rt, "add-message-1").contains("Run `hc machine add vps` in a terminal"),
        "{}",
        text_of(&rt, "add-message-1")
    );
    assert_eq!(text_of(&rt, "add-state-1"), "⏎ copy");
    key(&mut rt, Key::Enter);
    assert_eq!(machine_calls(&fake).len(), 1, "Enter copies instead");
    assert!(toasts(&rt).contains(&("Copied".into(), "hc machine add vps".into())));

    // Done in a terminal: opening the dialog again tries again.
    fake.core().machine_fails.clear();
    key(&mut rt, Key::Escape);
    open(&mut rt, Popup::MachineAdd);
    assert_eq!(text_of(&rt, "add-state-1"), "⏎ add");
    key(&mut rt, Key::Down);
    key(&mut rt, Key::Enter);
    assert_eq!(
        machine_calls(&fake).last(),
        Some(&MachineCall::Add("vps".into()))
    );
    assert!(rt.state.model.world().machine(Some("vps")).is_some());
}

#[test]
fn the_palette_reaches_the_machine_dialogs() {
    let (mut rt, _, _) = runtime("palette");
    let rows = model::popups::palette_rows(&rt.state.model, "machines");
    let titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
    assert!(titles.contains(&"Machines"), "{titles:?}");
    assert!(titles.contains(&"Machine filter"), "{titles:?}");
    let add = model::popups::palette_rows(&rt.state.model, "add a machine");
    assert_eq!(add[0].title, "Add a machine");
    open(&mut rt, Popup::MachineAdd);
    assert_eq!(list(&rt).popup, Popup::MachineAdd);
}

#[test]
fn a_new_space_on_a_machine_that_needs_a_login_says_what_to_run_with_copy() {
    let (mut rt, _fake, _dir) = runtime("login-card");
    let board = rune::clipboard::memory();
    rt.use_clipboard(&board);
    rt.state
        .step(0.0, model::Action::SpaceNew(Some("omarchy".into())));
    frames(&mut rt, 2);
    let shown = rt.state.toasts.shown.last().expect("a toast").clone();
    assert_eq!(shown.toast.title, "omarchy needs a login: run");
    assert_eq!(shown.toast.body, "hc machine login omarchy");
    let copy = format!("toast-{}-copy", shown.id);
    let (x, y) = rt.find(&copy).expect("its Copy button");
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, y);
    frames(&mut rt, 2);
    assert_eq!(
        board.text().as_deref().map(String::as_str),
        Some("hc machine login omarchy")
    );
    let after = rt.state.toasts.shown.iter().find(|t| t.id == shown.id);
    assert!(
        after.is_some_and(|t| t.copied),
        "the card stays and says Copied"
    );
}
