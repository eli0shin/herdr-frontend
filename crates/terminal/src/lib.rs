//! The libghostty-vt wrapper: one worker thread per terminal, the snapshots it hands the UI, the
//! input encoders (`docs/research/terminal-rendering.md` §2 "Data flow", stage 2 of §8), and the
//! cell metrics.
//!
//! A [`Terminal`] owns a worker thread that runs a [`Vt`]. The app sends it the program's output
//! ([`Terminal::write`]), user input to encode ([`Terminal::key`], [`Terminal::mouse`],
//! [`Terminal::paste`], [`Terminal::input`]) and mouse selections ([`Terminal::select`],
//! [`Terminal::copy`]). The worker parses on its own thread, so the UI thread never touches the
//! emulator, and hands back:
//!
//! - **Snapshots**, at most one per UI frame. The worker publishes a [`Snapshot`] into a one-slot
//!   handoff only after the UI took the previous one, and calls the waker. Output that arrives in
//!   the meantime is folded into the next snapshot, so a busy program never queues frames.
//! - **[`Output`]** on a channel, in order: query replies (send them to herdr-core as
//!   AUTOMATIC_INPUT), encoded user input (DATA), selected text to copy, and [`Event`]s (title,
//!   bell, clipboard, ...).
//!
//! Another thread (the one reading the core's attach stream) feeds output through a
//! [`Writer`]. [`Terminal::snapshot_now`] waits for a settled screen, for tests and scenes.
//!
//! A [`Vt`] can also be driven directly, with no thread, as the replay tests do.
//!
//! The cell metrics are ported from Ghostty, so a cell here is the same whole-pixel cell Ghostty
//! draws ([`face`], [`metrics`]), with the grid and padding math in [`size`].

pub mod face;
pub mod links;
pub mod metrics;
pub mod size;

mod input;
mod snapshot;
mod vt;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

pub use face::{DesiredSize, FaceError};
pub use metrics::{FaceMetrics, Metrics, Modifier, Modifiers};
pub use size::{CellSize, GridSize, Padding, PaddingBalance, ScreenSize, Size, SizeReport};

pub use input::{
    Focus, Key, KeyAction, KeyEvent, Mods, MouseAction, MouseButton, MouseEvent, OptionAsAlt,
    PasteRejected, SelectAction, SelectEvent, encode_focus,
};
pub use snapshot::{
    ActiveScreen, Cell, CellFlags, Colors, CopyView, Cursor, CursorStyle, Dirty, Found, Grapheme,
    Match, Modes, Rgb, Row, RowId, Scrollbar, Snapshot, Span, Underline, Width,
};
pub use vt::{
    AUTOSCROLL_EVERY, ClipboardLocation, ColorScheme, Config, Event, Link, Progress, Scroll,
    Scrollback, Theme, Vt,
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
    /// The selected text, for the clipboard ([`Terminal::copy`]).
    Selection(String),
    /// The link a [`Terminal::open_link`] found.
    Link(Link),
    /// What a copy-mode yank copied, for the clipboard. A yank that copies nothing sends
    /// nothing and leaves its reason in [`Snapshot::copy`]; copy mode goes on.
    Yanked(String),
    Event(Event),
}

/// The most the worker parses before it offers a frame, so a flood of output still shows.
const BATCH_BYTES: usize = 1 << 20;

/// Called on the worker thread once a write's bytes are parsed: where a viewer returns the
/// core's credit for them (`herdr-core-protocol.md` rule 2).
pub type Parsed = Box<dyn FnOnce() + Send>;

