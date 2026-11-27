//! Peek (`feature-inventory.md` §15): a live terminal in an overlay over the window, without
//! moving focus. Herdr app's `terminal.peek` and its shipped `peek` mod (`cfg/mods/peek/main.js`,
//! `app:src/window.rs` `peek_terminal`, `keep_terminal`).
//!
//! - It opens from a sub-agent's row, a jump to an agent with no pane, `agent.focus` and
//!   `terminal.peek`, and Cmd-click on a live terminal's address in terminal text (the word
//!   under the pointer, `terminal::links::word`, that [`terminal_address`] finds live).
//! - While it is open the peeked terminal has the keyboard (Esc too: agents use it), unless a
//!   popup opens over it. The focused pane stays focused behind it. Closing a tab or pane by key
//!   closes the peek instead. It closes on its ✕, a click outside it, or when its terminal ends.
//! - Its bar ([`PeekBar`]) says who it is and how it's doing. "↗ go to" focuses its pane, if it
//!   has one. "↗ keep" gives it one: a new tab, after the selected one, in the shown space when
//!   that is on the terminal's machine, else its nearest ancestor's space there, else the first
//!   space there. Keeping labels the terminal `herdr.space` and `herdr.tab`, sets `keep`, and
//!   only then puts its tab in the layout.

use std::collections::BTreeSet;

use herdr_proto::methods::{TerminalKeep, TerminalLabel};
use herdr_proto::{RpcError, Terminal};
use herdr_shared::attention::Displayed;
use herdr_shared::{Content, Intent, fresh_id_on, labels, local, machine_of, qualify};

use crate::agents::parent_of;
use crate::effect::{Effect, Effects, Level, Request, Toast};
use crate::model::{Model, Pending};

/// A keep under way ([`crate::model::Pending::Keep`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Keep {
    pub(crate) address: String,
    pub(crate) space: String,
    pub(crate) tab: String,
    pub(crate) pane: String,
    pub(crate) name: String,
    /// The labels are on; `keep` is next.
    pub(crate) labeled: bool,
}

/// What the bar's "↗" does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Go {
    /// "↗ go to": focus the terminal's pane.
    Pane(String),
    /// "↗ keep": give it a pane, in this space.
    Keep(String),
}

/// What the peek's bar shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeekBar {
    pub address: String,
    pub status: Option<Displayed>,
    /// The terminal that started it, by name, else address.
    pub from: Option<String>,
    /// Hidden when it has no pane and its machine has no space to keep it in.
    pub go: Option<Go>,
}

impl Model {
    /// The terminal the peek shows.
    pub fn peeked(&self) -> Option<&str> {
        self.peek.as_deref()
    }

