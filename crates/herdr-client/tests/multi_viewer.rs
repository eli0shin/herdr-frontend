//! Two framed connections on one terminal of a real, isolated herdr-core: the size rules
//! herdr-frontend's viewer policy rests on (`herdr-core-protocol.md` §4), checked against the
//! core itself rather than read from its source. `app` stands for Herdr app, which attaches
//! every pane as an active viewer; `us` for herdr-frontend.

mod support;

use std::time::{Duration, Instant};

use herdr_client::{AttachStream, ConnectOptions, Connection, StreamItem};
use herdr_proto::methods::{InitialSize, TerminalAttach, TerminalGet, TerminalRun};
use support::{IsolatedCore, eventually};

const WAIT: Duration = Duration::from_secs(20);

/// How long a frame that must change nothing gets to change something anyway.
const QUIET: Duration = Duration::from_millis(400);

fn connect(core: &IsolatedCore) -> Connection {
    Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect")
}

/// A program that never reads or writes, started at `cols` × `rows`.
fn idle(conn: &Connection, core: &IsolatedCore, cols: u16, rows: u16) -> String {
    conn.call(TerminalRun {
        argv: vec!["sleep".into(), "600".into()],
        cwd: Some(core.folder().display().to_string()),
        size: Some(InitialSize { cols, rows }),
        ..Default::default()
    })
    .wait()
    .expect("terminal.run")
    .terminal
    .address
}

fn attach(conn: &Connection, params: TerminalAttach) -> AttachStream {
    conn.attach(params).wait().expect("terminal.attach")
}

fn viewer(terminal: &str, cols: u16, rows: u16, resumed: bool) -> TerminalAttach {
    TerminalAttach {
        terminal: terminal.into(),
        cols,
        rows,
        resumed,
        ..Default::default()
    }
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

fn becomes(conn: &Connection, terminal: &str, want: (u16, u16), why: &str) {
    eventually(WAIT, why, || (size(conn, terminal) == want).then_some(()));
}

fn stays(conn: &Connection, terminal: &str, want: (u16, u16), why: &str) {
    std::thread::sleep(QUIET);
    assert_eq!(size(conn, terminal), want, "{why}");
}

/// Credits what a stream has so far, so no backlog builds up.
fn drain(stream: &mut AttachStream) {
    while let Some(item) = stream.try_recv() {
        if let StreamItem::Data(bytes) = item {
            stream.credit(bytes.len());
        }
    }
}

/// The next item that isn't output, crediting output before it, and the output that came first.
fn next_signal(stream: &mut AttachStream) -> (StreamItem, Vec<u8>) {
    let deadline = Instant::now() + WAIT;
    let mut before = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "nothing but output came");
        match stream.recv_timeout(left) {
            Some(StreamItem::Data(bytes)) => {
                stream.credit(bytes.len());
                before.extend_from_slice(&bytes);
            }
            Some(item) => return (item, before),
            None => {}
        }
    }
}

/// The output that follows, up to a quiet moment.
fn output(stream: &mut AttachStream) -> Vec<u8> {
    let mut out = Vec::new();
    let deadline = Instant::now() + WAIT;
    while out.is_empty() && Instant::now() < deadline {
        while let Some(StreamItem::Data(bytes)) = stream.recv_timeout(Duration::from_millis(200)) {
            stream.credit(bytes.len());
            out.extend_from_slice(&bytes);
        }
    }
    out
}

