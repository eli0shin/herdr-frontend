//! The app next to a second viewer that answers the program's in-band size query (mode 2048)
//! from its own terminal, at its own grid, as Herdr app does (`docs/interop.md` "Two viewers and
//! in-band size reports"). herdr-core passes every viewer's answer on to the program, and a
//! program with mode 2048 on (Neovim) ignores SIGWINCH and lays out at the last report it heard.
//! So the app starts each terminal it makes at its pane's grid, and focusing a pane tells the
//! program the app's grid again.
//!
//! The core is a `test-*` session with scratch dirs under `/tmp` (herdr-client's test support),
//! never the person's. Without an `hc` the test skips, except under CI.

#[path = "../../herdr-client/tests/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use herdr_client::{AttachStream, ConnectOptions, Connection, EventItem};
use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::{pane, split, tab};
use herdr_frontend::link::Live;
use herdr_frontend::{SIZE, State, app};
use herdr_proto::event::{EventData, Notification};
use herdr_proto::methods::{
    DocGet, DocSet, EventsSubscribe, InitialSize, TerminalAttach, TerminalGet, TerminalRun,
};
use rune::{Env, Key, KeyEvent, Mods, Runtime};
use serde_json::json;
use support::IsolatedCore;

const WAIT: Duration = Duration::from_secs(30);

/// The grid terminals start at before the app shows them.
const START: (u16, u16) = (90, 20);

/// The other viewer's grid, which it answers the program's size query with.
const OTHER: (u16, u16) = (77, 21);

fn connect(core: &IsolatedCore) -> Connection {
    Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect")
}

/// Runs `script` under `sh` at `START`.
fn run(conn: &Connection, core: &IsolatedCore, script: &str) -> String {
    conn.call(TerminalRun {
        argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
        cwd: Some(core.folder().display().to_string()),
        keep: true,
        size: Some(InitialSize {
            cols: START.0,
            rows: START.1,
        }),
        ..Default::default()
    })
    .wait()
    .expect("terminal.run")
    .terminal
    .address
}

