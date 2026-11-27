//! The app against a real, isolated herdr-core (`docs/plan.md` "Testing rules"): it reads the
//! layout another client wrote, draws the shown tab's terminals live, lets only the focused one
//! take the size, types into it, and never writes the layout.
//!
//! The core is a `test-*` session with scratch dirs under `/tmp` (herdr-client's test support),
//! never the person's. Without an `hc` the test skips, except under CI.

#[path = "../../herdr-client/tests/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use herdr_client::{ConnectOptions, Connection};
use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::{pane, split, tab};
use herdr_frontend::link::Live;
use herdr_frontend::{SIZE, State, app};
use herdr_proto::methods::{DocGet, DocSet, InitialSize, TerminalGet, TerminalRun};
use rune::{Env, Key, KeyEvent, Mods, Runtime};
use serde_json::json;
use support::IsolatedCore;

const WAIT: Duration = Duration::from_secs(30);

/// Runs `script` under `sh`, at `size` if given.
fn run(conn: &Connection, core: &IsolatedCore, script: &str, size: Option<(u16, u16)>) -> String {
    conn.call(TerminalRun {
        argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
        cwd: Some(core.folder().display().to_string()),
        keep: true,
        size: size.map(|(cols, rows)| InitialSize { cols, rows }),
        ..Default::default()
    })
    .wait()
    .expect("terminal.run")
    .terminal
    .address
}

fn record(conn: &Connection, address: &str) -> herdr_proto::Terminal {
    conn.call(TerminalGet {
        terminal: address.into(),
        ..Default::default()
    })
    .wait()
    .expect("terminal.get")
}

/// Frames until `done` holds, the way a window would draw them as its waker fires.
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

#[test]
fn the_app_draws_and_types_into_live_core_terminals_and_leaves_the_layout_alone() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect");
    // `cat` echoes what's typed through the terminal, then prints it again.
    let a = run(&conn, &core, "printf 'hello-from-core\\n'; exec cat", None);
    let b = run(
        &conn,
        &core,
        "printf 'second pane\\n'; exec cat",
        Some((100, 30)),
    );

    // Another client (Herdr app, say) wrote the layout: one tab, `a | b`.
    let doc = json!({"spaces": [{
        "id": "sp-1", "name": "isolated", "folder": core.folder().display().to_string(),
        "tabs": [tab("tb-1", "work", split("tb-1", "row", vec![pane("pn-a", &a), pane("pn-b", &b)]))],
    }]});
    let read = conn
        .call(DocGet {
            name: "herdr-app".into(),
            ..Default::default()
        })
        .wait()
        .expect("doc.get");
    let written = conn
        .call(DocSet {
            name: "herdr-app".into(),
            value: doc,
            version: read.version,
            ..Default::default()
        })
        .wait()
        .expect("doc.set")
        .version;

    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.with_settings(Settings::fixture());
    rt.state.connect(Box::new(Live::new(core.transport())));

    until(&mut rt, "both panes to draw their terminals", |rt| {
        text_of(rt, &a).contains("hello-from-core") && text_of(rt, &b).contains("second pane")
    });

    // The focused pane takes the size: the core's grid becomes the pane's.
    let grid = |rt: &Runtime<State>, pane: &str| {
        let (_, rect, _) = rt
            .state
            .shown_panes()
            .into_iter()
            .find(|(p, ..)| p == pane)
            .expect("a shown pane");
        let g = rt.state.grid_for(rect);
        (g.columns, g.rows)
    };
    let want = grid(&rt, "pn-a");
    until(&mut rt, "the focused pane to take the size", |rt| {
        let r = record(&conn, &a);
        rt.state.in_use() == Some(a.as_str()) && (r.cols, r.rows) == want
    });
    // And the app draws it at the core's grid.
    until(&mut rt, "the local terminal to follow the core", |rt| {
        rt.state.viewers[&a]
            .pane
            .as_ref()
            .is_some_and(|p| (p.snapshot.cols, p.snapshot.rows) == want)
    });

    // The unfocused pane attached `resumed` and took nothing.
    let r = record(&conn, &b);
    assert_eq!((r.cols, r.rows), (100, 30));

    // Plain typing goes to the focused terminal.
    for key in [
        Key::Text("t".into()),
        Key::Text("y".into()),
        Key::Text("p".into()),
        Key::Text("e".into()),
        Key::Text("d".into()),
        Key::Enter,
    ] {
        rt.key(KeyEvent {
            key,
            mods: Mods::default(),
        });
        rt.frame(1.0 / 60.0);
    }
    until(&mut rt, "the typed line and cat's copy of it", |rt| {
        text_of(rt, &a).matches("typed").count() >= 2
    });
    assert!(!text_of(&rt, &b).contains("typed"));

    // The layout was only read.
    let now = conn
        .call(DocGet {
            name: "herdr-app".into(),
            ..Default::default()
        })
        .wait()
        .expect("doc.get");
    assert_eq!(now.version, written, "the app wrote the layout");

    drop(rt);
}

/// A program that floods its terminal, with the app's parser held up: the core stops at the
/// credit window, so what waits unparsed never outgrows it (rule 2). Once the parser runs again,
/// the output flows on.
#[test]
fn a_parser_behind_the_output_holds_the_core_to_the_credit_window() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect");
    let flood = run(&conn, &core, "exec yes flood-line", None);
    let doc = json!({"spaces": [{
        "id": "sp-1", "name": "isolated", "folder": core.folder().display().to_string(),
        "tabs": [tab("tb-1", "flood", split("tb-1", "row", vec![pane("pn-1", &flood)]))],
    }]});
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

    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.with_settings(Settings::fixture());
    rt.state.connect(Box::new(Live::new(core.transport())));
    until(&mut rt, "the flood to show", |rt| {
        text_of(rt, &flood).contains("flood-line")
    });

    // Hold the terminal's worker inside a parsed callback: it parses nothing more.
    let (release, held) = std::sync::mpsc::channel::<()>();
    let (entered, inside) = std::sync::mpsc::channel::<()>();
    rt.state.viewers[&flood].terminal.writer().write_then(
        Vec::new(),
        Box::new(move || {
            let _ = entered.send(());
            let _ = held.recv();
        }),
    );
    // The stall waits behind whatever output was queued already.
    inside
        .recv_timeout(WAIT)
        .expect("the worker took the stall");

    // The program goes on writing; the core may send only what the credit window allows.
    let window = herdr_proto::frame::WINDOW;
    let mut most = 0;
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        most = most.max(rt.state.viewers[&flood].terminal.pending_bytes());
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(most > 0, "the flood stopped on its own");
    assert!(
        most <= window,
        "{most} bytes wait unparsed, past the {window}-byte credit window"
    );

    // Running again, the worker parses the backlog, credits it, and the core sends more.
    let generation = |rt: &Runtime<State>| {
        rt.state.viewers[&flood]
            .pane
            .as_ref()
            .map_or(0, |p| p.snapshot.generation)
    };
    drop(release);
    until(&mut rt, "the backlog to drain", |rt| {
        rt.state.viewers[&flood].terminal.pending_bytes() < window
    });
    let before = generation(&rt);
    until(&mut rt, "the flood to go on", |rt| {
        generation(rt) > before + 2
    });
    drop(rt);
}
