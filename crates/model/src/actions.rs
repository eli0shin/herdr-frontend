//! What each [`Action`] does to the model. Layout changes go through the machine's `Mirror` as
//! [`Intent`]s; terminals start and end through core calls; everything else is this screen's view.
//! Actions whose feature isn't built yet return [`Effect::NotYet`].

use herdr_shared::{Axis, Intent, Node, fresh_id_on, machine_of};

use crate::action::{Action, CopyAction, Direction, Popup, PopupAction, Split};
use crate::effect::{Effect, Effects, Level, Pick, TerminalCommand, Toast};
use crate::geometry;
use crate::mode::{
    self, Confirm, Confirmed, CopyState, JumpPopup, ListPopup, Mode, PopupState, Prompt, Rename,
    Visual,
};
use crate::model::{EDITOR_LABEL, Launch, Model};
use crate::world::SpaceExt;

impl Model {
    pub(crate) fn run(&mut self, action: Action, fx: &mut Effects) {
        match action {
            Action::App(command) => fx.push(Effect::App(command)),
            Action::Open(popup) => self.open(popup, fx),
            Action::CopyMode => self.copy_mode(fx),
            Action::TabNew => self.new_tab(fx),
            Action::TabClose => {
                if let Some(tab) = self.shown_tab().map(|t| t.id.clone()) {
                    self.close_tab(&tab, fx);
                }
            }
            Action::CloseTabOf(tab) => self.close_tab(&tab, fx),
            Action::TabNext => self.step_tab(1, fx),
            Action::TabPrev => self.step_tab(-1, fx),
            Action::TabSelect(n) => {
                let tab = self.active_space().and_then(|s| {
                    let shown = self.shown_tabs(s);
                    shown
                        .get(usize::from(n).checked_sub(1)?)
                        .map(|t| t.id.clone())
                });
                if let Some(tab) = tab {
                    self.view.select_tab(&self.world, &tab);
                }
            }
            Action::ShowTab(tab) => {
                self.view.select_tab(&self.world, &tab);
            }
            Action::TabRename => {
                if let Some(tab) = self.shown_tab().map(|t| t.id.clone()) {
                    self.rename(Rename::Tab(tab));
                }
            }
            Action::RenameTabOf(tab) => self.rename(Rename::Tab(tab)),
            Action::MoveTab { tab, to } => {
                if self.world.tab(&tab).is_some() {
                    self.edit(Intent::MoveTab { tab, to }, fx);
                }
            }
            Action::JumpBack | Action::JumpForward => {
                let world = &self.world;
                let live = |t: &str| world.tab(t).is_some();
                let to = if action == Action::JumpBack {
                    self.view.jumps.back(live)
                } else {
                    self.view.jumps.forward(live)
                };
                if let Some(tab) = to {
                    self.view.select_tab(&self.world, &tab);
                }
            }
            Action::SpaceNext | Action::SpacePrev => {
                let ids: Vec<String> = self.world.spaces().map(|s| s.id.clone()).collect();
                let active = self.active_space().map(|s| s.id.clone());
                if let Some(i) = ids.iter().position(|id| Some(id) == active.as_ref()) {
                    let n = ids.len();
                    let j = if action == Action::SpaceNext {
                        (i + 1) % n
                    } else {
                        (i + n - 1) % n
                    };
                    self.view.active = Some(ids[j].clone());
                }
            }
            Action::SpaceSelect(n) => {
                let id = usize::from(n)
                    .checked_sub(1)
                    .and_then(|i| self.world.spaces().nth(i))
                    .map(|s| s.id.clone());
                if let Some(id) = id {
                    self.view.active = Some(id);
                }
            }
            Action::SpaceRename => {
                if let Some(space) = self.active_space().map(|s| s.id.clone()) {
                    self.rename(Rename::Space(space));
                }
            }
            Action::SpaceClose => {
                if let Some(space) = self.active_space().map(|s| s.id.clone()) {
                    self.ask_close_space(&space);
                }
            }
            Action::AgentNext | Action::AgentPrev => {
                let by = if action == Action::AgentNext { 1 } else { -1 };
                let order = self.agent_order();
                let focused = self.focused_terminal().map(str::to_owned);
                let at = order.iter().position(|a| Some(a) == focused.as_ref());
                if let Some(agent) = step_in(&order, at, by) {
                    self.focus_agent(&agent, fx);
                }
            }
            Action::AgentSelect(n) => {
                let rows = self.agent_rows();
                if let Some(agent) = usize::from(n).checked_sub(1).and_then(|i| rows.get(i)) {
                    let agent = agent.clone();
                    self.focus_agent(&agent, fx);
                }
            }
            Action::FocusAgent(address) => self.focus_agent(&address, fx),
            Action::FocusPane(pane) => {
                self.view.focus_pane(&self.world, &pane);
            }
            Action::PaneSplit(split) => self.split(split, fx),
            Action::PaneClose => self.close_pane(fx),
            Action::PaneZoom => {
                if let Some(pane) = self.focused_pane() {
                    self.view.toggle_zoom(&self.world, &pane);
                }
            }
            Action::PaneGo(direction) => self.go(direction),
            Action::PaneNavigate(direction) => {
                let neovim = self
                    .focused_terminal()
                    .and_then(|a| self.world.terminal(a))
                    .is_some_and(|t| t.label(EDITOR_LABEL).is_some_and(|v| !v.is_empty()));
                if neovim {
                    // Through Neovim's splits first (ticket 022).
                    fx.push(Effect::NotYet(action));
                } else {
                    self.go(direction);
                }
            }
            Action::PaneSwap(direction) => self.swap(direction, fx),
            Action::ResizeSplit { sizes } => {
                if sizes.iter().any(|(id, _)| self.world.node(id).is_some()) {
                    self.edit(Intent::Resize { sizes }, fx);
                }
            }
            Action::Terminal(command) => {
                if let Some(address) = self.focused_terminal().map(str::to_owned) {
                    fx.push(Effect::Terminal {
                        address,
                        command: TerminalCommand::Action(command),
                    });
                } else {
                    fx.handled = false;
                }
            }
            Action::Popup(popup) => self.popup(popup, fx),
            Action::AgentJumpKey(c) => self.jump_key(c, fx),
            Action::Copy(copy) => self.copy(copy, fx),
            Action::Review
            | Action::PaneResize(_)
            | Action::TabMenu(_)
            | Action::Peek(_)
            | Action::FoldSubagents(_)
            | Action::TerminalMenu(_)
            | Action::OpenLink { .. }
            | Action::RearrangePane { .. } => fx.push(Effect::NotYet(action)),
        }
    }

