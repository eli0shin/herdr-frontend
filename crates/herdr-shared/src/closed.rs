//! When a pane's terminal is gone, and what closing or parking a pane does to its terminals
//! (`herdr-core-protocol.md` §3 "How herdr-app reconciles the layout"; `app:src/window.rs`
//! `terminal_ended`, `check_closed`, `checked_closed`, `just_born`, `close_terminals`,
//! `kill_targets`).
//!
//! Closed panes: a pane goes ([`Intent::RemovePane`](crate::Intent::RemovePane)) when its attach
//! stream ends with [`stream_closed`], or when its terminal is missing from the live list
//! ([`missing`]) and `terminal.get` then says so ([`closed_by_lookup`]). Both apps may remove the
//! same pane; under CAS that converges. Never put an address in the document before
//! `terminal.run` has returned it.
//!
//! Close vs park: closing a pane or tab kills its terminals ([`close_targets`]); parking takes the
//! pane out but keeps the program running ([`crate::labels::for_park`] and `keep: false`, then
//! remove the pane once both calls succeed).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use herdr_proto::{CloseReason, Reason, RpcError, Terminal, Version, rpc::ErrorKind};

use crate::document::Document;
use crate::labels;

/// A terminal this screen started is spared [`missing`] for this long: it may not be listed yet.
/// Terminals other clients start get no grace.
pub const BIRTH_GRACE: Duration = Duration::from_secs(5);
/// How long a birth is remembered (for a launch-failure notice).
pub const BIRTH_MEMORY: Duration = Duration::from_secs(30);
/// The first core that ends `die_with_parent` children itself.
pub const CHILD_LIFETIME_CORE: Version = Version::new(0, 1, 13);

/// An attach stream's CLOSE: the program ended on its own or was killed, so its pane goes now. A
/// stream that just broke (the core went away) says neither; the next listing decides.
pub fn stream_closed(reason: &CloseReason) -> bool {
    matches!(
        reason.reason,
        Some(Reason::Exited | Reason::Killed | Reason::Finished)
    )
}

/// `terminal.get` for a terminal missing from the live list: closed when it exited or was
/// killed, or the core no longer knows it. A failed lookup otherwise is not evidence.
pub fn closed_by_lookup(result: Result<&Terminal, &RpcError>) -> bool {
    match result {
        Ok(record) => matches!(record.reason, Some(Reason::Exited | Reason::Killed)),
        Err(error) => error.kind == ErrorKind::NotFound,
    }
}

/// The panes whose terminal isn't live, as `(pane, address)`: look each up with `terminal.get`.
/// `just_born` spares the ones this screen just started ([`Births::just_born`]).
pub fn missing(
    doc: &Document,
    live: impl Fn(&str) -> bool,
    just_born: impl Fn(&str) -> bool,
) -> Vec<(String, String)> {
    doc.tabs()
        .flat_map(|t| t.root.terminals())
        .filter_map(|n| {
            let address = n.terminal()?;
            (!live(address) && !just_born(address)).then(|| (n.id.clone(), address.to_owned()))
        })
        .collect()
}

/// The terminals this screen started, and when.
#[derive(Clone, Debug, Default)]
pub struct Births {
    born: BTreeMap<String, Instant>,
}

impl Births {
    /// `terminal.run` returned this address.
    pub fn born(&mut self, address: &str, now: Instant) {
        self.born.insert(address.to_owned(), now);
    }

    /// Started by this screen less than [`BIRTH_GRACE`] ago.
    pub fn just_born(&mut self, address: &str, now: Instant) -> bool {
        self.born
            .retain(|_, at| now.saturating_duration_since(*at) < BIRTH_MEMORY);
        self.born
            .get(address)
            .is_some_and(|at| now.saturating_duration_since(*at) < BIRTH_GRACE)
    }

    /// Forgets a birth, returning whether it was remembered: a pane closing right after its
    /// launch is a launch failure worth telling.
    pub fn take(&mut self, address: &str) -> bool {
        self.born.remove(address).is_some()
    }
}

