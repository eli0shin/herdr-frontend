//! Herdr's shared conventions as pure logic, with no I/O (`docs/research/herdr-core-protocol.md`
//! §3, §5 and §7). herdr-frontend shares a core with a running Herdr app; these rules are what
//! keep the two from breaking each other's layout, labels and attention.
//!
//! - [`document`]: the lossless `herdr-app` layout [`Document`], `qualify`/`local`, fresh ids.
//! - [`tree`]: node and document operations, as herdr-app does them.
//! - [`intent`]: edits as [`Intent`]s, idempotent so they replay safely.
//! - [`mirror`]: one [`Mirror`] per machine: version, one round in flight, replay on conflict.
//! - [`labels`]: label keys, the label writes, and the `herdr.seen` rules ([`SeenTracker`]).
//! - [`attention`]: displayed status, rollup, and when to raise an in-app toast.
//! - [`adoption`]: placing labeled terminals no pane shows.
//! - [`closed`]: when a pane's terminal is gone, and close vs park.

pub mod adoption;
pub mod attention;
pub mod closed;
pub mod document;
pub mod intent;
pub mod labels;
pub mod mirror;
pub mod tree;

pub use adoption::Orphan;
pub use attention::{Agent, Alert, Attention, Displayed, Raise};
pub use document::{
    Axis, Content, DOCUMENT, Document, Extra, Kind, Node, Shows, Size, Space, Tab, Undecodable,
    fresh_id, fresh_id_on, local, machine_of, qualify,
};
pub use intent::{Adopt, Intent, Placement};
pub use labels::{SeenTracker, SeenWrite};
pub use mirror::{EditError, Layout, Mirror, Report, Round};
pub use tree::{Closed, Slot};
