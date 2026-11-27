//! What each [`Action`] does to the model. Layout changes go through the machine's `Mirror` as
//! [`Intent`]s; terminals start and end through core calls; everything else is this screen's view.
//! Actions whose feature isn't built yet return [`Effect::NotYet`].

use herdr_proto::methods::{MachineExec, TerminalLabel};
use herdr_shared::{Axis, Intent, Node, Size, fresh_id_on, labels, machine_of};

use crate::action::{Action, CopyAction, Direction, Edge, Popup, PopupAction, Split};
use crate::effect::{Effect, Effects, Level, Pick, Request, TerminalCommand, Toast};
use crate::folders::{self, Lookup, NewSpace};
use crate::geometry;
use crate::links;
use crate::machines::{MachineAction, MachineCall};
use crate::mode::{
    self, Confirm, Confirmed, CopyState, JumpPopup, ListPopup, Menu, Mode, PopupState, Prompt,
    Rename, Visual,
};
use crate::model::{EDITOR_LABEL, Launch, Model, NAVIGATE_TIMEOUT, Pending};
use crate::popups;
use crate::world::SpaceExt;

impl Model {
    pub(crate) fn run(&mut self, action: Action, fx: &mut Effects) {
        match action {
            Action::App(command) => fx.push(Effect::App(command)),
            Action::Open(popup) => self.open(popup, fx),
            Action::CopyMode => self.copy_mode(fx),
            Action::Review => self.review(fx),
            Action::TabNew => self.new_tab(None, fx),
            Action::NewTabIn(space) => {
                if self.world.space(&space).is_some() {
                    self.new_tab(Some(&space), fx);
                }
            }
            Action::TabMenu(tab) => {
                if let Some(space) = self.world.space_of(&tab).filter(|s| s.tab(&tab).is_some()) {
                    let menu = Menu::tab(&tab, &space.id);
                    self.close_mode(fx);
                    self.open_popup(PopupState::Menu(menu));
                }
            }
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
                let focused = self
                    .focused_terminal()
                    .filter(|a| self.world.is_agent(a))
                    .map(|a| self.root_agent(a));
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
            Action::PaneNavigate(direction) => self.navigate(direction, fx),
            Action::PaneSwap(direction) => self.swap(direction, fx),
            Action::PaneResize(direction) => self.resize(direction, fx),
            Action::ResizeSplit { sizes } => {
                let changes = sizes.iter().any(|(id, size)| {
                    self.world
                        .node(id)
                        .is_some_and(|n| !same_size(n.size(), *size))
                });
                if changes {
                    self.edit(Intent::Resize { sizes }, fx);
                }
            }
            Action::DragDivider {
                container,
                index,
                at,
            } => self.drag_divider(&container, index, at, fx),
            Action::RearrangePane { pane, target, edge } => {
                self.rearrange(&pane, &target, edge, fx)
            }
            Action::Terminal(command) => {
                if let Some(address) = self.keyboard_terminal().map(str::to_owned) {
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
            Action::TerminalMenu(pane) => {
                if let Some(menu) = popups::terminal_menu(self, &pane) {
                    self.close_mode(fx);
                    self.open_popup(PopupState::Menu(menu));
                }
            }
            Action::SpaceNew(machine) => self.new_space(machine, None, fx),
            Action::NewSpaceIn { machine, folder } => self.new_space(machine, folder, fx),
            Action::ShowSpace(id) => {
                let tab = self
                    .world
                    .space(&id)
                    .map(|space| popups::terminal_tab(self, space));
                match tab {
                    Some(Some(tab)) => {
                        self.view.select_tab(&self.world, &tab);
                    }
                    Some(None) => self.view.active = Some(id),
                    None => {}
                }
            }
            Action::FromPalette(action) => {
                if let Some(name) = action.name() {
                    self.view.remember_command(&name);
                }
                self.run(*action, fx);
            }
            Action::RightClicks {
                terminal,
                to_program,
            } => {
                if self.world.is_live(&terminal) {
                    let to = if to_program { "program" } else { "" };
                    let labels = [(labels::RIGHT_CLICK.to_owned(), to.to_owned())].into();
                    self.call(
                        fx,
                        Request::Label(TerminalLabel {
                            terminal,
                            labels,
                            ..Default::default()
                        }),
                        None,
                    );
                }
            }
            Action::Park(terminal) => {
                if let Some(pane) = self.world.pane_of(&terminal) {
                    self.call(
                        fx,
                        Request::Label(TerminalLabel {
                            terminal: terminal.clone(),
                            labels: labels::for_park(),
                            ..Default::default()
                        }),
                        Some(Pending::Park {
                            pane,
                            terminal,
                            labelled: false,
                        }),
                    );
                }
            }
            Action::EditFile(path) => self.edit_file(&path, fx),
            Action::RenamePaneOf(pane) => self.rename(Rename::Pane(pane)),
            Action::Machine(action) => self.machine(action, fx),
            Action::CopyText(text) => fx.push(Effect::Clipboard(text)),
            Action::Peek(address) => self.open_peek(&address, fx),
            Action::ClosePeek => self.peek = None,
            Action::KeepPeek => self.keep_peeked(fx),
            Action::FoldSubagents(_) => fx.push(Effect::NotYet(action)),
            Action::OpenLink { pane, link } => self.open_link(&pane, &link, fx),
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

    /// `tab.new`: a tab after the selected one of `space` (else the shown space), in the focused
    /// terminal's folder, or the space's own when it isn't the shown one. With no space, a new
    /// space on this machine.
    fn new_tab(&mut self, space: Option<&str>, fx: &mut Effects) {
        self.new_tab_running(space, "", Vec::new(), fx);
    }

    /// [`Model::new_tab`] named `name` (empty: by its number), running `argv` (empty: the login
    /// shell) inside the login shell of the space's machine ([`crate::shell`]): herdr-app's
    /// `tab.new {space, name, argv}`.
    pub(crate) fn new_tab_running(
        &mut self,
        space: Option<&str>,
        name: &str,
        argv: Vec<String>,
        fx: &mut Effects,
    ) {
        let shell = self.shell.clone();
        let wrap = |argv: Vec<String>, machine: Option<&str>| match (argv.is_empty(), machine) {
            (true, _) => argv,
            (false, Some(_)) => crate::shell::remote(argv),
            (false, None) => crate::shell::login(&shell, argv),
        };
        let shown = self.active_space().map(|s| s.id.clone());
        let space = match space {
            Some(id) => self.world.space(id),
            None => self.active_space(),
        };
        let folder = match space {
            Some(s) if Some(&s.id) != shown.as_ref() && !s.folder.is_empty() => s.folder.clone(),
            _ => self.new_folder(),
        };
        let Some(space) = space else {
            let launch = Launch::Space {
                space: fresh_id_on("sp", None),
                name: String::new(),
                folder: folder.clone(),
                tab: fresh_id_on("tb", None),
                tab_name: name.to_owned(),
                pane: fresh_id_on("p", None),
                focus: true,
            };
            self.launch(None, &folder, launch, wrap(argv, None), None, fx);
            return;
        };
        let machine = machine_of(&space.id).map(str::to_owned);
        let m = machine.as_deref();
        let argv = wrap(argv, m);
        let launch = Launch::Tab {
            space: space.id.clone(),
            tab: fresh_id_on("tb", m),
            name: name.to_owned(),
            pane: fresh_id_on("p", m),
            after: self
                .view
                .selected_tab(&self.world, space)
                .map(|t| t.id.clone()),
        };
        self.launch(machine, &folder, launch, argv, None, fx);
    }

    /// `space.new` from the window: a space on `machine` (this computer for `None`), shown at
    /// once, in `folder`; with none, in the focused terminal's folder when that is on the same
    /// machine, else home (on another machine, its core's default).
    fn new_space(&mut self, machine: Option<String>, folder: Option<String>, fx: &mut Effects) {
        let m = machine.as_deref();
        let folder = folder
            .filter(|f| !f.is_empty())
            .or_else(|| {
                self.focused_terminal()
                    .filter(|t| machine_of(t) == m)
                    .and_then(|t| self.world.folder_of(t))
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| {
                if m.is_some() {
                    String::new()
                } else {
                    self.home.clone()
                }
            });
        let launch = Launch::Space {
            space: fresh_id_on("sp", m),
            name: String::new(),
            folder: folder.clone(),
            tab: fresh_id_on("tb", m),
            tab_name: String::new(),
            pane: fresh_id_on("p", m),
            focus: true,
        };
        self.launch(machine, &folder, launch, Vec::new(), None, fx);
    }

    /// The settings dialog's files: a new tab on this computer running `$EDITOR` (else `nvim`)
    /// on `path`, in the shown space when it is this computer's, else the first of this
    /// computer's spaces, else a new one.
    fn edit_file(&mut self, path: &str, fx: &mut Effects) {
        let folder = std::path::Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| self.home.clone());
        let argv = editor_argv(path);
        let space = self
            .active_space()
            .filter(|s| machine_of(&s.id).is_none())
            .or_else(|| self.world.spaces().find(|s| machine_of(&s.id).is_none()))
            .map(|s| s.id.clone());
        let launch = match space {
            Some(space) => {
                let after = self
                    .world
                    .space(&space)
                    .and_then(|s| self.view.selected_tab(&self.world, s))
                    .map(|t| t.id.clone());
                Launch::Tab {
                    space,
                    tab: fresh_id_on("tb", None),
                    name: String::new(),
                    pane: fresh_id_on("p", None),
                    after,
                }
            }
            None => Launch::Space {
                space: fresh_id_on("sp", None),
                name: String::new(),
                folder: folder.clone(),
                tab: fresh_id_on("tb", None),
                tab_name: String::new(),
                pane: fresh_id_on("p", None),
                focus: true,
            },
        };
        self.launch(None, &folder, launch, argv, None, fx);
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
        self.launch(machine, &folder, launch, Vec::new(), None, fx);
    }

    /// A Cmd-click on a link in pane `pane` ([`crate::links`]): a URL goes to the system's
    /// opener; a path or a `file:` URI is looked up on the pane's machine, and the answer
    /// ([`Model::link_found`]) opens it.
    fn open_link(&mut self, pane: &str, link: &str, fx: &mut Effects) {
        let Some(address) = self.world.address_of(pane).map(str::to_owned) else {
            return;
        };
        let machine = machine_of(&address).map(str::to_owned);
        // A `file:` URI (Pi's OSC 8 links) names a file on the pane's machine, as a path does.
        let file_uri = link
            .get(..5)
            .is_some_and(|s| s.eq_ignore_ascii_case("file:"));
        if links::has_scheme(link) && !file_uri {
            fx.push(Effect::Open(link.to_owned()));
            return;
        }
        let Some(lookup) = links::Lookup::of(link) else {
            return;
        };
        let cwd = self
            .world
            .folder_of(&address)
            .unwrap_or_default()
            .to_owned();
        let exec = herdr_proto::methods::MachineExec {
            argv: lookup.argv(&cwd),
            on: machine.clone(),
            timeout: Some(links::LOOKUP_TIMEOUT),
            ..Default::default()
        };
        self.call(
            fx,
            Request::Exec(exec),
            Some(Pending::Link {
                pane: pane.to_owned(),
                local: machine.is_none(),
                lookup,
            }),
        );
    }

    /// Where a clicked path turned out to be: a file Neovim edits opens in Neovim to the right
    /// of the pane, anything else here goes to the system's opener; a path found nowhere, or
    /// anything but a Neovim file on another machine, opens nothing. A lookup that couldn't run
    /// says so.
    pub(crate) fn link_found(
        &mut self,
        pane: &str,
        local: bool,
        lookup: &links::Lookup,
        result: Result<herdr_proto::methods::ExecResult, herdr_proto::RpcError>,
        fx: &mut Effects,
    ) {
        let output = match result {
            Ok(output) => output,
            Err(error) => {
                fx.push(Effect::Toast(Toast::new(
                    Level::Error,
                    "Couldn't open the link",
                    error.message,
                )));
                return;
            }
        };
        if !output.succeeded() {
            return;
        }
        let Some(found) = links::found(lookup, &output.stdout_text()) else {
            return;
        };
        if found.file && links::is_neovim_file(&found.path) {
            let argv = links::neovim(&found.path, found.place);
            self.split_running(pane, argv, fx);
        } else if local {
            fx.push(Effect::Open(found.path));
        }
    }

    /// A split to the right of `pane` running `argv`, on the pane's machine, in its terminal's
    /// folder.
    fn split_running(&mut self, pane: &str, argv: Vec<String>, fx: &mut Effects) {
        if self.world.tab_of(pane).is_none() {
            return;
        }
        let machine = machine_of(pane).map(str::to_owned);
        let m = machine.as_deref();
        let folder = self
            .world
            .address_of(pane)
            .and_then(|a| self.world.folder_of(a))
            .map(str::to_owned)
            .unwrap_or_else(|| self.new_folder());
        let launch = Launch::Split {
            target: pane.to_owned(),
            axis: Axis::Row,
            pane: fresh_id_on("p", m),
            container: fresh_id_on("s", m),
        };
        self.launch(machine, &folder, launch, argv, None, fx);
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

    pub(crate) fn close_space(&mut self, id: &str, fx: &mut Effects) {
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
    pub(crate) fn focus_agent(&mut self, address: &str, fx: &mut Effects) {
        match self.world.pane_of(address) {
            Some(pane) => {
                self.view.focus_pane(&self.world, &pane);
            }
            None => self.open_peek(address, fx),
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

    pub(crate) fn go(&mut self, direction: Direction) {
        if let Some((_, to)) = self.neighbor(direction, true) {
            self.view.focus_pane(&self.world, &to);
        }
    }

    /// Ctrl+H/J/K/L: a terminal whose Neovim labeled it gets the key over Neovim's RPC server,
    /// on the terminal's machine, so the move goes through Neovim's own splits first; Neovim
    /// asks for the pane move at its edge ([`crate::model::NAVIGATOR_LABEL`]). Anything else
    /// moves the pane at once. Never `hc keys`: the core holds an agent's keys until its person
    /// stops typing (`cfg/mods/navigator-status/navigation.js`).
    fn navigate(&mut self, direction: Direction, fx: &mut Effects) {
        let Some(pane) = self.focused_pane() else {
            return;
        };
        let address = self.world.address_of(&pane).map(str::to_owned);
        let server = address
            .as_deref()
            .and_then(|a| self.world.terminal(a))
            .and_then(|t| t.label(EDITOR_LABEL))
            .filter(|v| !v.is_empty())
            .map(str::to_owned);
        let (Some(address), Some(server)) = (address, server) else {
            self.go(direction);
            return;
        };
        let key = match direction {
            Direction::Left => 'h',
            Direction::Down => 'j',
            Direction::Up => 'k',
            Direction::Right => 'l',
        };
        let exec = MachineExec {
            argv: vec![
                "nvim".into(),
                "--server".into(),
                server,
                "--remote-send".into(),
                format!("<C-{key}>"),
            ],
            on: machine_of(&address).map(str::to_owned),
            timeout: Some(NAVIGATE_TIMEOUT),
            ..Default::default()
        };
        let tab = self.shown_tab().map(|t| t.id.clone());
        self.call(
            fx,
            Request::Exec(exec),
            Some(Pending::Navigate {
                address,
                pane,
                tab,
                direction,
            }),
        );
    }

    /// `alt-shift-h/j/k/l`: the focused pane's nearest divider along `direction`'s axis moves
    /// [`geometry::RESIZE_STEP`] that way, as dragging it would. Nothing in a zoomed tab, or
    /// where nothing is split that way (`app:src/window.rs` `resize_pane`).
    fn resize(&mut self, direction: Direction, fx: &mut Effects) {
        let Some(tab) = self.shown_tab() else {
            return;
        };
        let Some(pane) = self.view.focused_in(tab) else {
            return;
        };
        if self.view.zoomed_in(tab).is_some() {
            return;
        }
        let Some((container, index)) = geometry::resize_divider(&tab.root, &pane, direction) else {
            return;
        };
        let Some(before) = tab
            .root
            .find(&container)
            .and_then(|c| c.children().iter().filter(|c| !c.hidden()).nth(index))
            .map(|n| n.id.clone())
        else {
            return;
        };
        let rects = geometry::nodes(&tab.root, &self.geometry);
        let Some(rect) = rects.iter().find(|(id, _)| *id == before).map(|(_, r)| *r) else {
            return;
        };
        let step = geometry::RESIZE_STEP * self.geometry.zoom;
        let at = match direction {
            Direction::Left => rect.right() - step,
            Direction::Right => rect.right() + step,
            Direction::Up => rect.bottom() - step,
            Direction::Down => rect.bottom() + step,
        };
        self.drag_divider(&container, index, at, fx);
    }

    /// A divider of the shown tab follows the pointer (see [`geometry::drag_sizes`]).
    fn drag_divider(&mut self, container: &str, index: usize, at: f32, fx: &mut Effects) {
        let Some(tab) = self.shown_tab() else {
            return;
        };
        if self.view.zoomed_in(tab).is_some() {
            return;
        }
        let Some(sizes) = geometry::drag_sizes(&tab.root, container, index, at, &self.geometry)
        else {
            return;
        };
        let unchanged = sizes
            .iter()
            .all(|(id, size)| same_size(tab.root.find(id).and_then(Node::size), *size));
        if !unchanged {
            self.edit(Intent::Resize { sizes }, fx);
        }
    }

    /// Whether a pane of the shown tab may be dragged to rearrange it: not in a zoomed or
    /// one-pane tab, nor under a node that sets `movable: false`.
    pub fn pane_movable(&self, pane: &str) -> bool {
        let Some(tab) = self.shown_tab() else {
            return false;
        };
        self.view.zoomed_in(tab).is_none()
            && tab.root.panes().len() > 1
            && tab.root.find(pane).is_some_and(Node::is_pane)
            && tab.root.movable_at(pane)
    }

    /// A pane's grip dropped on another pane of its tab: the center swaps them, an edge places
    /// it beside. The moved pane takes focus (`app:src/model.rs` `move_pane`).
    fn rearrange(&mut self, pane: &str, target: &str, edge: Edge, fx: &mut Effects) {
        if pane == target || !self.pane_movable(pane) {
            return;
        }
        let Some(tab) = self
            .shown_tab()
            .filter(|t| t.root.find(target).is_some_and(Node::is_pane))
            .map(|t| t.id.clone())
        else {
            return;
        };
        let machine = machine_of(pane);
        let Some(doc) = self.world.machine(machine).and_then(|m| m.shown()) else {
            return;
        };
        let intent = match edge.side() {
            None => Intent::swap_panes(doc, pane, target),
            Some(side) => Intent::move_pane(doc, pane, target, side, fresh_id_on("s", machine)),
        };
        if let Some(intent) = intent
            && self.edit(intent, fx)
        {
            self.view.focus(&tab, pane);
        }
    }

    /// The dividers of the shown tab as drawn: none while it is zoomed.
    pub fn dividers(&self) -> Vec<geometry::Divider> {
        self.shown_tab()
            .map(|tab| geometry::dividers(self.view.shown_root(tab), &self.geometry))
            .unwrap_or_default()
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
            Popup::NewSpace => {
                self.look_for_folders(fx);
                PopupState::List(ListPopup::new(popup))
            }
            _ => PopupState::List(ListPopup::new(popup)),
        };
        self.open_popup(state);
    }

    /// The new-space picker opens: it offers the focused terminal's folder at once and asks the
    /// shown space's machine for zoxide's folders, the repos config and (on another machine)
    /// `$HOME`, each within [`folders::LOOKUP_TIMEOUT`] (`cfg/mods/new-space/main.js` `init`).
    fn look_for_folders(&mut self, fx: &mut Effects) {
        self.openings += 1;
        let opening = self.openings;
        let machine = self
            .active_space()
            .and_then(|s| machine_of(&s.id))
            .map(str::to_owned);
        let focused = self
            .focused_terminal()
            .and_then(|t| self.world.folder_of(t))
            .map(str::to_owned);
        let mut waiting = vec![Lookup::Zoxide, Lookup::Repos];
        if machine.is_some() {
            waiting.insert(0, Lookup::Home);
        }
        let home = machine.is_none().then(|| self.home.clone());
        for &lookup in &waiting {
            let exec = MachineExec {
                argv: lookup.argv(),
                on: machine.clone(),
                timeout: Some(folders::LOOKUP_TIMEOUT),
                ..Default::default()
            };
            self.call(
                fx,
                Request::Exec(exec),
                Some(Pending::Folders { opening, lookup }),
            );
        }
        self.new_space = Some(NewSpace {
            opening,
            machine,
            focused,
            home,
            zoxide: None,
            repos: None,
            waiting,
        });
    }

    /// A rename prompt for a tab, space or pane, its current name selected (an unnamed pane's is
    /// empty). A second rename while one is open closes it instead of stacking another.
    fn rename(&mut self, rename: Rename) {
        if let Mode::Popup(PopupState::Prompt(_)) = self.mode {
            self.mode = Mode::Normal;
            return;
        }
        let value = match &rename {
            Rename::Tab(id) => self.world.tab(id).map(|t| t.name.clone()),
            Rename::Space(id) => self.world.space(id).map(|s| s.name.clone()),
            Rename::Pane(id) => self
                .world
                .tab_of(id)
                .and_then(|t| t.root.find(id))
                .filter(|n| n.is_pane())
                .map(|n| n.title().unwrap_or_default().to_owned()),
            Rename::Machine(name) => self.world.machine(Some(name)).and(Some(name.clone())),
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
            (PopupState::List(list), PopupAction::PageUp) => list.step(-mode::PAGE),
            (PopupState::List(list), PopupAction::PageDown) => list.step(mode::PAGE),
            (PopupState::Menu(menu), PopupAction::Up) => menu.step(-1),
            (PopupState::Menu(menu), PopupAction::Down) => menu.step(1),
            (PopupState::Menu(menu), PopupAction::Choose) => {
                let actions = menu
                    .items
                    .get(menu.cursor)
                    .map(|item| item.actions.clone())
                    .unwrap_or_default();
                // Closes first, so what it runs can open a prompt of its own.
                self.mode = Mode::Normal;
                for action in actions {
                    self.run(action, fx);
                }
            }
            (PopupState::List(list), PopupAction::Query(query)) => {
                list.query = query;
                list.cursor = 0;
            }
            // Enter while the folders are still coming with nothing matching waits, instead of
            // falling back to the focused folder the filter left out.
            (PopupState::List(list), PopupAction::Choose)
                if list.popup == Popup::NewSpace
                    && self
                        .new_space
                        .as_ref()
                        .is_some_and(|ns| ns.loading() && ns.rows(&list.query).is_empty()) => {}
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
                // Closes first, so the popup can't take focus back from where it leads. The add
                // dialog stays: it shows the machine connecting, and what went wrong.
                if how == Pick::Choose && list.popup != Popup::MachineAdd {
                    self.mode = Mode::Normal;
                }
            }
            (PopupState::Prompt(prompt), PopupAction::Query(value)) => prompt.value = value,
            (PopupState::Prompt(prompt), PopupAction::Choose) => {
                let name = prompt.value.trim().to_owned();
                let intent = match prompt.rename.clone() {
                    Rename::Tab(tab) => Intent::RenameTab { tab, name },
                    Rename::Space(space) => Intent::RenameSpace { space, name },
                    Rename::Pane(pane) => Intent::RenamePane { pane, name },
                    Rename::Machine(machine) => {
                        self.mode = Mode::Normal;
                        // A machine keeps its name when none is given.
                        if !name.is_empty() && name != machine {
                            fx.push(Effect::Machine(MachineCall::Rename { machine, to: name }));
                        }
                        return;
                    }
                };
                self.mode = Mode::Normal;
                self.edit(intent, fx);
            }
            (PopupState::Confirm(confirm), PopupAction::Choose) => {
                let then = confirm.then.clone();
                self.mode = Mode::Normal;
                match then {
                    Confirmed::CloseSpace(space) => self.close_space(&space, fx),
                    Confirmed::RemoveMachine(machine) => {
                        fx.push(Effect::Machine(MachineCall::Remove(machine)))
                    }
                }
            }
            _ => {}
        }
    }

    /// A machine action from the Machines dialog or the add dialog: a call, or the prompt or
    /// confirm dialog that leads to one. The dialog it came from is closed by the pick.
    fn machine(&mut self, action: MachineAction, fx: &mut Effects) {
        match action {
            MachineAction::Menu(name) => {
                let Some(view) = self
                    .world
                    .machine(Some(&name))
                    .and_then(|m| m.status.clone())
                else {
                    return;
                };
                self.close_mode(fx);
                self.open_popup(PopupState::Menu(crate::machines::machine_menu(&view)));
            }
            MachineAction::Connect(name) => fx.push(Effect::Machine(MachineCall::Connect(name))),
            MachineAction::Add(target) => {
                let target = target.trim();
                if !target.is_empty() {
                    fx.push(Effect::Machine(MachineCall::Add(target.to_owned())));
                }
            }
            MachineAction::Rename(name) => {
                self.close_mode(fx);
                self.rename(Rename::Machine(name));
            }
            MachineAction::Remove(name) => {
                if self.world.machine(Some(&name)).is_none() {
                    return;
                }
                self.close_mode(fx);
                self.open_popup(PopupState::Confirm(Confirm {
                    title: "Remove machine?".into(),
                    detail: format!("{name}: its terminals keep running there."),
                    then: Confirmed::RemoveMachine(name),
                }));
            }
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
        let Some(address) = self.keyboard_terminal().map(str::to_owned) else {
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

/// Whether a node's size is already `size` (no size is a weight of 1).
fn same_size(now: Option<Size>, size: Size) -> bool {
    now == Some(size) || (now.is_none() && size == Size::Weight(1.0))
}

/// What runs `path` in the person's editor: `$EDITOR` as the core's environment has it (its words
/// split as a shell splits them), else Neovim.
pub(crate) fn editor_argv(path: &str) -> Vec<String> {
    ["/bin/sh", "-c", "exec ${EDITOR:-nvim} \"$1\"", "sh", path]
        .into_iter()
        .map(str::to_owned)
        .collect()
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
