//! The control surface (`docs/cli.md`) through `Model::ls`, `Model::status` and `Model::step`,
//! against a fake core: herdr-app's `ls`/`status` shapes, its argument rules, and what each
//! served `do` command writes, starts, kills and shows.

mod support;

use herdr_proto::types::{MachineState, MachineView};
use herdr_proto::{ErrorKind, StatusState};
use model::{Action, Call, CallId, Effect, Event, Model, Popup};
use serde_json::{Value, json};
use support::*;

/// This machine: `work` (tb1: p1 t1 + p2 t2, the agent; tb2 `build`: p3 t3) and an unnamed space
/// (tb4: p4 t4). devbox: sp9 (tb9: p9 t9). t5 is t2's sub-agent, without a pane.
fn world() -> Core {
    let mut work = space(
        "sp1",
        vec![
            tab("tb1", vec![pane("p1", "t1"), pane("p2", "t2")]),
            tab("tb2", vec![pane("p3", "t3")]),
        ],
    );
    work["name"] = json!("work");
    work["tabs"][1]["name"] = json!("build");
    let mut other = space("sp2", vec![tab("tb4", vec![pane("p4", "t4")])]);
    other["folder"] = json!("/home/me/code/y");
    let mut pi = agent("t2", StatusState::Idle);
    pi.status.as_mut().unwrap().app = "pi".into();
    pi.name = Some("pi".into());
    let mut sub = agent("t5", StatusState::Working);
    sub.parent = Some("t2".into());
    let mut core = Core::new(
        doc(vec![work, other]),
        vec![
            shell("t1", "/home/me/code/one"),
            pi,
            shell("t3", "/home/me/code/x"),
            shell("t4", "/home/me/code/four"),
            sub,
            shell("devbox/t9", "/srv"),
        ],
    );
    core.docs.insert(
        Some("devbox".into()),
        (
            doc(vec![space("sp9", vec![tab("tb9", vec![pane("p9", "t9")])])]),
            1,
        ),
    );
    core
}

fn devbox() -> MachineView {
    let mut view = MachineView {
        state: MachineState::Connected,
        ..Default::default()
    };
    view.machine.name = "devbox".into();
    view
}

fn connected_with_devbox(core: &mut Core) -> Model {
    let mut m = connected(core);
    drive(&mut m, core, Event::Machines(vec![devbox()]));
    m
}

/// Runs `do <name> <args>` and returns its one answer.
fn call(m: &mut Model, core: &mut Core, name: &str, args: Value) -> Value {
    let id = CallId(7);
    let fx = drive(
        m,
        core,
        Call {
            id,
            name: name.into(),
            args,
        },
    );
    answers(&fx, id)
}

fn answers(fx: &[Effect], id: CallId) -> Value {
    let found: Vec<&Value> = fx
        .iter()
        .filter_map(|e| match e {
            Effect::Answer { id: i, reply } if *i == id => Some(reply),
            _ => None,
        })
        .collect();
    assert_eq!(found.len(), 1, "one answer per call: {fx:?}");
    found[0].clone()
}

fn error(reply: &Value) -> &str {
    reply["error"].as_str().unwrap_or_else(|| panic!("{reply}"))
}