    /// The open peek's bar.
    pub fn peek_bar(&self) -> Option<PeekBar> {
        let address = self.peek.as_deref()?;
        let world = &self.world;
        let from = world
            .terminal(address)
            .and_then(|t| parent_of(world, t))
            .and_then(|p| world.terminal(p))
            .map(|p| {
                p.name
                    .clone()
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| p.address.clone())
            });
        let go = match world.pane_of(address) {
            Some(pane) => Some(Go::Pane(pane)),
            None => self.keep_space(address).map(Go::Keep),
        };
        Some(PeekBar {
            address: address.to_owned(),
            status: self.attention.displayed(address),
            from,
            go,
        })
    }

    /// The space "↗ keep" puts a terminal in: a pane stays on its terminal's machine
    /// (`cfg/mods/peek/main.js` `keepSpace`).
    fn keep_space(&self, address: &str) -> Option<String> {
        let world = &self.world;
        let machine = machine_of(address);
        let shown = self.active_space().or_else(|| world.spaces().next());
        if let Some(shown) = shown.filter(|s| machine_of(&s.id) == machine) {
            return Some(shown.id.clone());
        }
        let there: Vec<&str> = world
            .spaces()
            .filter(|s| machine_of(&s.id) == machine)
            .map(|s| s.id.as_str())
            .collect();
        if there.len() < 2 {
            return there.first().map(|s| (*s).to_owned());
        }
        let mut seen = BTreeSet::new();
        let mut parent = world.terminal(address).and_then(|t| parent_of(world, t));
        while let Some(p) = parent
            && seen.insert(p)
        {
            let space = world
                .pane_of(p)
                .and_then(|pane| world.space_of(&pane))
                .map(|s| s.id.as_str())
                .filter(|id| there.contains(id));
            if let Some(space) = space {
                return Some(space.to_owned());
            }
            parent = world.terminal(p).and_then(|t| parent_of(world, t));
        }
        Some(there[0].to_owned())
    }

    /// Opens the peek on a live terminal, in place of an open popup, copy mode or another peek.
    pub(crate) fn open_peek(&mut self, address: &str, fx: &mut Effects) {
        if !self.world.is_live(address) {
            return;
        }
        self.close_mode(fx);
        self.peek = Some(address.to_owned());
    }

    /// The bar's "↗": go to the terminal's pane, or give it one. Either closes the peek.
    pub(crate) fn keep_peeked(&mut self, fx: &mut Effects) {
        let Some(bar) = self.peek_bar() else {
            return;
        };
        match bar.go {
            Some(Go::Pane(pane)) => {
                self.view.focus_pane(&self.world, &pane);
            }
            Some(Go::Keep(space)) => self.keep(&bar.address, &space, fx),
            None => return,
        }
        self.peek = None;
    }

    /// Shows `space` and starts giving `address` a tab in it (`app:src/window.rs`
    /// `keep_terminal`, `start_terminal_change`).
    fn keep(&mut self, address: &str, space: &str, fx: &mut Effects) {
        let machine = machine_of(address);
        if !self.layout_ready(machine, fx) {
            return;
        }
        if self.active_space().map(|s| s.id.as_str()) != Some(space) {
            self.view.active = Some(space.to_owned());
        }
        let tab = fresh_id_on("tb", machine);
        let record = self.world.terminal(address);
        let name = record
            .and_then(|t| t.name.clone())
            .filter(|n| !n.is_empty())
            .or_else(|| {
                record
                    .and_then(|t| t.status.as_ref())
                    .map(|s| s.app.clone())
                    .filter(|a| !a.is_empty())
            })
            .unwrap_or_else(|| address.to_owned());
        let labels = [
            (labels::SPACE.to_owned(), local(space, machine).to_owned()),
            (labels::TAB.to_owned(), local(&tab, machine).to_owned()),
        ]
        .into();
        let keep = Keep {
            address: address.to_owned(),
            space: space.to_owned(),
            pane: fresh_id_on("p", machine),
            tab,
            name,
            labeled: false,
        };
        self.call(
            fx,
            Request::Label(TerminalLabel {
                terminal: address.to_owned(),
                labels,
                ..Default::default()
            }),
            Some(Pending::Keep(keep)),
        );
    }

    /// A keep's call came back: after the labels, `keep`; after that, its tab, focused. A space
    /// closed meanwhile takes the labels back off, as parking does.
    pub(crate) fn kept(
        &mut self,
        keep: Keep,
        result: Result<Box<Terminal>, RpcError>,
        fx: &mut Effects,
    ) {
        if let Err(error) = result {
            fx.push(Effect::Toast(Toast::new(
                Level::Error,
                "Could not change terminal pane",
                error.message,
            )));
            return;
        }
        let address = keep.address.clone();
        if !keep.labeled {
            self.call(
                fx,
                Request::Keep(TerminalKeep {
                    terminal: address,
                    keep: true,
                    ..Default::default()
                }),
                Some(Pending::Keep(Keep {
                    labeled: true,
                    ..keep
                })),
            );
            return;
        }
        if !self.world.is_live(&address) {
            return;
        }
        let Some(space) = self.world.space(&keep.space) else {
            self.park(&address, fx);
            return;
        };
        if self.world.pane_of(&address).is_none() {
            let after = self
                .view
                .selected_tab(&self.world, space)
                .map(|t| t.id.clone());
            let intent = Intent::NewTab {
                space: keep.space,
                tab: keep.tab,
                name: keep.name,
                pane: keep.pane,
                content: Content::terminal(address.clone()),
                after,
            };
            self.edit(intent, fx);
        }
        if let Some(pane) = self.world.pane_of(&address) {
            self.view.focus_pane(&self.world, &pane);
        }
    }

    /// Takes a terminal's place labels off and stops keeping it (`app:src/window.rs`
    /// `TerminalChange::Park`).
    pub(crate) fn park(&mut self, address: &str, fx: &mut Effects) {
        self.call(
            fx,
            Request::Label(TerminalLabel {
                terminal: address.to_owned(),
                labels: labels::for_park(),
                ..Default::default()
            }),
            None,
        );
        self.call(
            fx,
            Request::Keep(TerminalKeep {
                terminal: address.to_owned(),
                keep: false,
                ..Default::default()
            }),
            None,
        );
    }
}

/// The live terminal a word names. A bare address means the machine of `source`, the terminal
/// showing it; `machine/address` names another. Names aren't addresses (`app:src/links.rs`
/// `terminal_address`).
pub fn terminal_address<'a>(
    word: &str,
    source: &str,
    live: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let address = if word.contains('/') {
        word.to_owned()
    } else {
        qualify(word, machine_of(source))
    };
    live.into_iter().any(|a| a == address).then_some(address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_addresses_are_the_showing_terminals_machine_and_only_live_ones_count() {
        let live = ["t4", "devbox/t4", "devbox/t9"];
        let find = |w, src| terminal_address(w, src, live.iter().copied());
        assert_eq!(find("t4", "t1").as_deref(), Some("t4"));
        assert_eq!(find("t4", "devbox/t2").as_deref(), Some("devbox/t4"));
        assert_eq!(find("t9", "t1"), None, "t9 runs on devbox");
        assert_eq!(find("devbox/t9", "t1").as_deref(), Some("devbox/t9"));
        assert_eq!(find("worker", "t1"), None, "names aren't addresses");
    }
}
