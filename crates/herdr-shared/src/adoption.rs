//! Adoption: placing live terminals labeled `herdr.space` that no pane shows
//! (`herdr-core-protocol.md` §3 "How herdr-app reconciles the layout"; `app:src/window.rs`
//! `reconcile`, `start_adoption`, `adopt`, `adoptable`; `feature-inventory.md` §0).
//!
//! Agents and scripts (`spawn-worker`, `agent-status`) start terminals with
//! `--label herdr.space=… --label herdr.tab=…` and fresh ids. A space or tab the layout doesn't
//! know becomes a new unnamed one, shown without taking focus.
//!
//! The steps, on every listing:
//! 1. [`orphans`] lists the candidates, once their machine's layout is read.
//! 2. For each, `terminal.get` it and check [`adoptable`]: a listing taken just before it ended
//!    still shows it live, and labels outlive the program.
//! 3. If [`needs_keep`], `terminal.keep {keep: true}`.
//! 4. If it still has no pane, edit the machine's `Mirror` with [`Intent::Adopt`]
//!    ([`Adopt`] carries the ids; [`folder`] gives a new space's folder).

use herdr_proto::Terminal;

use crate::document::{fresh_id_on, local, machine_of, qualify};
use crate::intent::{Adopt, Intent};
use crate::labels;

/// A live terminal its labels place in a space and tab, with ids qualified for its machine.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Orphan {
    pub terminal: String,
    pub space: String,
    pub tab: Option<String>,
}

impl Orphan {
    /// The edit that places it. `after` is this screen's selected tab in that space (a new tab
    /// goes after it); `folder` is a new space's folder.
    pub fn intent(&self, folder: String, after: Option<String>) -> Intent {
        let machine = machine_of(&self.terminal);
        Intent::Adopt(Adopt {
            terminal: self.terminal.clone(),
            space: self.space.clone(),
            tab: self.tab.clone(),
            pane: fresh_id_on("p", machine),
            new_tab: fresh_id_on("tb", machine),
            container: fresh_id_on("s", machine),
            folder,
            after,
        })
    }
}

/// The live terminals to place, sorted by address. `placed`: a pane shows it. `available`: its
/// machine's layout has been read (`None`: this machine). `busy`: a placement, park or close of
/// it is already under way.
pub fn orphans<'a>(
    live: impl IntoIterator<Item = &'a Terminal>,
    placed: impl Fn(&str) -> bool,
    available: impl Fn(Option<&str>) -> bool,
    busy: impl Fn(&str) -> bool,
) -> Vec<Orphan> {
    let mut found: Vec<Orphan> = live
        .into_iter()
        .filter(|t| t.is_live() && !placed(&t.address) && !busy(&t.address))
        .filter_map(|t| {
            let machine = machine_of(&t.address);
            if !available(machine) {
                return None;
            }
            let space = labels::space(&t.labels)?;
            Some(Orphan {
                terminal: t.address.clone(),
                space: qualify(space, machine),
                tab: labels::tab(&t.labels).map(|tab| qualify(tab, machine)),
            })
        })
        .collect();
    found.sort();
    found
}

/// Whether a fresh `terminal.get` record still puts the terminal where the orphan says, and it
/// still runs.
pub fn adoptable(record: &Terminal, orphan: &Orphan) -> bool {
    let machine = machine_of(&orphan.terminal);
    record.is_live()
        && record.labels.get(labels::SPACE).map(String::as_str)
            == Some(local(&orphan.space, machine))
        && labels::tab(&record.labels) == orphan.tab.as_deref().map(|t| local(t, machine))
}

/// An adopted terminal must be kept, so a lost program is replaced in its pane.
pub fn needs_keep(record: &Terminal) -> bool {
    !record.keep
}

