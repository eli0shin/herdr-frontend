//! A [`Terminal`] on its worker thread: the one-slot snapshot handoff, the waker, and the
//! ordered output channel.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use ghostty_vt_sys::{GHOSTTY_KEY_A, GHOSTTY_KEY_ARROW_UP, GHOSTTY_KEY_C};
use terminal::{
    Config, Event, KeyEvent, Mods, Output, PasteRejected, Scroll, SelectAction, SelectEvent,
    Snapshot, Terminal,
};

const PATIENCE: Duration = Duration::from_secs(10);

/// A terminal whose waker counts into a channel, as a UI's frame scheduler would.
fn spawn(config: Config) -> (Terminal, Receiver<()>) {
    let (woke, wakes) = mpsc::channel();
    let woke = std::sync::Mutex::new(woke);
    let terminal = Terminal::spawn(
        config,
        Arc::new(move || {
            let _ = woke.lock().unwrap().send(());
        }),
    );
    (terminal, wakes)
}

fn config() -> Config {
    Config {
        cell_width: 10,
        cell_height: 22,
        ..Config::default()
    }
}

/// Waits, frame by frame as the waker asks, for a snapshot that satisfies `done`.
fn snapshot_where(
    terminal: &Terminal,
    wakes: &Receiver<()>,
    mut done: impl FnMut(&Snapshot) -> bool,
) -> Snapshot {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Some(s) = terminal.take_snapshot()
            && done(&s)
        {
            return s;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            wakes.recv_timeout(left).is_ok(),
            "no matching snapshot in time"
        );
    }
}

/// The next output, waiting for it.
fn next_output(terminal: &Terminal) -> Output {
    terminal
        .output()
        .recv_timeout(PATIENCE)
        .expect("output in time")
}

/// Waits until the worker has handled everything sent so far: a device status query sent
/// last is answered last.
fn sync(terminal: &Terminal) -> Vec<Output> {
    terminal.write(b"\x1b[5n".to_vec());
    let mut before = Vec::new();
    loop {
        match next_output(terminal) {
            Output::Reply(bytes) if bytes.ends_with(b"\x1b[0n") => {
                let rest = &bytes[..bytes.len() - 4];
                if !rest.is_empty() {
                    before.push(Output::Reply(rest.to_vec()));
                }
                return before;
            }
            other => before.push(other),
        }
    }
}

#[test]
fn the_first_snapshot_is_ready_when_the_waker_runs() {
    let (terminal, wakes) = spawn(config());
    wakes.recv_timeout(PATIENCE).expect("woken");
    let s = terminal.take_snapshot().expect("first snapshot");
    assert_eq!((s.generation, s.cols, s.rows), (1, 80, 24));
    assert_eq!(s.text(), "");
    assert_eq!(terminal.take_snapshot(), None, "taken once");
}

#[test]
fn output_shows_in_the_next_snapshot() {
    let (terminal, wakes) = spawn(config());
    terminal.write(b"hello\r\nworld".to_vec());
    let s = snapshot_where(&terminal, &wakes, |s| s.line(1) == "world");
    assert_eq!(s.line(0), "hello");
    assert_eq!(s.cursor.position, Some((5, 1)));
}

#[test]
fn output_while_the_ui_is_busy_folds_into_one_snapshot() {
    let (terminal, wakes) = spawn(config());
    snapshot_where(&terminal, &wakes, |_| true);

    // The UI doesn't take a frame while 200 writes go in.
    for i in 0..200 {
        terminal.write(format!("\rchunk {i}").into_bytes());
    }
    sync(&terminal);

    // At most two snapshots get published: one right after the first write, and a newer one
    // when the UI takes that.
    let mut taken = 0;
    let last = snapshot_where(&terminal, &wakes, |s| {
        taken += 1;
        s.line(0) == "chunk 199"
    });
    assert!(taken <= 2, "{taken} snapshots taken");
    assert!(last.generation <= 3, "generation {}", last.generation);
}

