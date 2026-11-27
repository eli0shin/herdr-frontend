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

fn read_doc(conn: &Connection) -> (serde_json::Value, u64) {
    let read = conn
        .call(DocGet {
            name: "herdr-app".into(),
            ..Default::default()
        })
        .wait()
        .expect("doc.get");
    (read.value, read.version)
}

fn write_doc(conn: &Connection, value: serde_json::Value) -> u64 {
    let (_, version) = read_doc(conn);
    conn.call(DocSet {
        name: "herdr-app".into(),
        value,
        version,
        ..Default::default()
    })
    .wait()
    .expect("doc.set")
    .version
}

fn press(rt: &mut Runtime<State>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.frame(1.0 / 60.0);
}

/// Splits made here go into the core's `herdr-app` document as Herdr app writes them: the new
/// pane beside the focused one with half its share, its terminal started labeled for the space
/// and tab, a split across the axis wrapped in a container of its own; and a keyboard resize
/// writes the two sizes. The app then draws what it wrote.
#[test]
fn a_split_made_here_shows_in_the_document_as_herdr_app_writes_it() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect");
    let a = run(&conn, &core, "printf 'first\\n'; exec cat", None);
    write_doc(
        &conn,
        json!({"spaces": [{
            "id": "sp-1", "name": "isolated", "folder": core.folder().display().to_string(),
            "tabs": [tab("tb-1", "work", split("tb-1", "row", vec![pane("pn-a", &a)]))],
        }]}),
    );

    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.connect(Box::new(Live::new(core.transport())));
    until(&mut rt, "the pane to draw", |rt| {
        text_of(rt, &a).contains("first")
    });

    // Cmd+D: split right.
    let cmd = Mods {
        cmd: true,
        ..Default::default()
    };
    press(&mut rt, Key::Text("d".into()), cmd);
    let mut doc = json!(null);
    until(&mut rt, "the split to reach the document", |_| {
        doc = read_doc(&conn).0;
        doc["spaces"][0]["tabs"][0]["root"]["children"]
            .as_array()
            .is_some_and(|c| c.len() == 2)
    });
    let root = &doc["spaces"][0]["tabs"][0]["root"];
    assert_eq!(root["id"], "tb-1");
    assert_eq!(root["type"], "row");
    let (first, new) = (&root["children"][0], &root["children"][1]);
    assert_eq!(first["id"], "pn-a");
    assert_eq!(first["size"], json!({"weight": 0.5}));
    assert_eq!(new["type"], "block");
    assert_eq!(new["size"], json!({"weight": 0.5}));
    let new_pane = new["id"].as_str().expect("an id").to_owned();
    assert!(new_pane.starts_with('p'), "{new_pane}");
    let b = new["content"]["terminal"]
        .as_str()
        .expect("a terminal block")
        .to_owned();
    let r = record(&conn, &b);
    assert!(r.is_live(), "the new pane's terminal runs");
    assert_eq!(r.label("herdr.space"), Some("sp-1"));
    assert_eq!(r.label("herdr.tab"), Some("tb-1"));
    // The new pane is focused and draws its terminal.
    assert_eq!(
        rt.state.model.focused_pane().as_deref(),
        Some(new_pane.as_str())
    );
    until(&mut rt, "the new pane to attach", |rt| {
        rt.state.viewers.contains_key(&b)
    });

    // Cmd+Shift+D on it: split down, across the row, so the new pane is wrapped in a column.
    let cmd_shift = Mods {
        cmd: true,
        shift: true,
        ..Default::default()
    };
    press(&mut rt, Key::Text("d".into()), cmd_shift);
    until(&mut rt, "the second split to reach the document", |_| {
        doc = read_doc(&conn).0;
        doc["spaces"][0]["tabs"][0]["root"]["children"][1]["type"] == "column"
    });
    let column = &doc["spaces"][0]["tabs"][0]["root"]["children"][1];
    assert!(column["id"].as_str().is_some_and(|id| id.starts_with('s')));
    assert_eq!(
        column["size"],
        json!({"weight": 0.5}),
        "it took the slot's size"
    );
    let below = &column["children"];
    // Inside the new column the two share equally: neither has a size of its own.
    assert_eq!(below[0]["id"], new_pane.as_str());
    assert_eq!(below[0].get("size"), None);
    assert_eq!(below[1]["type"], "block");
    assert_eq!(below[1].get("size"), None);

    // Alt+Shift+H from the bottom pane: the row's divider moves 40 pt left, so pn-a shrinks.
    let (_, before) = read_doc(&conn);
    let alt_shift = Mods {
        alt: true,
        shift: true,
        ..Default::default()
    };
    press(&mut rt, Key::Text("h".into()), alt_shift);
    until(&mut rt, "the resize to reach the document", |_| {
        let (value, version) = read_doc(&conn);
        doc = value;
        version > before
    });
    let root = &doc["spaces"][0]["tabs"][0]["root"];
    let weight = |n: &serde_json::Value| n["size"]["weight"].as_f64().expect("a weight");
    let (wa, wc) = (weight(&root["children"][0]), weight(&root["children"][1]));
    assert!(wa < 0.5 && wc > 0.5, "{wa} {wc}");
    assert!((wa + wc - 1.0).abs() < 1e-4);
    // What the app shows is what it wrote: three panes.
    assert_eq!(rt.state.shown_panes().len(), 3);
    drop(rt);
}

