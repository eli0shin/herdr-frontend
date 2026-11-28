//! The app and Herdr app on one isolated herdr-core, showing the same terminals
//! (`herdr-core-protocol.md` §4, `docs/interop.md`). Herdr app is played by a second framed
//! connection that does what herdr-app does: it attaches every pane as an active viewer, types,
//! refits, and reattaches without `resumed` after the core restarts. The app takes the size for
//! every pane it shows while its window is in front, takes it back only on real use or when the
//! pane's grid changes, draws every terminal at the core's grid, and answers the program's
//! queries through AUTOMATIC_INPUT, which takes nothing.
//!
//! The core is a `test-*` session with scratch dirs under `/tmp` (herdr-client's test support),
//! never the person's. Without an `hc` the test skips, except under CI.

#[path = "../../herdr-client/tests/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use herdr_client::{AttachStream, ConnectOptions, Connection};
use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::{pane, split, tab};
use herdr_frontend::link::Live;
use herdr_frontend::state::{Attach, Status};
use herdr_frontend::{SIZE, State, app};
use herdr_proto::methods::{DocGet, DocSet, InitialSize, TerminalAttach, TerminalGet, TerminalRun};
use rune::{Env, Key, KeyEvent, Mods, Runtime};
use serde_json::json;
use support::{IsolatedCore, eventually};

const WAIT: Duration = Duration::from_secs(30);

/// How long the app gets to take a size it must leave alone.
const QUIET: Duration = Duration::from_millis(500);

/// Herdr app's window: its panes' grid.
const HERDR: (u16, u16) = (100, 30);

/// The grid `b` starts at, before Herdr app shows it.
const START: (u16, u16) = (90, 20);

fn connect(core: &IsolatedCore) -> Connection {
    Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect")
}

/// Runs `script` under `sh` at `size`, kept, so it outlives a core restart.
fn run(conn: &Connection, core: &IsolatedCore, script: &str, (cols, rows): (u16, u16)) -> String {
    conn.call(TerminalRun {
        argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
        cwd: Some(core.folder().display().to_string()),
        keep: true,
        size: Some(InitialSize { cols, rows }),
        ..Default::default()
    })
    .wait()
    .expect("terminal.run")
    .terminal
    .address
}

/// Herdr app attaching a pane's terminal: an active viewer at its pane's grid.
fn herdr_attach(conn: &Connection, terminal: &str, (cols, rows): (u16, u16)) -> AttachStream {
    conn.attach(TerminalAttach {
        terminal: terminal.into(),
        cols,
        rows,
        history: 2000,
        ..Default::default()
    })
    .wait()
    .expect("terminal.attach")
}

