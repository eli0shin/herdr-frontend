//! herdr-client against a real, isolated herdr-core: handshake, run/attach/write/read-back,
//! events, reconnect after the core restarts, and credit backpressure.

mod support;

use std::time::{Duration, Instant};

use herdr_client::{
    AttachStream, ConnectOptions, Connection, Error, EventItem, MachineUpdate, MachineWatcher,
    StreamItem, Supervisor, SupervisorOptions, USES, Update, no_wake,
};
use herdr_proto::frame::WINDOW;
use herdr_proto::methods::{
    DocSet, EventsSubscribe, TerminalAttach, TerminalKill, TerminalLabel, TerminalList,
    TerminalRead, TerminalRun, TerminalWait,
};
use herdr_proto::{MIN_CORE, Mismatch, Reason, TerminalSet, Version};
use serde_json::json;
use support::{IsolatedCore, eventually};

const WAIT: Duration = Duration::from_secs(20);

fn connect(core: &IsolatedCore) -> Connection {
    Connection::open(&core.transport(), &ConnectOptions::default()).expect("connect")
}

fn run(conn: &Connection, core: &IsolatedCore, argv: &[&str], keep: bool) -> String {
    conn.call(TerminalRun {
        argv: argv.iter().map(|a| (*a).to_owned()).collect(),
        cwd: Some(core.folder().display().to_string()),
        keep,
        ..Default::default()
    })
    .wait()
    .expect("terminal.run")
    .terminal
    .address
}

fn attach(conn: &Connection, terminal: &str, resumed: bool) -> AttachStream {
    conn.attach(TerminalAttach {
        terminal: terminal.into(),
        cols: 80,
        rows: 24,
        resumed,
        ..Default::default()
    })
    .wait()
    .expect("terminal.attach")
}

/// Reads (and credits) a stream until its output so far contains `needle`.
fn read_until(stream: &mut AttachStream, needle: &str) -> String {
    let deadline = Instant::now() + WAIT;
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            !left.is_zero(),
            "no {needle:?} in {:?}",
            String::from_utf8_lossy(&seen)
        );
        match stream.recv_timeout(left) {
            Some(StreamItem::Data(bytes)) => {
                stream.credit(bytes.len());
                seen.extend_from_slice(&bytes);
                let text = String::from_utf8_lossy(&seen);
                if text.contains(needle) {
                    return text.into_owned();
                }
            }
            Some(StreamItem::Resize { .. }) | None => {}
            Some(end) => panic!("the stream ended waiting for {needle:?}: {end:?}"),
        }
    }
}

/// The stream's end, skipping (and crediting) output before it.
fn end_of(stream: &mut AttachStream) -> StreamItem {
    let deadline = Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "the stream didn't end");
        match stream.recv_timeout(left) {
            Some(StreamItem::Data(bytes)) => stream.credit(bytes.len()),
            Some(item) if item.is_end() => return item,
            _ => {}
        }
    }
}

#[test]
fn the_handshake_checks_the_core() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = connect(&core);
    let hello = conn.hello();
    assert_eq!(hello.generation, 1);
    assert!(hello.framed);
    assert!(hello.version().unwrap() >= MIN_CORE, "{}", hello.version);
    assert!(USES.iter().all(|m| hello.has_method(m)));
    let caps = conn.capabilities();
    assert!(caps.framed && caps.attach && caps.events);
    drop(conn);

    let strict = ConnectOptions {
        min_version: Version::new(99, 0, 0),
        ..ConnectOptions::default()
    };
    match Connection::open(&core.transport(), &strict) {
        Err(Error::Incompatible(Mismatch::TooOld { need, .. })) => {
            assert_eq!(need, Version::new(99, 0, 0));
        }
        other => panic!("{:?}", other.err()),
    }
}

