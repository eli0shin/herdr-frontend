// Portions of this file are ported from Herdr app (https://github.com/herdrdev/herdr-app at
// v0.1.18, 9a9ae13d998e6ed74c76e8eb13bdeb2e1420f50c), licensed under the Apache License 2.0
// (assets/programs/LICENSE-herdr-app.txt), and were modified for herdr-frontend. See
// THIRD-PARTY-NOTICES.md.

//! Edits to a machine's layout, kept as what the person meant so they can be replayed on a newer
//! copy after a `conflict` (`herdr-core-protocol.md` §3 "Concurrency and versioning", rule 3).
//!
//! Each intent carries every id it creates, made once with [`fresh_id_on`](crate::fresh_id_on),
//! so a replay makes the same ids. Applying one is idempotent: an intent whose new ids already
//! exist, or whose terminal already has a pane, does nothing. So replaying a write whose reply was
//! lost never doubles it, and a terminal both apps placed never shows twice. An intent whose
//! target is gone does nothing either; the `Mirror` then drops it.
//!
//! Ids are as the app holds them: qualified for their machine (`devbox/p…`), bare on this one.

use crate::document::{Axis, Content, Document, Node, Size, Space, Tab};
use crate::tree::{Side, Slot};

/// One edit to one machine's layout.
#[derive(Clone, Debug, PartialEq)]
pub enum Intent {
    /// A space with one tab holding one pane, at the end of the machine's spaces.
    NewSpace {
        space: String,
        name: String,
        folder: String,
        tab: String,
        tab_name: String,
        pane: String,
        content: Content,
    },
    /// A tab holding one pane, after `after` (the tab this screen has selected), else at the end.
    NewTab {
        space: String,
        tab: String,
        name: String,
        pane: String,
        content: Content,
        after: Option<String>,
    },
    /// A pane beside `target` (see [`Node::split`]).
    Split {
        target: String,
        axis: Axis,
        pane: String,
        content: Content,
        /// The container made when the target's parent runs the other way.
        container: String,
    },
    /// Takes a pane out: closing it (its terminal is killed) and parking it (it keeps running)
    /// both do this to the layout. Its tab, then its space, close when left empty.
    RemovePane {
        pane: String,
    },
    CloseTab {
        tab: String,
    },
    CloseSpace {
        space: String,
    },
    RenameSpace {
        space: String,
        name: String,
    },
    RenameTab {
        tab: String,
        name: String,
    },
    /// Names a pane: its block's `title`, as herdr-app's `pane.rename` writes it. An empty name
    /// takes the title away (unnamed again).
    RenamePane {
        pane: String,
        name: String,
    },
    /// To a 0-based place among the machine's spaces.
    MoveSpace {
        space: String,
        to: usize,
    },
    /// To a 0-based place within its space.
    MoveTab {
        tab: String,
        to: usize,
    },
    /// Two panes of one tab trade places. `from` is where `pane` was when the person swapped
    /// them: once it has moved, the swap is done (or overtaken), so a replay doesn't swap back.
    /// Make it with [`Intent::swap_panes`].
    SwapPanes {
        pane: String,
        target: String,
        from: Slot,
    },
    /// A pane dragged beside another pane of its tab (see [`Document::move_pane`]). As with
    /// [`Intent::SwapPanes`], `from` is where it was: once it has moved, a replay does nothing.
    /// Make it with [`Intent::move_pane`].
    MovePane {
        pane: String,
        target: String,
        side: Side,
        /// The container made when the target's parent runs the other way.
        container: String,
        from: Slot,
    },
    /// Split sizes (a divider drag). Nodes that are gone are skipped.
    Resize {
        sizes: Vec<(String, Size)>,
    },
    /// Places a terminal its labels put in a space and tab (see [`crate::adoption`]).
    Adopt(Adopt),
}

/// Places a labeled terminal that no pane shows, as herdr-app's `adopt` does: into the labeled
/// tab, split right of its last pane; else a new tab in the labeled space; else a new space under
/// the labeled id. A space or tab made this way has no name, so it's called Herdr's way.
#[derive(Clone, Debug, PartialEq)]
pub struct Adopt {
    pub terminal: String,
    /// `herdr.space`, qualified.
    pub space: String,
    /// `herdr.tab`, qualified, when labeled.
    pub tab: Option<String>,
    /// The new pane's id.
    pub pane: String,
    /// The new tab's id when `tab` is `None`.
    pub new_tab: String,
    /// The container a split may need.
    pub container: String,
    /// A new space's folder (see [`crate::adoption::folder`]).
    pub folder: String,
    /// Where a new tab goes: after this screen's selected tab of that space.
    pub after: Option<String>,
}

