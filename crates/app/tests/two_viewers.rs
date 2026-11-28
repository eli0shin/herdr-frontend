//! The app next to a second viewer that answers the program's in-band size query (mode 2048)
//! from its own terminal, at its own grid, as Herdr app does (`docs/interop.md` "Two viewers and
//! in-band size reports"). herdr-core passes every viewer's answer on to the program, and a
//! program with mode 2048 on (Neovim) ignores SIGWINCH and lays out at the last report it heard.
//! So the app starts each terminal it makes at its pane's grid, focusing a pane tells the
//! program the app's grid again, and so does the pane in use a moment after its program turns
//! the mode on.
//!
//! The core is a `test-*` session with scratch dirs under `/tmp` (herdr-client's test support),
//! never the person's. Without an `hc` the test skips, except under CI.

#[path = "../../herdr-client/tests/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use herdr_client::{AttachStream, ConnectOptions, Connection, EventItem, StreamItem};
use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::{pane, split, tab};
use herdr_frontend::link::Live;
use herdr_frontend::state::REASSERT_AFTER;
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

/// A program that turns on in-band size reports and prints each report it hears, read a byte at
/// a time up to its final `t` (POSIX `sh`, which has no `read -d`). Like Neovim, it goes by the
/// last one. `before` runs first.
fn listener(before: &str) -> String {
    format!(
        "stty -echo -icanon min 1; {before} printf '\\033[?2048h'; printf 'listening\\n'; \
         while :; do reply=; \
         while c=$(dd bs=1 count=1 2>/dev/null) && [ \"$c\" != t ]; do reply=\"$reply$c\"; done; \
         printf 'report=%s|\\n' \"${{reply#?}}\"; done"
    )
}

/// The grids in the size reports `text` shows the program printed (`report=[48;<rows>;<cols>;…|`),
/// in order.
fn reports(text: &str) -> Vec<(u16, u16)> {
    text.split("report=")
        .skip(1)
        .filter_map(|report| {
            let report = report.split('|').next()?;
            // `[48;rows;cols;height;width`
            let mut fields = report.trim_start_matches('[').split(';').skip(1);
            let rows = fields.next()?.parse().ok()?;
            let cols = fields.next()?.parse().ok()?;
            Some((cols, rows))
        })
        .collect()
}

/// The grid in the last size report the program printed.
fn last_report(rt: &Runtime<State>, address: &str) -> Option<(u16, u16)> {
    reports(&text_of(rt, address)).last().copied()
}