/// Writes the layout: one tab, `root`.
fn layout(conn: &Connection, core: &IsolatedCore, root: serde_json::Value) {
    let doc = json!({"spaces": [{
        "id": "sp-1", "name": "isolated", "folder": core.folder().display().to_string(),
        "tabs": [tab("tb-1", "work", root)],
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
}

/// The app's window on `core`, in front.
fn open_app(core: &IsolatedCore) -> Runtime<State> {
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.with_settings(Settings::fixture());
    rt.state.connect(Box::new(Live::new(core.transport())));
    rt
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

fn text_of(rt: &Runtime<State>, address: &str) -> String {
    rt.state
        .viewers
        .get(address)
        .and_then(|v| v.pane.as_ref())
        .map(|p| p.snapshot.text())
        .unwrap_or_default()
}

/// The grid pane `pane` holds in the app's window.
fn grid(rt: &Runtime<State>, pane: &str) -> Option<(u16, u16)> {
    let (_, rect, _) = rt
        .state
        .shown_panes()
        .into_iter()
        .find(|(p, ..)| p == pane)?;
    let g = rt.state.grid_for(rect);
    Some((g.columns, g.rows))
}

fn cmd(key: &str) -> KeyEvent {
    KeyEvent {
        key: Key::Text(key.into()),
        mods: Mods {
            cmd: true,
            ..Mods::default()
        },
    }
}

/// The grid in the last size report the program printed (`report=<rows>;<cols>|`).
fn last_report(rt: &Runtime<State>, address: &str) -> Option<(u16, u16)> {
    let text = text_of(rt, address);
    let report = text
        .rsplit("report=")
        .next()
        .filter(|_| text.contains("report="))?;
    let report = report.split('|').next()?;
    // `[48;rows;cols;height;width`
    let mut fields = report.trim_start_matches('[').split(';').skip(1);
    let rows = fields.next()?.parse().ok()?;
    let cols = fields.next()?.parse().ok()?;
    Some((cols, rows))
}

#[test]
fn a_terminal_the_app_makes_starts_at_its_panes_grid() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let other = connect(&core);
    let a = run(&other, &core, "printf 'cat-ready\\n'; exec cat");
    layout(&other, &core, pane("pn-a", &a));
    let mut rt = open_app(&core);
    until(&mut rt, "the pane to draw", |rt| {
        text_of(rt, &a).contains("cat-ready")
    });

    // The other viewer hears of each new terminal, and attaches it at the record's grid, as
    // Herdr app attaches a pane in a tab it doesn't show.
    let (_, mut events) = other
        .subscribe(EventsSubscribe::default())
        .wait()
        .expect("events.subscribe");
    rt.key(cmd("t"));
    let deadline = Instant::now() + WAIT;
    let created = loop {
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        assert!(Instant::now() < deadline, "no new terminal");
        match events.next_timeout(Duration::from_millis(20)) {
            Some(EventItem::Notification(Notification::Event(e))) => {
                if let EventData::Created(t) = e.data
                    && t.address != a
                {
                    break *t;
                }
            }
            Some(end @ (EventItem::Closed(_) | EventItem::Lost)) => panic!("{end:?}"),
            _ => {}
        }
    };
    let _attached: AttachStream = other
        .attach(TerminalAttach {
            terminal: created.address.clone(),
            cols: created.cols,
            rows: created.rows,
            ..Default::default()
        })
        .wait()
        .expect("terminal.attach");

    let mut placed = None;
    until(&mut rt, "the new tab's pane", |rt| {
        placed = rt
            .state
            .shown_panes()
            .into_iter()
            .find(|(_, _, t)| t.as_deref() == Some(created.address.as_str()))
            .map(|(p, ..)| p);
        placed.is_some()
    });
    let mine = grid(&rt, &placed.unwrap()).expect("its grid");
    assert_eq!(
        (created.cols, created.rows),
        mine,
        "the terminal started at another grid than its pane's"
    );
    assert_eq!(size(&other, &created.address), mine);
    drop(rt);
}

#[test]
fn focusing_a_pane_tells_the_program_the_apps_grid_again() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let other = connect(&core);
    let a = run(&other, &core, "printf 'cat-ready\\n'; exec cat");
    // `b` turns on in-band size reports and prints each report it hears, read a byte at a time
    // up to its final `t` (POSIX `sh`, which has no `read -d`). Like Neovim, it goes by the
    // last one.
    let b = run(
        &other,
        &core,
        "stty -echo -icanon min 1; printf '\\033[?2048h'; printf 'listening\\n'; \
         while :; do reply=; \
         while c=$(dd bs=1 count=1 2>/dev/null) && [ \"$c\" != t ]; do reply=\"$reply$c\"; done; \
         printf 'report=%s|\\n' \"${reply#?}\"; done",
    );
    layout(
        &other,
        &core,
        split("tb-1", "row", vec![pane("pn-a", &a), pane("pn-b", &b)]),
    );
    let mut rt = open_app(&core);
    until(&mut rt, "both panes to draw", |rt| {
        text_of(rt, &a).contains("cat-ready") && text_of(rt, &b).contains("listening")
    });
    let mine = grid(&rt, "pn-b").expect("pane b");
    assert!(
        ![START, OTHER].contains(&mine),
        "the test needs the grids to differ"
    );
    assert_eq!(rt.state.in_use(), Some(a.as_str()), "`b` starts unfocused");

    // The app holds `b`'s size, and its terminal told the program so.
    until(&mut rt, "the app's grid and report", |rt| {
        size(&other, &b) == mine && last_report(rt, &b) == Some(mine)
    });

    // The other viewer, attached without taking the size, answers last at its own grid.
    let stand_in = other
        .attach(TerminalAttach {
            terminal: b.clone(),
            cols: OTHER.0,
            rows: OTHER.1,
            resumed: true,
            ..Default::default()
        })
        .wait()
        .expect("terminal.attach");
    assert!(stand_in.caps().automatic_input);
    let report = format!(
        "\x1b[48;{};{};{};{}t",
        OTHER.1,
        OTHER.0,
        OTHER.1 * 40,
        OTHER.0 * 20
    );
    stand_in.automatic(report.as_bytes());
    until(
        &mut rt,
        "the program to go by the other viewer's report",
        |rt| last_report(rt, &b) == Some(OTHER),
    );
    assert_eq!(size(&other, &b), mine, "the pty stays at the app's grid");

    // Focusing `b` in the app tells the program the app's grid again.
    rt.key(KeyEvent {
        key: Key::Right,
        mods: Mods {
            cmd: true,
            alt: true,
            ..Mods::default()
        },
    });
    until(&mut rt, "focus to move to `b`", |rt| {
        rt.state.in_use() == Some(b.as_str())
    });
    until(&mut rt, "the program to go by the app's grid again", |rt| {
        last_report(rt, &b) == Some(mine)
    });
    assert_eq!(size(&other, &b), mine);
    drop(rt);
}
