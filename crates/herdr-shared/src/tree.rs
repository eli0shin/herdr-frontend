//! Tree operations on nodes and documents, ported from herdr-app (`app:src/layout.rs` `Node`,
//! `app:src/model.rs` `Model`) so both apps edit the shared tree the same way: split sizes, what
//! collapses, which neighbor gets a closed pane's share.

use serde_json::Value;

use crate::document::{Axis, Content, Document, Kind, Node, Shows, Size, Space, Tab};

impl Node {
    pub fn children(&self) -> &[Node] {
        match &self.kind {
            Kind::Row { children } | Kind::Column { children } | Kind::Stack { children } => {
                children
            }
            Kind::Other {
                children: Some(children),
                ..
            } => children,
            Kind::Block { .. } | Kind::Other { children: None, .. } => &[],
        }
    }

    pub fn children_mut(&mut self) -> Option<&mut Vec<Node>> {
        match &mut self.kind {
            Kind::Row { children } | Kind::Column { children } | Kind::Stack { children } => {
                Some(children)
            }
            Kind::Other {
                children: Some(children),
                ..
            } => Some(children),
            Kind::Block { .. } | Kind::Other { children: None, .. } => None,
        }
    }

    pub fn axis(&self) -> Option<Axis> {
        match self.kind {
            Kind::Row { .. } => Some(Axis::Row),
            Kind::Column { .. } => Some(Axis::Column),
            _ => None,
        }
    }

    pub fn content(&self) -> Option<&Content> {
        match &self.kind {
            Kind::Block { content } => Some(content),
            _ => None,
        }
    }

    /// The terminal address, when this is a terminal block.
    pub fn terminal(&self) -> Option<&str> {
        self.content().and_then(Content::address)
    }

    /// A pane in a tab: a terminal, or a mod beside them.
    pub fn is_pane(&self) -> bool {
        matches!(
            self.content().map(Content::shows),
            Some(Shows::Terminal(_) | Shows::Mod { .. })
        )
    }

    /// Every node, depth first, itself included.
    pub fn walk(&self) -> Vec<&Node> {
        let mut out = vec![self];
        for child in self.children() {
            out.extend(child.walk());
        }
        out
    }

    pub fn find(&self, id: &str) -> Option<&Node> {
        if self.id == id {
            return Some(self);
        }
        self.children().iter().find_map(|c| c.find(id))
    }

    pub fn find_mut(&mut self, id: &str) -> Option<&mut Node> {
        if self.id == id {
            return Some(self);
        }
        self.children_mut()?.iter_mut().find_map(|c| c.find_mut(id))
    }

    /// The ids from this node down to `id`, both included.
    pub fn path_to(&self, id: &str) -> Option<Vec<String>> {
        if self.id == id {
            return Some(vec![self.id.clone()]);
        }
        self.children()
            .iter()
            .find_map(|c| c.path_to(id))
            .map(|mut path| {
                path.insert(0, self.id.clone());
                path
            })
    }

    pub fn parent_of(&self, id: &str) -> Option<&Node> {
        let path = self.path_to(id)?;
        (path.len() >= 2)
            .then(|| self.find(&path[path.len() - 2]))
            .flatten()
    }

    /// Terminal blocks, in tree order.
    pub fn terminals(&self) -> Vec<&Node> {
        self.walk()
            .into_iter()
            .filter(|n| n.terminal().is_some())
            .collect()
    }

    /// Pane blocks, in tree order.
    pub fn panes(&self) -> Vec<&Node> {
        self.walk().into_iter().filter(|n| n.is_pane()).collect()
    }

    /// Terminal addresses, in tree order.
    pub fn addresses(&self) -> Vec<String> {
        self.terminals()
            .iter()
            .filter_map(|n| n.terminal().map(str::to_owned))
            .collect()
    }

    /// Removes the node `id`. Its share goes back to the neighbor it was split from. A row or
    /// column left with one child gives way to it (which takes its slot's size), and one left
    /// empty vanishes; stacks, unknown containers and this node itself stay.
    pub fn remove(&mut self, id: &str) -> Option<Node> {
        let removed = remove_inner(self, id)?;
        collapse(self);
        Some(removed)
    }

