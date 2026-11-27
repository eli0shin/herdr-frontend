//! The whole app as the window runs it ([`live`], its home and core link), headless, on an
//! isolated herdr-core and an isolated home, and the real `herdr-frontend` binary as the CLI that
//! drives it (`docs/cli.md`). Shared by `window_control.rs` and `tool_calls.rs`; each declares
//! herdr-client's `support` module, which this one uses for the core.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use herdr_client::Connection;
use herdr_frontend::control::{HOME_VAR, SOCKET};
use herdr_frontend::home::Home;
use herdr_frontend::link::Live;
use herdr_frontend::{SIZE, State, live};
use herdr_proto::methods::{DocGet, DocSet, TerminalGet, TerminalRun};
use rune::{Env, Runtime};
use serde_json::{Value, json};

use crate::support::IsolatedCore;

pub const WAIT: Duration = Duration::from_secs(30);

/// A scratch herdr-frontend home under `/tmp` (socket paths must stay short), removed when
/// dropped. `tag` keeps the tests of one binary, which share a process id, apart.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(tag: &str) -> Scratch {
        let dir = PathBuf::from(format!("/tmp/hf-{tag}-{}", std::process::id()));
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

/// The window on `core`, serving `home`'s socket. The bundled font, so the settings have
/// nothing to report whatever fonts the machine has installed; the zoom and size stay Eli's
/// defaults.
pub fn open(core: &IsolatedCore, home: &Path) -> Runtime<State> {
    std::fs::write(
        home.join("settings.json"),
        r#"{"font": "JetBrains Mono", "ui_font": "JetBrains Mono"}"#,
    )
    .expect("settings.json");
    let rt = live(Live::new(core.transport()), Some(Home::at(home)))
        .headless_runtime(SIZE, Env::new(true, 2.0));
    assert_eq!(rt.state.control_socket(), Some(home.join(SOCKET).as_path()));
    rt
}

/// Starts `cat` under `sh` on the core: a terminal that stays up until it's ended.
pub fn run(conn: &Connection, core: &IsolatedCore) -> String {
    run_sh(conn, core, "exec cat")
}

/// Starts `sh -c <script>` on the core, in its scratch folder, kept.
pub fn run_sh(conn: &Connection, core: &IsolatedCore, script: &str) -> String {
    conn.call(TerminalRun {
        argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
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
pub fn is_live(conn: &Connection, address: &str) -> bool {
    conn.call(TerminalGet {
        terminal: address.into(),
        ..Default::default()
    })
    .wait()
    .is_ok_and(|t| t.is_live())
}

/// The layout document on the core.
pub fn layout(conn: &Connection) -> Value {
    conn.call(DocGet {
        name: "herdr-app".into(),
        ..Default::default()
    })
    .wait()
    .expect("doc.get")
    .value
}

/// Writes the layout document as another client (Herdr app, say) would.
pub fn set_layout(conn: &Connection, doc: Value) {
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
}

/// Frames until the core's layout satisfies `done`. A `do` answers once the model made its
/// edit; the link writes it to the core just after, on its own connection.
pub fn layout_until(
    rt: &mut Runtime<State>,
    conn: &Connection,
    what: &str,
    done: impl Fn(&Value) -> bool,
) {
    until(rt, what, |_| done(&layout(conn)));
}

pub fn space_in<'a>(doc: &'a Value, id: &str) -> Option<&'a Value> {
    doc["spaces"].as_array()?.iter().find(|s| s["id"] == id)
}

/// Frames, as the window draws them while its wakers fire, until `done` holds.
pub fn until(
    rt: &mut Runtime<State>,
    what: &str,
    mut done: impl FnMut(&mut Runtime<State>) -> bool,
) {
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
pub fn cli(rt: &mut Runtime<State>, home: &Path, args: &[&str]) -> Output {
    cli_with(rt, home, args, |_| {})
}

/// Runs `herdr-frontend` with no arguments against `home` and `core`'s environment, the window
/// drawing frames until it exits. Should the launch open a window instead of raising this one,
/// that window is on the isolated core, and it's killed when the wait times out.
pub fn bare_launch(rt: &mut Runtime<State>, core: &IsolatedCore, home: &Path) -> Output {
    cli_with(rt, home, &[], |command| {
        core.isolate(command);
    })
}

fn cli_with(
    rt: &mut Runtime<State>,
    home: &Path,
    args: &[&str],
    setup: impl FnOnce(&mut Command),
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_herdr-frontend"));
    setup(&mut command);
    let mut child = command
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

/// Runs `herdr-frontend <args>` against `home` with no window drawing.
pub fn cli_without_window(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_herdr-frontend"))
        .env(HOME_VAR, home)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

pub fn json_of(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| panic!("{e}: {output:?}"))
}

/// `herdr-frontend <args>`, which must exit 0 and print `{"ok": true}`.
pub fn ok(rt: &mut Runtime<State>, home: &Path, args: &[&str]) {
    let out = cli(rt, home, args);
    assert_eq!(out.status.code(), Some(0), "{args:?}: {out:?}");
    assert_eq!(json_of(&out), json!({"ok": true}), "{args:?}");
}

/// `herdr-frontend <args>`, which must exit 1 and print `{"error": error}`.
pub fn fails(rt: &mut Runtime<State>, home: &Path, args: &[&str], error: &str) {
    let out = cli(rt, home, args);
    assert_eq!(out.status.code(), Some(1), "{args:?}: {out:?}");
    assert_eq!(json_of(&out), json!({ "error": error }), "{args:?}");
}

/// `herdr-frontend ls`, which must exit 0.
pub fn ls(rt: &mut Runtime<State>, home: &Path) -> Value {
    let out = cli(rt, home, &["ls"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    json_of(&out)
}

pub fn status(rt: &mut Runtime<State>, home: &Path) -> Value {
    let out = cli(rt, home, &["status"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    json_of(&out)
}