/// Ctrl+L in a terminal whose Neovim label names a server nobody listens on: `nvim
/// --remote-send` runs on the core and fails, so the label is cleared and the pane moves. Skips
/// without `nvim` on the core's PATH.
#[test]
fn ctrl_l_past_a_dead_neovim_clears_its_label_and_moves_the_pane() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect");
    let probe = conn
        .call(herdr_proto::methods::MachineExec {
            argv: vec!["nvim".into(), "--version".into()],
            timeout: Some(10.0),
            ..Default::default()
        })
        .wait();
    if !probe.is_ok_and(|r| r.code == Some(0)) {
        eprintln!("skipped: no nvim on the isolated core's PATH");
        return;
    }
    let a = run(&conn, &core, "exec cat", None);
    let b = run(&conn, &core, "exec cat", None);
    let dead = core.folder().join("no-such-nvim.sock");
    conn.call(herdr_proto::methods::TerminalLabel {
        terminal: a.clone(),
        labels: [(model::EDITOR_LABEL.to_owned(), dead.display().to_string())].into(),
        ..Default::default()
    })
    .wait()
    .expect("terminal.label");
    write_doc(
        &conn,
        json!({"spaces": [{
            "id": "sp-1", "name": "isolated", "folder": core.folder().display().to_string(),
            "tabs": [tab("tb-1", "work", split("tb-1", "row", vec![pane("pn-a", &a), pane("pn-b", &b)]))],
        }]}),
    );
    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.connect(Box::new(Live::new(core.transport())));
    until(&mut rt, "the label to show", |rt| {
        rt.state
            .model
            .world()
            .terminal(&a)
            .is_some_and(|t| t.label(model::EDITOR_LABEL).is_some())
            && rt.state.model.focused_pane().as_deref() == Some("pn-a")
    });
    let ctrl = Mods {
        ctrl: true,
        ..Default::default()
    };
    press(&mut rt, Key::Text("l".into()), ctrl);
    until(&mut rt, "the pane to move", |rt| {
        rt.state.model.focused_pane().as_deref() == Some("pn-b")
    });
    until(&mut rt, "the label to be cleared", |_| {
        record(&conn, &a).label(model::EDITOR_LABEL).is_none()
    });
    drop(rt);
}

/// The terminal menu's Rename names the pane in the core's `herdr-app` document the way Herdr app
/// 0.1.18's `pane.rename` does, which is what it reads back: the block's `title`, trimmed, and
/// nothing else changed; an empty name takes the `title` away, leaving the document as it was
/// (`app:src/window.rs` `pane.rename`, `app:src/layout.rs` `Node::title`).
#[test]
fn a_pane_renamed_here_shows_in_the_document_as_herdr_app_reads_it() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect");
    let a = run(&conn, &core, "printf 'first\\n'; exec cat", None);
    let written = json!({"spaces": [{
        "id": "sp-1", "name": "isolated", "folder": core.folder().display().to_string(),
        "tabs": [tab("tb-1", "work", split("tb-1", "row", vec![pane("pn-a", &a)]))],
    }]});
    write_doc(&conn, written.clone());

    let mut rt = app().headless_runtime(SIZE, Env::new(true, 2.0));
    rt.state.connect(Box::new(Live::new(core.transport())));
    until(&mut rt, "the pane to draw", |rt| {
        text_of(rt, &a).contains("first")
    });

    let rename = |rt: &mut Runtime<State>, name: &str| {
        // A right-click on the pane opens the terminal menu; Rename is its first item.
        let (x, y) = rt.find("pane-pn-a").expect("the pane");
        rt.secondary_down(x, y);
        rt.button_up(x, y, rune::PointerButton::Secondary);
        until(rt, "the terminal menu", |rt| {
            matches!(
                rt.state.model.mode(),
                model::Mode::Popup(model::mode::PopupState::Menu(_))
            )
        });
        press(rt, Key::Enter, Mods::default());
        until(rt, "the prompt", |rt| {
            matches!(
                rt.state.model.mode(),
                model::Mode::Popup(model::mode::PopupState::Prompt(_))
            )
        });
        let shown = rt.state.popup_field.text.clone();
        for c in name.chars() {
            press(rt, Key::Text(c.to_string()), Mods::default());
        }
        if name.is_empty() {
            press(rt, Key::Backspace, Mods::default());
        }
        press(rt, Key::Enter, Mods::default());
        shown
    };

    assert_eq!(
        rename(&mut rt, "  server "),
        "",
        "an unnamed pane's prompt is empty"
    );
    let mut doc = json!(null);
    until(&mut rt, "the name to reach the document", |_| {
        doc = read_doc(&conn).0;
        doc["spaces"][0]["tabs"][0]["root"]["children"][0]
            .get("title")
            .is_some()
    });
    let mut expected = written.clone();
    expected["spaces"][0]["tabs"][0]["root"]["children"][0]["title"] = "server".into();
    assert_eq!(doc, expected, "only the pane's title changed");
    assert_eq!(
        doc["spaces"][0]["tabs"][0]["root"]["children"][0],
        json!({"id": "pn-a", "type": "block", "title": "server", "content": {"terminal": a}}),
    );

    // The prompt shows the name; clearing it leaves the document as it was written.
    assert_eq!(rename(&mut rt, ""), "server");
    until(&mut rt, "the name to leave the document", |_| {
        doc = read_doc(&conn).0;
        doc == written
    });
    drop(rt);
}
