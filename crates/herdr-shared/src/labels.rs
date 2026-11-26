//! The labels Herdr clients share on terminals (`herdr-core-protocol.md` §3 "Labels";
//! `app:docs/labels.md`), and the `herdr.seen` rules.
//!
//! Labels are free-form and the core never reads them. Setting one to `""` with
//! `terminal.label` removes it.

use std::collections::BTreeMap;

use herdr_proto::{Labels, Terminal};

pub use herdr_proto::terminal_set::SEEN_LABEL as SEEN;

/// The space a terminal belongs to: an unqualified id on its machine. A live terminal with it
/// that no pane shows gets placed (see [`crate::adoption`]).
pub const SPACE: &str = "herdr.space";
/// Its tab in that space. Without it, adoption makes a new tab.
pub const TAB: &str = "herdr.tab";
/// `1`: pinned; pinned agents sort first. Unpin by writing it empty.
pub const PINNED: &str = "herdr.pinned";
/// `program`: right-clicks go to the program instead of the pane menu.
pub const RIGHT_CLICK: &str = "herdr.right-click";
/// `<space name> / <tab name>`, shown by `hc ls`. herdr-app writes it; herdr-frontend leaves it
/// to herdr-app, since two clients naming spaces differently would overwrite each other forever.
pub const WHERE: &str = "where";

fn labels<const N: usize>(pairs: [(&str, String); N]) -> Labels {
    pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
}

/// `herdr.seen`, when it's there and a number. A missing label is not 0.
pub fn seen(labels: &Labels) -> Option<u64> {
    labels.get(SEEN).and_then(|v| v.parse().ok())
}

/// Present and non-empty.
pub fn pinned(labels: &Labels) -> bool {
    labels.get(PINNED).is_some_and(|v| !v.is_empty())
}

pub fn right_click_to_program(labels: &Labels) -> bool {
    labels.get(RIGHT_CLICK).map(String::as_str) == Some("program")
}

/// `herdr.space`, when non-empty.
pub fn space(labels: &Labels) -> Option<&str> {
    labels
        .get(SPACE)
        .map(String::as_str)
        .filter(|s| !s.is_empty())
}

/// `herdr.tab`, when non-empty.
pub fn tab(labels: &Labels) -> Option<&str> {
    labels
        .get(TAB)
        .map(String::as_str)
        .filter(|s| !s.is_empty())
}

/// The labels `terminal.run` gives a terminal started for a pane: its space and tab, unqualified,
/// and `herdr.seen=0`. Its pane must also go into the document, or herdr-app places it elsewhere.
pub fn for_new_pane(space: &str, tab: &str) -> Labels {
    labels([
        (SPACE, space.to_owned()),
        (TAB, tab.to_owned()),
        (SEEN, "0".to_owned()),
    ])
}

/// The labels that put a running terminal in a pane ("keep in a pane"), unqualified. Also set
/// `keep: true`.
pub fn for_keep(space: &str, tab: &str) -> Labels {
    labels([(SPACE, space.to_owned()), (TAB, tab.to_owned())])
}

/// The labels that park a terminal: out of the layout, still running. Also set `keep: false`,
/// then take its pane out once both calls succeed.
pub fn for_park() -> Labels {
    labels([(SPACE, String::new()), (TAB, String::new())])
}

/// Pin or unpin.
pub fn for_pin(pinned: bool) -> Labels {
    labels([(PINNED, if pinned { "1" } else { "" }.to_owned())])
}

/// Seen up to `turn`.
pub fn for_seen(turn: u64) -> Labels {
    labels([(SEEN, turn.to_string())])
}

/// "Mark unread", the one lower write: `turn - 1`. At turn 0 nothing has finished yet, so there
/// is nothing to mark.
pub fn for_unread(turn: u64) -> Option<Labels> {
    (turn > 0).then(|| for_seen(turn - 1))
}

/// A `herdr.seen` label to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeenWrite {
    pub terminal: String,
    pub turn: u64,
}

/// What this screen shares as `herdr.seen` (`app:src/window.rs` `share_seen`).
///
/// A terminal counts as looked at only when the window is in the foreground and the agent is
/// drawn on screen (a peek counts, a covered pane doesn't). It is written once per turn while it
/// stays on screen, only to raise the label (or when it's missing), so another client's "mark
/// unread" isn't undone by a screen nobody watches. Decide when a `turn` event lands, which comes
/// after its `status` event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeenTracker {
    /// The turn each terminal on screen was last looked at.
    looked: BTreeMap<String, u64>,
    /// Writes the core hasn't echoed back yet.
    written: BTreeMap<String, u64>,
}