    /// Puts `new` after `target` along `axis`. A parent already running along `axis` takes it,
    /// and it takes half of the target's share; otherwise the target is wrapped in a new container
    /// `container_id`, which takes the target's slot size.
    pub fn split(&mut self, target: &str, axis: Axis, mut new: Node, container_id: String) -> bool {
        if let Some(parent) = self.parent_of(target).map(|p| p.id.clone())
            && let Some(parent) = self.find_mut(&parent)
            && parent.axis() == Some(axis)
        {
            let children = parent
                .children_mut()
                .expect("rows and columns have children");
            let i = children
                .iter()
                .position(|n| n.id == target)
                .expect("the parent holds the target");
            let size = children[i].size();
            let share = match size {
                Some(Size::Weight(weight)) => weight,
                None | Some(Size::Px(_)) => 1.0,
            };
            if !matches!(size, Some(Size::Px(_))) {
                children[i].set_size(Some(Size::Weight(share / 2.0)));
            }
            new.set_size(Some(Size::Weight(share / 2.0)));
            children.insert(i + 1, new);
            return true;
        }
        let Some(slot) = self.find_mut(target) else {
            return false;
        };
        let mut old = std::mem::replace(slot, Node::container(container_id, axis, Vec::new()));
        let size = old.extra.remove("size");
        if let Some(children) = slot.children_mut() {
            children.extend([old, new]);
        }
        if let Some(size) = size {
            slot.extra.insert("size".into(), size);
        }
        true
    }

    /// Swaps two nodes' places; sizes stay with the places. Refuses a node and its own subtree.
    pub fn swap(&mut self, a: &str, b: &str) -> bool {
        let (Some(na), Some(nb)) = (self.find(a).cloned(), self.find(b).cloned()) else {
            return false;
        };
        if na.find(b).is_some() || nb.find(a).is_some() {
            return false;
        }
        let placeholder = format!("\u{0}swap-{b}");
        if let Some(x) = self.find_mut(a) {
            let size = x.extra.get("size").cloned();
            *x = with_size(nb, size);
            x.id.clone_from(&placeholder);
        }
        if let Some(y) = self.find_mut(b) {
            let size = y.extra.get("size").cloned();
            *y = with_size(na, size);
        }
        if let Some(x) = self.find_mut(&placeholder) {
            x.id = b.to_owned();
        }
        true
    }
}

fn with_size(mut node: Node, size: Option<Value>) -> Node {
    match size {
        Some(size) => node.extra.insert("size".into(), size),
        None => node.extra.remove("size"),
    };
    node
}

fn weight(size: Option<Size>) -> Option<f32> {
    match size {
        Some(Size::Weight(weight)) => Some(weight),
        None => Some(1.0),
        Some(Size::Px(_)) => None,
    }
}

fn remove_inner(node: &mut Node, id: &str) -> Option<Node> {
    let children = node.children_mut()?;
    let Some(i) = children.iter().position(|n| n.id == id) else {
        return children.iter_mut().find_map(|n| remove_inner(n, id));
    };
    let removed = children.remove(i);
    match node.kind {
        Kind::Row { .. } | Kind::Column { .. } => {
            let children = node.children_mut().expect("rows and columns have children");
            if let Some(gone) = weight(removed.size())
                && let Some(neighbor) = children.get_mut(i.saturating_sub(1))
                && let Some(w) = weight(neighbor.size())
            {
                neighbor.set_size(Some(Size::Weight(w + gone)));
            }
        }
        Kind::Stack { .. } if node.active() == Some(id) => {
            // The neighbor that slides into its place, else the last one, becomes active.
            let children = node.children();
            let next = children.get(i).or(children.last()).map(|n| n.id.clone());
            node.set_active(next);
        }
        _ => {}
    }
    Some(removed)
}

fn is_split(node: &Node) -> bool {
    matches!(node.kind, Kind::Row { .. } | Kind::Column { .. })
}

