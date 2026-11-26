//! Attach and events streams: what the core sends on a channel, and what a viewer sends back.
//!
//! Credit (rule 2): the core sends a stream at most 256 KiB past what the client has credited.
//! An attach stream's owner credits each `Data` once its bytes are written into the VT
//! ([`AttachStream::credit`]); an [`EventStream`] credits each frame itself once it is parsed.

use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use std::time::Duration;

use herdr_proto::methods::AttachResult;
use herdr_proto::{
    CloseReason, Display, EventLines, Frame, Kind, Notification, StreamCaps, Terminal,
};

use crate::connection::Inner;

/// What arrives on a stream.
#[derive(Clone, Debug, PartialEq)]
pub enum StreamItem {
    /// Bytes: terminal output, or event lines. Credit them once consumed.
    Data(Vec<u8>),
    /// The real grid, for an observer whose attach reply confirmed `resize_notifications`. An
    /// authoritative repaint follows. It uses no credit.
    Resize { cols: u16, rows: u16 },
    /// The core ended the stream: the program exited, was killed or was lost (a kept terminal:
    /// wait for `restarted`, then attach again).
    Closed(CloseReason),
    /// The connection dropped. The terminal goes on; reattach on the next connection with
    /// `resumed: true`.
    Lost,
}

impl StreamItem {
    /// Bytes it holds, for buffering limits.
    pub(crate) fn len(&self) -> usize {
        match self {
            StreamItem::Data(bytes) => bytes.len(),
            _ => 0,
        }
    }

    /// Whether nothing follows it.
    pub fn is_end(&self) -> bool {
        matches!(self, StreamItem::Closed(_) | StreamItem::Lost)
    }
}

/// Receives a stream's items, and remembers when it ended so it isn't CLOSEd twice.
struct Items {
    rx: mpsc::Receiver<StreamItem>,
    ended: bool,
}

impl Items {
    fn seen(&mut self, item: StreamItem) -> StreamItem {
        if item.is_end() {
            self.ended = true;
        }
        item
    }

    fn try_recv(&mut self) -> Option<StreamItem> {
        if self.ended {
            return None;
        }
        match self.rx.try_recv() {
            Ok(item) => Some(self.seen(item)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(self.seen(StreamItem::Lost)),
        }
    }

    fn recv_timeout(&mut self, timeout: Duration) -> Option<StreamItem> {
        if self.ended {
            return None;
        }
        match self.rx.recv_timeout(timeout) {
            Ok(item) => Some(self.seen(item)),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => Some(self.seen(StreamItem::Lost)),
        }
    }
}

/// Sends a viewer's frames on one attach channel. Cheap to clone, so input can come from the UI
/// thread while a worker reads the stream.
#[derive(Clone)]
pub struct StreamControl {
    inner: Arc<Inner>,
    channel: u32,
    caps: StreamCaps,
}

impl StreamControl {
    pub fn channel(&self) -> u32 {
        self.channel
    }

    /// What this stream may use: the feature in `hello` and in the attach reply.
    pub fn caps(&self) -> StreamCaps {
        self.caps
    }

    /// The connection generation it belongs to.
    pub fn generation(&self) -> u64 {
        self.inner.generation()
    }

    /// Human input, as DATA (rule 9). It counts as activity: it takes the size.
    pub fn input(&self, bytes: &[u8]) {
        self.inner.send_split(self.channel, Kind::Data, bytes);
    }

    /// Replies the terminal engine generated, as AUTOMATIC_INPUT when both `hello` and the attach
    /// reply allow it, else as DATA (rule 9). Not activity.
    pub fn automatic(&self, bytes: &[u8]) {
        let kind = if self.caps.automatic_input {
            Kind::AutomaticInput
        } else {
            Kind::Data
        };
        self.inner.send_split(self.channel, kind, bytes);
    }

    /// RESIZE with the pane's grid. Sending it takes the size (§4), even when unchanged.
    pub fn resize(&self, cols: u16, rows: u16) {
        self.inner.send(Frame::resize(self.channel, cols, rows));
    }

    /// FOCUS. Gaining focus is activity and applies the size this viewer last sent, so send
    /// RESIZE with the current grid first.
    pub fn focus(&self, gained: bool) {
        self.inner.send(Frame::focus(self.channel, gained));
    }

    /// FACTS: colors and cell size, for the program's queries.
    pub fn facts(&self, display: &Display) {
        self.inner.send(Frame::facts(self.channel, display));
    }