#[test]
fn run_attach_write_and_read_back() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = connect(&core);
    let terminal = run(&conn, &core, &["cat"], false);
    let mut stream = attach(&conn, &terminal, false);
    assert_eq!(stream.terminal().address, terminal);
    assert_eq!(stream.generation(), 1);

    stream.input(b"hello-hfc\r");
    // The tty echoes the line, then cat prints it again.
    let text = read_until(&mut stream, "hello-hfc\r\nhello-hfc");
    assert!(text.contains("hello-hfc"));

    let screen = conn
        .call(TerminalRead {
            terminal: terminal.clone(),
            ..Default::default()
        })
        .wait()
        .unwrap();
    assert_eq!(
        screen.text.matches("hello-hfc").count(),
        2,
        "{:?}",
        screen.text
    );

    conn.call(TerminalKill {
        terminal: terminal.clone(),
        ..Default::default()
    })
    .wait()
    .unwrap();
    match end_of(&mut stream) {
        StreamItem::Closed(reason) => assert_eq!(reason.reason, Some(Reason::Killed)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn events_follow_terminals_and_documents() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = connect(&core);
    let (snapshot, mut events) = conn.subscribe(EventsSubscribe::default()).wait().unwrap();
    let mut set = TerminalSet::new(snapshot.seq, snapshot.terminals);
    let mut changes = herdr_proto::Changes::default();
    let mut pump = |set: &mut TerminalSet,
                    done: &dyn Fn(&TerminalSet, &herdr_proto::Changes) -> bool| {
        let deadline = Instant::now() + WAIT;
        while !done(set, &changes) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "timed out; changes so far: {changes:?}");
            match events.next_timeout(left) {
                Some(EventItem::Notification(n)) => set.apply_into(n, &mut changes),
                Some(EventItem::Malformed(line)) => panic!("malformed event: {line}"),
                Some(end @ (EventItem::Closed(_) | EventItem::Lost)) => panic!("{end:?}"),
                None => {}
            }
        }
    };

    let terminal = run(&conn, &core, &["cat"], false);
    pump(&mut set, &|set, _| {
        set.get(&terminal).is_some_and(|t| t.is_live())
    });

    conn.call(TerminalLabel {
        terminal: terminal.clone(),
        labels: [("hfc.test".to_owned(), "1".to_owned())].into(),
        ..Default::default()
    })
    .wait()
    .unwrap();
    pump(&mut set, &|set, _| {
        set.get(&terminal)
            .is_some_and(|t| t.label("hfc.test") == Some("1"))
    });

    let written = conn
        .call(DocSet {
            name: "hfc-test".into(),
            value: json!({"hello": 1}),
            version: 0,
            ..Default::default()
        })
        .wait()
        .unwrap();
    pump(&mut set, &|_, changes| {
        changes.docs.iter().any(|(key, version)| {
            key.name == "hfc-test" && key.machine.is_none() && *version == written.version
        })
    });

    conn.call(TerminalKill {
        terminal: terminal.clone(),
        ..Default::default()
    })
    .wait()
    .unwrap();
    pump(&mut set, &|set, _| {
        set.get(&terminal).is_none_or(|t| !t.is_live())
    });
}

#[test]
fn a_call_that_times_out_is_cancelled_and_the_connection_goes_on() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = connect(&core);
    let terminal = run(&conn, &core, &["cat"], false);
    let waiting = conn.call_timeout(
        TerminalWait {
            terminal: Some(terminal.clone()),
            ..Default::default()
        },
        Some(Duration::from_millis(300)),
    );
    match waiting.wait() {
        Err(Error::Timeout { method, .. }) => assert_eq!(method, "terminal.wait"),
        other => panic!("{other:?}"),
    }
    let list = conn.call(TerminalList::default()).wait().unwrap();
    assert!(list.terminals.iter().any(|t| t.address == terminal));
}

#[test]
fn credit_holds_a_stream_back_without_blocking_calls() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let conn = connect(&core);
    // Output starts only once the stream is attached: then all of it goes through the window.
    let terminal = run(
        &conn,
        &core,
        &[
            "sh",
            "-c",
            "read go; head -c 3000000 /dev/zero | tr '\\0' x; sleep 600",
        ],
        false,
    );
    let mut stream = attach(&conn, &terminal, false);
    stream.input(b"\r");

    // Read without crediting: the core stops at the window.
    let mut received = 0usize;
    let mut quiet_since = Instant::now();
    let deadline = Instant::now() + WAIT;
    while quiet_since.elapsed() < Duration::from_millis(1500) {
        assert!(Instant::now() < deadline, "the stream never went quiet");
        if let Some(StreamItem::Data(bytes)) = stream.recv_timeout(Duration::from_millis(100)) {
            received += bytes.len();
            quiet_since = Instant::now();
        }
    }
    assert!(
        received <= WINDOW,
        "{received} bytes past a {WINDOW}-byte window"
    );
    assert!(
        received > WINDOW / 2,
        "only {received} bytes arrived before the window ran out"
    );

    // A stalled stream never holds up channel 0.
    let list = conn.call(TerminalList::default()).wait().unwrap();
    assert!(list.terminals.iter().any(|t| t.address == terminal));

    // Credit what was consumed, and more comes.
    stream.credit(received);
    let mut more = 0usize;
    eventually(WAIT, "output after credit", || {
        if let Some(StreamItem::Data(bytes)) = stream.recv_timeout(Duration::from_millis(100)) {
            more += bytes.len();
        }
        (more > WINDOW / 2).then_some(())
    });
    assert!(more <= received, "{more} bytes after crediting {received}");

    conn.call(TerminalKill {
        terminal,
        ..Default::default()
    })
    .wait()
    .unwrap();
}