fn collapse(node: &mut Node) {
    // A stack's children keep their identity even with one pane or none. Unknown containers are
    // left as authored.
    let simplify = is_split(node);
    let Some(children) = node.children_mut() else {
        return;
    };
    for child in children.iter_mut() {
        collapse(child);
    }
    if !simplify {
        return;
    }
    children.retain(|n| !(is_split(n) && n.children().is_empty()));
    for child in children.iter_mut() {
        if is_split(child) && child.children().len() == 1 {
            let size = child.extra.get("size").cloned();
            let only = child
                .children_mut()
                .expect("rows and columns have children")
                .remove(0);
            *child = with_size(only, size);
        }
    }
}

/// Where a node sits: its parent's id and its index there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub parent: String,
    pub index: usize,
}

impl Node {
    /// Where the node `id` sits below this one. `None` for this node itself.
    pub fn slot_of(&self, id: &str) -> Option<Slot> {
        let parent = self.parent_of(id)?;
        let index = parent.children().iter().position(|c| c.id == id)?;
        Some(Slot {
            parent: parent.id.clone(),
            index,
        })
    }
}

/// What closed along with a pane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Closed {
    /// Its tab, which had no other pane.
    pub tab: bool,
    /// Its space, whose last tab that was.
    pub space: bool,
}

impl Document {
    pub fn tabs(&self) -> impl Iterator<Item = &Tab> {
        self.spaces.iter().flat_map(|s| s.tabs.iter())
    }

    pub fn space(&self, id: &str) -> Option<&Space> {
        self.spaces.iter().find(|s| s.id == id)
    }

    pub fn space_mut(&mut self, id: &str) -> Option<&mut Space> {
        self.spaces.iter_mut().find(|s| s.id == id)
    }

    pub fn tab(&self, id: &str) -> Option<&Tab> {
        self.tabs().find(|t| t.id == id)
    }

    pub fn tab_mut(&mut self, id: &str) -> Option<&mut Tab> {
        self.spaces
            .iter_mut()
            .flat_map(|s| s.tabs.iter_mut())
            .find(|t| t.id == id)
    }

    /// The space holding a tab, or a node inside one.
    pub fn space_of(&self, node: &str) -> Option<&Space> {
        self.spaces.iter().find(|s| {
            s.tabs
                .iter()
                .any(|t| t.id == node || t.root.find(node).is_some())
        })
    }

    /// The tab holding a node (a pane or a container).
    pub fn tab_of(&self, node: &str) -> Option<&Tab> {
        self.tabs().find(|t| t.root.find(node).is_some())
    }

    pub fn tab_of_mut(&mut self, node: &str) -> Option<&mut Tab> {
        self.spaces
            .iter_mut()
            .flat_map(|s| s.tabs.iter_mut())
            .find(|t| t.root.find(node).is_some())
    }

    /// The pane showing a terminal.
    pub fn pane_of(&self, address: &str) -> Option<String> {
        self.tabs()
            .flat_map(|t| t.root.terminals())
            .find(|n| n.terminal() == Some(address))
            .map(|n| n.id.clone())
    }

    /// The terminal a pane shows.
    pub fn address_of(&self, pane: &str) -> Option<&str> {
        self.tabs()
            .find_map(|t| t.root.find(pane))
            .and_then(Node::terminal)
    }

    /// Every terminal in every tab, in space, tab and pane order.
    pub fn addresses(&self) -> Vec<String> {
        self.tabs().flat_map(|t| t.root.addresses()).collect()
    }

    /// Where a node sits in its tab.
    pub fn slot_of(&self, node: &str) -> Option<Slot> {
        self.tab_of(node)?.root.slot_of(node)
    }

    /// Whether a space, tab or node already has this id.
    pub fn has_id(&self, id: &str) -> bool {
        self.space(id).is_some() || self.tab(id).is_some() || self.tab_of(id).is_some()
    }

    /// Adds a space at the end of the machine's spaces.
    pub fn add_space(&mut self, space: Space) {
        self.spaces.push(space);
    }

