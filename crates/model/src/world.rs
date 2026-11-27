// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! The shared world as this screen sees it: each machine's layout from its [`Mirror`], and the
//! core's terminal records. Everything here is shared with Herdr app and every other client; this
//! screen's own choices (focus, zoom, the selected tab) are in [`crate::view`].
//!
//! Ids are qualified, as herdr-app holds them: `devbox/sp…` for another machine's, bare for this
//! one's. Spaces are in machine order: this machine first, then the others by name; within a
//! machine, the shared order (`herdr-core-protocol.md` §3 "The `herdr-app` document").

use std::collections::BTreeMap;

use herdr_proto::Terminal;
use herdr_proto::types::{CoreInfo, MachineState, MachineView};
use herdr_shared::{Document, Mirror, Node, Space, Tab, machine_of};

use crate::checkouts::Checkout;

/// A machine: `None` is the core this screen is connected to.
pub type MachineKey = Option<String>;

/// One machine's layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Machine {
    pub mirror: Mirror,
    /// What [`Mirror::shown`] returned last: the document with this screen's pending edits.
    shown: Option<Document>,
    /// `machine.list`'s view of it. `None` for this machine.
    pub status: Option<MachineView>,
}

impl Machine {
    fn new(key: MachineKey) -> Machine {
        Machine {
            mirror: Mirror::new(key),
            shown: None,
            status: None,
        }
    }

    /// The layout as shown (`None` until read).
    pub fn shown(&self) -> Option<&Document> {
        self.shown.as_ref()
    }

    /// Whether calls to it can work now: this machine always, another one while connected.
    pub fn reachable(&self) -> bool {
        self.status
            .as_ref()
            .is_none_or(|s| s.state == MachineState::Connected)
    }

    pub(crate) fn refresh(&mut self) {
        self.shown = self.mirror.shown();
    }
}

/// Every machine's layout plus the terminal records.
#[derive(Clone, Debug, PartialEq)]
pub struct World {
    machines: BTreeMap<MachineKey, Machine>,
    terminals: BTreeMap<String, Terminal>,
    /// What git last said about each terminal's folder ([`crate::checkouts`]), where it is in a
    /// checkout.
    checkouts: BTreeMap<String, Checkout>,
    /// The core this screen is connected to (`hello.core`), and every machine `machine.list`
    /// has, temporary ones too: what a record's `parent` may name a machine by.
    core: Option<CoreInfo>,
    every_machine: Vec<MachineView>,
}

impl Default for World {
    fn default() -> Self {
        World::new()
    }
}

impl World {
    /// This machine, nothing read yet.
    pub fn new() -> World {
        let mut machines = BTreeMap::new();
        machines.insert(None, Machine::new(None));
        World {
            machines,
            terminals: BTreeMap::new(),
            checkouts: BTreeMap::new(),
            core: None,
            every_machine: Vec::new(),
        }
    }

    /// The machines, this one first, then by name.
    pub fn machines(&self) -> impl Iterator<Item = (&MachineKey, &Machine)> {
        self.machines.iter()
    }

    pub fn machine(&self, key: Option<&str>) -> Option<&Machine> {
        self.machines.get(&key.map(str::to_owned))
    }

    pub(crate) fn machine_mut(&mut self, key: Option<&str>) -> Option<&mut Machine> {
        self.machines.get_mut(&key.map(str::to_owned))
    }

    pub(crate) fn machines_mut(&mut self) -> impl Iterator<Item = (&MachineKey, &mut Machine)> {
        self.machines.iter_mut()
    }

    /// Takes a `machine.list`: other machines appear and go (temporary ones are left out, as
    /// herdr-app does). This machine always stays. Returns whether the set changed.
    pub(crate) fn set_machines(&mut self, list: &[MachineView]) -> bool {
        self.every_machine = list.to_vec();
        let wanted: BTreeMap<MachineKey, &MachineView> = list
            .iter()
            .filter(|m| !m.machine.temporary && !m.machine.name.is_empty())
            .map(|m| (Some(m.machine.name.clone()), m))
            .collect();
        let before: Vec<MachineKey> = self.machines.keys().cloned().collect();
        self.machines
            .retain(|key, _| key.is_none() || wanted.contains_key(key));
        for (key, view) in wanted {
            let machine = self
                .machines
                .entry(key.clone())
                .or_insert_with(|| Machine::new(key));
            machine.status = Some(view.clone());
        }
        before != self.machines.keys().cloned().collect::<Vec<_>>()
    }