enum Request {
    Write(Vec<u8>, Option<Parsed>),
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
    Input(Vec<u8>),
    Select(SelectEvent),
    ClearSelection,
    SelectAll,
    Copy(Option<KeyEvent>),
    Find(String),
    FindNext {
        back: bool,
    },
    Hover(Option<(f32, f32)>, Option<String>),
    OpenLink {
        x: f32,
        y: f32,
    },
    CopyStart,
    CopyRun(copy_mode::Command),
    CopyEnd,
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
    /// Bytes written and not parsed yet.
    pending: Arc<AtomicUsize>,
    /// Set on drop: the worker stops at its next request, however many are queued.
    stopping: Arc<AtomicBool>,
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
        let pending = Arc::new(AtomicUsize::new(0));
        let stopping = Arc::new(AtomicBool::new(false));
        let (worker_pending, worker_stopping) = (pending.clone(), stopping.clone());
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
                    pending: worker_pending,
                    stopping: worker_stopping,
                }
                .run()
            })
            .expect("spawn the terminal worker");
        Terminal {
            requests,
            pending,
            stopping,
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
            pending: self.pending.clone(),
        }
    }

    /// Bytes written and not parsed yet: the worker's backlog.
    pub fn pending_bytes(&self) -> usize {
        self.pending.load(Ordering::Acquire)
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

    /// Bytes for the program as they are, as [`Output::Input`] in order with the keys around
    /// them: a key binding's `send:` text, or what an input method composed.
    pub fn input(&self, bytes: impl Into<Vec<u8>>) {
        let bytes = bytes.into();
        if !bytes.is_empty() {
            self.send(Request::Input(bytes));
        }
    }

    /// One step of a mouse selection, for a program that doesn't take the mouse. A drag held
    /// past the grid's top or bottom keeps scrolling on the worker until it moves back or ends.
    pub fn select(&self, event: SelectEvent) {
        self.send(Request::Select(event));
    }

    pub fn clear_selection(&self) {
        self.send(Request::ClearSelection);
    }

    /// Selects the whole scrollback and screen.
    pub fn select_all(&self) {
        self.send(Request::SelectAll);
    }

    /// Searches the screen and scrollback for `needle` as output keeps coming; empty stops.
    /// Snapshots carry the counts ([`Snapshot::find`]) and mark the hits on their rows.
    pub fn find(&self, needle: impl Into<String>) {
        self.send(Request::Find(needle.into()));
    }

    /// Selects the next hit up into history, or with `back` the next one down, scrolling to it.
    pub fn find_next(&self, back: bool) {
        self.send(Request::FindNext { back });
    }

    /// The pointer, in device pixels from the grid's top left, while Cmd is held: the link
    /// under it is underlined ([`Snapshot::link`]). `None` takes the underline away. `address`
    /// is the word under it ([`links::word`]) when the caller found it names a live terminal:
    /// that word is underlined rather than a link, for as long as it's still the word there.
    pub fn hover(&self, at: Option<(f32, f32)>, address: Option<String>) {
        self.send(Request::Hover(at, address));
    }

    /// Looks up the link at a pixel position: [`Output::Link`] if there is one.
    pub fn open_link(&self, x: f32, y: f32) {
        self.send(Request::OpenLink { x, y });
    }

    /// The selected text, as [`Output::Selection`]. With nothing selected, `or_key` is encoded
    /// instead, as [`Terminal::key`] would (`cmd-c` reaches the program when there's nothing
    /// to copy).
    pub fn copy(&self, or_key: Option<KeyEvent>) {
        self.send(Request::Copy(or_key));
    }

    /// Starts copy mode at the program's cursor (`docs/research/terminal-rendering.md` §6).
    /// Snapshots then carry its cursor and [`Snapshot::copy`].
    pub fn copy_mode_start(&self) {
        self.send(Request::CopyStart);
    }

    /// Runs a copy-mode command on the terminal's buffer. A yank that copies text comes out
    /// as [`Output::Yanked`].
    pub fn copy_mode(&self, command: copy_mode::Command) {
        self.send(Request::CopyRun(command));
    }

    /// Leaves copy mode: the selection goes and the view returns to the bottom.
    pub fn copy_mode_end(&self) {
        self.send(Request::CopyEnd);
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
    pending: Arc<AtomicUsize>,
}

impl Writer {
    /// The program's output, as [`Terminal::write`].
    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        let bytes = bytes.into();
        if !bytes.is_empty() {
            self.send(bytes, None);
        }
    }

    /// The program's output, and `parsed` to run on the worker once it's parsed (even when
    /// `bytes` is empty). A dropped terminal runs it never.
    pub fn write_then(&self, bytes: impl Into<Vec<u8>>, parsed: Parsed) {
        self.send(bytes.into(), Some(parsed));
    }

    fn send(&self, bytes: Vec<u8>, parsed: Option<Parsed>) {
        let n = bytes.len();
        self.pending.fetch_add(n, Ordering::AcqRel);
        if self.requests.send(Request::Write(bytes, parsed)).is_err() {
            self.pending.fetch_sub(n, Ordering::AcqRel);
        }
    }

    /// Starts the screen over, as [`Terminal::reset`]: before a repaint that replaces it.
    pub fn reset(&self) {
        let _ = self.requests.send(Request::Reset);
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // A `Writer` may still hold the channel open, so say stop, ahead of whatever output is
        // queued; closing the channel ends the loop too.
        self.stopping.store(true, Ordering::Release);
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
    pending: Arc<AtomicUsize>,
    stopping: Arc<AtomicBool>,
}

impl Worker {
    fn run(mut self) {
        self.publish();
        loop {
            // Sleep until a request comes, until a render hold has to end, or until a selection
            // drag held past an edge scrolls again.
            let deadline = match (self.vt.hold_deadline(), self.vt.autoscroll_deadline()) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
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
                if self.stopping.load(Ordering::Acquire) {
                    return;
                }
                match &request {
                    Request::Write(bytes, _) => parsed += bytes.len(),
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
            if self.vt.autoscroll(Instant::now()) {
                self.changed = true;
            }
            self.flush();
            self.publish();
        }
    }

    fn handle(&mut self, request: Request) {
        let changed = match request {
            Request::Write(bytes, parsed) => {
                self.vt.write(&bytes);
                self.pending.fetch_sub(bytes.len(), Ordering::AcqRel);
                if let Some(parsed) = parsed {
                    parsed();
                }
                !bytes.is_empty()
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
                let typed = !bytes.is_empty() && event.action != KeyAction::Release;
                self.input(bytes);
                typed && self.typed()
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
                    Ok(bytes) if !bytes.is_empty() => {
                        self.input(bytes);
                        self.typed()
                    }
                    Ok(_) => false,
                    Err(rejected) => {
                        self.out(Output::Event(Event::PasteRejected(rejected)));
                        false
                    }
                }
            }
            Request::Input(bytes) => {
                self.flush();
                self.input(bytes);
                self.typed()
            }
            Request::Select(event) => {
                self.vt.select(&event, Instant::now());
                true
            }
            Request::ClearSelection => {
                let had = self.vt.has_selection();
                self.vt.clear_selection();
                had
            }
            Request::SelectAll => {
                self.vt.select_all();
                true
            }
            Request::Find(needle) => {
                let was = self.vt.finding().map(str::to_owned);
                self.vt.find(&needle);
                was.as_deref() != self.vt.finding()
            }
            Request::FindNext { back } => {
                self.vt.find_next(back);
                true
            }
            Request::Hover(at, address) => self.vt.hover(at, address),
            Request::OpenLink { x, y } => {
                if let Some(link) = self.vt.link_at(x, y) {
                    self.out(Output::Link(link));
                }
                false
            }
            Request::Copy(or_key) => {
                self.flush();
                match (self.vt.selection_text(), or_key) {
                    (Some(text), _) => self.out(Output::Selection(text)),
                    (None, Some(key)) => {
                        let bytes = self.vt.encode_key(&key);
                        self.input(bytes);
                    }
                    (None, None) => {}
                }
                false
            }
            Request::CopyStart => {
                self.vt.copy_start();
                true
            }
            Request::CopyRun(command) => {
                if let Some(text) = self.vt.copy_run(&command) {
                    self.out(Output::Yanked(text));
                }
                true
            }
            Request::CopyEnd => {
                let was = self.vt.copying();
                self.vt.copy_end();
                was
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

    /// The person typed into the program, as Ghostty and Herdr app treat it: the selection
    /// goes and the viewport returns to the bottom. True if either changed the screen.
    fn typed(&mut self) -> bool {
        let selected = self.vt.has_selection();
        if selected {
            self.vt.clear_selection();
        }
        let scrolled = !self.vt.at_bottom();
        if scrolled {
            self.vt.scroll(Scroll::Bottom);
        }
        selected || scrolled
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