#[test]
fn the_most_recently_active_viewer_holds_the_size() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let (on_app, on_us) = (connect(&core), connect(&core));
    let t = idle(&on_app, &core, 90, 20);

    // Herdr app attaches as an active viewer: its grid.
    let mut app = attach(&on_app, viewer(&t, 100, 30, false));
    becomes(&on_us, &t, (100, 30), "Herdr app's attach to take the size");

    // We attach resumed, at our pane's grid: nothing moves.
    let mut us = attach(&on_us, viewer(&t, 80, 24, true));
    assert!(us.caps().automatic_input, "{:?}", us.caps());
    assert!(!us.caps().resize_notifications, "only observers get it");
    stays(&on_us, &t, (100, 30), "a resumed attach took the size");

    // The engine's replies are not activity.
    us.automatic(b"\x1b[48;24;80;888;800t");
    stays(&on_us, &t, (100, 30), "AUTOMATIC_INPUT took the size");

    // FOCUS(1) alone applies the grid we last reported, the attach's, however stale: so the
    // pane in use sends RESIZE first.
    us.focus(true);
    becomes(&on_us, &t, (80, 24), "FOCUS(1) to apply the attach's grid");

    // RESIZE takes the size with the pane's grid.
    us.resize(90, 25);
    becomes(&on_us, &t, (90, 25), "our RESIZE");

    // Herdr app typing takes it back at the grid it reported last.
    app.input(b"x");
    becomes(&on_us, &t, (100, 30), "Herdr app's DATA");

    // Losing focus is not activity.
    us.focus(false);
    stays(&on_us, &t, (100, 30), "FOCUS(0) took the size");

    // A RESIZE at the grid we told already is activity all the same: how a click takes the
    // size back.
    us.resize(90, 25);
    becomes(&on_us, &t, (90, 25), "the same RESIZE again");
    app.resize(100, 30);
    becomes(&on_us, &t, (100, 30), "Herdr app's RESIZE");

    // When the leader leaves, the next most active viewer's grid applies.
    drain(&mut app);
    drop(app);
    becomes(&on_us, &t, (90, 25), "our grid once Herdr app detached");

    // With no viewer left that may size it, the headless size.
    drain(&mut us);
    drop(us);
    becomes(&on_app, &t, (200, 120), "the headless size");
}

/// With no active viewer, resumed viewers tie and the last attached sizes the terminal: so when
/// herdr-frontend is a terminal's only viewer, its attach grid applies.
#[test]
fn resumed_viewers_alone_go_by_the_last_attached() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let (one, two) = (connect(&core), connect(&core));
    let t = idle(&one, &core, 90, 20);
    let _first = attach(&one, viewer(&t, 80, 24, true));
    becomes(&one, &t, (80, 24), "the only (resumed) viewer's grid");
    let _second = attach(&two, viewer(&t, 70, 22, true));
    becomes(&one, &t, (70, 22), "the last resumed viewer's grid");

    // An active viewer beats them both, and keeps the size when another resumed one comes.
    let active = attach(&one, viewer(&t, 100, 30, false));
    becomes(&one, &t, (100, 30), "the active viewer's grid");
    let _third = attach(&two, viewer(&t, 60, 20, true));
    stays(
        &one,
        &t,
        (100, 30),
        "a resumed attach beat an active viewer",
    );
    drop(active);
}

#[test]
fn only_an_observer_hears_the_real_grid_in_band() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let (on_app, on_us) = (connect(&core), connect(&core));
    let t = idle(&on_app, &core, 90, 20);
    let mut app = attach(&on_app, viewer(&t, 100, 30, false));
    becomes(&on_us, &t, (100, 30), "Herdr app's attach");

    // A viewer that may type never gets RESIZE: it follows `size` events.
    let plain = attach(
        &on_us,
        TerminalAttach {
            resize_notifications: true,
            ..viewer(&t, 80, 24, true)
        },
    );
    assert!(!plain.caps().resize_notifications, "{:?}", plain.caps());
    drop(plain);

    // An observer hears the grid before its first paint, and takes nothing.
    let mut observer = attach(
        &on_us,
        TerminalAttach {
            observe: true,
            resize_notifications: true,
            ..viewer(&t, 50, 10, false)
        },
    );
    assert!(
        observer.caps().resize_notifications,
        "{:?}",
        observer.caps()
    );
    let (first, before) = next_signal(&mut observer);
    assert_eq!(
        first,
        StreamItem::Resize {
            cols: 100,
            rows: 30
        }
    );
    assert!(before.is_empty(), "output before the grid: {before:?}");
    stays(&on_us, &t, (100, 30), "an observer took the size");

    // On every change: RESIZE, then an authoritative repaint, which starts with CAN.
    drain(&mut app);
    app.resize(110, 32);
    let (next, _) = next_signal(&mut observer);
    assert_eq!(
        next,
        StreamItem::Resize {
            cols: 110,
            rows: 32
        }
    );
    let repaint = output(&mut observer);
    assert_eq!(repaint.first(), Some(&0x18), "{repaint:?}");
}
