// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! This screen's own view of the shared world: the shown space, each space's selected tab, each
//! tab's focused pane, focus history and zoom, settled tabs, recently closed spaces, seen
//! attention and the tab jump list. None of it goes in the shared document (protocol rule 6):
//! it is saved as JSON in herdr-frontend's home ([`View::to_json`]), like herdr-app's `view.json`
//! (`app:src/mirror.rs` `View`).
//!
//! The view keeps ids, not copies, and falls back where an id is gone: the first space, the first
//! tab, the first pane. [`View::reconcile`] forgets what a read layout no longer has, and leaves
//! alone what belongs to a machine not read yet, so a slow machine's choices survive startup
//! (`View::preserve_unloaded`). The jump list lives in memory only and starts empty
//! (`feature-inventory.md` §4).

use std::collections::BTreeMap;

use herdr_proto::StatusState;
use herdr_shared::{Node, Space, Tab};
use serde::{Deserialize, Serialize};

use crate::world::{SpaceExt, World};

/// Focus history per tab: how many panes it remembers.
pub const FOCUS_HISTORY: usize = 32;
/// Recently closed spaces remembered.
pub const RECENT_SPACES: usize = 20;

/// How many recent palette commands are kept (the palette shows the first five).
pub const PALETTE_RECENT: usize = 20;

/// One tab's view.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TabView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focused: Option<String>,
    /// Panes focused before, most recent last: closing the focused pane goes back.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub before: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zoomed: Option<String>,
    /// Put away (ms since the epoch): out of the tab keys, still running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settled: Option<u64>,
}

impl TabView {
    fn is_empty(&self) -> bool {
        self == &TabView::default()
    }
}

/// A closed space, so opening it again doesn't ask for its name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recent {
    pub name: String,
    pub folder: String,
}

/// What this screen last knew about an agent's attention: the fallback when no client shares
/// `herdr.seen`. Restored only while the agent still reports the same word.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeenRecord {
    pub word: Option<StatusState>,
    pub seen: bool,
}

/// Ctrl+G O / Ctrl+G I over tabs, like Neovim's jump list without `jumpoptions=stack`
/// (`cfg/mods/navigator-status/jump-list.js`): a tab joins once it was on screen for
/// [`JumpList::DWELL`] seconds, a tab is in the list once, at its last place, and going somewhere
/// new after jumping back drops nothing. Closed tabs are skipped.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JumpList {
    list: Vec<String>,
    /// Where you are in the list: its length when past the end (not jumping).
    pos: usize,
    /// The tab on screen, and since when.
    current: Option<String>,
    since: f64,
    /// The tab a jump asked for, until it shows.
    pending: Option<String>,
}

impl JumpList {
    pub const DWELL: f64 = 1.0;
    pub const MAX: usize = 100;

    pub fn entries(&self) -> &[String] {
        &self.list
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    fn push(&mut self, tab: &str) {
        self.list.retain(|t| t != tab);
        self.list.push(tab.to_owned());
        if self.list.len() > Self::MAX {
            self.list.drain(..self.list.len() - Self::MAX);
        }
    }

    /// The tab on screen now (call after every change). `now` is the app clock in seconds.
    pub fn shown(&mut self, tab: Option<&str>, now: f64) {
        let Some(tab) = tab else { return };
        if self.current.as_deref() == Some(tab) {
            return;
        }
        if self.pending.as_deref() == Some(tab) {
            self.pending = None;
        } else {
            // You went there yourself: the tab you left is an entry if you stayed.
            self.pending = None;
            if let Some(current) = self.current.clone()
                && now - self.since >= Self::DWELL
            {
                self.push(&current);
            }
            self.pos = self.list.len();
        }
        self.current = Some(tab.to_owned());
        self.since = now;
    }

    fn jump(&mut self, from: isize, step: isize, live: &impl Fn(&str) -> bool) -> Option<String> {
        let mut i = from;
        while i >= 0 && (i as usize) < self.list.len() {
            let tab = &self.list[i as usize];
            if self.current.as_ref() != Some(tab) && live(tab) {
                self.pos = i as usize;
                self.pending = Some(tab.clone());
                return Some(tab.clone());
            }
            i += step;
        }
        None
    }

    /// The tab Ctrl+G O shows, if any. `live`: whether a tab is still open.
    pub fn back(&mut self, live: impl Fn(&str) -> bool) -> Option<String> {
        // Leaving the end: remember where you are, so Ctrl+G I comes back. Only when there is
        // somewhere to go.
        if self.pos >= self.list.len()
            && let Some(current) = self.current.clone()
        {
            // Closed tabs go first, so making room can't drop the one to go back to.
            self.list.retain(|t| live(t));
            if !self.list.iter().any(|t| *t != current) {
                return None;
            }
            self.push(&current);
            self.pos = self.list.len() - 1;
        }
        self.jump(self.pos as isize - 1, -1, &live)
    }

    /// The tab Ctrl+G I shows, if any.
    pub fn forward(&mut self, live: impl Fn(&str) -> bool) -> Option<String> {
        self.jump(self.pos as isize + 1, 1, &live)
    }
}

/// The layout's order and this screen's choices just before a change, so what replaces a
/// closed space or tab can be picked from where it was.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Before {
    /// Every space and its tabs, in order.
    pub spaces: Vec<(String, Vec<String>)>,
    /// The space shown.
    pub active: Option<String>,
    /// Each space's selected tab.
    pub selected: BTreeMap<String, String>,
    /// The placed agents in the Agents list order, as `(address, tab)`.
    pub agents: Vec<(String, String)>,
}

