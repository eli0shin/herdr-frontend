//! Another machine through the local core (`feature-inventory.md` §9, `docs/machines.md`),
//! against real, isolated herdr-cores: the local one has a second isolated core added as the
//! machine `devbox` (`hc machine add --via`, a fake second machine on this computer). The app
//! reads devbox's own `herdr-app` document, draws and types into its terminal, lists its agent
//! under the machine filter, shows its line in the Machines dialog, and renames and forgets it.
//!
//! Both cores are `test-*` sessions with scratch dirs under `/tmp`, never the person's. Without
//! an `hc` the test skips, except under CI.

#[path = "../../herdr-client/tests/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use herdr_client::{ConnectOptions, Connection};
use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::{pane, split, tab};
use herdr_frontend::link::Live;
use herdr_frontend::{SIZE, State, app};
use herdr_proto::methods::{DocGet, DocSet, TerminalRun};
use herdr_proto::types::MachineState;
use model::machines::{MachineAction, MachineCall};
use model::{Action, Popup};
use rune::{Env, Runtime};
use serde_json::json;
use support::IsolatedCore;

const WAIT: Duration = Duration::from_secs(60);

fn until(rt: &mut Runtime<State>, what: &str, mut done: impl FnMut(&mut Runtime<State>) -> bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        if done(rt) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn text_of(rt: &Runtime<State>, address: &str) -> String {
    rt.state
        .viewers
        .get(address)
        .and_then(|v| v.pane.as_ref())
        .map(|p| p.snapshot.text())
        .unwrap_or_default()
}

fn label(rt: &Runtime<State>, key: &str) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
        .and_then(|e| e.label.or(e.text))
}

fn state_of(rt: &Runtime<State>, machine: &str) -> Option<MachineState> {
    let m = rt.state.model.world().machine(Some(machine))?;
    m.status.as_ref().map(|v| v.state.clone())
}

