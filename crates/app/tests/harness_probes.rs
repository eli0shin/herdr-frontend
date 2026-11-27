//! The end-to-end harness's requests (`herdr_frontend::e2e`, `docs/e2e.md`), against the whole
//! app as the window runs it ([`live_with`]), headless, on a real isolated herdr-core and an
//! isolated home: typed keys reach the focused terminal through the app's own key handling, the
//! window answers its number, and a window that hasn't drawn has no tree. A window the harness
//! didn't start refuses them all.
//!
//! The core is a `test-*` session with scratch dirs under `/tmp` (herdr-client's test support),
//! never the person's. Without an `hc` the test skips, except under CI.

#[path = "../../herdr-client/tests/support/mod.rs"]
mod support;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use herdr_client::{ConnectOptions, Connection};
use herdr_frontend::control::SOCKET;
use herdr_frontend::fixtures::{pane, split, tab};
use herdr_frontend::home::Home;
use herdr_frontend::link::Live;
use herdr_frontend::{SIZE, State, live_with};
use herdr_proto::methods::{DocGet, DocSet, TerminalRun};
use rune::{Env, Runtime};
use serde_json::{Value, json};
use support::IsolatedCore;

const WAIT: Duration = Duration::from_secs(30);

/// A scratch home under `/tmp` (socket paths must stay short), removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = PathBuf::from(format!("/tmp/hf-probe-{tag}-{}", std::process::id()));
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

/// Frames until `done` holds, as the window draws them while its wakers fire.
fn until(rt: &mut Runtime<State>, what: &str, mut done: impl FnMut(&mut Runtime<State>) -> bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        if done(rt) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// One request on the window's socket, the window drawing frames until the answer comes.
fn ask(rt: &mut Runtime<State>, home: &Path, request: Value) -> Value {
    let socket = home.join(SOCKET);
    let asking = std::thread::spawn(move || {
        let mut stream = UnixStream::connect(&socket).unwrap();
        stream.write_all(format!("{request}\n").as_bytes()).unwrap();
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        serde_json::from_str::<Value>(&line).unwrap()
    });
    until(rt, "the window's answer", |_| asking.is_finished());
    asking.join().unwrap()
}

fn text_of(rt: &Runtime<State>, address: &str) -> String {
    rt.state
        .viewers
        .get(address)
        .and_then(|v| v.pane.as_ref())
        .map(|p| p.snapshot.text())
        .unwrap_or_default()
}

#[test]
fn a_harness_window_takes_keys_and_reports_itself() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect");
    let address = conn
        .call(TerminalRun {
            argv: vec!["/bin/sh".into(), "-c".into(), "exec cat".into()],
            cwd: Some(core.folder().display().to_string()),
            keep: true,
            ..Default::default()
        })
        .wait()
        .expect("terminal.run")
        .terminal
        .address;
    let doc = json!({"spaces": [{"id": "sp-1", "name": "work", "tabs": [
        tab("tb-1", "1", split("tb-1", "row", vec![pane("pn-a", &address)])),
    ]}]});
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

    let scratch = Scratch::new("on");
    let home = scratch.0.as_path();
    let mut rt = live_with(Live::new(core.transport()), Some(Home::at(home)), true)
        .headless_runtime(SIZE, Env::new(true, 2.0));
    until(&mut rt, "the terminal attached", |rt| {
        rt.state.in_use() == Some(address.as_str())
    });

    // Typed text and a chord go through the window's key handling into the focused terminal,
    // which `cat` echoes back.
    assert_eq!(
        ask(&mut rt, home, json!({"type": "typed-by-harness\n"})),
        json!({"ok": true})
    );
    until(&mut rt, "the typed line on screen", |rt| {
        text_of(rt, &address).matches("typed-by-harness").count() >= 2
    });
    assert_eq!(
        ask(&mut rt, home, json!({"keys": "shift-o k enter"})),
        json!({"ok": true})
    );
    until(&mut rt, "the chords on screen", |rt| {
        text_of(rt, &address).matches("Ok").count() >= 2
    });
    assert_eq!(
        ask(&mut rt, home, json!({"keys": "hyper-x"})),
        json!({"error": "keys: can't read the key \"hyper-x\""})
    );

    // Headless, there is no native window and no drawn frame.
    let window = ask(&mut rt, home, json!({"window": true}));
    assert_eq!(window["window"], Value::Null);
    assert_eq!(window["pid"], json!(std::process::id()));
    assert_eq!(
        ask(&mut rt, home, json!({"tree": true})),
        json!({"error": "the window has drawn no frame yet"})
    );
}

#[test]
fn a_window_the_harness_didnt_start_refuses_its_requests() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let scratch = Scratch::new("off");
    let home = scratch.0.as_path();
    let mut rt = live_with(Live::new(core.transport()), Some(Home::at(home)), false)
        .headless_runtime(SIZE, Env::new(true, 2.0));
    for request in [
        json!({"keys": "ctrl-c"}),
        json!({"type": "rm -rf ~\n"}),
        json!({"tree": true}),
        json!({"window": true}),
    ] {
        assert_eq!(
            ask(&mut rt, home, request),
            json!({"error": "expected { do }, { ls }, { commands } or { status }"})
        );
    }
}
