//! The libghostty-vt wrapper: one worker thread per terminal, the snapshots it hands the UI, the
//! input encoders (`docs/research/terminal-rendering.md` §2 "Data flow", stage 2 of §8), and the
//! cell metrics.
//!
//! A [`Terminal`] owns a worker thread that runs a [`Vt`]. The app sends it the program's output
//! ([`Terminal::write`]) and user input to encode ([`Terminal::key`], [`Terminal::mouse`],
//! [`Terminal::paste`]). The worker parses on its own thread, so the UI thread never touches the
//! emulator, and hands back:
//!
//! - **Snapshots**, at most one per UI frame. The worker publishes a [`Snapshot`] into a one-slot
//!   handoff only after the UI took the previous one, and calls the waker. Output that arrives in
//!   the meantime is folded into the next snapshot, so a busy program never queues frames.
//! - **[`Output`]** on a channel, in order: query replies (send them to herdr-core as
//!   AUTOMATIC_INPUT), encoded user input (DATA), and [`Event`]s (title, bell, clipboard, ...).
//!
//! Another thread (the one reading the core's attach stream) feeds output through a
//! [`Writer`]. [`Terminal::snapshot_now`] waits for a settled screen, for tests and scenes.
//!
//! A [`Vt`] can also be driven directly, with no thread, as the replay tests do.
//!
//! The cell metrics are ported from Ghostty, so a cell here is the same whole-pixel cell Ghostty
//! draws ([`face`], [`metrics`]), with the grid and padding math in [`size`].

pub mod face;
pub mod metrics;
pub mod size;

mod input;
mod snapshot;
mod vt;

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

pub use face::{DesiredSize, FaceError};
pub use metrics::{FaceMetrics, Metrics, Modifier, Modifiers};
pub use size::{CellSize, GridSize, Padding, PaddingBalance, ScreenSize, Size, SizeReport};

pub use input::{
    Focus, Key, KeyAction, KeyEvent, Mods, MouseAction, MouseButton, MouseEvent, OptionAsAlt,
    PasteRejected, encode_focus,
};
pub use snapshot::{
    ActiveScreen, Cell, CellFlags, Colors, Cursor, CursorStyle, Dirty, Grapheme, Modes, Rgb, Row,
    RowId, Scrollbar, Snapshot, Underline, Width,
};
pub use vt::{
    ClipboardLocation, ColorScheme, Config, Event, Progress, Scroll, Scrollback, Theme, Vt,
};

/// Called from the worker thread when there is something new for the UI: a snapshot to take or
/// output to drain. It should only schedule a frame (Rune's proxy wake, for example).
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// What the worker sends out, in the order of the requests that caused it. Within one write, its
/// replies come before its events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// The terminal's answers to the program's queries, and mode 2048 size reports. Send them
    /// as AUTOMATIC_INPUT, so the core doesn't count them as the user typing.
    Reply(Vec<u8>),
    /// User input encoded for the program's current modes: keys, mouse reports, pastes. Send
    /// them as DATA.
    Input(Vec<u8>),
    Event(Event),
}

/// The most the worker parses before it offers a frame, so a flood of output still shows.
const BATCH_BYTES: usize = 1 << 20;

enum Request {
    Write(Vec<u8>),
    Resize {
        cols: u16,
        rows: u16,
        cell_width: u32,
        cell_height: u32,
    },
    Scroll(Scroll),
    Scrollback(Scrollback),
    Theme(Theme),
    Reset,
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste {
        text: String,
        allow_unsafe: bool,
    },
    /// The UI took the snapshot in the handoff and the worker has a newer frame waiting.
    Publish,
    /// Snapshot everything handled so far, now, and empty the handoff.
    SnapshotNow(Sender<Snapshot>),
    /// The terminal is dropped: stop, even while [`Writer`]s keep the channel open.
    Stop,
}

/// The one-slot handoff between the worker and the UI.
#[derive(Default)]
struct Handoff {
    snapshot: Option<Snapshot>,
    /// The worker has a frame newer than `snapshot` and waits for the slot to empty.
    waiting: bool,
}

/// A terminal on its own worker thread. Dropping it stops the thread.
pub struct Terminal {
    requests: Sender<Request>,
    handoff: Arc<Mutex<Handoff>>,
    output: Receiver<Output>,
    worker: Option<JoinHandle<()>>,
}

