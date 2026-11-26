//! How the app reaches herdr-core: a [`Link`] the UI thread drives without blocking, and drains
//! once a frame (`docs/research/rune.md` §8 "State and event flow").
//!
//! [`Live`] is the real core, through herdr-client's supervisor (connect, subscribe, reconnect),
//! its machine watcher, and calls answered through `Pending::try_take`. Attach streams are read
//! on one thread of their own, which writes a terminal's output straight into its terminal
//! worker and returns the credit, so a hidden terminal's output never wakes the UI.
//!
//! [`Fake`] is an in-process core for headless scenes and tests: it answers document rounds
//! under compare-and-set and terminal calls, shows each terminal's recorded screen on attach,
//! and keeps every frame the app sent so a test can read them back.

mod fake;
mod live;

pub use fake::{Fake, FakeCore};
pub use live::Live;

use herdr_proto::methods::TerminalAttach;
use herdr_proto::{CloseReason, Display, ErrorKind, RpcError, Terminal};
use herdr_shared::Round;
use model::{Request, Token};

/// What the core said, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum News {
    /// A connection is up. Its snapshot follows as a resynced [`model::Event::Terminals`].
    Connected,
    /// The connection dropped; the supervisor reconnects.
    Lost(String),
    /// A connect attempt failed; the next one comes on its own.
    Unavailable(String),
    /// For the model: terminals, machines, document rounds and call replies.
    Event(model::Event),
    /// Kept terminals whose program came back: attach them again.
    Restarted(Vec<String>),
    /// A [`Link::attach`] finished: the terminal's record, or why it failed.
    Attached {
        address: String,
        result: Result<Box<Terminal>, String>,
    },
    /// An attach stream ended. `None`: the connection dropped under it.
    Ended {
        address: String,
        reason: Option<CloseReason>,
    },
}

/// A frame for one terminal's attach stream (`herdr-core-protocol.md` §4 and rule 9).
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    /// Human input, DATA: it takes the size.
    Input(Vec<u8>),
    /// The terminal engine's replies, AUTOMATIC_INPUT: not activity.
    Automatic(Vec<u8>),
    /// The pane's grid: it takes the size.
    Resize {
        cols: u16,
        rows: u16,
    },
    Focus(bool),
    /// Colors and the cell size, for the program's queries.
    Facts(Display),
}

/// The app's way to the core. Every method returns at once; answers come back through
/// [`Link::poll`].
pub trait Link {
    /// Starts connecting. `wake` is called from other threads when there is news.
    fn start(&mut self, wake: rune::Waker);

    /// Moves what arrived since the last call into `news`.
    fn poll(&mut self, news: &mut Vec<News>);

    /// A call the model asked for; answered as [`model::Event::Reply`].
    fn call(&mut self, token: Token, request: Request);

    /// A document round; answered as [`model::Event::DocRead`] or [`model::Event::DocWritten`].
    fn doc(&mut self, machine: Option<String>, round: Round);

    /// Attaches a terminal. Its output goes into `writer` (after a reset of the screen when
    /// `fresh`, for a repaint that replaces what it shows); the answer is [`News::Attached`].
    fn attach(&mut self, params: TerminalAttach, writer: terminal::Writer, fresh: bool);

    /// Sends a frame on a terminal's stream; dropped while it isn't attached.
    fn send(&mut self, address: &str, out: Out);

    /// Detaches a terminal.
    fn detach(&mut self, address: &str);
}

/// A link that never connects: what scenes and tests start with until they put a core in.
#[derive(Default)]
pub struct Offline;

impl Link for Offline {
    fn start(&mut self, _wake: rune::Waker) {}
    fn poll(&mut self, _news: &mut Vec<News>) {}
    fn call(&mut self, _token: Token, _request: Request) {}
    fn doc(&mut self, _machine: Option<String>, _round: Round) {}
    fn attach(&mut self, _params: TerminalAttach, _writer: terminal::Writer, _fresh: bool) {}
    fn send(&mut self, _address: &str, _out: Out) {}
    fn detach(&mut self, _address: &str) {}
}

/// An error that isn't the core's own, in the shape the model reads: the layout or call is
/// unavailable for now.
pub(crate) fn unavailable(message: impl Into<String>) -> RpcError {
    RpcError {
        code: -32000,
        message: message.into(),
        kind: ErrorKind::Unavailable,
        hint: None,
        data: Default::default(),
    }
}