#[test]
fn replies_input_and_events_come_out_in_order() {
    let (terminal, _wakes) = spawn(config());
    terminal.write(b"\x1b]2;title\x07\x1b[c".to_vec());
    // Application cursor keys, then a key: the key sees the mode the write turned on.
    terminal.write(b"\x1b[?1h".to_vec());
    terminal.key(KeyEvent::press(GHOSTTY_KEY_ARROW_UP, Mods::NONE));
    terminal.key(KeyEvent::press(0, Mods::NONE));
    terminal.paste("ls\nrm", false);
    terminal.write(b"\x1b[?2004h".to_vec());
    terminal.paste("ls\nrm", false);
    assert_eq!(
        sync(&terminal),
        [
            // Within one write, replies go first.
            Output::Reply(b"\x1b[?62;22c".to_vec()),
            Output::Event(Event::Title("title".into())),
            Output::Input(b"\x1bOA".to_vec()),
            // The unidentified key sent nothing, so there's no output for it.
            Output::Event(Event::PasteRejected(PasteRejected {
                text: "ls\nrm".into(),
            })),
            Output::Input(b"\x1b[200~ls\nrm\x1b[201~".to_vec()),
        ]
    );
}

#[test]
fn a_resize_sends_the_size_report_and_a_full_frame() {
    let (terminal, wakes) = spawn(config());
    terminal.write(b"\x1b[?2048h".to_vec());
    assert_eq!(
        sync(&terminal),
        [Output::Reply(b"\x1b[48;24;80;528;800t".to_vec())]
    );
    terminal.resize(100, 30, 21, 45);
    assert_eq!(
        sync(&terminal),
        [Output::Reply(b"\x1b[48;30;100;1350;2100t".to_vec())]
    );
    let s = snapshot_where(&terminal, &wakes, |s| s.cols == 100);
    assert_eq!((s.rows, s.cell_width, s.cell_height), (30, 21, 45));
}

#[test]
fn a_render_hold_keeps_the_frame_until_it_ends() {
    let (terminal, wakes) = spawn(config());
    terminal.write(b"before".to_vec());
    snapshot_where(&terminal, &wakes, |s| s.line(0) == "before");

    terminal.write(b"\x1b[?2026h\r\x1b[Kduring".to_vec());
    sync(&terminal);
    let held = snapshot_where(&terminal, &wakes, |_| true);
    assert_eq!(held.line(0), "before");
    assert!(held.modes.held);
    terminal.write(b" more".to_vec());
    sync(&terminal);
    assert_eq!(
        terminal.take_snapshot(),
        None,
        "nothing shows during the hold"
    );

    terminal.write(b"\x1b[?2026l".to_vec());
    let s = snapshot_where(&terminal, &wakes, |s| !s.modes.held);
    assert_eq!(s.line(0), "during more");
}

#[test]
fn a_render_hold_that_never_ends_times_out() {
    let (terminal, wakes) = spawn(Config {
        render_hold_timeout: Duration::from_millis(100),
        ..config()
    });
    snapshot_where(&terminal, &wakes, |_| true);
    let start = Instant::now();
    terminal.write(b"\x1b[?2026hstuck".to_vec());
    // With nothing else sent, the worker wakes itself at the deadline.
    let s = snapshot_where(&terminal, &wakes, |s| s.line(0) == "stuck");
    assert!(!s.modes.held);
    assert!(start.elapsed() >= Duration::from_millis(100));
}