    /// The folder a new terminal starts in: the focused terminal's, else the shown space's,
    /// else home.
    fn new_folder(&self) -> String {
        self.focused_terminal()
            .and_then(|a| self.world.folder_of(a))
            .map(str::to_owned)
            .or_else(|| self.active_space().map(|s| s.folder.clone()))
            .filter(|f| !f.is_empty())
            .unwrap_or_else(|| self.home.clone())
    }

    /// `tab.new`: a tab after the selected one, in the focused terminal's folder. With no space,
    /// a new space on this machine.
    fn new_tab(&mut self, fx: &mut Effects) {
        let folder = self.new_folder();
        let Some(space) = self.active_space() else {
            let launch = Launch::Space {
                space: fresh_id_on("sp", None),
                folder: folder.clone(),
                tab: fresh_id_on("tb", None),
                pane: fresh_id_on("p", None),
            };
            self.launch(None, &folder, launch, fx);
            return;
        };
        let machine = machine_of(&space.id).map(str::to_owned);
        let m = machine.as_deref();
        let launch = Launch::Tab {
            space: space.id.clone(),
            tab: fresh_id_on("tb", m),
            pane: fresh_id_on("p", m),
            after: self
                .view
                .selected_tab(&self.world, space)
                .map(|t| t.id.clone()),
        };
        self.launch(machine, &folder, launch, fx);
    }

    /// A split of the focused pane; the new terminal stays on that pane's machine.
    fn split(&mut self, split: Split, fx: &mut Effects) {
        let Some(target) = self.focused_pane() else {
            return;
        };
        let machine = machine_of(&target).map(str::to_owned);
        let m = machine.as_deref();
        let axis = match split {
            Split::Right => Axis::Row,
            Split::Down => Axis::Column,
        };
        let folder = self.new_folder();
        let launch = Launch::Split {
            target,
            axis,
            pane: fresh_id_on("p", m),
            container: fresh_id_on("s", m),
        };
        self.launch(machine, &folder, launch, fx);
    }

    /// `pane.close`: the focused pane and its terminal. A tab's last pane closes the tab.
    fn close_pane(&mut self, fx: &mut Effects) {
        let Some(pane) = self.focused_pane() else {
            return;
        };
        let Some(tab) = self.world.tab_of(&pane) else {
            return;
        };
        if tab.root.panes().len() == 1 {
            let tab = tab.id.clone();
            self.close_tab(&tab, fx);
            return;
        }
        let address = self.world.address_of(&pane).map(str::to_owned);
        if self.edit(Intent::RemovePane { pane }, fx) {
            self.kill(address.into_iter().collect(), fx);
        }
    }