    /// The core this screen is connected to, from its `hello`.
    pub(crate) fn set_core(&mut self, core: CoreInfo) {
        self.core = Some(core);
    }

    /// An address as this core names it. Another core's record may name a machine by that
    /// core's host name or core id (`host-alpha/t4`, `alpha-id/t4`), or name this machine
    /// itself: a prefix that is this core's id or name means a local `t4`; otherwise it maps
    /// through `machine.list` by name, then id, then a host only one machine has
    /// (`herdr-core-protocol.md` §5 "Parents and children"). Unknown prefixes stay as they are.
    pub fn normalize<'a>(&self, address: &'a str) -> std::borrow::Cow<'a, str> {
        use std::borrow::Cow;
        let Some((prefix, id)) = address.split_once('/') else {
            return Cow::Borrowed(address);
        };
        if self
            .core
            .as_ref()
            .is_some_and(|c| c.id == prefix || c.name == prefix)
        {
            return Cow::Owned(id.to_owned());
        }
        let machines = &self.every_machine;
        let by = |f: &dyn Fn(&MachineView) -> bool| machines.iter().find(|m| f(m));
        let named = by(&|m| m.machine.name == prefix).or_else(|| by(&|m| m.machine.id == prefix));
        let named = named.or_else(|| {
            let mut hosts = machines.iter().filter(|m| m.machine.host == prefix);
            match (hosts.next(), hosts.next()) {
                (Some(only), None) => Some(only),
                _ => None,
            }
        });
        match named {
            Some(m) if m.machine.name != prefix => Cow::Owned(format!("{}/{id}", m.machine.name)),
            _ => Cow::Borrowed(address),
        }
    }

    /// Whether a machine's layout has been read (`None`: this machine).
    pub fn loaded(&self, machine: Option<&str>) -> bool {
        self.machine(machine).is_some_and(|m| m.shown.is_some())
    }

    /// Whether the machine an id belongs to has been read, so an id missing from it is gone.
    pub fn loaded_for(&self, id: &str) -> bool {
        self.loaded(machine_of(id))
    }

    /// Every space, in order.
    pub fn spaces(&self) -> impl Iterator<Item = &Space> {
        self.machines
            .values()
            .filter_map(|m| m.shown.as_ref())
            .flat_map(|d| d.spaces.iter())
    }

    pub fn space(&self, id: &str) -> Option<&Space> {
        self.doc_for(id)?.spaces.iter().find(|s| s.id == id)
    }

    /// Every tab, in space order.
    pub fn tabs(&self) -> impl Iterator<Item = &Tab> {
        self.spaces().flat_map(|s| s.tabs.iter())
    }

    pub fn tab(&self, id: &str) -> Option<&Tab> {
        self.doc_for(id)?.tab(id)
    }

    /// The space holding a tab, or a node inside one.
    pub fn space_of(&self, node: &str) -> Option<&Space> {
        self.doc_for(node)?.space_of(node)
    }

    /// The tab holding a node (a pane or a container).
    pub fn tab_of(&self, node: &str) -> Option<&Tab> {
        self.doc_for(node)?.tab_of(node)
    }

    /// The pane showing a terminal.
    pub fn pane_of(&self, address: &str) -> Option<String> {
        self.doc_for(address)?.pane_of(address)
    }

    /// The terminal a pane shows.
    pub fn address_of(&self, pane: &str) -> Option<&str> {
        self.doc_for(pane)?.address_of(pane)
    }

    /// A node by id, anywhere.
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.tab_of(id)?.root.find(id)
    }

    fn doc_for(&self, id: &str) -> Option<&Document> {
        self.machine(machine_of(id))?.shown.as_ref()
    }

    pub fn terminal(&self, address: &str) -> Option<&Terminal> {
        self.terminals.get(address)
    }

    pub fn terminals(&self) -> impl Iterator<Item = &Terminal> {
        self.terminals.values()
    }

    pub fn live(&self) -> impl Iterator<Item = &Terminal> + Clone {
        self.terminals.values().filter(|t| t.is_live())
    }

    pub fn is_live(&self, address: &str) -> bool {
        self.terminals.get(address).is_some_and(Terminal::is_live)
    }

    /// A live terminal whose program reports a status: an agent (herdr-app's `agents()`).
    pub fn is_agent(&self, address: &str) -> bool {
        self.terminals
            .get(address)
            .is_some_and(|t| t.is_live() && t.status.is_some())
    }

    pub(crate) fn replace_terminals(&mut self, terminals: Vec<Terminal>) {
        self.terminals = terminals
            .into_iter()
            .map(|t| (t.address.clone(), t))
            .collect();
        let terminals = &self.terminals;
        self.checkouts.retain(|a, _| terminals.contains_key(a));
    }

    pub(crate) fn update_terminals(&mut self, updated: Vec<Terminal>, removed: &[String]) {
        for t in updated {
            self.terminals.insert(t.address.clone(), t);
        }
        for address in removed {
            self.terminals.remove(address);
            self.checkouts.remove(address);
        }
    }

    /// The checkout a terminal's folder is in, as git last said; `None` outside one, or before
    /// git said.
    pub fn checkout(&self, address: &str) -> Option<&Checkout> {
        self.checkouts.get(address)
    }

    /// What git said about a terminal's folder: the checkout it is in, or `None` (not one).
    pub(crate) fn set_checkout(&mut self, address: &str, checkout: Option<Checkout>) {
        match checkout {
            Some(c) => self.checkouts.insert(address.to_owned(), c),
            None => self.checkouts.remove(address),
        };
    }

    /// The folder a new terminal beside this one starts in: its live folder, else its launch
    /// folder.
    pub fn folder_of(&self, address: &str) -> Option<&str> {
        let t = self.terminals.get(address)?;
        t.foreground
            .as_ref()
            .map(|f| f.cwd.as_str())
            .filter(|cwd| !cwd.is_empty())
            .or_else(|| (!t.cwd.is_empty()).then_some(t.cwd.as_str()))
    }

    /// What a space is called: its name, else its first terminal's checkout (a worktree's own
    /// top), else that terminal's folder, else the space's own folder (`app:src/model.rs`
    /// `space_name`). A checkout stands until git answers for a folder the terminal moved to.
    pub fn space_name(&self, space: &Space, home: &str) -> String {
        if !space.name.is_empty() {
            return space.name.clone();
        }
        let first = space
            .tabs
            .iter()
            .flat_map(|t| t.root.terminals())
            .find_map(|n| n.terminal());
        let folder = first
            .and_then(|a| {
                self.checkouts
                    .get(a)
                    .map(|c| c.top.as_str())
                    .or_else(|| self.folder_of(a))
            })
            .unwrap_or(&space.folder);
        folder_name(folder, home)
    }
}

