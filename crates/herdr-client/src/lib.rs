//! herdr-core client I/O (`docs/research/herdr-core-protocol.md` §1 and §7).
//!
//! - [`Connection`]: one framed connection over `hc connect` or the core's socket
//!   ([`Transport`]), with the hello handshake and the capability check (rule 10), calls routed
//!   by id with timeouts and `$/cancelRequest`, and attach and events streams.
//! - [`AttachStream`] and [`EventStream`]: a terminal's bytes and the events, with credit
//!   (rule 2). Input, automatic input, resize, focus and facts go back on an attach stream.
//! - [`Supervisor`]: connects, subscribes for a fresh snapshot, pumps events into a
//!   `TerminalSet`, and reconnects with backoff, a new generation each time.
//! - [`MachineWatcher`]: polls `machine.list` for line state.
//!
//! There is no async runtime. Work happens on the connection's own threads; results come back
//! through channels and [`Pending`] (wait, poll or await), and a [`Wake`] hook says when to look.
//! Blocking calls (`Pending::wait`, `Connection::open`) never belong on the UI thread.

use std::sync::Arc;

mod connection;
mod error;
mod machines;
mod pending;
mod stream;
mod supervisor;
mod transport;

pub use connection::{CALL_TIMEOUT, ConnectOptions, Connection, EVENTS_EXTRA_CREDIT, NEEDS, USES};
pub use error::{Error, StartupError};
pub use machines::{MachineStatus, MachineUpdate, MachineWatcher, Machines, POLL_EVERY};
pub use pending::Pending;
pub use stream::{AttachStream, EventItem, EventStream, StreamControl, StreamItem};
pub use supervisor::{Backoff, Supervisor, SupervisorHandle, SupervisorOptions, Update};
#[cfg(unix)]
pub use transport::socket_path;
pub use transport::{HcCommand, Transport, find_hc, hc_version};

/// Called from a client thread when something is ready to look at. Rune's `Waker` is this type.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// A wake hook that does nothing, for consumers that block on their channel.
pub fn no_wake() -> Wake {
    Arc::new(|| {})
}