fn space_names(m: &Model) -> Vec<String> {
    m.ls()["spaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn ls_lists_spaces_tabs_and_terminals_in_herdr_apps_shape() {
    let mut core = world();
    let m = connected_with_devbox(&mut core);
    assert_eq!(
        m.ls(),
        json!({
            "spaces": [
                {
                    "id": "sp1", "name": "work", "folder": "/home/me/code/x", "machine": null,
                    "active": true,
                    "tabs": [
                        {"id": "tb1", "name": "1", "active": true, "settled": false, "terminals": [
                            {"terminal": "t1", "pane": "p1", "status": null, "app": null,
                             "name": null, "sub_agents": []},
                            {"terminal": "t2", "pane": "p2", "status": "idle", "app": "pi",
                             "name": "pi", "sub_agents": ["t5"]},
                        ]},
                        {"id": "tb2", "name": "build", "active": false, "settled": false,
                         "terminals": [
                            {"terminal": "t3", "pane": "p3", "status": null, "app": null,
                             "name": null, "sub_agents": []},
                        ]},
                    ],
                },
                {
                    "id": "sp2", "name": "four", "folder": "/home/me/code/y", "machine": null,
                    "active": false,
                    "tabs": [
                        {"id": "tb4", "name": "1", "active": true, "settled": false, "terminals": [
                            {"terminal": "t4", "pane": "p4", "status": null, "app": null,
                             "name": null, "sub_agents": []},
                        ]},
                    ],
                },
                {
                    "id": "devbox/sp9", "name": "srv", "folder": "/home/me/code/x",
                    "machine": "devbox", "active": false,
                    "tabs": [
                        {"id": "devbox/tb9", "name": "1", "active": true, "settled": false,
                         "terminals": [
                            {"terminal": "devbox/t9", "pane": "devbox/p9", "status": null,
                             "app": null, "name": null, "sub_agents": []},
                        ]},
                    ],
                },
            ],
            "recent": [],
        })
    );
}

#[test]
fn status_says_what_is_shown_and_which_layouts_fail() {
    let mut core = world();
    let mut m = connected_with_devbox(&mut core);
    let status = Value::Object(m.status());
    assert_eq!(
        status,
        json!({
            "space": "sp1", "tab": "tb1", "focused_pane": "p1", "layout": "eli",
            "agent": "t2", "floats": {}, "overlays": [], "errors": [],
        }),
        "the focused shell isn't an agent: the first agent in the order"
    );
    drive(&mut m, &mut core, Action::FocusPane("p2".into()));
    drive(&mut m, &mut core, Action::Open(Popup::Picker));
    core.down.push(Some("devbox".into()));
    let event = core.write_elsewhere(Some("devbox"), json!({"spaces": []}));
    drive(&mut m, &mut core, event);
    let status = m.status();
    assert_eq!(status["focused_pane"], "p2");
    assert_eq!(status["agent"], "t2");
    assert_eq!(status["overlays"], json!(["picker"]));
    assert_eq!(status["errors"], json!(["layout devbox: machine down"]));
}

#[test]
fn arguments_follow_herdr_apps_rules() {
    let mut core = world();
    let mut m = connected(&mut core);
    for (name, args, want) in [
        (
            "space.close",
            json!({"space": "sp1", "confirmed": true}),
            "space.close: unknown argument(s): space; valid arguments: id, confirmed",
        ),
        (
            "space.rename",
            json!({"id": 3, "name": "x"}),
            "space.rename: argument id must be a string (or null to omit it)",
        ),
        (
            "pane.close",
            json!(["p1"]),
            "pane.close: arguments must be an object; valid arguments: pane",
        ),
        ("agent.focus", json!({}), "agent.focus needs terminal"),
        (
            "agent.focus",
            json!({"terminal": "  "}),
            "agent.focus needs terminal",
        ),
        (
            "tab.new",
            json!({}),
            "unknown command \"tab.new\": see `herdr-frontend commands` for the ones it serves",
        ),
        (
            "space.new",
            json!({"focus": "false"}),
            "space.new focus is true or false",
        ),
        (
            "space.new",
            json!({"argv": ["pi"]}),
            "space.new: argv and task are not served by herdr-frontend yet",
        ),
        (
            "space.rename",
            json!({"id": "sp1"}),
            "space.rename needs name (empty: automatic)",
        ),
        (
            "space.rename",
            json!({"id": "gone", "name": "x"}),
            "no space \"gone\"",
        ),
    ] {
        assert_eq!(error(&call(&mut m, &mut core, name, args)), want, "{name}");
    }
    assert!(core.runs().is_empty() && core.kills().is_empty());
    // Null means omitted: the current target. So does no argument object at all.
    let reply = call(
        &mut m,
        &mut core,
        "tab.rename",
        json!({"id": null, "name": "named"}),
    );
    assert_eq!(reply, json!({"ok": true}));
    assert_eq!(core.doc(None).tab("tb1").unwrap().name, "named");
    assert_eq!(
        call(&mut m, &mut core, "layout.switch", Value::Null),
        json!({"ok": true})
    );
    // `name:value`, as keys write commands: a number fills `index`.
    call(&mut m, &mut core, "space.select:2", Value::Null);
    assert_eq!(m.active_space().unwrap().id, "sp2");
}

#[test]
fn a_new_space_without_focus_waits_behind_the_shown_one_and_answers_its_ids() {
    let mut core = world();
    let mut m = connected(&mut core);
    let reply = call(
        &mut m,
        &mut core,
        "space.new",
        json!({"name": " repo@branch ", "folder": "/home/me/code/repo-branch", "focus": false}),
    );
    let (space, tab, terminal) = (
        reply["space"].as_str().unwrap(),
        reply["tab"].as_str().unwrap(),
        reply["terminal"].as_str().unwrap(),
    );
    assert!(space.starts_with("sp") && tab.starts_with("tb"), "{reply}");
    assert_eq!(reply.as_object().unwrap().len(), 3, "{reply}");
    let run = core.runs().pop().unwrap();
    assert_eq!(run.cwd.as_deref(), Some("/home/me/code/repo-branch"));
    assert_eq!(run.labels["herdr.space"], space);
    assert_eq!(run.labels["herdr.tab"], tab);
    assert_eq!(run.on, None);
    let written = core.doc(None);
    let made = written.space(space).unwrap();
    assert_eq!(made.name, "repo@branch", "trimmed");
    assert_eq!(made.folder, "/home/me/code/repo-branch");
    assert_eq!(made.tabs[0].id, tab);
    assert_eq!(m.active_space().unwrap().id, "sp1", "the shown space stays");
    assert_eq!(m.focused_pane().as_deref(), Some("p1"));
    let listed = m.ls();
    let entry = listed["spaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == space)
        .unwrap();
    assert_eq!(entry["name"], "repo@branch");
    assert_eq!(entry["active"], false);
    assert_eq!(entry["tabs"][0]["terminals"][0]["terminal"], terminal);
}

#[test]
fn a_new_space_with_focus_shows_and_names_its_first_tab() {
    let mut core = world();
    let mut m = connected(&mut core);
    drive(&mut m, &mut core, Action::FocusPane("p2".into()));
    let reply = call(&mut m, &mut core, "space.new", json!({"tab": "first"}));
    let space = reply["space"].as_str().unwrap();
    assert_eq!(m.active_space().unwrap().id, space);
    assert_eq!(m.shown_tab().unwrap().name, "first");
    assert_eq!(m.focused_terminal(), reply["terminal"].as_str());
    assert_eq!(
        core.runs().pop().unwrap().cwd.as_deref(),
        Some("/home/me/code/x"),
        "no folder: the focused terminal's"
    );
}

#[test]
fn a_new_space_without_focus_shows_when_no_space_is_shown() {
    let mut core = Core::new(doc(vec![]), vec![]);
    let mut m = connected(&mut core);
    let reply = call(&mut m, &mut core, "space.new", json!({"focus": false}));
    assert_eq!(m.active_space().unwrap().id, reply["space"]);
    assert_eq!(core.runs().pop().unwrap().cwd.as_deref(), Some(HOME));
}

#[test]
fn a_new_space_on_another_machine_starts_there_in_its_home() {
    let mut core = world();
    let mut m = connected_with_devbox(&mut core);
    let reply = call(
        &mut m,
        &mut core,
        "space.new",
        json!({"machine": "devbox", "focus": false}),
    );
    let space = reply["space"].as_str().unwrap();
    assert!(space.starts_with("devbox/sp"), "{reply}");
    assert!(reply["terminal"].as_str().unwrap().starts_with("devbox/t"));
    let run = core.runs().pop().unwrap();
    assert_eq!(run.on.as_deref(), Some("devbox"));
    assert_eq!(run.cwd, None, "that machine's home");
    assert!(
        core.doc(Some("devbox"))
            .space(space.strip_prefix("devbox/").unwrap())
            .is_some()
    );
    let reply = call(
        &mut m,
        &mut core,
        "space.new",
        json!({"machine": "nowhere"}),
    );
    assert!(error(&reply).starts_with("herdr-frontend has no machine called nowhere"));
}

#[test]
fn a_new_space_whose_terminal_fails_to_start_answers_the_error() {
    let mut core = world();
    let mut m = connected(&mut core);
    let id = CallId(3);
    let fx = m.step(
        m.now(),
        Call {
            id,
            name: "space.new".into(),
            args: json!({"name": "x"}),
        },
    );
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::Answer { .. })),
        "it waits for terminal.run"
    );
    let token = fx
        .iter()
        .find_map(|e| match e {
            Effect::Call { token, .. } => Some(*token),
            _ => None,
        })
        .unwrap();
    let fx = m.step(
        m.now(),
        Event::Reply {
            token,
            result: Err(support::error(ErrorKind::NotFound, "no such folder")),
        },
    );
    assert_eq!(answers(&fx.list, id), json!({"error": "no such folder"}));
}