    /// Adds a tab to a space after the tab `after` (the one this screen has selected), else at
    /// the end.
    pub fn add_tab(&mut self, space: &str, tab: Tab, after: Option<&str>) -> bool {
        let Some(space) = self.space_mut(space) else {
            return false;
        };
        let at = after
            .and_then(|a| space.tabs.iter().position(|t| t.id == a))
            .map_or(space.tabs.len(), |i| i + 1);
        space.tabs.insert(at, tab);
        true
    }

    /// Splits `target` with `block` (see [`Node::split`]).
    pub fn split(&mut self, target: &str, axis: Axis, block: Node, container: String) -> bool {
        self.tab_of_mut(target)
            .is_some_and(|t| t.root.split(target, axis, block, container))
    }

    /// Removes a pane; a tab left without panes closes, then a space left without tabs.
    pub fn remove_pane(&mut self, pane: &str) -> Option<Closed> {
        let tab = self.tab_of_mut(pane)?;
        if tab.root.id == pane {
            return None;
        }
        tab.root.remove(pane);
        if !tab.root.panes().is_empty() {
            return Some(Closed::default());
        }
        let id = tab.id.clone();
        let (_, space) = self.remove_tab(&id)?;
        Some(Closed { tab: true, space })
    }

    /// Removes a tab. Returns its terminals and whether its space closed with it.
    pub fn remove_tab(&mut self, tab: &str) -> Option<(Vec<String>, bool)> {
        let si = self
            .spaces
            .iter()
            .position(|s| s.tabs.iter().any(|t| t.id == tab))?;
        let space = &mut self.spaces[si];
        let ti = space.tabs.iter().position(|t| t.id == tab)?;
        let removed = space.tabs.remove(ti);
        let closed = space.tabs.is_empty();
        if closed {
            self.spaces.remove(si);
        }
        Some((removed.root.addresses(), closed))
    }

    /// Removes a space. Returns its terminals.
    pub fn remove_space(&mut self, id: &str) -> Option<Vec<String>> {
        let i = self.spaces.iter().position(|s| s.id == id)?;
        let space = self.spaces.remove(i);
        Some(space.tabs.iter().flat_map(|t| t.root.addresses()).collect())
    }

    /// Moves a space to `to` (0-based, clamped) among this machine's spaces.
    pub fn move_space(&mut self, id: &str, to: usize) -> bool {
        let Some(from) = self.spaces.iter().position(|s| s.id == id) else {
            return false;
        };
        let space = self.spaces.remove(from);
        let to = to.min(self.spaces.len());
        self.spaces.insert(to, space);
        true
    }

    /// Moves a tab to `to` (0-based, clamped) within its space. Only a tab id: a pane or
    /// container has no place among the tabs.
    pub fn move_tab(&mut self, id: &str, to: usize) -> bool {
        let Some((space, from)) = self
            .spaces
            .iter_mut()
            .find_map(|s| s.tabs.iter().position(|t| t.id == id).map(|i| (s, i)))
        else {
            return false;
        };
        let tab = space.tabs.remove(from);
        let to = to.min(space.tabs.len());
        space.tabs.insert(to, tab);
        true
    }

    /// Swaps two panes of one tab (`app:src/model.rs` `swap_panes`).
    pub fn swap_panes(&mut self, pane: &str, target: &str) -> Result<(), String> {
        let is_pane = |d: &Document, id: &str| {
            d.tab_of(id)
                .and_then(|t| t.root.find(id))
                .is_some_and(Node::is_pane)
        };
        if !is_pane(self, pane) {
            return Err(format!("no pane {pane:?}"));
        }
        if !is_pane(self, target) {
            return Err(format!("no target pane {target:?}"));
        }
        if pane == target {
            return Err("pane.swap target is the pane itself".into());
        }
        let tab = self.tab_of_mut(pane).expect("checked above");
        if tab.root.find(target).is_none() {
            return Err(format!("pane.swap target {target:?} is in another tab"));
        }
        tab.root.swap(pane, target);
        Ok(())
    }
}

