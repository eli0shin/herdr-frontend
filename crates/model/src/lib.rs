//! herdr-frontend's app state as pure logic, with no I/O and no Rune types
//! (`docs/research/rune.md` §8, `docs/plan.md`).
//!
//! - [`world`]: machines, spaces, tabs and pane trees from each machine's `Mirror`, plus the
//!   core's terminal records. Shared with Herdr app.
//! - [`view`]: this screen's own state (focus, zoom, selected tabs, settled tabs, recents, seen,
//!   the jump list), saved in herdr-frontend's home, never in shared state.
//! - [`action`]: every action of the keybinding table (`feature-inventory.md` §22).
//! - [`keys`]: chords in Eli's key syntax and the default keymap.
//! - [`mode`]: who has the keyboard: normal, a prefix sequence, a popup, copy mode.
//! - [`geometry`]: where panes sit, and which pane lies in a direction.
//! - [`effect`]: what a step asks the app to do.
//! - [`control`]: the `ls | do | status` surface Eli's tools drive (`docs/cli.md`).
//!
//! [`Model::step`] is the one transition: `step(now, Event | Action) -> Effects`. User actions
//! become layout [`herdr_shared::Intent`]s and core calls; replies come back as events.

pub mod action;
mod actions;
pub mod control;
pub mod effect;
pub mod geometry;
pub mod keys;
pub mod mode;
mod model;
pub mod view;
pub mod world;

pub use action::{
    Action, AppCommand, CopyAction, Direction, Edge, Popup, PopupAction, Split, TerminalAction,
};
pub use control::{Call, CallId};
pub use effect::{Effect, Effects, Focus, Level, Pick, Request, TerminalCommand, Toast, Token};
pub use geometry::{Geometry, Rect};
pub use keys::{Chord, Context, Key, Keymap, Lookup, Mods};
pub use mode::{Mode, PopupState};
pub use model::{EDITOR_LABEL, Event, Input, Model, RETRY, REVIEW_LABEL, TerminalUpdate};
pub use view::{JumpList, View};
pub use world::{Machine, MachineKey, SpaceExt, World, folder_name, tab_name};