#[test]
fn a_launch_under_way_when_the_connection_drops_is_answered_once() {
    let mut core = world();
    let mut m = connected(&mut core);
    let id = CallId(4);
    m.step(
        m.now(),
        Call {
            id,
            name: "space.new".into(),
            args: Value::Null,
        },
    );
    let fx = m.step(m.now(), Event::Lost);
    assert_eq!(
        answers(&fx.list, id),
        json!({"error": "the connection to herdr-core dropped"})
    );
    let fx = m.step(m.now(), Event::Connected);
    assert!(!fx.iter().any(|e| matches!(e, Effect::Answer { .. })));
    let reply = call(&mut m, &mut core, "space.rename", json!({"name": "x"}));
    assert_eq!(
        reply,
        json!({"ok": true}),
        "connected again, before any layout read: the last read layout takes the edit"
    );
    m.step(m.now(), Event::Lost);
    let reply = call(&mut m, &mut core, "space.rename", json!({"name": "y"}));
    assert_eq!(
        error(&reply),
        "herdr-frontend is not connected to herdr-core"
    );
}

#[test]
fn spaces_select_by_id_or_position() {
    let mut core = world();
    let mut m = connected_with_devbox(&mut core);
    assert_eq!(
        call(
            &mut m,
            &mut core,
            "space.select",
            json!({"id": "devbox/sp9"})
        ),
        json!({"ok": true})
    );
    assert_eq!(m.active_space().unwrap().id, "devbox/sp9");
    call(&mut m, &mut core, "space.select", json!({"index": 1}));
    assert_eq!(m.active_space().unwrap().id, "sp1");
    for args in [
        json!({"id": "gone"}),
        json!({"index": 0}),
        json!({"index": 9}),
        json!({}),
    ] {
        assert_eq!(
            error(&call(&mut m, &mut core, "space.select", args)),
            "no such space"
        );
    }
}