impl Before {
    pub fn capture(view: &View, world: &World) -> Before {
        Before {
            spaces: world
                .spaces()
                .map(|s| (s.id.clone(), s.tabs.iter().map(|t| t.id.clone()).collect()))
                .collect(),
            active: view.active_space(world).map(|s| s.id.clone()),
            selected: world
                .spaces()
                .filter_map(|s| Some((s.id.clone(), view.selected_tab(world, s)?.id.clone())))
                .collect(),
            agents: Vec::new(),
        }
    }
}

/// This screen's view.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct View {
    /// The space shown.
    pub active: Option<String>,
    /// Each space's selected tab.
    pub spaces: BTreeMap<String, String>,
    pub tabs: BTreeMap<String, TabView>,
    /// Closed spaces, most recent first.
    pub recent: Vec<Recent>,
    /// Attention per agent address, as last saved (see [`SeenRecord`]).
    pub seen: BTreeMap<String, SeenRecord>,
    /// The palette's commands by name, most recent first ([`PALETTE_RECENT`] at most).
    pub palette: Vec<String>,
    #[serde(skip)]
    pub jumps: JumpList,
}

impl View {
    /// Reads a saved view. Unknown fields are ignored, missing ones take their defaults.
    pub fn from_json(text: &str) -> Result<View, serde_json::Error> {
        serde_json::from_str(text)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a view always serializes")
    }