/// Looking a tab up in one space.
pub trait SpaceExt {
    fn tab(&self, id: &str) -> Option<&Tab>;
}

impl SpaceExt for Space {
    fn tab(&self, id: &str) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }
}

/// What a tab is called: its name, else its place in its space (1, 2, …) (`app:src/model.rs`
/// `tab_name`).
pub fn tab_name(space: &Space, tab: &Tab) -> String {
    if !tab.name.is_empty() {
        return tab.name.clone();
    }
    (space.tabs.iter().position(|t| t.id == tab.id).unwrap_or(0) + 1).to_string()
}

/// A folder as a name: its last part, `~` for the home folder (`app:src/model.rs`
/// `folder_name`).
pub fn folder_name(path: &str, home: &str) -> String {
    let full = path;
    let path = path.trim_end_matches(['/', '\\']);
    let home = home.trim_end_matches(['/', '\\']);
    if full.is_empty() || path == home {
        return "~".into();
    }
    if path.is_empty() {
        return full[..1].to_owned();
    }
    path.rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(path)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use herdr_proto::methods::DocGet;
    use herdr_proto::{Foreground, methods::DocGetResult};
    use herdr_shared::Round;
    use serde_json::json;

    fn read(world: &mut World, machine: Option<&str>, value: serde_json::Value) {
        let m = world.machine_mut(machine).unwrap();
        assert!(matches!(
            m.mirror.next_round(),
            Some(Round::Read(DocGet { .. }))
        ));
        m.mirror.finish_read(Ok(DocGetResult {
            value,
            version: 1,
            extra: Default::default(),
        }));
        m.refresh();
    }

    fn machine(name: &str, state: MachineState, temporary: bool) -> MachineView {
        let mut view = MachineView {
            state,
            ..Default::default()
        };
        view.machine.name = name.into();
        view.machine.temporary = temporary;
        view
    }

    fn doc(space: &str, tab: &str, pane: &str, terminal: &str) -> serde_json::Value {
        json!({"spaces": [{"id": space, "name": "", "folder": "/home/me/code/x", "tabs": [
            {"id": tab, "name": "", "root": {"id": tab, "type": "row", "children": [
                {"id": pane, "type": "block", "content": {"terminal": terminal}}]}}]}]})
    }

    #[test]
    fn spaces_come_this_machine_first_then_machines_by_name_with_qualified_ids() {
        let mut w = World::new();
        w.set_machines(&[
            machine("omarchy", MachineState::Connected, false),
            machine("devbox", MachineState::Connected, false),
            machine("sandbox", MachineState::Connected, true),
        ]);
        let keys: Vec<_> = w.machines().map(|(k, _)| k.clone()).collect();
        assert_eq!(keys, [None, Some("devbox".into()), Some("omarchy".into())]);
        read(&mut w, Some("omarchy"), doc("sp3", "tb3", "p3", "t3"));
        read(&mut w, None, doc("sp1", "tb1", "p1", "t1"));
        read(&mut w, Some("devbox"), doc("sp2", "tb2", "p2", "t2"));
        let ids: Vec<&str> = w.spaces().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["sp1", "devbox/sp2", "omarchy/sp3"]);
        assert_eq!(w.pane_of("devbox/t2").as_deref(), Some("devbox/p2"));
        assert_eq!(w.address_of("omarchy/p3"), Some("omarchy/t3"));
        assert_eq!(w.tab_of("devbox/p2").unwrap().id, "devbox/tb2");
        assert_eq!(w.space_of("devbox/tb2").unwrap().id, "devbox/sp2");
        assert!(w.loaded_for("devbox/p2"));
        assert!(w.tab("sp1").is_none());
    }

    #[test]
    fn addresses_naming_a_machine_by_its_core_id_or_host_or_this_core_are_normalized() {
        let mut w = World::new();
        w.set_core(CoreInfo {
            id: "here-id".into(),
            name: "laptop".into(),
            ..Default::default()
        });
        let mut alpha = machine("alpha", MachineState::Connected, false);
        alpha.machine.id = "alpha-id".into();
        alpha.machine.host = "host-alpha".into();
        let mut b1 = machine("b1", MachineState::Connected, false);
        b1.machine.host = "twin".into();
        let mut b2 = machine("b2", MachineState::Connected, true);
        b2.machine.host = "twin".into();
        w.set_machines(&[alpha, b1, b2]);
        let n = |a: &str| w.normalize(a).into_owned();
        assert_eq!(n("t4"), "t4");
        assert_eq!(n("alpha/t4"), "alpha/t4");
        assert_eq!(n("alpha-id/t4"), "alpha/t4");
        assert_eq!(n("host-alpha/t4"), "alpha/t4");
        assert_eq!(n("here-id/t4"), "t4");
        assert_eq!(n("laptop/t4"), "t4");
        assert_eq!(n("twin/t4"), "twin/t4", "two machines share that host");
        assert_eq!(n("nobody/t4"), "nobody/t4");
    }

    #[test]
    fn a_machine_that_leaves_the_list_takes_its_layout_with_it() {
        let mut w = World::new();
        assert!(w.set_machines(&[machine("devbox", MachineState::Connected, false)]));
        read(&mut w, Some("devbox"), doc("sp2", "tb2", "p2", "t2"));
        assert!(!w.set_machines(&[machine("devbox", MachineState::Unreachable, false)]));
        assert!(!w.machine(Some("devbox")).unwrap().reachable());
        assert!(w.machine(None).unwrap().reachable());
        assert!(w.set_machines(&[]));
        assert_eq!(w.spaces().count(), 0);
        assert!(w.machine(None).is_some(), "this machine always stays");
    }

    #[test]
    fn unnamed_spaces_take_their_first_terminals_folder_and_tabs_their_number() {
        let mut w = World::new();
        read(&mut w, None, doc("sp1", "tb1", "p1", "t1"));
        let space = w.space("sp1").unwrap().clone();
        assert_eq!(w.space_name(&space, "/home/me"), "x", "the space's folder");
        w.replace_terminals(vec![Terminal {
            address: "t1".into(),
            cwd: "/home/me/code/launch".into(),
            ..Default::default()
        }]);
        assert_eq!(w.space_name(&space, "/home/me"), "launch");
        w.update_terminals(
            vec![Terminal {
                address: "t1".into(),
                cwd: "/home/me/code/launch".into(),
                foreground: Some(Foreground {
                    cwd: "/home/me".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            &[],
        );
        assert_eq!(w.space_name(&space, "/home/me"), "~");
        let mut named = space.clone();
        named.name = "work".into();
        assert_eq!(w.space_name(&named, "/home/me"), "work");
        assert_eq!(tab_name(&space, &space.tabs[0]), "1");
        w.update_terminals(vec![], &["t1".into()]);
        assert!(w.terminal("t1").is_none());
    }

    #[test]
    fn an_unnamed_space_takes_its_first_terminals_checkout() {
        let mut w = World::new();
        read(&mut w, None, doc("sp1", "tb1", "p1", "t1"));
        let space = w.space("sp1").unwrap().clone();
        w.replace_terminals(vec![Terminal {
            address: "t1".into(),
            cwd: "/home/me/code/repo/crates/app".into(),
            ..Default::default()
        }]);
        assert_eq!(w.space_name(&space, "/home/me"), "app", "before git said");
        let checkout = Checkout {
            top: "/home/me/code/repo".into(),
            branch: Some("main".into()),
        };
        w.set_checkout("t1", Some(checkout.clone()));
        assert_eq!(w.space_name(&space, "/home/me"), "repo");
        assert_eq!(w.checkout("t1"), Some(&checkout));
        w.set_checkout("t1", None);
        assert_eq!(w.space_name(&space, "/home/me"), "app", "not a checkout");
        w.set_checkout("t1", Some(checkout));
        w.replace_terminals(vec![]);
        assert_eq!(w.checkout("t1"), None, "gone with its terminal");
    }

    #[test]
    fn folder_names_follow_herdr() {
        assert_eq!(folder_name("/home/me/code/herdr/", "/home/me"), "herdr");
        assert_eq!(folder_name("/home/me", "/home/me/"), "~");
        assert_eq!(folder_name("", "/home/me"), "~");
        assert_eq!(folder_name("/", "/home/me"), "/");
        assert_eq!(folder_name("C:\\work\\repo", "/home/me"), "repo");
    }

    #[test]
    fn agents_are_live_terminals_that_report_a_status() {
        let mut w = World::new();
        w.replace_terminals(vec![
            Terminal {
                address: "t1".into(),
                state: herdr_proto::TerminalState::Live,
                status: Some(Default::default()),
                ..Default::default()
            },
            Terminal {
                address: "t2".into(),
                state: herdr_proto::TerminalState::Live,
                ..Default::default()
            },
            Terminal {
                address: "t3".into(),
                state: herdr_proto::TerminalState::Finished,
                status: Some(Default::default()),
                ..Default::default()
            },
        ]);
        assert!(w.is_agent("t1"));
        assert!(!w.is_agent("t2"));
        assert!(!w.is_agent("t3"));
        assert_eq!(w.live().count(), 2);
    }
}
