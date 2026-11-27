//! The whole app serving `herdr-frontend ls | do | status` (`docs/cli.md`): the app as the window
//! runs it ([`live`], its home and core link), on a real, isolated herdr-core and an isolated
//! home, driven by the real binary as the CLI. Every command the doc lists goes through the
//! window's frame loop: the socket's request into the model, the effects out to the core, and
//! `space.new`'s answer once its terminal runs.
//!
//! The core is a `test-*` session with scratch dirs under `/tmp` (herdr-client's test support),
//! never the person's. Without an `hc` the test skips, except under CI.

#[path = "../../herdr-client/tests/support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use herdr_client::{ConnectOptions, Connection};
use herdr_frontend::control::{HOME_VAR, SOCKET};
use herdr_frontend::fixtures::{pane, split, tab};
use herdr_frontend::home::Home;
use herdr_frontend::link::Live;
use herdr_frontend::{SIZE, State, live};
use herdr_proto::methods::{DocGet, DocSet, TerminalGet, TerminalRun};
use rune::{Env, Runtime};
use serde_json::{Value, json};
use support::IsolatedCore;

const WAIT: Duration = Duration::from_secs(30);

/// A scratch herdr-frontend home under `/tmp` (socket paths must stay short), removed when
/// dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Scratch {
        let dir = PathBuf::from(format!("/tmp/hf-win-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Starts `cat` under `sh` on the core: a terminal that stays up until it's ended.
fn run(conn: &Connection, core: &IsolatedCore) -> String {
    conn.call(TerminalRun {
        argv: vec!["/bin/sh".into(), "-c".into(), "exec cat".into()],
        cwd: Some(core.folder().display().to_string()),
        keep: true,
        ..Default::default()
    })
    .wait()
    .expect("terminal.run")
    .terminal
    .address
}

/// Whether the core still runs `address`.
fn is_live(conn: &Connection, address: &str) -> bool {
    conn.call(TerminalGet {
        terminal: address.into(),
        ..Default::default()
    })
    .wait()
    .is_ok_and(|t| t.is_live())
}

/// The layout document on the core.
fn layout(conn: &Connection) -> Value {
    conn.call(DocGet {
        name: "herdr-app".into(),
        ..Default::default()
    })
    .wait()
    .expect("doc.get")
    .value
}

/// Frames until the core's layout satisfies `done`. A `do` answers once the model made its
/// edit; the link writes it to the core just after, on its own connection.
fn layout_until(
    rt: &mut Runtime<State>,
    conn: &Connection,
    what: &str,
    done: impl Fn(&Value) -> bool,
) {
    until(rt, what, |_| done(&layout(conn)));
}

fn space_in<'a>(doc: &'a Value, id: &str) -> Option<&'a Value> {
    doc["spaces"].as_array()?.iter().find(|s| s["id"] == id)
}

/// Frames, as the window draws them while its wakers fire, until `done` holds.
fn until(rt: &mut Runtime<State>, what: &str, mut done: impl FnMut(&mut Runtime<State>) -> bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        rt.frame(1.0 / 60.0);
        if done(rt) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Runs `herdr-frontend <args>` against `home`, the window drawing frames until it exits.
fn cli(rt: &mut Runtime<State>, home: &Path, args: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_herdr-frontend"))
        .env(HOME_VAR, home)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + WAIT;
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("herdr-frontend {args:?} hangs");
        }
        rt.frame(1.0 / 60.0);
        std::thread::sleep(Duration::from_millis(2));
    }
    child.wait_with_output().unwrap()
}

fn json_of(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| panic!("{e}: {output:?}"))
}

/// `herdr-frontend <args>`, which must exit 0 and print `{"ok": true}`.
fn ok(rt: &mut Runtime<State>, home: &Path, args: &[&str]) {
    let out = cli(rt, home, args);
    assert_eq!(out.status.code(), Some(0), "{args:?}: {out:?}");
    assert_eq!(json_of(&out), json!({"ok": true}), "{args:?}");
}

