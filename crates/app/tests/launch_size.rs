//! A pane the app makes starts at its own grid: `terminal.run {size, display}` with the grid its
//! RESIZE would send. Without a size, the core started the program at its headless 200 × 120,
//! and a viewer that sized its terminal from the record before the app's RESIZE landed (Herdr
//! app attaching a pane it doesn't show) kept it there. Such a terminal answers the program's
//! size queries from 200 × 120, and Neovim, which takes its size from in-band reports, laid out
//! its first paint for it on a smaller grid: every row's overflow on the start of the next, the
//! panels and status lines out of place (ticket 047).
//!
//! The core is a `test-*` session with scratch dirs under `/tmp` (herdr-client's test support),
//! never the person's. Without an `hc` the test skips, except under CI.

#[path = "../../herdr-client/tests/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use herdr_client::{ConnectOptions, Connection, EventItem, EventStream, StreamItem};
use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::{pane, tab};
use herdr_frontend::link::Live;
use herdr_frontend::{SIZE, State, app};
use herdr_proto::methods::{
    DocGet, DocSet, EventsSubscribe, InitialSize, TerminalAttach, TerminalRun,
};
use herdr_proto::{EventData, Notification};
use model::Action;
use model::action::Split;
use rune::{Env, Runtime};
use serde_json::json;
use support::IsolatedCore;

const WAIT: Duration = Duration::from_secs(30);

/// Turns on in-band size reports (mode 2048), collects every report that comes for a second, and
/// prints the last one's `rows;cols`: the grid a program like Neovim then lays out for.
const LAST_REPORT: &str = "stty raw -echo; printf '\\033[?2048h'; sleep 1; stty min 0 time 10; \
     r=$(dd bs=4096 count=1 2>/dev/null | tr '\\033' E); stty sane; \
     r=${r##*E\\[48;}; printf '\\nlast-%s=%s|\\n' report \"${r%;*;*t*}\"\n";

fn frames(rt: &mut Runtime<State>, d: Duration) {
    let end = Instant::now() + d;
    while Instant::now() < end {
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        std::thread::sleep(Duration::from_millis(8));
    }
}

fn until(rt: &mut Runtime<State>, what: &str, mut done: impl FnMut(&mut Runtime<State>) -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done(rt) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        frames(rt, Duration::from_millis(20));
    }
}

/// The record of the next terminal the core creates, as its `created` event carried it: the
/// grid its program started at.
fn next_created(events: &mut EventStream) -> (String, (u16, u16)) {
    let deadline = Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "no terminal was created");
        if let Some(EventItem::Notification(Notification::Event(event))) = events.next_timeout(left)
            && let EventData::Created(record) = event.data
        {
            return (record.address.clone(), (record.cols, record.rows));
        }
    }
}

/// The grid the app's window gives the pane drawing `terminal`.
fn pane_grid(rt: &Runtime<State>, terminal: &str) -> Option<(u16, u16)> {
    let (_, rect, _) = rt
        .state
        .shown_panes()
        .into_iter()
        .find(|(_, _, a)| a.as_deref() == Some(terminal))?;
    let grid = rt.state.grid_for(rect);
    Some((grid.columns, grid.rows))
}

fn shown_text(rt: &Runtime<State>, terminal: &str) -> String {
    rt.state
        .viewers
        .get(terminal)
        .and_then(|v| v.pane.as_ref())
        .map(|p| p.snapshot.text())
        .unwrap_or_default()
}

#[test]
fn a_pane_the_app_makes_starts_at_its_grid_for_every_viewer() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect");
    let first = conn
        .call(TerminalRun {
            argv: vec!["/bin/sh".into()],
            cwd: Some(core.folder().display().to_string()),
            size: Some(InitialSize { cols: 90, rows: 20 }),
            ..Default::default()
        })
        .wait()
        .expect("terminal.run")
        .terminal
        .address;
    let doc = json!({"spaces": [{
        "id": "sp-1", "name": "isolated", "folder": core.folder().display().to_string(),
        "tabs": [tab("tb-1", "work", pane("pn-a", &first))],
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
    let (_, mut events) = conn
        .subscribe(EventsSubscribe::default())
        .wait()
        .expect("events.subscribe");

    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.with_settings(Settings::fixture());
    rt.state.connect(Box::new(Live::new(core.transport())));
    until(&mut rt, "the first pane", |rt| {
        pane_grid(rt, &first).is_some()
    });

    // A new tab, then a split of its pane: each program starts at the grid its pane gets.
    let mut made = Vec::new();
    for (step, action) in [Action::TabNew, Action::PaneSplit(Split::Right)]
        .into_iter()
        .enumerate()
    {
        rt.state.step(f64::from(step as u32) + 1.0e6, action);
        let (address, created) = next_created(&mut events);
        until(&mut rt, "the new pane", |rt| {
            pane_grid(rt, &address).is_some()
        });
        let grid = pane_grid(&rt, &address).expect("laid out");
        assert_eq!(
            created, grid,
            "step {step}: started at another grid than its pane's"
        );
        made.push((address, created));
    }

    // Herdr app attaches the split's terminal at the grid the record had when it appeared, and
    // its terminal answers the program's queries from that grid, after the app's own answer.
    let (address, created) = made.pop().expect("the split");
    let herdr = Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect");
    let mut stream = herdr
        .attach(TerminalAttach {
            terminal: address.clone(),
            cols: created.0,
            rows: created.1,
            ..Default::default()
        })
        .wait()
        .expect("terminal.attach");
    let mut vt = terminal::Vt::new(&terminal::Config {
        cols: created.0,
        rows: created.1,
        cell_width: 10,
        cell_height: 20,
        ..Default::default()
    });
    // Its own attach took the size: the app takes it back on use, as a click would.
    let (pane, ..) = rt
        .state
        .shown_panes()
        .into_iter()
        .find(|(_, _, a)| a.as_deref() == Some(address.as_str()))
        .expect("a pane");
    let (x, y) = rt.find(&format!("pane-{pane}")).expect("the pane");
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    frames(&mut rt, Duration::from_millis(500));
    stream.automatic(LAST_REPORT.as_bytes());
    let deadline = Instant::now() + WAIT;
    while !shown_text(&rt, &address).contains("last-report=") {
        assert!(
            Instant::now() < deadline,
            "no report: {}",
            shown_text(&rt, &address)
        );
        if let Some(StreamItem::Data(bytes)) = stream.recv_timeout(Duration::from_millis(10)) {
            stream.credit(bytes.len());
            vt.write(&bytes);
            let replies = vt.take_replies();
            if !replies.is_empty() {
                std::thread::sleep(Duration::from_millis(50));
                stream.automatic(&replies);
            }
        }
        frames(&mut rt, Duration::from_millis(1));
    }
    let grid = pane_grid(&rt, &address).expect("laid out");
    let want = format!("last-report={};{}|", grid.1, grid.0);
    assert!(
        shown_text(&rt, &address).contains(&want),
        "the program was told another grid than its pane's {grid:?}: {}",
        shown_text(&rt, &address)
    );
    drop(rt);
}