/// Where an adopted terminal went. None of these take focus: the app keeps its active space, the
/// space's selected tab, and the tab's focus, zoom and focus history as they were (only a screen
/// with no active space shows the new one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placement {
    NewSpace,
    NewTab,
    /// Split right of the tab's last pane.
    Split {
        tab: String,
    },
}

impl Intent {
    /// Swaps two panes of one tab, as they are placed in `doc` now. `None` when `pane` isn't
    /// there.
    pub fn swap_panes(doc: &Document, pane: &str, target: &str) -> Option<Intent> {
        Some(Intent::SwapPanes {
            pane: pane.to_owned(),
            target: target.to_owned(),
            from: doc.slot_of(pane)?,
        })
    }

    /// Moves a pane beside another, as they are placed in `doc` now. `None` when `pane` isn't
    /// there.
    pub fn move_pane(
        doc: &Document,
        pane: &str,
        target: &str,
        side: Side,
        container: String,
    ) -> Option<Intent> {
        Some(Intent::MovePane {
            pane: pane.to_owned(),
            target: target.to_owned(),
            side,
            container,
            from: doc.slot_of(pane)?,
        })
    }
}

impl Document {
    /// Applies an intent. False when it does nothing: its target is gone, or it was already
    /// applied (its ids exist, or its terminal already has a pane).
    pub fn apply(&mut self, intent: &Intent) -> bool {
        match intent {
            Intent::NewSpace {
                space,
                name,
                folder,
                tab,
                tab_name,
                pane,
                content,
            } => {
                if [space, tab, pane].iter().any(|id| self.has_id(id)) || self.shows(content) {
                    return false;
                }
                self.add_space(Space {
                    id: space.clone(),
                    name: name.clone(),
                    folder: folder.clone(),
                    tabs: vec![Tab::with_pane(
                        tab.clone(),
                        tab_name.clone(),
                        Node::block(pane.clone(), content.clone()),
                    )],
                    extra: Default::default(),
                });
                true
            }
            Intent::NewTab {
                space,
                tab,
                name,
                pane,
                content,
                after,
            } => {
                if self.has_id(tab) || self.has_id(pane) || self.shows(content) {
                    return false;
                }
                let tab = Tab::with_pane(
                    tab.clone(),
                    name.clone(),
                    Node::block(pane.clone(), content.clone()),
                );
                self.add_tab(space, tab, after.as_deref())
            }
            Intent::Split {
                target,
                axis,
                pane,
                content,
                container,
            } => {
                let is_pane = self
                    .tab_of(target)
                    .and_then(|t| t.root.find(target))
                    .is_some_and(Node::is_pane);
                if !is_pane || self.has_id(pane) || self.shows(content) {
                    return false;
                }
                // A container id already taken is never reused: two nodes would share it.
                let wraps = self
                    .tab_of(target)
                    .and_then(|t| t.root.parent_of(target))
                    .and_then(Node::axis)
                    != Some(*axis);
                if wraps && self.has_id(container) {
                    return false;
                }
                let block = Node::block(pane.clone(), content.clone());
                self.split(target, *axis, block, container.clone())
            }
            Intent::RemovePane { pane } => self.remove_pane(pane).is_some(),
            Intent::CloseTab { tab } => self.remove_tab(tab).is_some(),
            Intent::CloseSpace { space } => self.remove_space(space).is_some(),
            Intent::RenameSpace { space, name } => self
                .space_mut(space)
                .map(|s| s.name.clone_from(name))
                .is_some(),
            Intent::RenameTab { tab, name } => {
                self.tab_mut(tab).map(|t| t.name.clone_from(name)).is_some()
            }
            Intent::RenamePane { pane, name } => self
                .tab_of_mut(pane)
                .and_then(|t| t.root.find_mut(pane))
                .filter(|n| n.is_pane())
                .map(|n| n.set_title(name))
                .is_some(),
            Intent::MoveSpace { space, to } => self.move_space(space, *to),
            Intent::MoveTab { tab, to } => self.move_tab(tab, *to),
            Intent::SwapPanes { pane, target, from } => {
                self.slot_of(pane).as_ref() == Some(from) && self.swap_panes(pane, target).is_ok()
            }
            Intent::MovePane {
                pane,
                target,
                side,
                container,
                from,
            } => {
                // A container id already taken is never reused: two nodes would share it.
                self.slot_of(pane).as_ref() == Some(from)
                    && !self.has_id(container)
                    && self.move_pane(pane, target, *side, container.clone())
            }
            Intent::Resize { sizes } => {
                let mut any = false;
                for (id, size) in sizes {
                    if let Some(node) = self.tab_of_mut(id).and_then(|t| t.root.find_mut(id)) {
                        node.set_size(Some(*size));
                        any = true;
                    }
                }
                any
            }
            Intent::Adopt(adopt) => self.adopt(adopt).is_some(),
        }
    }