    /// Closes a tab and its terminals. A space's last tab asks first, since it closes the space.
    fn close_tab(&mut self, tab: &str, fx: &mut Effects) {
        let Some(space) = self.world.space_of(tab).filter(|s| s.tab(tab).is_some()) else {
            return;
        };
        if space.tabs.len() == 1 {
            let space = space.id.clone();
            self.ask_close_space(&space);
            return;
        }
        let addresses = space
            .tab(tab)
            .map(|t| t.root.addresses())
            .unwrap_or_default();
        if self.edit(
            Intent::CloseTab {
                tab: tab.to_owned(),
            },
            fx,
        ) {
            self.kill(addresses, fx);
        }
    }

    fn ask_close_space(&mut self, id: &str) {
        let Some(space) = self.world.space(id) else {
            return;
        };
        let panes: usize = space.tabs.iter().map(|t| t.root.panes().len()).sum();
        let detail = format!(
            "{} \u{2014} {} tabs, {} terminals",
            self.world.space_name(space, &self.home),
            space.tabs.len(),
            panes
        );
        self.open_popup(PopupState::Confirm(Confirm {
            title: "Close space?".into(),
            detail,
            then: Confirmed::CloseSpace(id.to_owned()),
        }));
    }

    fn close_space(&mut self, id: &str, fx: &mut Effects) {
        let Some(space) = self.world.space(id) else {
            return;
        };
        let addresses: Vec<String> = space.tabs.iter().flat_map(|t| t.root.addresses()).collect();
        let (name, folder) = (space.name.clone(), space.folder.clone());
        if self.edit(
            Intent::CloseSpace {
                space: id.to_owned(),
            },
            fx,
        ) {
            self.view.remember_closed(&name, &folder);
            self.kill(addresses, fx);
        }
    }

    /// Next or previous shown, unsettled tab, wrapping; on an agent's tab, the next or previous
    /// agent in the Agents order instead (`cfg/mods/navigator-status/tab-order.js`).
    fn step_tab(&mut self, by: isize, fx: &mut Effects) {
        let Some(space) = self.active_space() else {
            return;
        };
        let active = self
            .view
            .selected_tab(&self.world, space)
            .map(|t| t.id.clone());
        if let Some(active) = &active
            && space.tab(active).is_some_and(|t| self.is_hidden_tab(t))
        {
            let order = self.agent_order();
            let tab_of = |a: &str| {
                self.world
                    .pane_of(a)
                    .and_then(|p| self.world.tab_of(&p))
                    .map(|t| t.id.as_str())
            };
            let focused = self.focused_terminal();
            let at = order
                .iter()
                .position(|a| Some(a.as_str()) == focused && tab_of(a) == Some(active))
                .or_else(|| order.iter().position(|a| tab_of(a) == Some(active)));
            if let Some(agent) = step_in(&order, at, by) {
                self.focus_agent(&agent, fx);
            }
            return;
        }
        let tabs = &space.tabs;
        let n = tabs.len() as isize;
        let from = active
            .as_ref()
            .and_then(|a| tabs.iter().position(|t| &t.id == a))
            .map_or(if by > 0 { -1 } else { n }, |i| i as isize);
        let target = (1..=n)
            .map(|i| &tabs[(from + by * i).rem_euclid(n) as usize])
            .find(|t| !self.is_hidden_tab(t) && !self.view.settled(&t.id))
            .filter(|t| Some(&t.id) != active.as_ref())
            .map(|t| t.id.clone());
        if let Some(target) = target {
            self.view.select_tab(&self.world, &target);
        }
    }

    /// Focuses an agent's pane. One without a pane (a sub-agent) is shown in a peek instead.
    fn focus_agent(&mut self, address: &str, fx: &mut Effects) {
        match self.world.pane_of(address) {
            Some(pane) => {
                self.view.focus_pane(&self.world, &pane);
            }
            None => fx.push(Effect::NotYet(Action::Peek(address.to_owned()))),
        }
    }