#[test]
fn renames_write_trimmed_names_and_empty_goes_back_to_automatic() {
    let mut core = world();
    let mut m = connected(&mut core);
    call(
        &mut m,
        &mut core,
        "space.rename",
        json!({"id": "sp2", "name": "  api "}),
    );
    assert_eq!(core.doc(None).space("sp2").unwrap().name, "api");
    assert_eq!(space_names(&m), ["work", "api"]);
    call(
        &mut m,
        &mut core,
        "space.rename",
        json!({"id": "sp2", "name": ""}),
    );
    assert_eq!(space_names(&m), ["work", "four"]);
    call(
        &mut m,
        &mut core,
        "tab.rename",
        json!({"id": "tb2", "name": " Fix login "}),
    );
    assert_eq!(core.doc(None).tab("tb2").unwrap().name, "Fix login");
    assert_eq!(
        error(&call(
            &mut m,
            &mut core,
            "tab.rename",
            json!({"id": "tb9", "name": "x"})
        )),
        "no tab \"tb9\""
    );
    assert_eq!(
        error(&call(&mut m, &mut core, "tab.rename", json!({}))),
        "tab.rename needs name (empty: its number)"
    );
}

#[test]
fn closing_a_space_needs_confirmed_then_ends_its_terminals() {
    let mut core = world();
    let mut m = connected(&mut core);
    assert_eq!(
        error(&call(
            &mut m,
            &mut core,
            "space.close",
            json!({"id": "sp1"})
        )),
        "space.close ends its terminals: pass confirmed: true"
    );
    assert!(core.doc(None).space("sp1").is_some());
    let reply = call(
        &mut m,
        &mut core,
        "space.close",
        json!({"id": "sp1", "confirmed": true}),
    );
    assert_eq!(reply, json!({"ok": true}));
    assert!(core.doc(None).space("sp1").is_none());
    let mut kills = core.kills();
    kills.sort();
    assert_eq!(kills, ["t1", "t2", "t3"]);
    assert_eq!(m.active_space().unwrap().id, "sp2");
    assert_eq!(
        m.ls()["recent"],
        json!([{"name": "work", "folder": "/home/me/code/x"}])
    );
    // Opened again by name, it is no longer a recently closed space; another name leaves it.
    call(&mut m, &mut core, "space.new", json!({"name": "other"}));
    assert_eq!(m.ls()["recent"].as_array().unwrap().len(), 1);
    call(&mut m, &mut core, "space.new", json!({"name": "work"}));
    assert_eq!(m.ls()["recent"], json!([]));
}

