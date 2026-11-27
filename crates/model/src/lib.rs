//! herdr-frontend's app state as pure logic, with no I/O and no Rune types
//! (`docs/research/rune.md` §8, `docs/plan.md`).
//!
//! - [`agents`]: the Agents list Eli's `eli-agents` mod builds: sections, rows, order.
//! - [`world`]: machines, spaces, tabs and pane trees from each machine's `Mirror`, plus the
//!   core's terminal records. Shared with Herdr app.
//! - [`view`]: this screen's own state (focus, zoom, selected tabs, settled tabs, recents, seen,
//!   the jump list), saved in herdr-frontend's home, never in shared state.
//! - [`action`]: every action of the keybinding table (`feature-inventory.md` §22).
//! - [`keys`]: chords in Eli's key syntax, the key file and the default keymap.
//! - [`help`]: the keys help's rows, from the keymap.
//! - [`names`]: what tabs are called on screen, and the tab bar's pills.
//! - [`mode`]: who has the keyboard: normal, a prefix sequence, a popup, copy mode.
//! - [`popups`]: what the pickers, the palette and the menus list, and what picking runs.
//! - [`filter`]: Eli's filter matching, which every typed filter uses.
//! - [`geometry`]: where panes sit, and which pane lies in a direction.
//! - [`peek`]: a terminal in an overlay, its bar, and keeping it in a pane.
//! - [`review`]: the review view, `cmd-r`.
//! - [`shell`]: a tab's program run in the login shell, as Herdr app's `tab.new` runs it.
//! - [`effect`]: what a step asks the app to do.
//! - [`control`]: the `ls | do | status` surface Eli's tools drive (`docs/cli.md`).
//!
//! [`Model::step`] is the one transition: `step(now, Event | Action) -> Effects`. User actions
//! become layout [`herdr_shared::Intent`]s and core calls; replies come back as events.

pub mod action;
mod actions;
pub mod agents;
pub mod control;
pub mod effect;
pub mod filter;
pub mod geometry;
pub mod help;
pub mod keys;
pub mod mode;
mod model;
pub mod names;
pub mod peek;
pub mod popups;
pub mod review;
pub mod shell;
pub mod view;
pub mod world;

pub use action::{
    Action, AppCommand, CopyAction, Direction, Edge, Popup, PopupAction, Split, TerminalAction,
};
pub use control::{Call, CallId};
pub use effect::{Effect, Effects, Focus, Level, Pick, Request, TerminalCommand, Toast, Token};
pub use geometry::{Divider, Geometry, Rect};
pub use keys::{
    Binding, Chord, Context, ELI_KEYS, KEY_FILE, Key, KeyFileError, Keymap, Lookup, Mods, Press,
};
pub use mode::{Menu, MenuIcon, MenuItem, MenuTarget, Mode, PopupState};
pub use model::{
    EDITOR_LABEL, Event, Input, Model, NAVIGATE_TIMEOUT, NAVIGATOR_LABEL, RETRY, REVIEW_LABEL,
    TerminalUpdate,
};
pub use names::TabPill;
pub use view::{JumpList, View};
pub use world::{Machine, MachineKey, SpaceExt, World, folder_name, tab_name};