    /// The pane next to the focused one in `direction`, among what is drawn. With `history`
    /// (focus moves), the most recently focused along the edge wins, and a zoomed pane looks at
    /// the whole tab; without (swaps), only what is drawn.
    fn neighbor(&self, direction: Direction, history: bool) -> Option<(String, String)> {
        let tab = self.shown_tab()?;
        let from = self.view.focused_in(tab)?;
        let zoomed = self.view.zoomed_in(tab);
        let root = if history && zoomed.as_deref() == Some(from.as_str()) {
            &tab.root
        } else {
            self.view.shown_root(tab)
        };
        let blocks = geometry::blocks(root, &self.geometry);
        let from_rect = blocks.iter().find(|(id, _)| *id == from)?.1;
        let candidates = blocks
            .iter()
            .filter(|(id, _)| *id != from && tab.root.find(id).is_some_and(Node::is_pane))
            .map(|(id, r)| (id.as_str(), *r));
        let empty = Vec::new();
        let history = if history { &self.pane_history } else { &empty };
        let to = geometry::nearest(from_rect, direction, candidates, history)?;
        Some((from, to.to_owned()))
    }

    fn go(&mut self, direction: Direction) {
        if let Some((_, to)) = self.neighbor(direction, true) {
            self.view.focus_pane(&self.world, &to);
        }
    }

    /// Swaps the focused pane with its neighbor; focus stays on the same pane, now moved.
    fn swap(&mut self, direction: Direction, fx: &mut Effects) {
        let Some((from, to)) = self.neighbor(direction, false) else {
            return;
        };
        let intent = self
            .world
            .machine(machine_of(&from))
            .and_then(|m| m.shown())
            .and_then(|doc| Intent::swap_panes(doc, &from, &to));
        if let Some(intent) = intent {
            self.edit(intent, fx);
        }
    }

    fn open_popup(&mut self, state: PopupState) {
        if let Mode::Copy(_) = self.mode {
            // Leaving copy mode for a popup: the popup replaces it.
            self.mode = Mode::Normal;
        }
        self.mode = Mode::Popup(state);
    }

    fn open(&mut self, popup: Popup, fx: &mut Effects) {
        let already = self.mode.popup().and_then(PopupState::popup) == Some(popup);
        self.close_mode(fx);
        if already {
            // Opening it again closes it.
            return;
        }
        let state = match popup {
            Popup::AgentJump => PopupState::Jump(JumpPopup::default()),
            Popup::SpacePicker => {
                // It opens with the shown space selected.
                let mut list = ListPopup::new(popup);
                let ids: Vec<String> = self.world.spaces().map(|s| s.id.clone()).collect();
                list.set_rows(ids.len());
                let active = self.active_space().map(|s| s.id.clone());
                list.cursor = ids
                    .iter()
                    .position(|id| Some(id) == active.as_ref())
                    .unwrap_or(0);
                PopupState::List(list)
            }
            _ => PopupState::List(ListPopup::new(popup)),
        };
        self.open_popup(state);
    }

    /// A rename prompt for a tab or space, its current name selected. A second rename while one
    /// is open closes it instead of stacking another.
    fn rename(&mut self, rename: Rename) {
        if let Mode::Popup(PopupState::Prompt(_)) = self.mode {
            self.mode = Mode::Normal;
            return;
        }
        let value = match &rename {
            Rename::Tab(id) => self.world.tab(id).map(|t| t.name.clone()),
            Rename::Space(id) => self.world.space(id).map(|s| s.name.clone()),
        };
        if let Some(value) = value {
            self.open_popup(PopupState::Prompt(Prompt { rename, value }));
        }
    }

    fn popup(&mut self, action: PopupAction, fx: &mut Effects) {
        let Mode::Popup(state) = &mut self.mode else {
            return;
        };
        match (state, action) {
            (_, PopupAction::Close) => self.mode = Mode::Normal,
            (PopupState::List(list), PopupAction::Up) => list.step(-1),
            (PopupState::List(list), PopupAction::Down) => list.step(1),
            (PopupState::List(list), PopupAction::Query(query)) => {
                list.query = query;
                list.cursor = 0;
            }
            (
                PopupState::List(list),
                pick @ (PopupAction::Choose | PopupAction::Toggle | PopupAction::CloseItem),
            ) => {
                let how = match pick {
                    PopupAction::Choose => Pick::Choose,
                    PopupAction::Toggle => Pick::Toggle,
                    _ => Pick::CloseItem,
                };
                fx.push(Effect::Pick {
                    popup: list.popup,
                    how,
                    cursor: list.cursor,
                    query: list.query.clone(),
                });
                if how == Pick::Choose {
                    // Closes first, so the popup can't take focus back from where it leads.
                    self.mode = Mode::Normal;
                }
            }
            (PopupState::Prompt(prompt), PopupAction::Query(value)) => prompt.value = value,
            (PopupState::Prompt(prompt), PopupAction::Choose) => {
                let name = prompt.value.trim().to_owned();
                let intent = match prompt.rename.clone() {
                    Rename::Tab(tab) => Intent::RenameTab { tab, name },
                    Rename::Space(space) => Intent::RenameSpace { space, name },
                };
                self.mode = Mode::Normal;
                self.edit(intent, fx);
            }
            (PopupState::Confirm(confirm), PopupAction::Choose) => {
                let then = confirm.then.clone();
                self.mode = Mode::Normal;
                match then {
                    Confirmed::CloseSpace(space) => self.close_space(&space, fx),
                }
            }
            _ => {}
        }
    }