impl SeenTracker {
    /// The seen turn to show for a terminal: its label, or this screen's own write when the core
    /// hasn't echoed it yet. `None`: no client shares one, so this screen's own seen rule decides.
    pub fn seen_turn(&self, terminal: &Terminal) -> Option<u64> {
        seen(&terminal.labels).max(self.written.get(&terminal.address).copied())
    }

    /// After applying a batch of events: writes the core has echoed are done, and its labels
    /// decide again. `highs` is the batch's highest `herdr.seen` per terminal
    /// ([`herdr_proto::Changes::seen`]), so a raise isn't hidden by another client's later, lower
    /// "mark unread" in the same batch. After a fresh snapshot (`resynced`) it can't tell, so every
    /// write counts as echoed.
    pub fn echoed(&mut self, highs: &BTreeMap<String, u64>, resynced: bool) {
        if resynced {
            self.written.clear();
        } else {
            self.written
                .retain(|address, turn| highs.get(address).is_none_or(|high| high < turn));
        }
    }

    /// Takes what the person is looking at now: live agents (terminals that report a status)
    /// drawn on screen while the window is in the foreground. Pass none when it isn't. Returns
    /// the labels to write; each is already counted by [`SeenTracker::seen_turn`].
    pub fn look<'a>(&mut self, looking: impl IntoIterator<Item = &'a Terminal>) -> Vec<SeenWrite> {
        let looking: Vec<&Terminal> = looking.into_iter().collect();
        self.looked
            .retain(|address, _| looking.iter().any(|t| &t.address == address));
        let mut writes = Vec::new();
        for terminal in looking {
            if terminal.status.is_none() || !terminal.is_live() {
                continue;
            }
            let turn = terminal.turn;
            if self.looked.get(&terminal.address) == Some(&turn) {
                continue;
            }
            self.looked.insert(terminal.address.clone(), turn);
            if self.seen_turn(terminal).is_some_and(|seen| seen >= turn) {
                continue;
            }
            self.written.insert(terminal.address.clone(), turn);
            writes.push(SeenWrite {
                terminal: terminal.address.clone(),
                turn,
            });
        }
        writes
    }

    /// A write that failed: forget it, so the label decides and the next look tries again.
    pub fn failed(&mut self, write: &SeenWrite) {
        if self.written.get(&write.terminal) == Some(&write.turn) {
            self.written.remove(&write.terminal);
            self.looked.remove(&write.terminal);
        }
    }

    /// Forgets terminals no longer live.
    pub fn retain(&mut self, live: impl Fn(&str) -> bool) {
        self.written.retain(|address, _| live(address));
        self.looked.retain(|address, _| live(address));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use herdr_proto::{Status, StatusState};

    fn agent(address: &str, turn: u64, seen: Option<u64>) -> Terminal {
        let mut t = Terminal {
            address: address.into(),
            turn,
            status: Some(Status {
                state: StatusState::Idle,
                ..Default::default()
            }),
            ..Default::default()
        };
        if let Some(seen) = seen {
            t.labels.insert(SEEN.into(), seen.to_string());
        }
        t
    }

    fn write(terminal: &str, turn: u64) -> SeenWrite {
        SeenWrite {
            terminal: terminal.into(),
            turn,
        }
    }

    #[test]
    fn label_values_read_as_herdr_app_reads_them() {
        let mut l = Labels::new();
        assert_eq!(seen(&l), None, "missing is not 0");
        l.insert(SEEN.into(), "x".into());
        assert_eq!(seen(&l), None);
        l.insert(SEEN.into(), "4".into());
        assert_eq!(seen(&l), Some(4));
        assert!(!pinned(&l));
        l.insert(PINNED.into(), "".into());
        assert!(!pinned(&l));
        l.insert(PINNED.into(), "1".into());
        assert!(pinned(&l));
        assert!(!right_click_to_program(&l));
        l.insert(RIGHT_CLICK.into(), "program".into());
        assert!(right_click_to_program(&l));
        l.insert(SPACE.into(), "".into());
        assert_eq!(space(&l), None);
        l.insert(SPACE.into(), "sp1".into());
        l.insert(TAB.into(), "tb1".into());
        assert_eq!((space(&l), tab(&l)), (Some("sp1"), Some("tb1")));
    }

    #[test]
    fn label_writes_match_herdr_app() {
        let pairs = |l: Labels| l.into_iter().collect::<Vec<_>>();
        let s = |v: &str| v.to_owned();
        assert_eq!(
            pairs(for_new_pane("sp1", "tb1")),
            [(s(SEEN), s("0")), (s(SPACE), s("sp1")), (s(TAB), s("tb1"))]
        );
        assert_eq!(
            pairs(for_keep("sp1", "tb1")),
            [(s(SPACE), s("sp1")), (s(TAB), s("tb1"))]
        );
        assert_eq!(pairs(for_park()), [(s(SPACE), s("")), (s(TAB), s(""))]);
        assert_eq!(pairs(for_pin(true)), [(s(PINNED), s("1"))]);
        assert_eq!(pairs(for_pin(false)), [(s(PINNED), s(""))]);
        assert_eq!(for_unread(0), None);
        assert_eq!(for_unread(3), Some(for_seen(2)));
    }

    #[test]
    fn a_look_writes_once_per_turn_and_only_raises() {
        let mut s = SeenTracker::default();
        let t = agent("t1", 3, Some(2));
        assert_eq!(s.look([&t]), [write("t1", 3)]);
        assert_eq!(s.seen_turn(&t), Some(3), "counted before the echo");
        assert_eq!(s.look([&t]), [], "once per turn");
        // Already seen up to this turn elsewhere: nothing to write.
        let mut s = SeenTracker::default();
        assert_eq!(s.look([&agent("t1", 3, Some(3))]), []);
        assert_eq!(s.look([&agent("t1", 3, Some(5))]), []);
        // Missing label: written, so terminals started elsewhere join.
        assert_eq!(s.look([&agent("t2", 0, None)]), [write("t2", 0)]);
    }

    #[test]
    fn a_mark_unread_elsewhere_holds_while_the_screen_stays_on_it() {
        let mut s = SeenTracker::default();
        let t = agent("t1", 4, Some(3));
        assert_eq!(s.look([&t]), [write("t1", 4)]);
        let mut highs = BTreeMap::new();
        highs.insert("t1".to_owned(), 4);
        s.echoed(&highs, false);
        // The phone marks it unread; this screen still shows it and writes nothing.
        let unread = agent("t1", 4, Some(3));
        assert_eq!(s.seen_turn(&unread), Some(3));
        assert_eq!(s.look([&unread]), []);
        // Looking away and back raises it again.
        assert_eq!(s.look([]), []);
        assert_eq!(s.look([&unread]), [write("t1", 4)]);
    }

    #[test]
    fn a_write_stays_known_under_a_later_lower_one_in_the_same_batch() {
        let mut s = SeenTracker::default();
        let t = agent("t1", 5, Some(4));
        assert_eq!(s.look([&t]), [write("t1", 5)]);
        // A batch carried another client's 3 only: ours (5) isn't echoed yet.
        let mut highs = BTreeMap::new();
        highs.insert("t1".to_owned(), 3);
        s.echoed(&highs, false);
        assert_eq!(s.seen_turn(&agent("t1", 5, Some(3))), Some(5));
        // Our 5 then a lower 3 in one batch: the high says ours landed.
        highs.insert("t1".to_owned(), 5);
        s.echoed(&highs, false);
        assert_eq!(s.seen_turn(&agent("t1", 5, Some(3))), Some(3));
    }

    #[test]
    fn a_resync_forgets_every_unechoed_write() {
        let mut s = SeenTracker::default();
        s.look([&agent("t1", 2, None)]);
        s.echoed(&BTreeMap::new(), false);
        assert_eq!(s.seen_turn(&agent("t1", 2, None)), Some(2));
        s.echoed(&BTreeMap::new(), true);
        assert_eq!(s.seen_turn(&agent("t1", 2, None)), None);
    }

    #[test]
    fn non_agents_finished_terminals_and_failed_writes() {
        let mut s = SeenTracker::default();
        let mut shell = agent("t1", 1, None);
        shell.status = None;
        let mut done = agent("t2", 1, None);
        done.state = herdr_proto::TerminalState::Finished;
        assert_eq!(s.look([&shell, &done]), []);
        let t = agent("t3", 1, None);
        let w = s.look([&t]);
        s.failed(&w[0]);
        assert_eq!(s.seen_turn(&t), None);
        assert_eq!(s.look([&t]), w, "tried again");
        s.retain(|_| false);
        assert_eq!(s, SeenTracker::default());
    }
}