impl Terminal {
    /// Starts the worker. The first snapshot is ready as soon as the waker first runs.
    pub fn spawn(config: Config, waker: Waker) -> Terminal {
        let (requests, inbox) = mpsc::channel();
        let (outbox, output) = mpsc::channel();
        let handoff = Arc::new(Mutex::new(Handoff::default()));
        let shared = handoff.clone();
        let worker = thread::Builder::new()
            .name("terminal".into())
            .spawn(move || {
                // A `Vt` never leaves the thread that made it.
                Worker {
                    vt: Vt::new(&config),
                    inbox,
                    outbox,
                    handoff: shared,
                    waker,
                    changed: true,
                }
                .run()
            })
            .expect("spawn the terminal worker");
        Terminal {
            requests,
            handoff,
            output,
            worker: Some(worker),
        }
    }

    /// Feeds the program's output (herdr-core DATA frames).
    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        self.writer().write(bytes);
    }

    /// A handle another thread feeds the program's output through (the thread that reads the
    /// core's attach stream). Requests from one thread arrive in the order it sent them, through
    /// this handle or the terminal itself.
    pub fn writer(&self) -> Writer {
        Writer {
            requests: self.requests.clone(),
        }
    }

    /// Resizes the grid; the cell is in whole device pixels. A program with mode 2048 on gets
    /// a size report, which comes out as an [`Output::Reply`].
    pub fn resize(&self, cols: u16, rows: u16, cell_width: u32, cell_height: u32) {
        self.send(Request::Resize {
            cols,
            rows,
            cell_width,
            cell_height,
        });
    }

    pub fn scroll(&self, scroll: Scroll) {
        self.send(Request::Scroll(scroll));
    }

    pub fn set_scrollback(&self, limits: Scrollback) {
        self.send(Request::Scrollback(limits));
    }

    pub fn set_theme(&self, theme: Theme) {
        self.send(Request::Theme(theme));
    }

    pub fn reset(&self) {
        self.send(Request::Reset);
    }

    /// Encodes a key for the program's modes at the moment the worker gets it, and sends the
    /// bytes out as [`Output::Input`] (nothing for a key that sends nothing).
    pub fn key(&self, event: KeyEvent) {
        self.send(Request::Key(event));
    }

    /// Encodes a mouse event as [`Output::Input`], if the program tracks the mouse.
    pub fn mouse(&self, event: MouseEvent) {
        self.send(Request::Mouse(event));
    }

    /// Encodes a paste as [`Output::Input`]. Text that could run commands comes back as
    /// [`Event::PasteRejected`] instead unless `allow_unsafe`.
    pub fn paste(&self, text: impl Into<String>, allow_unsafe: bool) {
        self.send(Request::Paste {
            text: text.into(),
            allow_unsafe,
        });
    }

    /// The newest frame, if there is one the UI hasn't taken. Call it once per frame.
    pub fn take_snapshot(&self) -> Option<Snapshot> {
        let mut handoff = self.handoff.lock().expect("handoff lock");
        let snapshot = handoff.snapshot.take();
        if snapshot.is_some() && std::mem::take(&mut handoff.waiting) {
            drop(handoff);
            self.send(Request::Publish);
        }
        snapshot
    }

    /// A snapshot of everything this thread sent the worker so far, waiting up to `timeout` for
    /// it. The handoff is emptied, so the next [`Terminal::take_snapshot`] has only what changes
    /// after. It blocks: for tests and scenes that need a settled screen, never a UI frame.
    pub fn snapshot_now(&self, timeout: std::time::Duration) -> Option<Snapshot> {
        let (tx, rx) = mpsc::channel();
        self.send(Request::SnapshotNow(tx));
        rx.recv_timeout(timeout).ok()
    }

    /// Replies, encoded input and events, in order. Drain it whenever the waker runs.
    pub fn output(&self) -> &Receiver<Output> {
        &self.output
    }

    fn send(&self, request: Request) {
        // The worker only stops when this handle drops, or if it panicked; then there is no one
        // left to tell.
        let _ = self.requests.send(request);
    }
}

/// Feeds a [`Terminal`]'s worker from another thread. Cheap to clone; once the terminal is
/// dropped, writes go nowhere.
#[derive(Clone)]
pub struct Writer {
    requests: Sender<Request>,
}