    /// The space shown: the chosen one, else the first.
    pub fn active_space<'w>(&self, world: &'w World) -> Option<&'w Space> {
        self.active
            .as_deref()
            .and_then(|id| world.space(id))
            .or_else(|| world.spaces().next())
    }

    /// A space's selected tab: the chosen one, else its first.
    pub fn selected_tab<'w>(&self, _world: &'w World, space: &'w Space) -> Option<&'w Tab> {
        self.spaces
            .get(&space.id)
            .and_then(|id| space.tab(id))
            .or_else(|| space.tabs.first())
    }

    /// The tab on screen.
    pub fn shown_tab<'w>(&self, world: &'w World) -> Option<&'w Tab> {
        self.selected_tab(world, self.active_space(world)?)
    }

    /// A tab's focused pane: the chosen one, else its first.
    pub fn focused_in(&self, tab: &Tab) -> Option<String> {
        self.tabs
            .get(&tab.id)
            .and_then(|v| v.focused.as_deref())
            .filter(|id| tab.root.find(id).is_some_and(Node::is_pane))
            .map(str::to_owned)
            .or_else(|| tab.root.panes().first().map(|n| n.id.clone()))
    }

    /// The focused pane of the tab on screen.
    pub fn focused(&self, world: &World) -> Option<String> {
        self.focused_in(self.shown_tab(world)?)
    }

    /// The zoomed pane, while it's there and the tab has more than one pane.
    pub fn zoomed_in(&self, tab: &Tab) -> Option<String> {
        self.tabs
            .get(&tab.id)
            .and_then(|v| v.zoomed.as_deref())
            .filter(|id| tab.root.find(id).is_some_and(Node::is_pane))
            .filter(|_| tab.root.panes().len() > 1)
            .map(str::to_owned)
    }

    /// What a tab draws: only the zoomed pane when one is zoomed.
    pub fn shown_root<'t>(&self, tab: &'t Tab) -> &'t Node {
        self.zoomed_in(tab)
            .and_then(|id| tab.root.find(&id))
            .unwrap_or(&tab.root)
    }

    pub fn settled(&self, tab: &str) -> bool {
        self.tabs.get(tab).is_some_and(|v| v.settled.is_some())
    }

    /// Focuses a pane of a tab and remembers the one it leaves. A zoomed tab shows the pane it
    /// focuses, never a hidden one (`app:src/model.rs` `Tab::focus`).
    pub fn focus(&mut self, tab: &str, pane: &str) {
        let v = self.tabs.entry(tab.to_owned()).or_default();
        if v.zoomed.is_some() {
            v.zoomed = Some(pane.to_owned());
        }
        if let Some(old) = v.focused.take().filter(|o| o != pane) {
            v.before.retain(|p| *p != old && p != pane);
            v.before.push(old);
            if v.before.len() > FOCUS_HISTORY {
                v.before.remove(0);
            }
        }
        v.focused = Some(pane.to_owned());
    }

    /// Selects a tab and shows its space. Shown again, a settled tab is back.
    pub fn select_tab(&mut self, world: &World, tab: &str) -> bool {
        let Some(space) = world.space_of(tab).filter(|s| s.tab(tab).is_some()) else {
            return false;
        };
        self.spaces.insert(space.id.clone(), tab.to_owned());
        self.active = Some(space.id.clone());
        self.unsettle(tab);
        true
    }

    /// Focuses a pane anywhere: shows its tab and space.
    pub fn focus_pane(&mut self, world: &World, pane: &str) -> bool {
        let Some(tab) = world
            .tab_of(pane)
            .filter(|t| t.root.find(pane).is_some_and(Node::is_pane))
        else {
            return false;
        };
        let tab = tab.id.clone();
        // Focus with the tab's real current focus as "the one it leaves".
        let current = world.tab(&tab).and_then(|t| self.focused_in(t));
        let v = self.tabs.entry(tab.clone()).or_default();
        if v.focused.is_none() {
            v.focused = current;
        }
        self.focus(&tab, pane);
        self.select_tab(world, &tab)
    }

    /// Zooms a pane, or unzooms it when it is the zoomed one. A one-pane tab doesn't zoom.
    /// Zooming focuses the pane. Returns whether it is zoomed now.
    pub fn toggle_zoom(&mut self, world: &World, pane: &str) -> bool {
        let Some(tab) = world.tab_of(pane) else {
            return false;
        };
        if tab.root.panes().len() < 2 {
            return false;
        }
        let id = tab.id.clone();
        if self.zoomed_in(tab).as_deref() == Some(pane) {
            self.tabs.entry(id).or_default().zoomed = None;
            return false;
        }
        self.tabs.entry(id.clone()).or_default().zoomed = Some(pane.to_owned());
        self.focus(&id, pane);
        true
    }

    pub fn unzoom(&mut self, tab: &str) {
        if let Some(v) = self.tabs.get_mut(tab) {
            v.zoomed = None;
        }
    }

    pub fn settle(&mut self, tab: &str, now_ms: u64) {
        self.tabs.entry(tab.to_owned()).or_default().settled = Some(now_ms);
    }

    pub fn unsettle(&mut self, tab: &str) -> bool {
        self.tabs
            .get_mut(tab)
            .is_some_and(|v| v.settled.take().is_some())
    }

    /// A command run from the palette, remembered first (`cfg/frame/palette/main.js` `go`).
    pub fn remember_command(&mut self, name: &str) {
        self.palette.retain(|n| n != name);
        self.palette.insert(0, name.to_owned());
        self.palette.truncate(PALETTE_RECENT);
    }

    /// A closed space, remembered first.
    pub fn remember_closed(&mut self, name: &str, folder: &str) {
        self.recent.retain(|r| r.name != name);
        self.recent.insert(
            0,
            Recent {
                name: name.to_owned(),
                folder: folder.to_owned(),
            },
        );
        self.recent.truncate(RECENT_SPACES);
    }

    /// After the layout changed: forget what read layouts no longer have, move focus off panes
    /// that went (back to the one focused before), end zooms of one-pane tabs, and replace a
    /// closed space or tab from where it was in `before` (the one that slides into its place, or
    /// the last; for a shown space gone without having been in `before`, what showed before, else
    /// the first). Ids of machines not read yet are kept as they are.
    pub fn reconcile(&mut self, world: &World, before: &Before) {
        let gone = |id: &str, exists: bool| !exists && world.loaded_for(id);
        // Tabs.
        self.tabs.retain(|id, _| !gone(id, world.tab(id).is_some()));
        for (id, v) in self.tabs.iter_mut() {
            let Some(tab) = world.tab(id) else { continue };
            let here = |p: &str| tab.root.find(p).is_some_and(Node::is_pane);
            v.before.retain(|p| here(p));
            if v.focused.as_deref().is_some_and(|f| !here(f)) {
                v.focused = v.before.pop();
            }
            if v.zoomed.as_deref().is_some_and(|z| !here(z)) || tab.root.panes().len() < 2 {
                v.zoomed = None;
            }
        }
        self.tabs.retain(|_, v| !v.is_empty());
        // Each space's selected tab.
        self.spaces
            .retain(|id, _| !gone(id, world.space(id).is_some()));
        for (space_id, tabs) in &before.spaces {
            let Some(space) = world.space(space_id) else {
                continue;
            };
            let Some(selected) = before.selected.get(space_id) else {
                continue;
            };
            if space.tab(selected).is_some() {
                continue;
            }
            // The tab that slides into the slot, or the last one.
            let at = tabs.iter().position(|t| t == selected).unwrap_or(0);
            let next = tabs[at + 1..]
                .iter()
                .find(|t| space.tab(t).is_some())
                .or_else(|| tabs[..at].iter().rev().find(|t| space.tab(t).is_some()))
                .cloned()
                .or_else(|| space.tabs.last().map(|t| t.id.clone()));
            match next {
                Some(next) => self.spaces.insert(space_id.clone(), next),
                None => self.spaces.remove(space_id),
            };
        }
        self.spaces
            .retain(|space, tab| world.space(space).is_none_or(|s| s.tab(tab).is_some()));
        // The space shown.
        if let Some(active) = self.active.clone()
            && gone(&active, world.space(&active).is_some())
        {
            let order: Vec<&String> = before.spaces.iter().map(|(id, _)| id).collect();
            let at = order.iter().position(|id| **id == active);
            self.active = at
                .and_then(|at| {
                    order[at + 1..]
                        .iter()
                        .find(|id| world.space(id).is_some())
                        .or_else(|| {
                            order[..at]
                                .iter()
                                .rev()
                                .find(|id| world.space(id).is_some())
                        })
                })
                .map(|id| (*id).clone())
                // A saved space whose machine wasn't read yet, gone once it is: keep what showed
                // meanwhile (`app:src/mirror.rs` `View::apply_machine`).
                .or_else(|| before.active.clone().filter(|id| world.space(id).is_some()))
                // A saved space gone at startup: the first, as herdr-app's `View::apply` does.
                .or_else(|| world.spaces().next().map(|s| s.id.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(tabs: &[&str]) -> impl Fn(&str) -> bool {
        let tabs: Vec<String> = tabs.iter().map(|s| s.to_string()).collect();
        move |t: &str| tabs.iter().any(|x| x == t)
    }

    /// Shows tabs one after another, each for `dwell` seconds.
    fn visit(j: &mut JumpList, tabs: &[&str], now: &mut f64) {
        for tab in tabs {
            j.shown(Some(tab), *now);
            *now += 2.0;
        }
    }

    #[test]
    fn a_tab_joins_after_a_second_on_screen() {
        let mut j = JumpList::default();
        j.shown(Some("a"), 0.0);
        j.shown(Some("b"), 0.5);
        assert!(j.entries().is_empty(), "a was on screen for half a second");
        j.shown(Some("c"), 2.0);
        assert_eq!(j.entries(), ["b"]);
        j.shown(None, 5.0);
        assert_eq!(j.entries(), ["b"], "no tab shown changes nothing");
    }

    #[test]
    fn back_and_forward_walk_the_list_and_skip_closed_tabs() {
        let mut j = JumpList::default();
        let mut now = 0.0;
        visit(&mut j, &["a", "b", "c", "d"], &mut now);
        assert_eq!(j.entries(), ["a", "b", "c"]);
        let all = live(&["a", "b", "c", "d"]);
        assert_eq!(j.back(&all).as_deref(), Some("c"));
        assert_eq!(
            j.entries(),
            ["a", "b", "c", "d"],
            "leaving the end remembers d"
        );
        j.shown(Some("c"), now);
        assert_eq!(j.back(&all).as_deref(), Some("b"));
        j.shown(Some("b"), now);
        assert_eq!(j.forward(&all).as_deref(), Some("c"));
        j.shown(Some("c"), now);
        assert_eq!(j.forward(&all).as_deref(), Some("d"));
        j.shown(Some("d"), now);
        assert_eq!(j.forward(&all), None);
        // b closes: back from d skips it.
        let no_b = live(&["a", "c", "d"]);
        assert_eq!(j.back(&no_b).as_deref(), Some("c"));
        j.shown(Some("c"), now);
        assert_eq!(j.back(&no_b).as_deref(), Some("a"));
    }

    #[test]
    fn going_somewhere_new_after_jumping_back_drops_nothing() {
        let mut j = JumpList::default();
        let mut now = 0.0;
        visit(&mut j, &["a", "b", "c"], &mut now);
        let all = live(&["a", "b", "c", "x"]);
        assert_eq!(j.back(&all).as_deref(), Some("b"));
        j.shown(Some("b"), now);
        now += 2.0;
        j.shown(Some("x"), now);
        assert_eq!(j.entries(), ["a", "c", "b"], "b moved to the end; c stays");
        assert_eq!(j.position(), 3);
    }

    #[test]
    fn back_with_nowhere_to_go_does_nothing() {
        let mut j = JumpList::default();
        assert_eq!(j.back(|_| true), None);
        j.shown(Some("a"), 0.0);
        assert_eq!(j.back(|_| true), None);
        assert!(j.entries().is_empty(), "nothing happened, so a didn't join");
    }

    #[test]
    fn the_list_holds_a_hundred_tabs_each_once() {
        let mut j = JumpList::default();
        let mut now = 0.0;
        for i in 0..150 {
            j.shown(Some(&format!("t{i}")), now);
            now += 2.0;
        }
        j.shown(Some("t3"), now);
        now += 2.0;
        j.shown(Some("t149"), now);
        assert_eq!(j.entries().len(), JumpList::MAX);
        assert_eq!(j.entries().last().map(String::as_str), Some("t3"));
        assert_eq!(j.entries().iter().filter(|t| *t == "t149").count(), 1);
    }

    #[test]
    fn focus_history_remembers_each_pane_once_and_is_bounded() {
        let mut v = View::default();
        for p in ["a", "b", "a", "c"] {
            v.focus("t", p);
        }
        assert_eq!(v.tabs["t"].focused.as_deref(), Some("c"));
        assert_eq!(v.tabs["t"].before, ["b", "a"]);
        for i in 0..40 {
            v.focus("t", &format!("p{i}"));
        }
        assert_eq!(v.tabs["t"].before.len(), FOCUS_HISTORY);
    }

    #[test]
    fn focusing_in_a_zoomed_tab_moves_the_zoom() {
        let mut v = View::default();
        v.tabs.insert(
            "t".into(),
            TabView {
                focused: Some("a".into()),
                zoomed: Some("a".into()),
                ..Default::default()
            },
        );
        v.focus("t", "b");
        assert_eq!(v.tabs["t"].zoomed.as_deref(), Some("b"));
    }

    #[test]
    fn recent_spaces_are_most_recent_first_once_each_and_bounded() {
        let mut v = View::default();
        for i in 0..25 {
            v.remember_closed(&format!("s{i}"), "/f");
        }
        v.remember_closed("s10", "/g");
        assert_eq!(v.recent.len(), RECENT_SPACES);
        assert_eq!(
            v.recent[0],
            Recent {
                name: "s10".into(),
                folder: "/g".into()
            }
        );
        assert_eq!(v.recent.iter().filter(|r| r.name == "s10").count(), 1);
    }

    #[test]
    fn the_saved_view_reads_back_without_the_jump_list() {
        let mut v = View {
            active: Some("sp1".into()),
            ..Default::default()
        };
        v.spaces.insert("sp1".into(), "tb1".into());
        v.focus("tb1", "p1");
        v.settle("tb2", 7);
        v.remember_closed("old", "/old");
        v.seen.insert(
            "t1".into(),
            SeenRecord {
                word: Some(StatusState::Done),
                seen: false,
            },
        );
        v.jumps.shown(Some("tb1"), 0.0);
        let back = View::from_json(&v.to_json()).unwrap();
        assert_eq!(back.active, v.active);
        assert_eq!(back.spaces, v.spaces);
        assert_eq!(back.tabs, v.tabs);
        assert_eq!(back.recent, v.recent);
        assert_eq!(back.seen, v.seen);
        assert_eq!(back.jumps, JumpList::default());
    }

    #[test]
    fn a_saved_view_with_unknown_or_missing_fields_still_reads() {
        let v = View::from_json(r#"{"active":"sp1","future":{"x":1}}"#).unwrap();
        assert_eq!(v.active.as_deref(), Some("sp1"));
        assert!(v.tabs.is_empty());
        let v = View::from_json("{}").unwrap();
        assert_eq!(v, View::default());
        assert!(View::from_json("[").is_err());
    }
}