/// What the other viewer sends as its answer to the program's enable: a size report at its own
/// grid, with a 20×40 cell.
fn other_report() -> String {
    format!(
        "\x1b[48;{};{};{};{}t",
        OTHER.1,
        OTHER.0,
        OTHER.1 * 40,
        OTHER.0 * 20
    )
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
    // `b` turns on in-band size reports and prints each report it hears.
    let b = run(&other, &core, &listener(""));
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
    stand_in.automatic(other_report().as_bytes());
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

/// The other viewer of a terminal whose program turns in-band size reports on when the test
/// says go: attached without taking the size, it answers the enable at its own grid right after
/// the app's answer reached the program, as Herdr app's answer coming in second does.
struct StandIn {
    stream: AttachStream,
    /// The program printing the app's report.
    ours: String,
    heard: Vec<u8>,
    answered: bool,
}

impl StandIn {
    fn attach(conn: &Connection, terminal: &str, mine: (u16, u16)) -> StandIn {
        let stream = conn
            .attach(TerminalAttach {
                terminal: terminal.into(),
                cols: OTHER.0,
                rows: OTHER.1,
                resumed: true,
                ..Default::default()
            })
            .wait()
            .expect("terminal.attach");
        assert!(stream.caps().automatic_input);
        StandIn {
            stream,
            ours: format!("report=[48;{};{};", mine.1, mine.0),
            heard: Vec::new(),
            answered: false,
        }
    }

    /// Reads what the stream has, and answers once the program printed the app's report. True
    /// once it has answered.
    fn answer(&mut self) -> bool {
        while let Some(item) = self.stream.try_recv() {
            match item {
                StreamItem::Data(bytes) => {
                    self.stream.credit(bytes.len());
                    self.heard.extend(bytes);
                }
                end if end.is_end() => panic!("{end:?}"),
                _ => {}
            }
        }
        if !self.answered && String::from_utf8_lossy(&self.heard).contains(&self.ours) {
            self.stream.automatic(other_report().as_bytes());
            self.answered = true;
        }
        self.answered
    }
}

/// Waits in the pane until the test makes `go`, then turns on in-band size reports, as Neovim
/// started in a pane does.
fn waits_to_listen(conn: &Connection, core: &IsolatedCore) -> String {
    run(
        conn,
        core,
        &listener("printf 'waiting\\n'; while [ ! -e go ]; do sleep 0.05; done;"),
    )
}

fn say_go(core: &IsolatedCore) {
    std::fs::write(core.folder().join("go"), "").expect("say go");
}

#[test]
fn a_program_that_turns_size_reports_on_in_the_pane_in_use_ends_at_the_apps_grid() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let other = connect(&core);
    let b = waits_to_listen(&other, &core);
    layout(&other, &core, pane("pn-b", &b));
    let mut rt = open_app(&core);
    until(&mut rt, "the pane to draw", |rt| {
        text_of(rt, &b).contains("waiting")
    });
    assert_eq!(
        rt.state.in_use(),
        Some(b.as_str()),
        "`b` is the pane in use"
    );
    let mine = grid(&rt, "pn-b").expect("pane b");
    assert!(
        ![START, OTHER].contains(&mine),
        "the test needs the grids to differ"
    );
    until(&mut rt, "the app's grid", |_| size(&other, &b) == mine);

    let mut stand_in = StandIn::attach(&other, &b, mine);
    say_go(&core);
    until(&mut rt, "the program to end at the app's grid", |rt| {
        let told = reports(&text_of(rt, &b));
        stand_in.answer() && told.contains(&OTHER) && told.last() == Some(&mine)
    });
    let told = reports(&text_of(&rt, &b));
    assert_eq!(
        told.first(),
        Some(&mine),
        "the app answered the enable first: {told:?}"
    );
    assert_eq!(size(&other, &b), mine, "the pty stays at the app's grid");
    drop(rt);
}

#[test]
fn a_program_that_turns_size_reports_on_in_a_pane_not_in_use_keeps_the_last_answer() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let other = connect(&core);
    let a = run(&other, &core, "printf 'cat-ready\\n'; exec cat");
    let b = waits_to_listen(&other, &core);
    layout(
        &other,
        &core,
        split("tb-1", "row", vec![pane("pn-a", &a), pane("pn-b", &b)]),
    );
    let mut rt = open_app(&core);
    until(&mut rt, "both panes to draw", |rt| {
        text_of(rt, &a).contains("cat-ready") && text_of(rt, &b).contains("waiting")
    });
    assert_eq!(rt.state.in_use(), Some(a.as_str()), "`b` isn't in use");
    let mine = grid(&rt, "pn-b").expect("pane b");
    assert!(
        ![START, OTHER].contains(&mine),
        "the test needs the grids to differ"
    );
    until(&mut rt, "the app's grid", |_| size(&other, &b) == mine);

    let mut stand_in = StandIn::attach(&other, &b, mine);
    say_go(&core);
    until(
        &mut rt,
        "the program to go by the other viewer's report",
        |rt| stand_in.answer() && last_report(rt, &b) == Some(OTHER),
    );
    // Past every re-assert the pane in use would make, nothing told it the app's grid again.
    // The app's clock moves by the frames' 1/60 s, and the sleeps give a report time to make the
    // round trip through the core.
    let quiet = ((REASSERT_AFTER[1] + 0.5) * 60.0).ceil() as usize;
    for _ in 0..quiet {
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        reports(&text_of(&rt, &b)),
        [mine, OTHER],
        "only the two answers to the enable"
    );
    assert_eq!(size(&other, &b), mine);
    drop(rt);
}