/// A closed space's terminals still carry its `herdr.space` label until the core ends them. A
/// slow kill leaves them live and in no pane for a while, which must not adopt them back into a
/// space under the closed id (`repos clean`, then `ls`: herdr-app keeps terminals it's closing
/// out of adoption, `closing_terminals`).
#[test]
fn a_closed_spaces_terminals_are_not_adopted_back_while_they_end() {
    let mut core = world();
    let t4 = core.terminals.get_mut("t4").unwrap();
    t4.labels.insert("herdr.space".into(), "sp2".into());
    t4.labels.insert("herdr.tab".into(), "tb4".into());
    let mut m = connected(&mut core);
    let id = CallId(7);
    let fx = m.step(
        core.now,
        Call {
            id,
            name: "space.close".into(),
            args: json!({"id": "sp2", "confirmed": true}),
        },
    );
    // The core hasn't ended t4 yet: its kill waits, and the terminal is still live.
    let (kills, rest): (Vec<Effect>, Vec<Effect>) = fx.list.into_iter().partition(|e| {
        matches!(
            e,
            Effect::Call {
                request: model::Request::Kill(_),
                ..
            }
        )
    });
    assert_eq!(kills.len(), 1, "t4's kill");
    let out = run_effects(&mut m, &mut core, rest);
    assert_eq!(answers(&out, id), json!({"ok": true}));
    // Something else changes the listing meanwhile, which makes the window look for orphans.
    let t1 = core.terminals["t1"].clone();
    drive(
        &mut m,
        &mut core,
        Event::Terminals(model::TerminalUpdate {
            terminals: vec![t1],
            ..Default::default()
        }),
    );
    assert!(core.doc(None).space("sp2").is_none(), "closed for good");
    assert!(m.world().space("sp2").is_none());
    assert!(
        !core
            .calls
            .iter()
            .any(|c| matches!(c, model::Request::Get(g) if g.terminal == "t4")),
        "t4 is not looked up for adoption: {:?}",
        core.calls
    );
    // The kill lands; the space stays closed.
    run_effects(&mut m, &mut core, kills);
    assert!(core.doc(None).space("sp2").is_none());
    assert!(m.world().space("sp2").is_none());
}

#[test]
fn closing_a_pane_closes_its_tab_when_last_but_never_the_space() {
    let mut core = world();
    let mut m = connected(&mut core);
    assert_eq!(
        call(&mut m, &mut core, "pane.close", json!({"pane": "p2"})),
        json!({"ok": true})
    );
    assert_eq!(core.kills(), ["t2"]);
    assert_eq!(core.doc(None).tab("tb1").unwrap().root.panes().len(), 1);
    call(&mut m, &mut core, "pane.close", json!({"pane": "p3"}));
    assert!(core.doc(None).tab("tb2").is_none(), "its tab's last pane");
    assert_eq!(core.kills(), ["t2", "t3"]);
    assert_eq!(
        error(&call(
            &mut m,
            &mut core,
            "pane.close",
            json!({"pane": "p4"})
        )),
        "space.close ends its terminals: pass confirmed: true"
    );
    assert!(core.doc(None).tab("tb4").is_some());
    for (pane, want) in [("tb1", "no pane \"tb1\""), ("p404", "no pane \"p404\"")] {
        assert_eq!(
            error(&call(
                &mut m,
                &mut core,
                "pane.close",
                json!({"pane": pane})
            )),
            want
        );
    }
    // No pane: the focused one, here the space's last.
    assert_eq!(
        error(&call(&mut m, &mut core, "pane.close", Value::Null)),
        "space.close ends its terminals: pass confirmed: true"
    );
}

#[test]
fn focusing_an_agent_shows_its_pane_tab_and_space() {
    let mut core = world();
    let mut m = connected(&mut core);
    call(&mut m, &mut core, "space.select", json!({"id": "sp2"}));
    assert_eq!(
        call(&mut m, &mut core, "agent.focus", json!({"terminal": "t2"})),
        json!({"ok": true})
    );
    assert_eq!(m.active_space().unwrap().id, "sp1");
    assert_eq!(m.focused_pane().as_deref(), Some("p2"));
    assert_eq!(
        error(&call(
            &mut m,
            &mut core,
            "agent.focus",
            json!({"terminal": "t99"})
        )),
        "no running terminal \"t99\""
    );
}

#[test]
fn eli_is_the_only_layout() {
    let mut core = world();
    let mut m = connected(&mut core);
    assert_eq!(
        call(&mut m, &mut core, "layout.switch", json!({"name": "eli"})),
        json!({"ok": true})
    );
    assert_eq!(
        error(&call(
            &mut m,
            &mut core,
            "layout.switch",
            json!({"name": "default"})
        )),
        "no layout \"default\": herdr-frontend has only the \"eli\" layout"
    );
}

#[test]
fn the_listing_names_every_served_command_with_its_arguments() {
    let listing = model::control::listing();
    let names: Vec<&str> = listing
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "space.new",
            "space.close",
            "space.rename",
            "space.select",
            "tab.rename",
            "pane.close",
            "agent.focus",
            "terminal.peek",
            "layout.switch",
        ]
    );
    assert_eq!(
        listing[6],
        json!({"name": "agent.focus", "args": [{"name": "terminal", "required": true}],
               "about": "Show an agent: focus its pane, showing its tab and space. One without \
                         a pane (a sub-agent) is peeked instead."})
    );
}
