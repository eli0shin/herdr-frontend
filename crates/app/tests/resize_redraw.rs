//! A program that redraws its whole screen when its terminal is resized (Pi, or any full-screen
//! program that follows SIGWINCH), in the pane in use, while the window grows and shrinks: after
//! each resize the app shows exactly the screen the core holds.
//!
//! The core applies the app's RESIZE and the program redraws for the new grid at once; the core's
//! `size` event comes on another channel and can arrive after that output. A terminal that waited
//! for the event to resize parsed the redraw at the old grid: a grid that grew left the screen
//! torn, its rows wrapped and shifted, until the program next redrew them (ticket 047, the Pi
//! session's bottom rows after the sidebar closed).
//!
//! The core is a `test-*` session with scratch dirs under `/tmp` (herdr-client's test support),
//! never the person's. Without an `hc` the test skips, except under CI.

#[path = "../../herdr-client/tests/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use herdr_client::{ConnectOptions, Connection, StreamItem};
use herdr_frontend::config::settings::Settings;
use herdr_frontend::fixtures::{pane, tab};
use herdr_frontend::link::Live;
use herdr_frontend::{SIZE, State, app};
use herdr_proto::methods::{DocGet, DocSet, InitialSize, TerminalAttach, TerminalGet, TerminalRun};
use rune::{Env, Runtime};
use serde_json::json;
use support::IsolatedCore;

const WAIT: Duration = Duration::from_secs(30);

/// Draws every row of the screen, `row N of COLSxROWS` padded to the last column, which holds
/// `|`, on the alternate screen; again on every SIGWINCH, as fast as a shell can (`printf` is a
/// builtin, so nothing forks but `stty`). Output parsed at a narrower grid than the one it was
/// drawn for wraps each row's `|` onto the next row and scrolls the screen.
const REDRAWER: &str = r#"
draw() {
    set -- $(stty size)
    rows=$1 cols=$2
    printf '\033[H\033[2J'
    y=0
    while [ "$y" -lt "$rows" ]; do
        printf '\033[%d;1H%-*s|' $((y + 1)) $((cols - 1)) "row $y of ${cols}x$rows"
        y=$((y + 1))
    done
}
trap draw WINCH
stty -echo
printf '\033[?1049h'
draw
while :; do sleep 1 & wait $!; done
"#;

fn connect(core: &IsolatedCore) -> Connection {
    Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect")
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

/// The core's screen, row by row: an observer's first paint, at the grid the core sends it,
/// replayed into a terminal of its own.
fn core_screen(conn: &Connection, terminal: &str) -> Vec<String> {
    let (cols, rows) = size(conn, terminal);
    let mut stream = conn
        .attach(TerminalAttach {
            terminal: terminal.into(),
            cols,
            rows,
            observe: true,
            resize_notifications: true,
            ..Default::default()
        })
        .wait()
        .expect("terminal.attach");
    let mut vt = terminal::Vt::new(&terminal::Config {
        cols,
        rows,
        ..Default::default()
    });
    while let Some(item) = stream.recv_timeout(Duration::from_millis(300)) {
        match item {
            StreamItem::Data(bytes) => {
                stream.credit(bytes.len());
                vt.write(&bytes);
            }
            StreamItem::Resize { cols, rows } => vt.resize(cols, rows, 0, 0),
            _ => break,
        }
    }
    let shown = vt.snapshot();
    (0..shown.screen.len()).map(|y| shown.line(y)).collect()
}

/// The app's screen for `terminal`, row by row, and its grid.
fn app_screen(rt: &Runtime<State>, terminal: &str) -> Option<(Vec<String>, (u16, u16))> {
    let shown = &rt.state.viewers.get(terminal)?.pane.as_ref()?.snapshot;
    let rows = (0..shown.screen.len()).map(|y| shown.line(y)).collect();
    Some((rows, (shown.cols, shown.rows)))
}

fn frames(rt: &mut Runtime<State>, d: Duration) {
    let deadline = Instant::now() + d;
    while Instant::now() < deadline {
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        std::thread::sleep(Duration::from_millis(8));
    }
}

/// Frames until the app shows the core's screen at the core's grid, failing with the first rows
/// that differ.
fn shows_the_core(rt: &mut Runtime<State>, conn: &Connection, terminal: &str, why: &str) {
    let deadline = Instant::now() + WAIT;
    loop {
        frames(rt, Duration::from_millis(300));
        let core = core_screen(conn, terminal);
        let grid = size(conn, terminal);
        let app = app_screen(rt, terminal);
        if app
            .as_ref()
            .is_some_and(|(rows, at)| *rows == core && *at == grid)
        {
            return;
        }
        if Instant::now() > deadline {
            let (rows, at) = app.unwrap_or_default();
            let diff: Vec<String> = rows
                .iter()
                .zip(&core)
                .enumerate()
                .filter(|(_, (a, c))| a != c)
                .take(3)
                .map(|(y, (a, c))| format!("row {y}\n  app:  {a:?}\n  core: {c:?}"))
                .collect();
            panic!(
                "{why}: the app shows a {at:?} screen unlike the core's {grid:?} one\n{}",
                diff.join("\n")
            );
        }
    }
}

#[test]
fn a_redraw_for_a_new_grid_lands_whole() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = connect(&core);
    let t = conn
        .call(TerminalRun {
            argv: vec!["/bin/sh".into(), "-c".into(), REDRAWER.into()],
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
        "tabs": [tab("tb-1", "work", pane("pn-a", &t))],
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
    shows_the_core(&mut rt, &conn, &t, "the first paint");

    // Wider and taller, narrower, back: a grid that grows is the one a late resize tore.
    let (w, h) = SIZE;
    for (step, size) in [
        (w + 300.0, h + 120.0),
        (w - 200.0, h - 100.0),
        (w + 160.0, h + 40.0),
        (w, h),
    ]
    .into_iter()
    .enumerate()
    {
        let before = app_screen(&rt, &t).map(|(_, grid)| grid);
        rt.set_viewport(size, 2.0);
        shows_the_core(&mut rt, &conn, &t, &format!("resize {step} to {size:?}"));
        assert_ne!(
            app_screen(&rt, &t).map(|(_, grid)| grid),
            before,
            "resize {step} didn't change the grid"
        );
    }
    drop(rt);
}