    /// Whether a terminal block's terminal already has a pane.
    fn shows(&self, content: &Content) -> bool {
        content
            .address()
            .is_some_and(|address| self.pane_of(address).is_some())
    }

    /// Places a labeled terminal (see [`Adopt`]). `None` when it already has a pane.
    pub fn adopt(&mut self, adopt: &Adopt) -> Option<Placement> {
        let Adopt {
            terminal,
            space,
            tab,
            pane,
            new_tab,
            container,
            folder,
            after,
        } = adopt;
        if self.pane_of(terminal).is_some() || self.has_id(pane) {
            return None;
        }
        let mut tab_id = tab.as_ref().unwrap_or(new_tab);
        let block = Node::pane(pane.clone(), terminal.clone());
        if self.space(space).is_none() {
            // A labeled tab that lives in another space would get a second copy: the new space
            // gets a fresh tab instead.
            if self.tab(tab_id).is_some() {
                tab_id = new_tab;
            }
            self.add_space(Space {
                id: space.clone(),
                name: String::new(),
                folder: folder.clone(),
                tabs: vec![Tab::with_pane(tab_id.clone(), "", block)],
                extra: Default::default(),
            });
            return Some(Placement::NewSpace);
        }
        // herdr-app looks for the labeled tab anywhere on the machine.
        if let Some(existing) = self.tab_mut(tab_id) {
            let id = existing.id.clone();
            let last = existing.root.panes().last().map(|p| p.id.clone());
            match last {
                Some(last) => {
                    let container = container.clone();
                    existing.root.split(&last, Axis::Row, block, container);
                }
                // An empty tab: the pane goes straight into its root.
                None => existing.root.children_mut()?.push(block),
            }
            return Some(Placement::Split { tab: id });
        }
        let tab = Tab::with_pane(tab_id.clone(), "", block);
        self.add_tab(space, tab, after.as_deref())
            .then_some(Placement::NewTab)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Size;

    fn doc() -> Document {
        let mut d = Document::default();
        assert!(d.apply(&Intent::NewSpace {
            space: "s1".into(),
            name: "work".into(),
            folder: "/w".into(),
            tab: "c1".into(),
            tab_name: "".into(),
            pane: "x1".into(),
            content: Content::terminal("t1"),
        }));
        d
    }

    fn split(target: &str, pane: &str, address: &str) -> Intent {
        Intent::Split {
            target: target.into(),
            axis: Axis::Row,
            pane: pane.into(),
            content: Content::terminal(address),
            container: format!("k-{pane}"),
        }
    }

    fn adopt(terminal: &str, space: &str, tab: Option<&str>) -> Adopt {
        Adopt {
            terminal: terminal.into(),
            space: space.into(),
            tab: tab.map(str::to_owned),
            pane: format!("p-{terminal}"),
            new_tab: format!("tb-{terminal}"),
            container: format!("k-{terminal}"),
            folder: "/f".into(),
            after: None,
        }
    }

    #[test]
    fn every_intent_applies_once() {
        let intents = [
            Intent::NewTab {
                space: "s1".into(),
                tab: "c2".into(),
                name: "".into(),
                pane: "x2".into(),
                content: Content::terminal("t2"),
                after: None,
            },
            split("x1", "x3", "t3"),
            Intent::NewSpace {
                space: "s2".into(),
                name: "".into(),
                folder: "".into(),
                tab: "c4".into(),
                tab_name: "".into(),
                pane: "x4".into(),
                content: Content::terminal("t4"),
            },
            Intent::Adopt(adopt("t5", "s1", Some("c1"))),
        ];
        let mut d = doc();
        for intent in &intents {
            assert!(d.apply(intent), "{intent:?}");
            let once = d.clone();
            assert!(!d.apply(intent), "again: {intent:?}");
            assert_eq!(d, once);
        }
        assert_eq!(d.addresses(), ["t1", "t3", "t5", "t2", "t4"]);
    }

    #[test]
    fn a_terminal_never_gets_a_second_pane() {
        let mut d = doc();
        assert!(!d.apply(&split("x1", "x9", "t1")));
        assert!(!d.apply(&Intent::NewTab {
            space: "s1".into(),
            tab: "c9".into(),
            name: "".into(),
            pane: "x9".into(),
            content: Content::terminal("t1"),
            after: None,
        }));
        assert!(!d.apply(&Intent::Adopt(adopt("t1", "s1", None))));
        assert_eq!(d.addresses(), ["t1"]);
        // Mods aren't terminals: two of the same mod are fine.
        let board = Content::mod_block("board", None);
        for pane in ["m1", "m2"] {
            assert!(d.apply(&Intent::Split {
                target: "x1".into(),
                axis: Axis::Column,
                pane: pane.into(),
                content: board.clone(),
                container: format!("k-{pane}"),
            }));
        }
    }

    #[test]
    fn intents_on_targets_that_are_gone_do_nothing() {
        let mut d = doc();
        for intent in [
            split("gone", "x2", "t2"),
            split("c1", "x2", "t2"),
            Intent::NewTab {
                space: "gone".into(),
                tab: "c2".into(),
                name: "".into(),
                pane: "x2".into(),
                content: Content::terminal("t2"),
                after: None,
            },
            Intent::RemovePane {
                pane: "gone".into(),
            },
            Intent::CloseTab { tab: "gone".into() },
            Intent::CloseSpace {
                space: "gone".into(),
            },
            Intent::RenameSpace {
                space: "gone".into(),
                name: "x".into(),
            },
            Intent::RenameTab {
                tab: "gone".into(),
                name: "x".into(),
            },
            Intent::RenamePane {
                pane: "gone".into(),
                name: "x".into(),
            },
            // A tab's root is no pane.
            Intent::RenamePane {
                pane: "c1".into(),
                name: "x".into(),
            },
            Intent::MoveSpace {
                space: "gone".into(),
                to: 0,
            },
            Intent::MoveTab {
                tab: "gone".into(),
                to: 0,
            },
            Intent::swap_panes(&d, "x1", "gone").unwrap(),
            Intent::Resize {
                sizes: vec![("gone".into(), Size::Weight(1.0))],
            },
        ] {
            let before = d.clone();
            assert!(!d.apply(&intent), "{intent:?}");
            assert_eq!(d, before, "{intent:?}");
        }
    }

    #[test]
    fn renames_moves_resizes_and_closes_apply() {
        let mut d = doc();
        d.apply(&split("x1", "x2", "t2"));
        assert!(d.apply(&Intent::RenameSpace {
            space: "s1".into(),
            name: "api".into()
        }));
        assert!(d.apply(&Intent::RenameTab {
            tab: "c1".into(),
            name: "review".into()
        }));
        assert_eq!(
            (d.spaces[0].name.as_str(), d.spaces[0].tabs[0].name.as_str()),
            ("api", "review")
        );
        let unnamed = d.clone();
        assert!(d.apply(&Intent::RenamePane {
            pane: "x2".into(),
            name: "server".into()
        }));
        let pane = |d: &Document| d.tab("c1").unwrap().root.find("x2").unwrap().clone();
        assert_eq!(pane(&d).title(), Some("server"));
        assert!(d.apply(&Intent::RenamePane {
            pane: "x2".into(),
            name: "".into()
        }));
        assert_eq!(d, unnamed, "an empty name leaves no title behind");
        d.apply(&Intent::RenamePane {
            pane: "x2".into(),
            name: "server".into(),
        });
        assert!(d.apply(&Intent::Resize {
            sizes: vec![
                ("x1".into(), Size::Weight(0.7)),
                ("x2".into(), Size::Weight(0.3)),
                ("gone".into(), Size::Px(1.0))
            ]
        }));
        assert_eq!(
            d.tab("c1").unwrap().root.find("x1").unwrap().size(),
            Some(Size::Weight(0.7))
        );
        let swap = Intent::swap_panes(&d, "x1", "x2").unwrap();
        assert!(d.apply(&swap));
        assert_eq!(d.addresses(), ["t2", "t1"]);
        assert_eq!(
            pane(&d).title(),
            Some("server"),
            "the name moves with its pane"
        );
        assert!(!d.apply(&swap), "a replay doesn't swap back");
        assert_eq!(d.addresses(), ["t2", "t1"]);
        assert_eq!(Intent::swap_panes(&d, "gone", "x1"), None);
        assert!(d.apply(&Intent::RemovePane { pane: "x1".into() }));
        assert!(d.apply(&Intent::CloseTab { tab: "c1".into() }));
        assert!(d.spaces.is_empty());
    }

    #[test]
    fn a_pane_move_replays_as_a_no_op_once_the_pane_has_moved() {
        let mut d = doc();
        d.apply(&split("x1", "x2", "t2"));
        let move_down = Intent::move_pane(&d, "x2", "x1", Side::Bottom, "k".into()).unwrap();
        assert!(d.apply(&move_down));
        let root = &d.tab("c1").unwrap().root;
        assert_eq!(root.find("k").unwrap().axis(), Some(Axis::Column));
        let once = d.clone();
        assert!(!d.apply(&move_down), "a replay doesn't move it again");
        assert_eq!(d, once);
        // A container id already in the document is never made twice.
        let taken = Intent::move_pane(&d, "x2", "x1", Side::Left, "c1".into()).unwrap();
        assert!(!d.apply(&taken));
        assert_eq!(
            Intent::move_pane(&d, "gone", "x1", Side::Left, "k9".into()),
            None
        );
    }

    #[test]
    fn a_taken_container_id_is_refused_only_when_a_wrap_needs_it() {
        let mut d = doc();
        d.apply(&Intent::Split {
            target: "x1".into(),
            axis: Axis::Column,
            pane: "x2".into(),
            content: Content::terminal("t2"),
            container: "k".into(),
        });
        let wrap = |pane: &str, axis| Intent::Split {
            target: "x2".into(),
            axis,
            pane: pane.into(),
            content: Content::terminal(format!("t-{pane}")),
            container: "k".into(),
        };
        assert!(
            !d.apply(&wrap("x3", Axis::Row)),
            "wrapping x2 would need a second k"
        );
        assert!(
            d.apply(&wrap("x4", Axis::Column)),
            "k's own axis: no container needed"
        );
        assert_eq!(
            d.tab("c1")
                .unwrap()
                .root
                .find("k")
                .unwrap()
                .children()
                .len(),
            3
        );
    }

    #[test]
    fn a_new_space_never_copies_a_tab_id_from_another_space() {
        let mut d = doc();
        assert_eq!(
            d.adopt(&adopt("t2", "sp9", Some("c1"))),
            Some(Placement::NewSpace)
        );
        assert_eq!(d.space("sp9").unwrap().tabs[0].id, "tb-t2");
        assert_eq!(d.tab("c1").unwrap().root.addresses(), ["t1"]);
    }

    #[test]
    fn adoption_follows_herdr_app() {
        let mut d = doc();
        // Labeled tab: split right of its last pane.
        assert_eq!(
            d.adopt(&adopt("t2", "s1", Some("c1"))),
            Some(Placement::Split { tab: "c1".into() })
        );
        assert_eq!(d.tab("c1").unwrap().root.addresses(), ["t1", "t2"]);
        // A tab the space doesn't have: a new unnamed tab under the labeled id, at the end.
        assert_eq!(
            d.adopt(&adopt("t3", "s1", Some("c9"))),
            Some(Placement::NewTab)
        );
        let tab = d.tab("c9").unwrap();
        assert_eq!(
            (tab.name.as_str(), tab.root.addresses()),
            ("", vec!["t3".to_owned()])
        );
        // No tab label: a new tab with a fresh id, after the given one.
        let mut a = adopt("t4", "s1", None);
        a.after = Some("c1".into());
        assert_eq!(d.adopt(&a), Some(Placement::NewTab));
        let tabs: Vec<_> = d.spaces[0].tabs.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(tabs, ["c1", "tb-t4", "c9"]);
        // An unknown space: a new unnamed space under the labeled id, in the terminal's folder.
        assert_eq!(
            d.adopt(&adopt("t5", "sp9", Some("tb9"))),
            Some(Placement::NewSpace)
        );
        let space = d.space("sp9").unwrap();
        assert_eq!((space.name.as_str(), space.folder.as_str()), ("", "/f"));
        assert_eq!(
            (space.tabs[0].id.as_str(), space.tabs[0].name.as_str()),
            ("tb9", "")
        );
        // Already placed: nothing.
        assert_eq!(d.adopt(&adopt("t5", "sp9", Some("tb9"))), None);
    }

    #[test]
    fn an_empty_labeled_tab_takes_the_pane_into_its_root() {
        let mut d = doc();
        d.add_tab(
            "s1",
            Tab {
                id: "c2".into(),
                name: "".into(),
                root: Node::container("c2", Axis::Row, vec![]),
                extra: Default::default(),
            },
            None,
        );
        assert_eq!(
            d.adopt(&adopt("t2", "s1", Some("c2"))),
            Some(Placement::Split { tab: "c2".into() })
        );
        assert_eq!(d.tab("c2").unwrap().root.addresses(), ["t2"]);
        assert_eq!(d.spaces[0].tabs.len(), 2);
    }
}
