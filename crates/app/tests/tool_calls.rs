//! Eli's tools driving the window with herdr-frontend selected (`docs/cli.md` "Frontend
//! selection"): each test makes one tool's calls exactly as the tool makes them (the same
//! arguments, the JSON form or the `--key value` form it uses, and the same `hc run` labels) and
//! checks the fields and exit codes the tool reads. The tools themselves live in other repos:
//!
//! - `repos`: `src/workspace-manager/herdr-app-adapter.ts`.
//! - `spawn-worker`: `~/.dotfiles/home/.agents/skills/orchestrator/scripts/spawn-worker`.
//! - `agent-status`: `~/.dotfiles/home/.agents/skills/agent-status/` (`scripts/agent-status`,
//!   `ACTIONS.md`).
//! - Pi's `auto-session-name`: `~/.dotfiles/home/.pi/agent/extensions/auto-session-name.ts`.
//! - `dot stow`: `~/.dotfiles/scripts/configure-herdr-app.sh`.
//!
//! Each runs the whole app headless on its own isolated core and home (`window`). Without an
//! `hc` the tests skip, except under CI.

#[path = "../../herdr-client/tests/support/mod.rs"]
mod support;
mod window;

use std::path::Path;

use herdr_client::{ConnectOptions, Connection};
use herdr_frontend::State;
use herdr_frontend::fixtures::{pane, split, tab};
use rune::Runtime;
use serde_json::{Value, json};
use support::IsolatedCore;
use window::{
    Scratch, bare_launch, cli, cli_without_window, fails, is_live, json_of, layout_until, ls, ok,
    open, run, run_sh, set_layout, space_in, status, until,
};

/// A fake Pi: reports itself idle over OSC 7501, as Pi does, then waits.
const PI: &str = r"printf '\033]7501;state=idle:app=pi\033\\'; exec cat";
/// A fake Claude, working.
const CLAUDE: &str = r"printf '\033]7501;state=working:app=claude\033\\'; exec cat";

/// A fresh space or tab id as the tools make them: `sp`/`tb` and 32 hex digits.
fn fresh(kind: &str, n: u128) -> String {
    format!("{kind}{n:032x}")
}

/// The local spaces `ls` lists, as every tool reads them (`machine` unset or null).
fn local_spaces(listed: &Value) -> Vec<&Value> {
    listed["spaces"]
        .as_array()
        .expect("ls has spaces")
        .iter()
        .filter(|s| s["machine"].is_null())
        .collect()
}

/// The terminals of a listed space, tab by tab.
fn terminals_of(space: &Value) -> Vec<&str> {
    space["tabs"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|tab| tab["terminals"].as_array().into_iter().flatten())
        .filter_map(|t| t["terminal"].as_str())
        .collect()
}

/// Frames and `ls` until `found` picks something out of the listing.
fn ls_until<T>(
    rt: &mut Runtime<State>,
    home: &Path,
    what: &str,
    mut found: impl FnMut(&Value) -> Option<T>,
) -> T {
    let mut got = None;
    until(rt, what, |rt| {
        got = found(&rt.state.model.ls());
        got.is_some()
    });
    // The CLI's listing is the model's; read it through the CLI as the tool does.
    found(&ls(rt, home)).unwrap_or_else(|| panic!("{what}: not in `herdr-frontend ls`"))
}

/// `hc run --bg` on the isolated core with `args`, as a tool runs it. Returns the new terminal.
fn hc_run(core: &IsolatedCore, args: &[&str]) -> String {
    let mut all = vec!["run"];
    all.extend_from_slice(args);
    let out = core.hc(&all);
    assert!(out.status.success(), "hc {all:?}: {out:?}");
    let reply: Value =
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {out:?}"));
    reply["address"]
        .as_str()
        .or_else(|| reply["terminal"]["address"].as_str())
        .unwrap_or_else(|| panic!("no address in {reply}"))
        .to_owned()
}