#[test]
fn another_machines_layout_terminals_and_line_come_through_the_local_core() {
    let Some(here) = IsolatedCore::new() else {
        return;
    };
    let Some(there) = IsolatedCore::new() else {
        return;
    };
    let added = here.hc(&[
        "machine",
        "add",
        "--via",
        &there.via(),
        "--name",
        "devbox",
        "--json",
    ]);
    assert!(
        added.status.success(),
        "hc machine add: {}{}",
        String::from_utf8_lossy(&added.stdout),
        String::from_utf8_lossy(&added.stderr)
    );

    // On devbox: an agent working, then echoing what's typed; its layout there holds it.
    let conn = Connection::open(&here.transport(), &ConnectOptions::default()).expect("connect");
    let script = "printf '\\033]7501;state=working:app=claude\\033\\\\'; \
                  printf 'remote-ready\\n'; exec cat";
    let address = conn
        .call(TerminalRun {
            argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
            on: Some("devbox".into()),
            keep: true,
            ..Default::default()
        })
        .wait()
        .expect("terminal.run on devbox")
        .terminal
        .address;
    let local = address.strip_prefix("devbox/").expect("a devbox address");
    let read = conn
        .call(DocGet {
            name: "herdr-app".into(),
            on: Some("devbox".into()),
            ..Default::default()
        })
        .wait()
        .expect("doc.get on devbox");
    // Its document holds bare ids, as every machine's does.
    let doc = json!({"spaces": [{
        "id": "sp-r", "name": "remote", "folder": "/tmp",
        "tabs": [tab("tb-r", "work", split("tb-r", "row", vec![pane("pn-r", local)]))],
    }]});
    conn.call(DocSet {
        name: "herdr-app".into(),
        value: doc,
        version: read.version,
        on: Some("devbox".into()),
        ..Default::default()
    })
    .wait()
    .expect("doc.set on devbox");

    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.with_settings(Settings::fixture());
    rt.state.connect(Box::new(Live::new(here.transport())));

    // devbox's space shows after this computer's (it has none), its ids qualified.
    until(&mut rt, "devbox's layout", |rt| {
        rt.state.model.world().space("devbox/sp-r").is_some()
    });
    rt.state.step(0.0, Action::ShowSpace("devbox/sp-r".into()));
    until(&mut rt, "devbox's terminal to draw", |rt| {
        text_of(rt, &address).contains("remote-ready")
    });
    assert_eq!(label(&rt, "tab-machine-name").as_deref(), Some("devbox"));
    let hello = rt.state.hello.clone().expect("the local core's hello");
    assert!(!hello.core.id.is_empty());

    // Its pane is focused and takes the size; typing reaches the program over there, and its
    // echo comes back.
    until(&mut rt, "devbox's pane to be in use", |rt| {
        rt.state.in_use() == Some(address.as_str())
    });
    for c in "over-there".chars() {
        rt.key(rune::KeyEvent {
            key: rune::Key::Text(c.to_string()),
            mods: rune::Mods::default(),
        });
        rt.frame(1.0 / 60.0);
    }
    until(&mut rt, "the typed text to echo from devbox", |rt| {
        text_of(rt, &address).contains("over-there")
    });

    // The machine filter: devbox is up, with its agent.
    until(&mut rt, "devbox's agent in the list", |rt| {
        rt.state.list.machines.get("devbox") == Some(&1)
    });
    rt.state.step(0.0, Action::Open(Popup::AgentMachines));
    until(&mut rt, "the filter's rows", |rt| {
        label(rt, "am-label-2").as_deref() == Some("devbox")
    });
    assert_eq!(label(&rt, "am-count-2").as_deref(), Some("1 agent"));
    assert!(label(&rt, "am-offline").is_none(), "devbox is connected");
    // Space on devbox hides its agent.
    rt.state.step(0.0, model::Event::PopupCursor(2));
    rt.state
        .step(0.0, Action::Popup(model::PopupAction::Toggle));
    until(&mut rt, "devbox's agent to leave the list", |rt| {
        rt.state.list.rows_order.iter().all(|a| a != &address)
    });
    assert!(rt.state.shared.hidden_machines().contains("devbox"));
    rt.state.step(0.0, Action::Popup(model::PopupAction::Close));

    // The Machines dialog: devbox's line and the hc version both cores run.
    rt.state.step(0.0, Action::Open(Popup::Machines));
    until(&mut rt, "the Machines dialog", |rt| {
        label(rt, "machine-name-1").as_deref() == Some("devbox")
    });
    let detail = label(&rt, "machine-detail-1").unwrap();
    assert!(detail.contains("connected"), "{detail}");
    assert!(
        detail.contains(&format!("hc {}", hello.version)),
        "{detail}"
    );
    assert_eq!(
        label(&rt, "machine-detail-0"),
        Some(format!("Connected · hc {}", hello.version))
    );
    rt.state.step(0.0, Action::Popup(model::PopupAction::Close));

    // Connect now answers for a connected machine too.
    rt.state.step(
        0.0,
        Action::Machine(MachineAction::Connect("devbox".into())),
    );
    until(&mut rt, "machine.connect", |rt| {
        rt.state.machines.busy.is_empty()
    });
    assert!(rt.state.toasts.shown.is_empty(), "connecting worked");

    // A target ssh can't reach fails, and the add dialog says why.
    rt.state.step(
        0.0,
        Action::Machine(MachineAction::Add(
            "nobody@herdr-frontend-test.invalid".into(),
        )),
    );
    until(&mut rt, "machine.add to fail", |rt| {
        rt.state.machines.busy.is_empty()
    });
    let failure = rt
        .state
        .machines
        .failed
        .get("nobody@herdr-frontend-test.invalid")
        .expect("a failure");
    assert!(!failure.message.is_empty());
    assert!(
        rt.state
            .model
            .world()
            .machine(Some("herdr-frontend-test.invalid"))
            .is_none()
    );

    // Rename, then forget it: the core's list follows each, and its space goes with it.
    rt.state
        .step(0.0, Action::Machine(MachineAction::Rename("devbox".into())));
    rt.state
        .step(0.0, Action::Popup(model::PopupAction::Query("box2".into())));
    rt.state
        .step(0.0, Action::Popup(model::PopupAction::Choose));
    until(&mut rt, "the rename to show", |rt| {
        state_of(rt, "box2").is_some() && state_of(rt, "devbox").is_none()
    });
    until(&mut rt, "its space under the new name", |rt| {
        rt.state.model.world().space("box2/sp-r").is_some()
    });
    rt.state
        .step(0.0, Action::Machine(MachineAction::Remove("box2".into())));
    rt.state
        .step(0.0, Action::Popup(model::PopupAction::Choose));
    until(&mut rt, "box2 to be forgotten", |rt| {
        state_of(rt, "box2").is_none()
    });
    assert!(rt.state.model.world().space("box2/sp-r").is_none());
    let said: Vec<String> = rt
        .state
        .toasts
        .shown
        .iter()
        .map(|t| t.toast.title.clone())
        .collect();
    assert!(
        said.contains(&MachineCall::Remove("box2".into()).done()),
        "{said:?}"
    );
    drop(rt);
    drop(conn);
    drop(here);
    drop(there);
}