/// Cores before 0.1.13 don't end `die_with_parent` children, so closing walks them itself.
pub fn legacy_child_lifetime(version: Option<Version>) -> bool {
    version.is_some_and(|v| v < CHILD_LIFETIME_CORE)
}

/// What to kill for `doomed`: those, plus (where `walk` says so) their live children, and theirs,
/// except children shown in a pane or labeled `herdr.space`. `parent` must be normalized to this
/// core's addresses.
pub fn kill_targets<'a>(
    mut doomed: Vec<String>,
    live: impl IntoIterator<Item = &'a Terminal> + Clone,
    placed: impl Fn(&str) -> bool,
    walk: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut i = 0;
    while i < doomed.len() {
        let parent = doomed[i].clone();
        i += 1;
        if !walk(&parent) {
            continue;
        }
        for child in live.clone() {
            if child.parent.as_deref() == Some(parent.as_str())
                && walk(&child.address)
                && !doomed.contains(&child.address)
                && !placed(&child.address)
                && labels::space(&child.labels).is_none()
            {
                doomed.push(child.address.clone());
            }
        }
    }
    doomed
}

/// What closing panes or tabs kills. On a core 0.1.13 or later, only their terminals: the core
/// ends every live `die_with_parent` child itself, even one in a pane or labeled `herdr.space`,
/// and only `outlive_parent` children survive. On older cores (`legacy` per address's machine),
/// their unplaced children too.
pub fn close_targets<'a>(
    addresses: Vec<String>,
    live: impl IntoIterator<Item = &'a Terminal> + Clone,
    placed: impl Fn(&str) -> bool,
    legacy: impl Fn(&str) -> bool,
) -> Vec<String> {
    kill_targets(addresses, live, placed, legacy)
}