impl Tab {
    /// A tab with one pane: its root is a row with the tab's id.
    pub fn with_pane(id: impl Into<String>, name: impl Into<String>, pane: Node) -> Tab {
        let id = id.into();
        Tab {
            root: Node::container(id.clone(), Axis::Row, vec![pane]),
            id,
            name: name.into(),
            extra: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn term(id: &str) -> Node {
        Node::pane(id, format!("t-{id}"))
    }

    fn weight_of(n: &Node, id: &str) -> f32 {
        match n.find(id).and_then(Node::size) {
            Some(Size::Weight(w)) => w,
            _ => 1.0,
        }
    }

    fn ids(nodes: &[Node]) -> Vec<&str> {
        nodes.iter().map(|n| n.id.as_str()).collect()
    }

    fn stack(id: &str, children: Vec<Node>, active: Option<&str>) -> Node {
        let mut node = Node::new(id, Kind::Stack { children });
        node.set_active(active.map(str::to_owned));
        node
    }

    #[test]
    fn splits_reuse_a_same_axis_parent_and_wrap_otherwise() {
        let mut n = stack("tab", vec![term("a")], None);
        assert!(n.split("a", Axis::Row, term("b"), "r1".into()));
        assert_eq!(n.find("r1").map(|r| r.children().len()), Some(2));
        assert!(n.split("b", Axis::Row, term("c"), "unused".into()));
        assert_eq!(n.find("r1").map(|r| r.children().len()), Some(3));
        assert!(n.split("c", Axis::Column, term("d"), "c1".into()));
        assert_eq!(
            n.find("c1").map(|c| ids(c.children())),
            Some(vec!["c", "d"])
        );
        let order: Vec<_> = n.terminals().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(order, ["a", "b", "c", "d"]);
        assert!(!n.split("nope", Axis::Row, term("e"), "x".into()));
    }

    #[test]
    fn a_split_halves_the_targets_share_and_a_wrap_keeps_its_slot_size() {
        let mut a = term("a");
        a.set_size(Some(Size::Weight(0.7)));
        let mut n = Node::container("tab", Axis::Row, vec![a, term("b")]);
        n.split("a", Axis::Row, term("c"), "unused".into());
        assert_eq!(ids(n.children()), ["a", "c", "b"]);
        assert_eq!((weight_of(&n, "a"), weight_of(&n, "c")), (0.35, 0.35));
        // A px slot keeps its px; the new pane takes a weight of 1/2.
        let mut px = term("x");
        px.set_size(Some(Size::Px(200.0)));
        let mut m = Node::container("tab", Axis::Row, vec![px]);
        m.split("x", Axis::Row, term("y"), "unused".into());
        assert_eq!(m.find("x").unwrap().size(), Some(Size::Px(200.0)));
        assert_eq!(m.find("y").unwrap().size(), Some(Size::Weight(0.5)));
        // Across the axis: the wrapper takes the slot's size, the old pane loses it.
        m.split("x", Axis::Column, term("z"), "c1".into());
        let wrapper = m.find("c1").unwrap();
        assert_eq!(wrapper.size(), Some(Size::Px(200.0)));
        assert_eq!(wrapper.axis(), Some(Axis::Column));
        assert_eq!(ids(wrapper.children()), ["x", "z"]);
        assert_eq!(m.find("x").unwrap().size(), None);
    }

    #[test]
    fn closing_a_split_gives_its_share_back_to_the_pane_it_came_from() {
        let mut a = term("a");
        a.set_size(Some(Size::Weight(0.7)));
        let mut b = term("b");
        b.set_size(Some(Size::Weight(0.3)));
        let mut n = Node::container("tab", Axis::Row, vec![a, b]);
        n.split("b", Axis::Row, term("c"), "unused".into());
        n.remove("c");
        assert_eq!((weight_of(&n, "a"), weight_of(&n, "b")), (0.7, 0.3));
        // Closing the first one hands its share to the next.
        n.split("a", Axis::Row, term("d"), "unused".into());
        n.remove("a");
        assert_eq!((weight_of(&n, "d"), weight_of(&n, "b")), (0.7, 0.3));
    }

    #[test]
    fn removing_collapses_single_children_but_keeps_stacks_and_the_root() {
        let tab = Node::container("t1", Axis::Row, vec![term("a"), term("b")]);
        let mut n = stack("tabs", vec![tab, term("solo")], Some("solo"));
        n.remove("b");
        assert_eq!(n.children()[0].id, "t1");
        assert_eq!(n.children()[0].children().len(), 1);
        n.remove("solo");
        assert_eq!(n.active(), Some("t1"));
        n.split("a", Axis::Column, term("c"), "c1".into());
        n.remove("c");
        assert_eq!(n.find("t1").unwrap().children()[0].id, "a");
        n.remove("t1");
        assert!(n.children().is_empty());
        let mut root = Node::container("ctx", Axis::Row, vec![term("x"), term("y")]);
        root.remove("y");
        assert_eq!((root.id.as_str(), root.children().len()), ("ctx", 1));
        assert!(root.remove("missing").is_none());
    }

    #[test]
    fn a_collapsed_split_hands_its_slot_size_to_its_last_child() {
        let mut inner = Node::container("c1", Axis::Column, vec![term("a"), term("b")]);
        inner.extra.insert("size".into(), json!({"weight": 2.0}));
        inner.children_mut().unwrap()[0].set_size(Some(Size::Weight(0.25)));
        let mut root = Node::container("tab", Axis::Row, vec![inner, term("c")]);
        root.remove("b");
        assert_eq!(ids(root.children()), ["a", "c"]);
        assert_eq!(root.children()[0].extra["size"], json!({"weight": 2.0}));
    }

    #[test]
    fn unknown_containers_are_not_simplified() {
        let grid = Node::new(
            "g",
            Kind::Other {
                kind: "grid".into(),
                children: Some(vec![Node::container(
                    "r",
                    Axis::Row,
                    vec![term("a"), term("b")],
                )]),
            },
        );
        let mut root = Node::container("tab", Axis::Row, vec![grid, term("c")]);
        root.remove("b");
        // Inside the grid nothing gives way, and the grid itself stays.
        assert_eq!(ids(root.find("g").unwrap().children()), ["r"]);
        assert_eq!(ids(root.find("r").unwrap().children()), ["a"]);
        root.remove("a");
        assert!(root.find("r").unwrap().children().is_empty());
        assert_eq!(ids(root.children()), ["g", "c"]);
    }

    #[test]
    fn swapping_keeps_sizes_with_places_and_refuses_ancestors() {
        let mut n = Node::container("r", Axis::Row, vec![term("a"), term("b"), term("c")]);
        n.find_mut("a").unwrap().set_size(Some(Size::Px(100.0)));
        assert!(n.swap("a", "c"));
        assert_eq!(ids(n.children()), ["c", "b", "a"]);
        assert_eq!(n.children()[0].size(), Some(Size::Px(100.0)));
        assert_eq!(n.children()[2].size(), None);
        assert!(!n.swap("r", "a"));
        assert!(!n.swap("a", "nope"));
    }

    #[test]
    fn paths_parents_and_panes() {
        let mut root = Node::container("tab", Axis::Row, vec![term("a")]);
        root.split(
            "a",
            Axis::Column,
            Node::block("m", Content::mod_block("board", None)),
            "c1".into(),
        );
        assert_eq!(root.path_to("m").unwrap(), ["tab", "c1", "m"]);
        assert_eq!(root.parent_of("m").map(|p| p.id.as_str()), Some("c1"));
        assert_eq!(
            root.slot_of("m"),
            Some(Slot {
                parent: "c1".into(),
                index: 1
            })
        );
        assert_eq!(root.slot_of("tab"), None);
        assert!(root.parent_of("tab").is_none());
        let panes: Vec<_> = root.panes().iter().map(|p| p.id.as_str()).collect();
        assert_eq!(panes, ["a", "m"]);
        assert_eq!(root.addresses(), ["t-a"]);
        assert!(Node::pane("x", "t").children_mut().is_none());
    }

    fn doc() -> Document {
        let mut d = Document::default();
        d.add_space(Space {
            id: "s1".into(),
            name: "work".into(),
            folder: "/w".into(),
            tabs: vec![Tab::with_pane("c1", "", term("x1"))],
            extra: Default::default(),
        });
        d
    }

    #[test]
    fn closing_the_last_pane_closes_the_tab_then_the_space() {
        let mut d = doc();
        assert!(d.split("x1", Axis::Row, term("x2"), "unused".into()));
        assert!(d.add_tab("s1", Tab::with_pane("c2", "", term("x3")), None));
        assert_eq!(d.remove_pane("x2"), Some(Closed::default()));
        assert_eq!(
            d.remove_pane("x3"),
            Some(Closed {
                tab: true,
                space: false
            })
        );
        assert_eq!(
            d.remove_pane("x1"),
            Some(Closed {
                tab: true,
                space: true
            })
        );
        assert!(d.spaces.is_empty());
        assert_eq!(d.remove_pane("x1"), None);
    }

    #[test]
    fn a_tab_root_is_not_a_pane_to_remove() {
        let mut d = doc();
        assert_eq!(d.remove_pane("c1"), None);
        assert!(d.tab("c1").is_some());
    }

    #[test]
    fn new_tabs_go_after_the_given_one_else_at_the_end() {
        let mut d = doc();
        d.add_tab("s1", Tab::with_pane("c2", "", term("x2")), None);
        d.add_tab("s1", Tab::with_pane("c3", "", term("x3")), Some("c1"));
        d.add_tab("s1", Tab::with_pane("c4", "", term("x4")), Some("gone"));
        let tabs: Vec<_> = d.spaces[0].tabs.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(tabs, ["c1", "c3", "c2", "c4"]);
        assert!(!d.add_tab("nope", Tab::with_pane("c5", "", term("x5")), None));
    }

    #[test]
    fn lookups_find_spaces_tabs_panes_and_terminals() {
        let mut d = doc();
        d.split("x1", Axis::Column, term("x2"), "k1".into());
        assert_eq!(d.space_of("k1").map(|s| s.id.as_str()), Some("s1"));
        assert_eq!(d.space_of("c1").map(|s| s.id.as_str()), Some("s1"));
        assert_eq!(d.tab_of("x2").map(|t| t.id.as_str()), Some("c1"));
        assert_eq!(d.pane_of("t-x2").as_deref(), Some("x2"));
        assert_eq!(d.address_of("x2"), Some("t-x2"));
        assert_eq!(d.addresses(), ["t-x1", "t-x2"]);
        for id in ["s1", "c1", "k1", "x2"] {
            assert!(d.has_id(id), "{id}");
        }
        assert!(!d.has_id("nope"));
    }

    #[test]
    fn spaces_and_tabs_move_and_clamp() {
        let mut d = doc();
        d.add_space(Space {
            id: "s2".into(),
            ..Default::default()
        });
        d.add_space(Space {
            id: "s3".into(),
            ..Default::default()
        });
        assert!(d.move_space("s1", usize::MAX));
        let order: Vec<_> = d.spaces.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(order, ["s2", "s3", "s1"]);
        assert!(d.move_space("s1", 0));
        assert!(!d.move_space("nope", 0));
        d.add_tab("s1", Tab::with_pane("c2", "", term("x2")), None);
        assert!(!d.move_tab("x2", 0), "a pane is not a tab");
        assert!(d.move_tab("c2", 0));
        let tabs: Vec<_> = d.spaces[0].tabs.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(tabs, ["c2", "c1"]);
    }

    #[test]
    fn swapping_panes_refuses_each_bad_pair() {
        let mut d = doc();
        d.split("x1", Axis::Row, term("x2"), "unused".into());
        d.add_tab("s1", Tab::with_pane("c2", "", term("x3")), None);
        assert!(d.swap_panes("x1", "x1").is_err());
        assert!(d.swap_panes("x1", "x3").is_err());
        assert!(d.swap_panes("c1", "x1").is_err());
        assert!(d.swap_panes("nope", "x1").is_err());
        assert!(d.swap_panes("x1", "x2").is_ok());
        assert_eq!(d.tab("c1").unwrap().root.addresses(), ["t-x2", "t-x1"]);
    }
}