#[test]
fn dropping_the_terminal_stops_its_worker() {
    let (terminal, wakes) = spawn(config());
    terminal.write(b"bye".to_vec());
    snapshot_where(&terminal, &wakes, |s| s.line(0) == "bye");
    drop(terminal);
    // The waker (and its sender) went with the worker thread.
    let deadline = Instant::now() + PATIENCE;
    while wakes.recv_timeout(Duration::from_millis(10)).is_ok() {
        assert!(Instant::now() < deadline);
    }
    assert!(matches!(
        wakes.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
}

#[test]
fn a_writer_feeds_the_worker_from_another_thread() {
    let (terminal, wakes) = spawn(config());
    let writer = terminal.writer();
    std::thread::spawn(move || writer.write(b"from a thread".to_vec()))
        .join()
        .unwrap();
    snapshot_where(&terminal, &wakes, |s| s.line(0) == "from a thread");
}

#[test]
fn a_writer_left_behind_doesnt_keep_the_worker_running() {
    let (terminal, wakes) = spawn(config());
    let writer = terminal.writer();
    drop(terminal);
    let deadline = Instant::now() + PATIENCE;
    while wakes.recv_timeout(Duration::from_millis(10)).is_ok() {
        assert!(Instant::now() < deadline);
    }
    assert!(matches!(
        wakes.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
    // Writing to a stopped terminal goes nowhere.
    writer.write(b"late".to_vec());
}

#[test]
fn a_snapshot_now_has_everything_sent_before_it_and_empties_the_handoff() {
    let (terminal, wakes) = spawn(config());
    // The first frame waits in the handoff, untaken.
    wakes.recv_timeout(PATIENCE).expect("woken");
    let writer = terminal.writer();
    writer.write(b"one\r\n".to_vec());
    terminal.write(b"two".to_vec());
    let s = terminal.snapshot_now(PATIENCE).expect("a snapshot");
    assert_eq!((s.line(0).as_str(), s.line(1).as_str()), ("one", "two"));
    assert_eq!(terminal.take_snapshot(), None, "the stale frame is gone");

    // What comes after shows as usual.
    terminal.write(b" three".to_vec());
    snapshot_where(&terminal, &wakes, |s| s.line(1) == "two three");

    // A reset through the writer starts the screen over.
    terminal.writer().reset();
    let s = terminal.snapshot_now(PATIENCE).expect("a snapshot");
    assert_eq!(s.text(), "");
}

/// Holds the worker inside a parsed callback until the returned sender is dropped or sent on.
fn stall(terminal: &Terminal) -> mpsc::Sender<()> {
    let (release, held) = mpsc::channel::<()>();
    let (entered, inside) = mpsc::channel::<()>();
    terminal.writer().write_then(
        Vec::new(),
        Box::new(move || {
            let _ = entered.send(());
            let _ = held.recv();
        }),
    );
    inside
        .recv_timeout(PATIENCE)
        .expect("the worker took the stall");
    release
}

#[test]
fn a_write_reports_parsed_only_after_the_worker_parsed_it() {
    let (terminal, wakes) = spawn(config());
    let release = stall(&terminal);
    let (parsed, heard) = mpsc::channel();
    terminal.writer().write_then(
        b"after the stall".to_vec(),
        Box::new(move || parsed.send(()).unwrap()),
    );
    // Queued behind a busy worker: counted, not parsed, not reported.
    assert_eq!(terminal.pending_bytes(), b"after the stall".len());
    assert!(heard.recv_timeout(Duration::from_millis(100)).is_err());

    drop(release);
    heard.recv_timeout(PATIENCE).expect("reported once parsed");
    assert_eq!(terminal.pending_bytes(), 0);
    snapshot_where(&terminal, &wakes, |s| s.line(0) == "after the stall");
}

#[test]
fn dropping_a_terminal_doesnt_parse_its_backlog_first() {
    let (terminal, _wakes) = spawn(config());
    let release = stall(&terminal);
    let parsed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let writer = terminal.writer();
    for _ in 0..1000 {
        let parsed = parsed.clone();
        writer.write_then(
            vec![b'x'; 64 * 1024],
            Box::new(move || {
                parsed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }),
        );
    }
    assert_eq!(terminal.pending_bytes(), 1000 * 64 * 1024);
    // Let the stall go once the drop has asked the worker to stop.
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        drop(release);
    });
    drop(terminal);
    assert_eq!(parsed.load(std::sync::atomic::Ordering::SeqCst), 0);
}

/// Selects `cols` cells of row 0 from its first, in `config()`'s 10x22 cells.
fn select_start_of_row(terminal: &Terminal, cols: u16) {
    terminal.select(SelectEvent::new(SelectAction::Press, 1.0, 5.0));
    let end = f32::from(cols) * 10.0 - 2.0;
    terminal.select(SelectEvent::new(SelectAction::Drag, end, 5.0));
    terminal.select(SelectEvent::new(SelectAction::Release, end, 5.0));
}

#[test]
fn copy_sends_the_selection_or_else_the_key() {
    let (terminal, _wakes) = spawn(config());
    terminal.write(b"hello world".to_vec());
    let cmd_c = KeyEvent {
        unshifted: Some('c'),
        ..KeyEvent::press(GHOSTTY_KEY_C, Mods::SUPER)
    };
    // Nothing selected: the key goes to the program, as it would under the kitty protocol.
    terminal.write(b"\x1b[>1u".to_vec());
    terminal.copy(Some(cmd_c.clone()));
    assert_eq!(sync(&terminal), [Output::Input(b"\x1b[99;9u".to_vec())]);

    select_start_of_row(&terminal, 5);
    terminal.copy(Some(cmd_c));
    terminal.copy(None);
    assert_eq!(
        sync(&terminal),
        [
            Output::Selection("hello".into()),
            Output::Selection("hello".into())
        ]
    );
    terminal.clear_selection();
    terminal.copy(None);
    assert_eq!(sync(&terminal), [], "nothing to copy, no key: nothing");
}

#[test]
fn typing_drops_the_selection_and_returns_to_the_bottom() {
    let (terminal, wakes) = spawn(Config {
        rows: 5,
        ..config()
    });
    let lines: Vec<u8> = (1..=20)
        .flat_map(|i| format!("line {i}\r\n").into_bytes())
        .collect();
    terminal.write(lines);
    terminal.scroll(Scroll::Delta(-3));
    select_start_of_row(&terminal, 4);
    snapshot_where(&terminal, &wakes, |s| {
        !s.scrollbar.at_bottom && s.screen[0].selection.is_some()
    });

    // A lone modifier types nothing and changes nothing.
    terminal.key(KeyEvent::press(
        ghostty_vt_sys::GHOSTTY_KEY_SHIFT_LEFT,
        Mods::SHIFT,
    ));
    assert_eq!(sync(&terminal), []);
    terminal.copy(None);
    assert_eq!(sync(&terminal), [Output::Selection("line".into())]);

    terminal.key(KeyEvent::text(GHOSTTY_KEY_A, "a"));
    assert_eq!(sync(&terminal), [Output::Input(b"a".to_vec())]);
    let s = snapshot_where(&terminal, &wakes, |s| s.scrollbar.at_bottom);
    assert!(s.screen.iter().all(|row| row.selection.is_none()));
}

/// Smooth scrolls are numbered, and the first snapshot that has moved by one says so, even
/// one that didn't move (the top stopped it). Typing at the bottom puts the part row away.
#[test]
fn smooth_scrolls_are_numbered_and_typing_puts_the_screen_back_on_its_rows() {
    let (terminal, wakes) = spawn(Config {
        rows: 5,
        ..config()
    });
    let lines: Vec<u8> = (1..=20)
        .flat_map(|i| format!("line {i}\r\n").into_bytes())
        .collect();
    terminal.write(lines);
    let first = terminal.scroll_rows(2.5);
    let s = snapshot_where(&terminal, &wakes, |s| s.scrolled >= first);
    assert_eq!((s.scrolled, s.nudge), (first, 0.5));
    let second = terminal.scroll_rows(-2.0);
    assert_eq!(second, first + 1);
    let s = snapshot_where(&terminal, &wakes, |s| s.scrolled >= second);
    assert!(s.scrollbar.at_bottom);
    assert_eq!(s.nudge, 0.5, "half a row up from the bottom");

    terminal.key(KeyEvent::text(GHOSTTY_KEY_A, "a"));
    assert_eq!(sync(&terminal), [Output::Input(b"a".to_vec())]);
    snapshot_where(&terminal, &wakes, |s| s.nudge == 0.0);

    terminal.scroll(Scroll::Top);
    let third = terminal.scroll_rows(1.0);
    let s = snapshot_where(&terminal, &wakes, |s| s.scrolled >= third);
    assert_eq!((s.scrollbar.offset, s.nudge), (0, 0.0), "nowhere to go");
}

#[test]
fn input_goes_out_as_it_is_in_order_with_keys() {
    let (terminal, _wakes) = spawn(config());
    terminal.key(KeyEvent::text(GHOSTTY_KEY_A, "a"));
    terminal.input(b"\x1bb".to_vec());
    terminal.input(Vec::new());
    terminal.key(KeyEvent::text(GHOSTTY_KEY_A, "a"));
    assert_eq!(
        sync(&terminal),
        [
            Output::Input(b"a".to_vec()),
            Output::Input(b"\x1bb".to_vec()),
            Output::Input(b"a".to_vec())
        ]
    );
}

#[test]
fn a_selection_drag_held_past_the_top_scrolls_on_the_worker() {
    let (terminal, wakes) = spawn(Config {
        rows: 5,
        ..config()
    });
    let lines: Vec<u8> = (1..=40)
        .flat_map(|i| format!("line {i}\r\n").into_bytes())
        .collect();
    terminal.write(lines);
    let bottom = snapshot_where(&terminal, &wakes, |s| s.line(3).contains("line 40"));
    terminal.select(SelectEvent::new(SelectAction::Press, 1.0, 50.0));
    terminal.select(SelectEvent::new(SelectAction::Drag, 1.0, -40.0));
    // No more events: the worker keeps scrolling while the drag is held there.
    let s = snapshot_where(&terminal, &wakes, |s| {
        s.scrollbar.offset + 5 <= bottom.scrollbar.offset
    });
    assert!(s.screen[0].selection.is_some());
    terminal.select(SelectEvent::new(SelectAction::Release, 1.0, -40.0));
}

/// A playing kitty animation needs no output to move on: the worker wakes when the next frame is
/// due and publishes it, and once the program stops it, the snapshots stop.
#[test]
fn a_kitty_animation_plays_on_the_worker() {
    let (terminal, wakes) = spawn(config());
    // libghostty-vt's example: red for 30 ms, then blue for 40 ms, looping.
    terminal.write(
        concat!(
            "\x1b_Ga=T,f=32,i=2,s=1,v=1,q=2;/wAA/w==\x1b\\",
            "\x1b_Ga=f,f=32,i=2,s=1,v=1,z=40,q=2;AAD//w==\x1b\\",
            "\x1b_Ga=a,i=2,r=1,z=30,q=2\x1b\\",
            "\x1b_Ga=a,i=2,s=3,q=2\x1b\\",
        )
        .as_bytes()
        .to_vec(),
    );
    let first = |s: &Snapshot| s.images.first().map(|p| p.image.rgba[..4].to_vec());
    for color in [[255, 0, 0, 255], [0, 0, 255, 255], [255, 0, 0, 255]] {
        snapshot_where(&terminal, &wakes, |s| first(s) == Some(color.to_vec()));
    }
    terminal.write(b"\x1b_Ga=a,i=2,s=1,q=2\x1b\\".to_vec());
    sync(&terminal);
    // Take what the stop itself published, until the worker has nothing more to say.
    let settle = Instant::now() + Duration::from_millis(150);
    while Instant::now() < settle {
        let _ = terminal.take_snapshot();
        let _ = wakes.recv_timeout(Duration::from_millis(10));
    }
    let _ = terminal.take_snapshot();
    while wakes.try_recv().is_ok() {}
    // Stopped: no more frames come by themselves.
    assert!(wakes.recv_timeout(Duration::from_millis(200)).is_err());
}

/// Once `handled` returns true, the key handed over before it is already encoded: an input
/// handler that waits for it can send the bytes in the same frame.
#[test]
fn handled_means_the_keys_sent_before_are_in_the_output() {
    let (terminal, _wakes) = spawn(config());
    // The worker has started and is idle.
    terminal.snapshot_now(PATIENCE).expect("a first snapshot");
    terminal.key(KeyEvent::text(GHOSTTY_KEY_A, "a"));
    assert!(terminal.handled(PATIENCE));
    assert_eq!(
        terminal.output().try_recv(),
        Ok(Output::Input(b"a".to_vec()))
    );
}