/// What an explicit kill (the kill command, not a close) kills: the terminal and its unplaced
/// sub-agents, on any core.
pub fn kill_tree_targets<'a>(
    address: &str,
    live: impl IntoIterator<Item = &'a Terminal> + Clone,
    placed: impl Fn(&str) -> bool,
) -> Vec<String> {
    kill_targets(vec![address.to_owned()], live, placed, |_| true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Node;
    use crate::document::{Axis, Content, Space, Tab};
    use crate::intent::Intent;

    fn ended(reason: Option<Reason>) -> CloseReason {
        CloseReason {
            reason,
            ..Default::default()
        }
    }

    #[test]
    fn a_stream_closes_its_pane_only_when_the_program_ended() {
        assert!(stream_closed(&ended(Some(Reason::Exited))));
        assert!(stream_closed(&ended(Some(Reason::Killed))));
        assert!(stream_closed(&ended(Some(Reason::Finished))));
        assert!(!stream_closed(&ended(Some(Reason::ServerStop))));
        assert!(!stream_closed(&ended(Some(Reason::Other(
            "vanished".into()
        )))));
        assert!(!stream_closed(&ended(None)), "a broken stream");
    }

    #[test]
    fn a_lookup_closes_exited_killed_and_unknown_terminals() {
        let record = |reason| Terminal {
            reason,
            ..Default::default()
        };
        assert!(closed_by_lookup(Ok(&record(Some(Reason::Exited)))));
        assert!(closed_by_lookup(Ok(&record(Some(Reason::Killed)))));
        assert!(!closed_by_lookup(Ok(&record(Some(Reason::Finished)))));
        assert!(!closed_by_lookup(Ok(&record(Some(Reason::HolderLost)))));
        assert!(
            !closed_by_lookup(Ok(&record(None))),
            "lost, a shell follows"
        );
        let error = |kind| RpcError {
            code: -32000,
            message: "x".into(),
            kind,
            hint: None,
            data: Default::default(),
        };
        assert!(closed_by_lookup(Err(&error(ErrorKind::NotFound))));
        assert!(!closed_by_lookup(Err(&error(ErrorKind::Unavailable))));
    }

    fn doc() -> Document {
        let mut tab = Tab::with_pane("c1", "", Node::pane("x1", "t1"));
        tab.root
            .split("x1", Axis::Row, Node::pane("x2", "t2"), "k".into());
        tab.root.split(
            "x2",
            Axis::Row,
            Node::block("m", Content::mod_block("board", None)),
            "k2".into(),
        );
        Document {
            spaces: vec![Space {
                id: "s1".into(),
                tabs: vec![tab],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn missing_panes_skip_live_and_just_born_terminals_and_mods() {
        let d = doc();
        assert_eq!(
            missing(&d, |a| a == "t1", |_| false),
            [("x2".to_owned(), "t2".to_owned())]
        );
        assert_eq!(missing(&d, |a| a == "t1", |a| a == "t2"), []);
        // Then the lookup removes it; the other screen may too.
        let mut d = d;
        assert!(d.apply(&Intent::RemovePane { pane: "x2".into() }));
        assert!(
            !d.apply(&Intent::RemovePane { pane: "x2".into() }),
            "converges"
        );
    }

    #[test]
    fn only_this_screens_births_get_five_seconds() {
        let t0 = Instant::now();
        let mut b = Births::default();
        b.born("t1", t0);
        assert!(b.just_born("t1", t0 + Duration::from_secs(4)));
        assert!(!b.just_born("t1", t0 + Duration::from_secs(5)));
        assert!(!b.just_born("t2", t0), "another client's");
        assert!(b.take("t1"));
        b.born("t3", t0);
        assert!(!b.just_born("t3", t0 + BIRTH_MEMORY));
        assert!(!b.take("t3"), "forgotten after 30 s");
    }

    #[test]
    fn legacy_cores_are_before_0_1_13() {
        assert!(legacy_child_lifetime(Version::parse("0.1.12")));
        assert!(!legacy_child_lifetime(Version::parse("0.1.13")));
        assert!(!legacy_child_lifetime(Version::parse("hc 0.1.22")));
        assert!(!legacy_child_lifetime(None));
    }

    fn child(address: &str, parent: &str, space: Option<&str>) -> Terminal {
        let mut t = Terminal {
            address: address.into(),
            parent: Some(parent.into()),
            ..Default::default()
        };
        if let Some(space) = space {
            t.labels.insert(labels::SPACE.into(), space.into());
        }
        t
    }

    #[test]
    fn closing_kills_only_the_panes_terminals_on_a_new_core() {
        let live = [child("t2", "t1", None), child("t3", "t2", None)];
        assert_eq!(
            close_targets(vec!["t1".into()], &live, |_| false, |_| false),
            ["t1"]
        );
    }

    #[test]
    fn closing_on_an_old_core_walks_unplaced_unlabeled_children() {
        let live = [
            child("t2", "t1", None),
            child("t3", "t2", None),
            child("t4", "t1", Some("sp1")),
            child("t5", "t1", None),
            child("t6", "t5", None),
            child("t7", "t1", Some("")),
        ];
        let placed = |a: &str| a == "t5";
        let targets = close_targets(vec!["t1".into()], &live, placed, |_| true);
        assert_eq!(targets, ["t1", "t2", "t7", "t3"]);
        // Only cores that need it are walked: a remote legacy machine, not this one.
        let remote = [
            child("devbox/t2", "devbox/t1", None),
            child("t2", "t1", None),
        ];
        let legacy = |a: &str| a.starts_with("devbox/");
        let targets = close_targets(
            vec!["devbox/t1".into(), "t1".into()],
            &remote,
            |_| false,
            legacy,
        );
        assert_eq!(targets, ["devbox/t1", "t1", "devbox/t2"]);
    }

    #[test]
    fn an_explicit_kill_walks_on_any_core() {
        let live = [child("t2", "t1", None), child("t3", "t1", Some("sp1"))];
        assert_eq!(kill_tree_targets("t1", &live, |_| false), ["t1", "t2"]);
    }

    #[test]
    fn parking_keeps_the_program_and_takes_the_pane_out() {
        assert_eq!(labels::for_park().values().collect::<Vec<_>>(), ["", ""]);
        let mut d = doc();
        assert!(d.apply(&Intent::RemovePane { pane: "x1".into() }));
        assert_eq!(d.addresses(), ["t2"]);
    }
}