    /// A key in the agent-jump bar: a whole label focuses its row's agent; the first of two
    /// letters waits for the second (`cfg/mods/agent-jump/main.js` `press`).
    fn jump_key(&mut self, c: char, fx: &mut Effects) {
        let Mode::Popup(PopupState::Jump(jump)) = &self.mode else {
            return;
        };
        let typed = format!("{}{c}", jump.typed);
        let rows = self.agent_rows();
        let labels = mode::jump_labels(rows.len());
        if let Some(i) = labels.iter().position(|l| *l == typed) {
            self.mode = Mode::Normal;
            let agent = rows[i].clone();
            self.focus_agent(&agent, fx);
            return;
        }
        let waits = labels.iter().any(|l| l.starts_with(&typed));
        if let Mode::Popup(PopupState::Jump(jump)) = &mut self.mode {
            jump.typed = if waits { typed } else { String::new() };
        }
    }

    /// `ctrl-g [`: copy mode in the focused terminal. With none, a warning, as the mod does.
    fn copy_mode(&mut self, fx: &mut Effects) {
        let Some(address) = self.focused_terminal().map(str::to_owned) else {
            fx.push(Effect::Toast(Toast::new(
                Level::Warning,
                "Copy mode",
                "No focused terminal",
            )));
            return;
        };
        self.close_mode(fx);
        fx.push(Effect::Terminal {
            address: address.clone(),
            command: TerminalCommand::CopyStart,
        });
        self.mode = Mode::Copy(CopyState {
            address,
            visual: None,
        });
    }

    fn copy(&mut self, action: CopyAction, fx: &mut Effects) {
        let Mode::Copy(copy) = &mut self.mode else {
            return;
        };
        let address = copy.address.clone();
        let command = |command| Effect::Terminal {
            address: address.clone(),
            command,
        };
        match action {
            CopyAction::SelectChars | CopyAction::SelectLines => {
                let kind = if action == CopyAction::SelectChars {
                    Visual::Chars
                } else {
                    Visual::Lines
                };
                // The same key again drops the selection.
                copy.visual = (copy.visual != Some(kind)).then_some(kind);
                fx.push(command(TerminalCommand::CopySelect(copy.visual)));
            }
            CopyAction::Escape if copy.visual.is_some() => {
                copy.visual = None;
                fx.push(command(TerminalCommand::CopySelect(None)));
            }
            CopyAction::Yank => {
                if copy.visual.is_some() {
                    fx.push(command(TerminalCommand::CopyYank));
                }
                self.close_mode(fx);
            }
            CopyAction::Quit | CopyAction::Escape => self.close_mode(fx),
            motion => fx.push(command(TerminalCommand::CopyMove(motion))),
        }
    }
}

/// `by` places from `at` in `order`, wrapping. From outside the order (`at` is `None`), next is
/// the first and previous the last; alone in it, nowhere.
fn step_in(order: &[String], at: Option<usize>, by: isize) -> Option<String> {
    if order.is_empty() {
        return None;
    }
    let Some(at) = at else {
        return Some(if by > 0 {
            order[0].clone()
        } else {
            order[order.len() - 1].clone()
        });
    };
    if order.len() < 2 {
        return None;
    }
    let n = order.len() as isize;
    Some(order[(at as isize + by).rem_euclid(n) as usize].clone())
}

#[cfg(test)]
mod tests {
    use super::step_in;

    fn order(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn stepping_wraps_and_starts_from_outside_at_the_ends() {
        let o = order(&["a", "b", "c"]);
        assert_eq!(step_in(&o, Some(0), 1).as_deref(), Some("b"));
        assert_eq!(step_in(&o, Some(2), 1).as_deref(), Some("a"));
        assert_eq!(step_in(&o, Some(0), -1).as_deref(), Some("c"));
        assert_eq!(step_in(&o, None, 1).as_deref(), Some("a"));
        assert_eq!(step_in(&o, None, -1).as_deref(), Some("c"));
        assert_eq!(step_in(&order(&["a"]), Some(0), 1), None);
        assert_eq!(step_in(&order(&["a"]), None, 1).as_deref(), Some("a"));
        assert_eq!(step_in(&[], None, 1), None);
    }
}