/// The terminal's grid as the core holds it.
fn size(conn: &Connection, terminal: &str) -> (u16, u16) {
    let t = conn
        .call(TerminalGet {
            terminal: terminal.into(),
            ..Default::default()
        })
        .wait()
        .expect("terminal.get");
    (t.cols, t.rows)
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

/// Frames for `QUIET`: what the app does on its own meanwhile.
fn idle(rt: &mut Runtime<State>) {
    let deadline = Instant::now() + QUIET;
    while Instant::now() < deadline {
        rt.invalidate();
        rt.frame(1.0 / 60.0);
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

/// The grid the app draws a terminal at.
fn drawn(rt: &Runtime<State>, address: &str) -> Option<(u16, u16)> {
    let pane = rt.state.viewers.get(address)?.pane.as_ref()?;
    Some((pane.snapshot.cols, pane.snapshot.rows))
}

/// The grid pane `pane` holds in the app's window.
fn grid(rt: &Runtime<State>, pane: &str) -> (u16, u16) {
    let (_, rect, _) = rt
        .state
        .shown_panes()
        .into_iter()
        .find(|(p, ..)| p == pane)
        .expect("a shown pane");
    let g = rt.state.grid_for(rect);
    (g.columns, g.rows)
}

fn type_line(rt: &mut Runtime<State>, text: &str) {
    for c in text.chars() {
        rt.key(KeyEvent {
            key: Key::Text(c.to_string()),
            mods: Mods::default(),
        });
        rt.frame(1.0 / 60.0);
    }
    rt.key(KeyEvent {
        key: Key::Enter,
        mods: Mods::default(),
    });
    rt.frame(1.0 / 60.0);
}

#[test]
fn the_app_and_herdr_app_share_terminals_without_fighting_over_the_size() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let herdr = connect(&core);
    // `a`, the pane the app focuses, echoes what's typed. `b` waits for a go, then turns on
    // in-band size reports (mode 2048) and prints the first report a viewer sends it, read a
    // byte at a time up to its final `t` (POSIX `sh`, which has no `read -d`).
    let a = run(&herdr, &core, "printf 'cat-ready\\n'; exec cat", START);
    let b = run(
        &herdr,
        &core,
        "stty -echo; printf 'query-ready\\n'; read go; stty -icanon min 1; \
         printf '\\033[?2048h'; reply=; \
         while c=$(dd bs=1 count=1 2>/dev/null) && [ \"$c\" != t ]; do reply=\"$reply$c\"; done; \
         printf 'reply=%s|\\n' \"${reply#?}\"; exec cat",
        START,
    );

    // Herdr app shows `a` already, as an active viewer at its own grid.
    let herdr_a = herdr_attach(&herdr, &a, HERDR);
    eventually(WAIT, "Herdr app's grid", || {
        (size(&herdr, &a) == HERDR).then_some(())
    });

    // Its layout: one tab, `a | b`.
    let doc = json!({"spaces": [{
        "id": "sp-1", "name": "isolated", "folder": core.folder().display().to_string(),
        "tabs": [tab("tb-1", "work", split("tb-1", "row", vec![pane("pn-a", &a), pane("pn-b", &b)]))],
    }]});
    let read = herdr
        .call(DocGet {
            name: "herdr-app".into(),
            ..Default::default()
        })
        .wait()
        .expect("doc.get");
    herdr
        .call(DocSet {
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
    until(&mut rt, "both panes to draw", |rt| {
        text_of(rt, &a).contains("cat-ready") && text_of(rt, &b).contains("query-ready")
    });
    let (mine, mine_b) = (grid(&rt, "pn-a"), grid(&rt, "pn-b"));
    assert!(
        ![HERDR, START].contains(&mine) && ![HERDR, START].contains(&mine_b),
        "the test needs the grids to differ"
    );

    // Every shown pane takes its size, the one not in use too.
    until(&mut rt, "the shown panes to take their sizes", |rt| {
        rt.state.in_use() == Some(a.as_str())
            && size(&herdr, &a) == mine
            && drawn(rt, &a) == Some(mine)
            && size(&herdr, &b) == mine_b
            && drawn(rt, &b) == Some(mine_b)
    });

    // Herdr app shows `b` now: its grid. The app draws `b` at it, from the core's `size`
    // event, and doesn't take it back while its pane's grid stays the same.
    let herdr_b = herdr_attach(&herdr, &b, HERDR);
    until(&mut rt, "the app to follow Herdr app's grid", |rt| {
        drawn(rt, &b) == Some(HERDR)
    });
    idle(&mut rt);
    assert_eq!(
        size(&herdr, &b),
        HERDR,
        "the unfocused pane took the size back"
    );

    // The program asks for size reports: the app answers from its terminal, at the core's grid,
    // as AUTOMATIC_INPUT, so the size stays Herdr app's.
    herdr_b.input(b"go\r");
    until(&mut rt, "the app's mode-2048 report", |rt| {
        text_of(rt, &b).contains("reply=")
    });
    let report = format!("reply=[48;{};{};", HERDR.1, HERDR.0);
    assert!(text_of(&rt, &b).contains(&report), "{}", text_of(&rt, &b));
    idle(&mut rt);
    assert_eq!(size(&herdr, &b), HERDR, "the app's reply took the size");

    // The panes' grids change (the sidebar hides, then shows again): each takes its size
    // again, `b` too.
    for _ in 0..2 {
        rt.key(KeyEvent {
            key: Key::Text("b".into()),
            mods: Mods {
                cmd: true,
                ..Mods::default()
            },
        });
        rt.frame(1.0 / 60.0);
    }
    until(&mut rt, "a grid change to take the size", |rt| {
        size(&herdr, &b) == mine_b && drawn(rt, &b) == Some(mine_b)
    });

    // Herdr app types into `a`: the size is its again. The app follows and doesn't take it
    // back on its own.
    herdr_a.input(b"from-herdr\r");
    until(&mut rt, "the app to draw at Herdr app's grid", |rt| {
        drawn(rt, &a) == Some(HERDR) && text_of(rt, &a).contains("from-herdr")
    });
    idle(&mut rt);
    assert_eq!(size(&herdr, &a), HERDR, "the app took the size back unused");

    // A click is use: it takes the size back.
    let (x, y) = rt.find("pane-pn-a").expect("pane a");
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    until(&mut rt, "the click to take the size", |rt| {
        size(&herdr, &a) == mine && drawn(rt, &a) == Some(mine)
    });

    // So is typing, after Herdr app refit it.
    herdr_a.resize(HERDR.0, HERDR.1);
    until(&mut rt, "Herdr app's refit", |rt| {
        drawn(rt, &a) == Some(HERDR)
    });
    type_line(&mut rt, "typed");
    until(&mut rt, "typing to take the size", |rt| {
        size(&herdr, &a) == mine && text_of(rt, &a).matches("typed").count() >= 2
    });

    // In the background, the app takes nothing; back in front, the shown panes take it.
    rt.window_focus(false);
    idle(&mut rt);
    herdr_a.resize(HERDR.0, HERDR.1);
    herdr_b.resize(HERDR.0, HERDR.1);
    until(&mut rt, "Herdr app's refit", |rt| {
        drawn(rt, &a) == Some(HERDR) && drawn(rt, &b) == Some(HERDR)
    });
    idle(&mut rt);
    for t in [&a, &b] {
        assert_eq!(
            size(&herdr, t),
            HERDR,
            "a window in the background took the size"
        );
    }
    rt.window_focus(true);
    until(&mut rt, "the window in front to take the sizes", |_| {
        size(&herdr, &a) == mine && size(&herdr, &b) == mine_b
    });

    // The core restarts, with the app in the background. Herdr app reattaches first, without
    // `resumed`, at a new grid; the app's reattach after it, `resumed`, leaves that alone.
    rt.window_focus(false);
    idle(&mut rt);
    drop((herdr_a, herdr_b));
    core.restart();
    let herdr = eventually(WAIT, "Herdr app to reconnect", || {
        Connection::open(&core.transport(), &ConnectOptions::default()).ok()
    });
    let moved = (HERDR.0 + 10, HERDR.1 + 2);
    let _again = [&a, &b].map(|t| herdr_attach(&herdr, t, moved));
    for t in [&a, &b] {
        eventually(WAIT, "Herdr app's reattach", || {
            (size(&herdr, t) == moved).then_some(())
        });
    }
    let viewers_attached = |rt: &Runtime<State>| {
        [&a, &b]
            .iter()
            .all(|t| rt.state.viewers[t.as_str()].attach == Attach::Attached)
    };
    until(&mut rt, "the app to lose the old connection", |rt| {
        matches!(rt.state.status, Status::Down(_)) || !viewers_attached(rt)
    });
    until(&mut rt, "the app to reattach", |rt| {
        rt.state.status == Status::Connected && viewers_attached(rt)
    });
    idle(&mut rt);
    for t in [&a, &b] {
        assert_eq!(size(&herdr, t), moved, "the app's reattach took the size");
    }
    until(&mut rt, "the app to draw at the new grid", |rt| {
        drawn(rt, &a) == Some(moved) && drawn(rt, &b) == Some(moved)
    });

    // In front again, the shown panes take their sizes.
    rt.window_focus(true);
    until(&mut rt, "the shown panes to take their sizes", |rt| {
        size(&herdr, &a) == mine
            && drawn(rt, &a) == Some(mine)
            && size(&herdr, &b) == mine_b
            && drawn(rt, &b) == Some(mine_b)
    });
    drop(rt);
}