/// A new space's folder: the terminal's live folder (when the app has read it), else the
/// record's foreground folder, else its launch folder, else the home folder.
pub fn folder(record: Option<&Terminal>, live_cwd: Option<&str>, home: &str) -> String {
    live_cwd
        .map(str::to_owned)
        .or_else(|| {
            record.and_then(|r| {
                r.foreground
                    .as_ref()
                    .map(|f| f.cwd.clone())
                    .or_else(|| Some(r.cwd.clone()))
            })
        })
        .unwrap_or_else(|| home.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Axis, Content, Document};
    use crate::intent::Placement;
    use herdr_proto::{Foreground, TerminalState};

    fn labeled(address: &str, space: Option<&str>, tab: Option<&str>) -> Terminal {
        let mut t = Terminal {
            address: address.into(),
            keep: true,
            cwd: "/launch".into(),
            ..Default::default()
        };
        if let Some(space) = space {
            t.labels.insert(labels::SPACE.into(), space.into());
        }
        if let Some(tab) = tab {
            t.labels.insert(labels::TAB.into(), tab.into());
        }
        t
    }

    fn orphan(terminal: &str, space: &str, tab: Option<&str>) -> Orphan {
        Orphan {
            terminal: terminal.into(),
            space: space.into(),
            tab: tab.map(str::to_owned),
        }
    }

    #[test]
    fn orphans_are_live_labeled_unplaced_terminals_of_read_layouts() {
        let mut finished = labeled("t5", Some("sp1"), None);
        finished.state = TerminalState::Finished;
        let live = [
            labeled("t3", Some("sp1"), Some("tb1")),
            labeled("t1", Some("sp1"), None),
            labeled("t2", None, Some("tb1")),
            labeled("t4", Some(""), None),
            finished,
            labeled("t6", Some("sp1"), None),
            labeled("t7", Some("sp1"), None),
            labeled("devbox/t1", Some("sp2"), Some("tb2")),
            labeled("omarchy/t1", Some("sp3"), None),
            labeled("t8", Some("sp1"), Some("")),
        ];
        let found = orphans(
            &live,
            |a| a == "t6",
            |m| m != Some("omarchy"),
            |a| a == "t7",
        );
        assert_eq!(
            found,
            [
                orphan("devbox/t1", "devbox/sp2", Some("devbox/tb2")),
                orphan("t1", "sp1", None),
                orphan("t3", "sp1", Some("tb1")),
                orphan("t8", "sp1", None),
            ]
        );
    }

    #[test]
    fn a_fresh_record_must_still_place_it_and_still_run() {
        let o = orphan("devbox/t1", "devbox/sp2", Some("devbox/tb2"));
        assert!(adoptable(
            &labeled("devbox/t1", Some("sp2"), Some("tb2")),
            &o
        ));
        assert!(!adoptable(
            &labeled("devbox/t1", Some("sp9"), Some("tb2")),
            &o
        ));
        assert!(!adoptable(&labeled("devbox/t1", Some("sp2"), None), &o));
        let mut ended = labeled("devbox/t1", Some("sp2"), Some("tb2"));
        ended.state = TerminalState::Finished;
        assert!(!adoptable(&ended, &o));
        let untabbed = orphan("t1", "sp1", None);
        assert!(adoptable(&labeled("t1", Some("sp1"), Some("")), &untabbed));
        assert!(!adoptable(
            &labeled("t1", Some("sp1"), Some("tb1")),
            &untabbed
        ));
    }

    #[test]
    fn adopted_terminals_are_kept() {
        let mut t = labeled("t1", Some("sp1"), None);
        assert!(!needs_keep(&t));
        t.keep = false;
        assert!(needs_keep(&t));
    }

    #[test]
    fn a_new_space_starts_where_the_terminal_is() {
        let mut t = labeled("t1", Some("sp1"), None);
        assert_eq!(folder(Some(&t), Some("/live"), "/home"), "/live");
        assert_eq!(folder(Some(&t), None, "/home"), "/launch");
        t.foreground = Some(Foreground {
            cwd: "/fg".into(),
            ..Default::default()
        });
        assert_eq!(folder(Some(&t), None, "/home"), "/fg");
        assert_eq!(folder(None, None, "/home"), "/home");
    }

    #[test]
    fn spawn_worker_spaces_and_tabs_appear_unnamed() {
        // spawn-worker: fresh sp…/tb… ids the layout has never seen.
        let t = labeled("devbox/t9", Some("sp9"), Some("tb9"));
        let found = orphans([&t], |_| false, |_| true, |_| false);
        let intent = found[0].intent("/w".into(), None);
        let Intent::Adopt(adopt) = &intent else {
            panic!()
        };
        assert!(adopt.pane.starts_with("devbox/p") && adopt.new_tab.starts_with("devbox/tb"));
        let mut doc = Document::default();
        assert_eq!(doc.adopt(adopt), Some(Placement::NewSpace));
        let space = doc.space("devbox/sp9").unwrap();
        assert_eq!(
            (
                space.name.as_str(),
                space.tabs[0].id.as_str(),
                space.tabs[0].name.as_str()
            ),
            ("", "devbox/tb9", "")
        );
        // agent-status resume: a tab the space doesn't have, in a space it does.
        let t = labeled("devbox/t10", Some("sp9"), Some("tb10"));
        let found = orphans([&t], |_| false, |_| true, |_| false);
        assert!(doc.apply(&found[0].intent("/w".into(), None)));
        assert_eq!(
            doc.tab("devbox/tb10").unwrap().root.addresses(),
            ["devbox/t10"]
        );
        assert_eq!(
            doc.localized(Some("devbox"))
                .space("sp9")
                .unwrap()
                .tabs
                .len(),
            2
        );
        // Already placed by the other app: replaying does nothing.
        assert!(!doc.apply(&found[0].intent("/w".into(), None)));
        // A pane can still be split next to it.
        assert!(doc.apply(&Intent::Split {
            target: doc.pane_of("devbox/t10").unwrap(),
            axis: Axis::Row,
            pane: "devbox/p1".into(),
            content: Content::terminal("devbox/t11"),
            container: "devbox/s1".into(),
        }));
    }
}