    /// Returns credit for `bytes` consumed (written into the VT).
    pub fn credit(&self, bytes: usize) {
        credit(&self.inner, self.channel, bytes);
    }
}

fn credit(inner: &Inner, channel: u32, mut bytes: usize) {
    while bytes > 0 {
        let n = bytes.min(u32::MAX as usize);
        inner.send(Frame::credit(channel, n as u32));
        bytes -= n;
    }
}

/// One `terminal.attach` stream. Dropping it detaches (CLOSE), unless the core already ended it.
pub struct AttachStream {
    control: StreamControl,
    terminal: Terminal,
    items: Items,
}

impl AttachStream {
    pub(crate) fn new(
        inner: Arc<Inner>,
        result: AttachResult,
        caps: StreamCaps,
        rx: mpsc::Receiver<StreamItem>,
    ) -> Self {
        AttachStream {
            control: StreamControl {
                inner,
                channel: result.channel,
                caps,
            },
            terminal: result.terminal,
            items: Items { rx, ended: false },
        }
    }

    pub fn channel(&self) -> u32 {
        self.control.channel
    }

    /// The terminal's record at attach time.
    pub fn terminal(&self) -> &Terminal {
        &self.terminal
    }

    pub fn caps(&self) -> StreamCaps {
        self.control.caps
    }

    pub fn generation(&self) -> u64 {
        self.control.generation()
    }

    /// A handle for sending on this stream from another thread.
    pub fn control(&self) -> StreamControl {
        self.control.clone()
    }

    /// The next item if one is waiting. `None` after the end.
    pub fn try_recv(&mut self) -> Option<StreamItem> {
        self.items.try_recv()
    }

    /// The next item, waiting up to `timeout`. `None` on timeout and after the end.
    pub fn recv_timeout(&mut self, timeout: Duration) -> Option<StreamItem> {
        self.items.recv_timeout(timeout)
    }

    /// Whether the stream has ended (`Closed` or `Lost` was received).
    pub fn is_ended(&self) -> bool {
        self.items.ended
    }

    pub fn input(&self, bytes: &[u8]) {
        self.control.input(bytes);
    }

    pub fn automatic(&self, bytes: &[u8]) {
        self.control.automatic(bytes);
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        self.control.resize(cols, rows);
    }

    pub fn focus(&self, gained: bool) {
        self.control.focus(gained);
    }

    pub fn facts(&self, display: &Display) {
        self.control.facts(display);
    }

    pub fn credit(&self, bytes: usize) {
        self.control.credit(bytes);
    }

    /// Detaches now.
    pub fn close(self) {}
}

impl Drop for AttachStream {
    fn drop(&mut self) {
        if !self.items.ended {
            self.control.inner.detach(self.control.channel);
        }
    }
}

/// What an [`EventStream`] yields.
#[derive(Clone, Debug, PartialEq)]
pub enum EventItem {
    Notification(Notification),
    /// A line that didn't parse; the stream goes on.
    Malformed(String),
    /// The core ended the subscription.
    Closed(CloseReason),
    /// The connection dropped: subscribe again on the next one for a fresh snapshot.
    Lost,
}

/// One `events.subscribe` stream. It credits bytes as it parses them. Dropping it unsubscribes.
pub struct EventStream {
    inner: Arc<Inner>,
    channel: u32,
    items: Items,
    lines: EventLines,
    /// The end, once seen, after the lines before it are out.
    end: Option<EventItem>,
}

impl EventStream {
    pub(crate) fn new(inner: Arc<Inner>, channel: u32, rx: mpsc::Receiver<StreamItem>) -> Self {
        EventStream {
            inner,
            channel,
            items: Items { rx, ended: false },
            lines: EventLines::new(),
            end: None,
        }
    }

    pub fn channel(&self) -> u32 {
        self.channel
    }

    pub fn generation(&self) -> u64 {
        self.inner.generation()
    }

    /// The next notification if one is ready. `None` when nothing is, and after the end.
    pub fn try_next(&mut self) -> Option<EventItem> {
        self.next_with(|items| items.try_recv())
    }

    /// The next notification, waiting up to `timeout`.
    pub fn next_timeout(&mut self, timeout: Duration) -> Option<EventItem> {
        self.next_with(|items| items.recv_timeout(timeout))
    }

    fn next_with(
        &mut self,
        mut recv: impl FnMut(&mut Items) -> Option<StreamItem>,
    ) -> Option<EventItem> {
        loop {
            if let Some(next) = self.lines.next_notification() {
                return Some(match next {
                    Ok(notification) => EventItem::Notification(notification),
                    Err(e) => EventItem::Malformed(e.to_string()),
                });
            }
            if let Some(end) = self.end.take() {
                return Some(end);
            }
            match recv(&mut self.items)? {
                StreamItem::Data(bytes) => {
                    self.lines.push(&bytes);
                    credit(&self.inner, self.channel, bytes.len());
                }
                StreamItem::Resize { .. } => {}
                StreamItem::Closed(reason) => self.end = Some(EventItem::Closed(reason)),
                StreamItem::Lost => self.end = Some(EventItem::Lost),
            }
        }
    }
}

impl Drop for EventStream {
    fn drop(&mut self) {
        if !self.items.ended {
            self.inner.detach(self.channel);
        }
    }
}