#[test]
fn the_supervisor_reconnects_after_the_core_restarts() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let supervisor = Supervisor::start(SupervisorOptions::new(core.transport()), no_wake());
    let next = |what: &str| {
        supervisor
            .recv_timeout(Duration::from_secs(60))
            .unwrap_or_else(|| panic!("no update: waiting for {what}"))
    };
    match next("the first connection") {
        Update::Connected { generation, hello } => {
            assert_eq!(generation, 1);
            assert_eq!(hello.generation, 1);
        }
        other => panic!("{other:?}"),
    }
    let first = supervisor.connection().unwrap();
    assert_eq!(first.generation(), 1);

    let terminal = run(&first, &core, &["cat"], true);
    eventually(WAIT, "the terminal in the pump's set", || {
        supervisor
            .with_terminals(|set| set.get(&terminal).is_some_and(|t| t.is_live()))
            .then_some(())
    });
    let mut stream = attach(&first, &terminal, false);
    stream.input(b"before\r");
    read_until(&mut stream, "before\r\nbefore");

    core.restart();

    // Every view gets Lost, not Closed: the pane stays and waits.
    assert_eq!(end_of(&mut stream), StreamItem::Lost);
    let mut lost = false;
    let hello = loop {
        match next("the reconnect") {
            Update::Lost { generation, .. } => {
                assert_eq!(generation, 1);
                lost = true;
            }
            Update::Connected { generation, hello } => {
                assert!(lost, "connected again before the loss was reported");
                assert_eq!(generation, 2);
                break hello;
            }
            Update::Changed { generation, .. } => assert_eq!(generation, 1),
            Update::Unavailable { .. } => {}
        }
    };
    assert_eq!(hello.generation, 1);
    assert!(!first.is_alive());
    assert!(
        first
            .call(TerminalList::default())
            .wait()
            .unwrap_err()
            .is_lost()
    );

    // The fresh snapshot still has the kept terminal; reattach without taking the size.
    assert!(supervisor.with_terminals(|set| set.get(&terminal).is_some_and(|t| t.is_live())));
    let second = supervisor.connection().unwrap();
    assert_eq!(second.generation(), 2);
    let mut stream = attach(&second, &terminal, true);
    assert_eq!(stream.generation(), 2);
    stream.input(b"after\r");
    read_until(&mut stream, "after\r\nafter");

    // The new connection's events reach the pump.
    second
        .call(TerminalLabel {
            terminal: terminal.clone(),
            labels: [("hfc.test".to_owned(), "2".to_owned())].into(),
            ..Default::default()
        })
        .wait()
        .unwrap();
    eventually(WAIT, "the label through the new subscription", || {
        supervisor
            .with_terminals(|set| set.get(&terminal).and_then(|t| t.label("hfc.test")) == Some("2"))
            .then_some(())
    });
    second
        .call(TerminalKill {
            terminal,
            ..Default::default()
        })
        .wait()
        .unwrap();
    supervisor.stop();
}

#[test]
fn the_machine_watcher_reports_the_machine_list() {
    let Some(core) = IsolatedCore::new() else {
        return;
    };
    let supervisor = Supervisor::start(SupervisorOptions::new(core.transport()), no_wake());
    let watcher = MachineWatcher::start(supervisor.handle(), Duration::from_secs(60), no_wake());
    match watcher.recv_timeout(Duration::from_secs(60)) {
        Some(MachineUpdate::Machines(machines)) => {
            assert_eq!(machines.generation, 1);
            assert!(machines.machines.is_empty(), "{:?}", machines.machines);
        }
        other => panic!("{other:?}"),
    }
    // Nothing changed, so a poke posts nothing new.
    watcher.poke();
    assert!(watcher.recv_timeout(Duration::from_millis(500)).is_none());
    watcher.stop();
    supervisor.stop();
}