impl Writer {
    /// The program's output, as [`Terminal::write`].
    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        let bytes = bytes.into();
        if !bytes.is_empty() {
            let _ = self.requests.send(Request::Write(bytes));
        }
    }

    /// Starts the screen over, as [`Terminal::reset`]: before a repaint that replaces it.
    pub fn reset(&self) {
        let _ = self.requests.send(Request::Reset);
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // A `Writer` may still hold the channel open, so say stop; closing it ends the loop
        // too, for a worker that never reads the request.
        self.send(Request::Stop);
        let (closed, _) = mpsc::channel();
        drop(std::mem::replace(&mut self.requests, closed));
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Worker {
    vt: Vt,
    inbox: Receiver<Request>,
    outbox: Sender<Output>,
    handoff: Arc<Mutex<Handoff>>,
    waker: Waker,
    /// Something changed since the last published snapshot.
    changed: bool,
}

impl Worker {
    fn run(mut self) {
        self.publish();
        loop {
            // Sleep until a request comes, or until a render hold has to end.
            let deadline = self.vt.hold_deadline();
            let first = match deadline {
                Some(deadline) => {
                    match self
                        .inbox
                        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    {
                        Ok(request) => Some(request),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                None => match self.inbox.recv() {
                    Ok(request) => Some(request),
                    Err(_) => return,
                },
            };
            // Handle everything already queued, up to a batch of output, then offer one frame.
            let mut parsed = 0;
            let mut next = first;
            while let Some(request) = next {
                match &request {
                    Request::Write(bytes) => parsed += bytes.len(),
                    Request::Stop => return,
                    _ => {}
                }
                self.handle(request);
                next = if parsed < BATCH_BYTES {
                    self.inbox.try_recv().ok()
                } else {
                    None
                };
            }
            if self.vt.expire_hold(Instant::now()) {
                self.changed = true;
            }
            self.flush();
            self.publish();
        }
    }

    fn handle(&mut self, request: Request) {
        let changed = match request {
            Request::Write(bytes) => {
                self.vt.write(&bytes);
                true
            }
            Request::Resize {
                cols,
                rows,
                cell_width,
                cell_height,
            } => {
                self.vt.resize(cols, rows, cell_width, cell_height);
                true
            }
            Request::Scroll(scroll) => {
                self.vt.scroll(scroll);
                true
            }
            Request::Scrollback(limits) => {
                self.vt.set_scrollback(limits);
                true
            }
            Request::Theme(theme) => {
                self.vt.set_theme(&theme);
                true
            }
            Request::Reset => {
                self.vt.reset();
                true
            }
            Request::Key(event) => {
                // Replies queued so far go out before the key, as the program asked first.
                self.flush();
                let bytes = self.vt.encode_key(&event);
                self.input(bytes);
                false
            }
            Request::Mouse(event) => {
                self.flush();
                let bytes = self.vt.encode_mouse(&event);
                self.input(bytes);
                false
            }
            Request::Paste { text, allow_unsafe } => {
                self.flush();
                match self.vt.paste(&text, allow_unsafe) {
                    Ok(bytes) => self.input(bytes),
                    Err(rejected) => self.out(Output::Event(Event::PasteRejected(rejected))),
                }
                false
            }
            Request::Publish | Request::Stop => false,
            Request::SnapshotNow(reply) => {
                self.flush();
                {
                    let mut handoff = self.handoff.lock().expect("handoff lock");
                    handoff.snapshot = None;
                    handoff.waiting = false;
                }
                self.changed = false;
                let _ = reply.send(self.vt.snapshot());
                return;
            }
        };
        self.changed |= changed;
        // Replies leave as soon as their write is parsed, so the program isn't kept waiting
        // for a frame.
        self.flush();
    }

    /// Sends the replies and events queued in the `Vt`.
    fn flush(&mut self) {
        let replies = self.vt.take_replies();
        let events = self.vt.take_events();
        if !replies.is_empty() {
            self.out(Output::Reply(replies));
        }
        for event in events {
            self.out(Output::Event(event));
        }
    }

    fn input(&mut self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            self.out(Output::Input(bytes));
        }
    }

    fn out(&mut self, output: Output) {
        // The receiver lives in the `Terminal`, which outlives this thread.
        let _ = self.outbox.send(output);
        (self.waker)();
    }

    /// Puts a new snapshot in the handoff if the frame changed, the program isn't holding it,
    /// and the UI took the last one. Otherwise asks to be told when the slot empties.
    fn publish(&mut self) {
        if !self.changed || !self.vt.can_show() {
            return;
        }
        {
            // One lock for the check and the flag, so the UI can't take the snapshot in between
            // and never send `Publish`.
            let mut handoff = self.handoff.lock().expect("handoff lock");
            if handoff.snapshot.is_some() {
                handoff.waiting = true;
                return;
            }
        }
        // Build outside the lock: only this thread fills the slot, so it stays empty.
        let snapshot = self.vt.snapshot();
        self.changed = false;
        self.handoff.lock().expect("handoff lock").snapshot = Some(snapshot);
        (self.waker)();
    }
}

impl Metrics {
    /// The cell to lay the grid out with.
    pub fn cell_size(&self) -> CellSize {
        CellSize {
            width: self.cell_width,
            height: self.cell_height,
        }
    }
}