/// `herdr-frontend <args>`, which must exit 1 and print `{"error": error}`.
fn fails(rt: &mut Runtime<State>, home: &Path, args: &[&str], error: &str) {
    let out = cli(rt, home, args);
    assert_eq!(out.status.code(), Some(1), "{args:?}: {out:?}");
    assert_eq!(json_of(&out), json!({ "error": error }), "{args:?}");
}

fn status(rt: &mut Runtime<State>, home: &Path) -> Value {
    let out = cli(rt, home, &["status"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    json_of(&out)
}

#[test]
fn the_window_serves_every_cli_command_on_a_live_core() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect");
    let (a, b, c, d) = (
        run(&conn, &core),
        run(&conn, &core),
        run(&conn, &core),
        run(&conn, &core),
    );
    // Another client (Herdr app, say) wrote the layout: `work` with two tabs, and `api`.
    let folder = core.folder().display().to_string();
    let doc = json!({"spaces": [
        {"id": "sp-1", "name": "work", "folder": folder, "tabs": [
            tab("tb-1", "1", split("tb-1", "row", vec![pane("pn-a", &a), pane("pn-b", &b)])),
            tab("tb-2", "2", split("tb-2", "row", vec![pane("pn-c", &c)])),
        ]},
        {"id": "sp-2", "name": "api", "folder": folder, "tabs": [
            tab("tb-3", "1", split("tb-3", "row", vec![pane("pn-d", &d)])),
        ]},
    ]});
    let read = conn
        .call(DocGet {
            name: "herdr-app".into(),
            ..Default::default()
        })
        .wait()
        .expect("doc.get");
    conn.call(DocSet {
        name: "herdr-app".into(),
        value: doc,
        version: read.version,
        ..Default::default()
    })
    .wait()
    .expect("doc.set");

    let scratch = Scratch::new();
    let home = scratch.0.as_path();
    let socket = home.join(SOCKET);
    // The bundled font, so the settings have nothing to report whatever font folders the
    // machine has; the zoom and size stay Eli's defaults.
    std::fs::write(
        home.join("settings.json"),
        r#"{"font": "JetBrains Mono", "ui_font": "JetBrains Mono"}"#,
    )
    .expect("settings.json");
    let mut rt = live(Live::new(core.transport()), Some(Home::at(home)))
        .headless_runtime(SIZE, Env::new(true, 2.0));
    assert_eq!(rt.state.control_socket(), Some(socket.as_path()));
    until(&mut rt, "the layout and its terminals", |rt| {
        let world = rt.state.model.world();
        [&a, &b, &c, &d].iter().all(|t| world.is_live(t))
            && rt.state.model.ls()["spaces"][1]["id"] == "sp-2"
    });

    // `ls`: the window's listing.
    let out = cli(&mut rt, home, &["ls"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let listed = json_of(&out);
    assert_eq!(listed, rt.state.model.ls());
    let spaces = listed["spaces"].as_array().unwrap();
    assert_eq!(spaces.len(), 2);
    assert_eq!(spaces[0]["name"], "work");
    assert_eq!(spaces[0]["tabs"][0]["terminals"][1]["terminal"], b.as_str());

    // `status`: the model's view and the window's own facts.
    let shown = status(&mut rt, home);
    assert_eq!(shown["space"], "sp-1");
    assert_eq!(shown["tab"], "tb-1");
    assert_eq!(shown["focused_pane"], "pn-a");
    assert_eq!(shown["layout"], "eli");
    assert_eq!(shown["errors"], json!([]));
    assert_eq!(
        (&shown["zoom"], &shown["font_size"], &shown["ui_font_size"]),
        (&json!(1.1), &json!(17.0), &json!(13.0))
    );

    // `space.new {focus: false}` answers the new ids once its terminal runs, and keeps the
    // shown space.
    let out = cli(
        &mut rt,
        home,
        &[
            "do",
            "space.new",
            "--name",
            "repo@branch",
            "--folder",
            &folder,
            "--focus",
            "false",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let made = json_of(&out);
    let (space, made_tab, terminal) = (
        made["space"].as_str().expect("the new space").to_owned(),
        made["tab"].as_str().expect("the new tab").to_owned(),
        made["terminal"]
            .as_str()
            .expect("the new terminal")
            .to_owned(),
    );
    assert!(is_live(&conn, &terminal), "space.new's terminal runs");
    layout_until(
        &mut rt,
        &conn,
        "the new space in the core's layout",
        |doc| space_in(doc, &space).is_some_and(|s| s["name"] == "repo@branch"),
    );
    let listed = json_of(&cli(&mut rt, home, &["ls"]));
    let new = &listed["spaces"][2];
    assert_eq!(new["id"], space.as_str());
    assert_eq!(new["tabs"][0]["id"], made_tab.as_str());
    assert_eq!(
        new["tabs"][0]["terminals"][0]["terminal"],
        terminal.as_str()
    );
    assert_eq!(status(&mut rt, home)["space"], "sp-1", "focus: false");

    // Renames write the shared layout.
    ok(
        &mut rt,
        home,
        &["do", "space.rename", "--id", &space, "--name", " renamed "],
    );
    ok(
        &mut rt,
        home,
        &["do", "tab.rename", r#"{"id": "tb-2", "name": "Fix login"}"#],
    );
    layout_until(&mut rt, &conn, "the renames in the core's layout", |doc| {
        space_in(doc, &space).is_some_and(|s| s["name"] == "renamed")
            && space_in(doc, "sp-1").is_some_and(|s| s["tabs"][1]["name"] == "Fix login")
    });

    // Selecting and focusing move the view.
    ok(&mut rt, home, &["do", "space.select", "--index", "2"]);
    assert_eq!(status(&mut rt, home)["space"], "sp-2");
    ok(&mut rt, home, &["do", "agent.focus", "--terminal", &b]);
    let shown = status(&mut rt, home);
    assert_eq!(
        (&shown["space"], &shown["focused_pane"]),
        (&json!("sp-1"), &json!("pn-b"))
    );
    fails(
        &mut rt,
        home,
        &["do", "agent.focus", "--terminal", "nope"],
        "no running terminal \"nope\"",
    );

    // `pane.close` ends the pane's terminal and takes it out of the layout.
    ok(&mut rt, home, &["do", "pane.close", "--pane", "pn-b"]);
    layout_until(&mut rt, &conn, "pn-b to leave the core's layout", |doc| {
        !doc.to_string().contains("pn-b")
    });
    until(&mut rt, "pn-b's terminal to end", |_| !is_live(&conn, &b));

    // `space.close` needs `confirmed`, then ends the space's terminals.
    fails(
        &mut rt,
        home,
        &["do", "space.close", "--id", &space],
        "space.close ends its terminals: pass confirmed: true",
    );
    ok(
        &mut rt,
        home,
        &["do", "space.close", "--id", &space, "--confirmed", "true"],
    );
    layout_until(
        &mut rt,
        &conn,
        "the space to leave the core's layout",
        |doc| space_in(doc, &space).is_none(),
    );
    until(&mut rt, "the closed space's terminal to end", |_| {
        !is_live(&conn, &terminal)
    });

    // `layout.switch` knows only `eli`; unknown commands say where the list is.
    ok(&mut rt, home, &["do", "layout.switch", "--name", "eli"]);
    fails(
        &mut rt,
        home,
        &["do", "layout.switch", "--name", "wide"],
        "no layout \"wide\": herdr-frontend has only the \"eli\" layout",
    );
    fails(
        &mut rt,
        home,
        &["do", "tab.new"],
        "unknown command \"tab.new\": see `herdr-frontend commands` for the ones it serves",
    );
    assert!(is_live(&conn, &a) && is_live(&conn, &c) && is_live(&conn, &d));

    // The socket goes with the window.
    drop(rt);
    assert!(!socket.exists(), "the window left its socket");
    let out = cli_without_window(home, &["ls"]);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).starts_with(&format!(
            "herdr-frontend: no Herdr window at {}",
            socket.display()
        )),
        "{out:?}"
    );
}

fn cli_without_window(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_herdr-frontend"))
        .env(HOME_VAR, home)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}
