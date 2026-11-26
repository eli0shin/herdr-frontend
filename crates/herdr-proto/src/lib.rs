//! Pure herdr-core protocol types, with no I/O (`docs/research/herdr-core-protocol.md` §7).
//!
//! - [`frame`]: the frame codec and the incremental [`Decoder`].
//! - [`rpc`]: the JSON-RPC envelope, [`RpcError`] and [`ErrorKind`].
//! - [`hello`]: the handshake, [`Capabilities`] and the compatibility check.
//! - [`methods`]: typed params and results, one [`Call`] per method.
//! - [`event`]: notifications on an events stream, and the line splitter.
//! - [`terminal_set`]: [`TerminalSet::apply`], the event rules, `seq` and `missed`.
//!
//! Every record keeps the fields it doesn't know in `extra`, and every word it doesn't know as
//! `Other`, so what a newer core sends survives.

pub mod event;
pub mod frame;
pub mod hello;
pub mod methods;
pub mod rpc;
pub mod terminal_set;
pub mod types;
mod words;

pub use event::{Event, EventData, EventKind, EventLines, LineSplitter, Notification, Snapshot};
pub use frame::{CloseReason, Decoder, Frame, FrameError, Kind};
pub use hello::{
    Capabilities, GENERATION, Hello, HelloParams, MIN_CORE, Mismatch, StreamCaps, Version,
    hello_line,
};
pub use rpc::{Call, ErrorKind, Id, Incoming, Request, Response, RpcError, cancel_request};
pub use terminal_set::{Changes, DocKey, TerminalSet};
pub use types::{
    Display, Foreground, Labels, Reason, Status, StatusState, Terminal, TerminalState,
};