/// `repos` (`herdr-app-adapter.ts`): `ls` to find spaces by folder and name; `do <command>
/// <JSON.stringify(args)>` for `space.new`, `space.rename`, `space.select` and `space.close`;
/// the bare binary to bring the window forward; and `/no Herdr window/` on stderr when none is
/// open.
#[test]
fn repos_workspace_manager_calls() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect");
    let a = run(&conn, &core);
    let folder = core.folder().display().to_string();
    let worktree = core.folder().join("repo-feature");
    let other = core.folder().join("repo-other");
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let (worktree, other) = (worktree.display().to_string(), other.display().to_string());
    set_layout(
        &conn,
        json!({"spaces": [
            {"id": "sp-1", "name": "work", "folder": folder, "tabs": [
                tab("tb-1", "1", split("tb-1", "row", vec![pane("pn-a", &a)])),
            ]},
        ]}),
    );
    let scratch = Scratch::new("repos");
    let home = scratch.0.as_path();
    let mut rt = open(&core, home);
    // `isHerdrAppTerminal`: `ls` exits 0 and a local space holds `HC_TERMINAL_ID`.
    let listed = ls_until(&mut rt, home, "the seeded space", |l| {
        local_spaces(l)
            .iter()
            .any(|s| terminals_of(s).contains(&a.as_str()))
            .then(|| l.clone())
    });
    // `listSpaces`: each space's `id`, `name` and `folder`.
    let work = local_spaces(&listed)[0];
    assert_eq!(
        (&work["id"], &work["name"], &work["folder"]),
        (&json!("sp-1"), &json!("work"), &json!(folder))
    );

    // `repos work --no-focus`: `ensureSpace` opens the space with `focus: false`.
    let args = format!(r#"{{"name":"repo@feature","folder":{worktree:?},"focus":false}}"#);
    let out = cli(&mut rt, home, &["do", "space.new", &args]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let space = json_of(&out)["space"]
        .as_str()
        .expect("space.new answers the space id")
        .to_owned();
    assert_eq!(status(&mut rt, home)["space"], "sp-1", "focus: false");
    let listed = ls(&mut rt, home);
    let made = local_spaces(&listed)
        .into_iter()
        .find(|s| s["id"] == space.as_str())
        .expect("the new space");
    assert_eq!(
        (&made["name"], &made["folder"]),
        (&json!("repo@feature"), &json!(worktree))
    );

    // A matched space under another name takes the collision-safe one.
    let args = format!(r#"{{"id":"{space}","name":"repo@feature-2"}}"#);
    ok(&mut rt, home, &["do", "space.rename", &args]);
    layout_until(&mut rt, &conn, "the rename in the core's layout", |doc| {
        space_in(doc, &space).is_some_and(|s| s["name"] == "repo@feature-2")
    });

    // `repos work` with focus: an existing space is selected, then the window raised.
    let args = format!(r#"{{"id":"{space}"}}"#);
    ok(&mut rt, home, &["do", "space.select", &args]);
    assert_eq!(status(&mut rt, home)["space"], space.as_str());
    // `raiseWindow`: the bare binary. With a window on the home it comes forward and the launch
    // exits 0 at once, without a window of its own.
    let out = bare_launch(&mut rt, &core, home);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert_eq!(status(&mut rt, home)["space"], space.as_str());

    // Without `focus`, `space.new` shows the space it opens.
    let args = format!(r#"{{"name":"repo@other","folder":{other:?}}}"#);
    let out = cli(&mut rt, home, &["do", "space.new", &args]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let shown = json_of(&out)["space"].as_str().unwrap().to_owned();
    assert_eq!(status(&mut rt, home)["space"], shown.as_str());

    // `repos clean`: `space.close` with `confirmed`, which ends the space's terminal.
    let terminal = terminals_of(
        local_spaces(&ls(&mut rt, home))
            .into_iter()
            .find(|s| s["id"] == space.as_str())
            .unwrap(),
    )[0]
    .to_owned();
    let args = format!(r#"{{"id":"{space}","confirmed":true}}"#);
    ok(&mut rt, home, &["do", "space.close", &args]);
    assert!(
        local_spaces(&ls(&mut rt, home))
            .iter()
            .all(|s| s["id"] != space.as_str())
    );
    until(&mut rt, "the closed space's terminal to end", |_| {
        !is_live(&conn, &terminal)
    });
    // A failed `do` answers `{error}` on stdout and exits non-zero, which `runCommand` reads.
    let args = format!(r#"{{"id":"{space}","confirmed":true}}"#);
    let out = cli(&mut rt, home, &["do", "space.close", &args]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(json_of(&out)["error"].is_string(), "{out:?}");

    // No window: `ls` exits non-zero with `no Herdr window` on stderr (`WINDOW_NOT_OPEN`).
    drop(rt);
    let out = cli_without_window(home, &["ls"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no Herdr window"),
        "{out:?}"
    );
}

/// `spawn-worker`: `ls` to see that the window shows the orchestrator's terminal and whether the
/// worker's space exists; `hc run` labeled with a fresh space and tab id; `ls` until the window
/// adopted it; then `do space.rename --id <space> --name <worker>`. A recovered worker gets a
/// new tab in its existing space the same way.
#[test]
fn spawn_worker_calls() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect");
    let orchestrator = run_sh(&conn, &core, PI);
    let folder = core.folder().display().to_string();
    let worktree = core.folder().join("repo-042-implement-widget");
    std::fs::create_dir_all(&worktree).unwrap();
    let worktree = worktree.display().to_string();
    set_layout(
        &conn,
        json!({"spaces": [
            {"id": "sp-1", "name": "repo@main", "folder": folder, "tabs": [
                tab("tb-1", "1", split("tb-1", "row", vec![pane("pn-a", &orchestrator)])),
            ]},
        ]}),
    );
    let scratch = Scratch::new("spawn");
    let home = scratch.0.as_path();
    let mut rt = open(&core, home);
    // `herdr_app_shows_terminal`: a local space's tabs hold `HC_TERMINAL_ID`.
    ls_until(&mut rt, home, "the orchestrator's terminal", |l| {
        local_spaces(l)
            .iter()
            .any(|s| terminals_of(s).contains(&orchestrator.as_str()))
            .then_some(())
    });
    let worker = "repo@042-implement-widget";
    // No space named after the worker yet.
    assert!(
        local_spaces(&ls(&mut rt, home))
            .iter()
            .all(|s| s["name"] != worker)
    );

    // A new worker: a fresh space and tab id the window doesn't know.
    let (space, tab_id) = (fresh("sp", 0x42), fresh("tb", 0x42));
    let terminal = hc_run(
        &core,
        &[
            "--bg",
            "--keep",
            "--no-parent",
            "--cwd",
            &worktree,
            "--label",
            &format!("herdr.space={space}"),
            "--label",
            &format!("herdr.tab={tab_id}"),
            "--",
            "/bin/sh",
            "-c",
            PI,
        ],
    );
    // The rename loop: `ls` until any space has the id, then rename it.
    ls_until(&mut rt, home, "the adopted worker space", |l| {
        l["spaces"]
            .as_array()?
            .iter()
            .any(|s| s["id"] == space.as_str())
            .then_some(())
    });
    let out = cli(
        &mut rt,
        home,
        &["do", "space.rename", "--id", &space, "--name", worker],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let listed = ls(&mut rt, home);
    let named: Vec<&Value> = local_spaces(&listed)
        .into_iter()
        .filter(|s| s["name"] == worker)
        .collect();
    assert_eq!(named.len(), 1, "{listed}");
    assert_eq!(named[0]["id"], space.as_str());
    assert_eq!(named[0]["tabs"][0]["id"], tab_id.as_str());
    assert_eq!(terminals_of(named[0]), vec![terminal.as_str()]);
    // Adopted without taking focus.
    assert_eq!(status(&mut rt, home)["space"], "sp-1");

    // A recovered worker: its space found by name, a new tab in it.
    let again = fresh("tb", 0x43);
    let recovered = hc_run(
        &core,
        &[
            "--bg",
            "--keep",
            "--no-parent",
            "--cwd",
            &worktree,
            "--label",
            &format!("herdr.space={space}"),
            "--label",
            &format!("herdr.tab={again}"),
            "--",
            "/bin/sh",
            "-c",
            PI,
        ],
    );
    let tabs = ls_until(&mut rt, home, "the recovered worker's tab", |l| {
        let s = l["spaces"]
            .as_array()?
            .iter()
            .find(|s| s["id"] == space.as_str())?;
        let tabs = s["tabs"].as_array()?;
        tabs.iter()
            .any(|t| t["id"] == again.as_str())
            .then(|| tabs.clone())
    });
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[1]["terminals"][0]["terminal"], recovered.as_str());
    assert_eq!(status(&mut rt, home)["space"], "sp-1");
}

/// The `agent-status` skill: `scripts/agent-status` reads `ls` (each local space's `id` and
/// `name`, each tab's `id` and `name`, each terminal's `terminal`, `pane`, `app` and `status`);
/// `ACTIONS.md` focuses with `do agent.focus --terminal`, closes with `do pane.close --pane`, and
/// resumes a session with `hc run` labeled with an existing space and a fresh tab, then
/// `do tab.rename --id <tab> --name <label>` once `ls` lists the tab.
#[test]
fn agent_status_calls() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect");
    let (pi, shell, claude) = (
        run_sh(&conn, &core, PI),
        run(&conn, &core),
        run_sh(&conn, &core, CLAUDE),
    );
    let folder = core.folder().display().to_string();
    set_layout(
        &conn,
        json!({"spaces": [
            {"id": "sp-1", "name": "work", "folder": folder, "tabs": [
                tab("tb-1", "Review", split("tb-1", "row", vec![pane("pn-a", &pi), pane("pn-b", &shell)])),
                tab("tb-2", "Fix", split("tb-2", "row", vec![pane("pn-c", &claude)])),
            ]},
        ]}),
    );
    let scratch = Scratch::new("agents");
    let home = scratch.0.as_path();
    let mut rt = open(&core, home);
    // `herdr_app_agents`: the terminals with an `app`, where they are.
    let agents = ls_until(&mut rt, home, "both agents' status", |l| {
        let mut agents = Vec::new();
        for space in local_spaces(l) {
            for (index, tab) in space["tabs"].as_array()?.iter().enumerate() {
                for term in tab["terminals"].as_array()? {
                    if term["app"].is_string() {
                        agents.push(json!({
                            "space_id": space["id"], "space": space["name"],
                            "tab_id": tab["id"], "tab": tab["name"], "index": index,
                            "terminal": term["terminal"], "pane": term["pane"],
                            "app": term["app"], "status": term["status"],
                        }));
                    }
                }
            }
        }
        (agents.len() == 2).then_some(agents)
    });
    assert_eq!(
        agents,
        vec![
            json!({"space_id": "sp-1", "space": "work", "tab_id": "tb-1", "tab": "Review",
                   "index": 0, "terminal": pi, "pane": "pn-a", "app": "pi", "status": "idle"}),
            json!({"space_id": "sp-1", "space": "work", "tab_id": "tb-2", "tab": "Fix",
                   "index": 1, "terminal": claude, "pane": "pn-c", "app": "claude",
                   "status": "working"}),
        ]
    );

    // Focus.
    ok(&mut rt, home, &["do", "agent.focus", "--terminal", &claude]);
    let shown = status(&mut rt, home);
    assert_eq!(
        (&shown["tab"], &shown["focused_pane"]),
        (&json!("tb-2"), &json!("pn-c"))
    );

    // Close.
    ok(&mut rt, home, &["do", "pane.close", "--pane", "pn-b"]);
    until(&mut rt, "the closed pane's terminal to end", |_| {
        !is_live(&conn, &shell)
    });

    // Resume into the existing space, in a fresh tab, then give it the old label.
    let resumed_tab = fresh("tb", 0x7e);
    let resumed = hc_run(
        &core,
        &[
            "--bg",
            "--keep",
            "--no-parent",
            "--cwd",
            &folder,
            "--label",
            "herdr.space=sp-1",
            "--label",
            &format!("herdr.tab={resumed_tab}"),
            "--",
            "/bin/sh",
            "-c",
            PI,
        ],
    );
    ls_until(&mut rt, home, "the resumed tab", |l| {
        local_spaces(l)
            .iter()
            .flat_map(|s| s["tabs"].as_array().into_iter().flatten())
            .any(|t| t["id"] == resumed_tab.as_str())
            .then_some(())
    });
    ok(
        &mut rt,
        home,
        &[
            "do",
            "tab.rename",
            "--id",
            &resumed_tab,
            "--name",
            "Old label",
        ],
    );
    let listed = ls(&mut rt, home);
    let resumed_in = local_spaces(&listed)[0]["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == resumed_tab.as_str())
        .unwrap()
        .clone();
    assert_eq!(resumed_in["name"], "Old label");
    assert_eq!(resumed_in["terminals"][0]["terminal"], resumed.as_str());
    // The resume didn't take focus.
    assert_eq!(status(&mut rt, home)["tab"], "tb-2");
}

/// Pi's `auto-session-name`: `ls`, the tab whose terminals hold `HC_TERMINAL_ID`, its first
/// terminal with `app == "pi"` as the lead, and `do tab.rename <JSON.stringify({id, name})>`.
#[test]
fn auto_session_name_calls() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect");
    let (lead, second) = (run_sh(&conn, &core, PI), run_sh(&conn, &core, PI));
    let folder = core.folder().display().to_string();
    set_layout(
        &conn,
        json!({"spaces": [
            {"id": "sp-1", "name": "work", "folder": folder, "tabs": [
                tab("tb-1", "", split("tb-1", "row", vec![pane("pn-a", &lead), pane("pn-b", &second)])),
            ]},
        ]}),
    );
    let scratch = Scratch::new("pi");
    let home = scratch.0.as_path();
    let mut rt = open(&core, home);
    // `nameHerdrTab` for the second Pi: its tab's lead is the first, so it writes nothing.
    let (id, name, lead_of) = ls_until(&mut rt, home, "both Pis' status", |l| {
        let tabs: Vec<&Value> = l["spaces"]
            .as_array()?
            .iter()
            .flat_map(|s| s["tabs"].as_array().into_iter().flatten())
            .collect();
        let tab = tabs.into_iter().find(|t| {
            t["terminals"]
                .as_array()
                .is_some_and(|ts| ts.iter().any(|x| x["terminal"] == second.as_str()))
        })?;
        let pis: Vec<&Value> = tab["terminals"]
            .as_array()?
            .iter()
            .filter(|x| x["app"] == "pi")
            .collect();
        (pis.len() == 2).then(|| {
            (
                tab["id"].as_str().unwrap().to_owned(),
                tab["name"].as_str().unwrap().to_owned(),
                pis[0]["terminal"].as_str().unwrap().to_owned(),
            )
        })
    });
    assert_eq!((id.as_str(), lead_of.as_str()), ("tb-1", lead.as_str()));
    // An unnamed tab is listed by its number.
    assert_eq!(name, "1");

    // The lead names the tab.
    let args = json!({"id": id, "name": "Fix login flow"}).to_string();
    ok(&mut rt, home, &["do", "tab.rename", &args]);
    layout_until(&mut rt, &conn, "the tab name in the core's layout", |doc| {
        space_in(doc, "sp-1").is_some_and(|s| s["tabs"][0]["name"] == "Fix login flow")
    });
    assert_eq!(
        ls(&mut rt, home)["spaces"][0]["tabs"][0]["name"],
        "Fix login flow"
    );
}

/// `dot stow` (`configure-herdr-app.sh`): `status` to learn whether a window is live (`no Herdr
/// window at ` on stderr when it isn't), then `do layout.switch --name <layout>`. Its
/// `navigator-status.set-shared:<json>` is a Herdr app mod's command, which herdr-frontend
/// doesn't serve.
#[test]
fn dot_stow_calls() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let scratch = Scratch::new("stow");
    let home = scratch.0.as_path();
    // Before the window opens.
    let out = cli_without_window(home, &["status"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no Herdr window at "),
        "{out:?}"
    );

    let mut rt = open(&core, home);
    assert_eq!(status(&mut rt, home)["layout"], "eli");
    let out = cli(&mut rt, home, &["do", "layout.switch", "--name", "eli"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    fails(
        &mut rt,
        home,
        &[
            "do",
            r#"navigator-status.set-shared:{"show_buttons":false}"#,
        ],
        "unknown command \"navigator-status.set-shared:{\\\"show_buttons\\\":false}\": see \
         `herdr-frontend commands` for the ones it serves",
    );
}
