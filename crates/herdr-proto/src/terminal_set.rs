//! Every terminal an events subscription describes, kept current by applying its notifications
//! (`herdr-core-protocol.md` §2 "Events"; `core:schema/README.md` "Events").
//!
//! This is for an unfiltered subscription, which is what a full client makes. With a label
//! filter the core also stops describing terminals that no longer match; that isn't modelled.
//!
//! Events carry only the fields their rows name, so fields no event carries (such as `status_ms`)
//! keep the value of the last whole record: the snapshot, `created`, `restarted` or `missed`.

use std::collections::{BTreeMap, BTreeSet};

use indexmap::IndexMap;

use crate::event::{Ended, Event, EventData, Notification, Snapshot};
use crate::methods::SubscribeResult;
use crate::types::{Labels, Terminal, TerminalState};

/// The label holding the last turn a person looked at (`herdr-core-protocol.md` §3.4).
pub const SEEN_LABEL: &str = "herdr.seen";

/// A document, by machine (`None` for this core) and name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocKey {
    pub machine: Option<String>,
    pub name: String,
}

/// What applying notifications changed. Apply a batch into one `Changes` to coalesce it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// A `missed` snapshot replaced everything: read every document again.
    pub resynced: bool,
    /// Terminals added or changed, still in the set.
    pub updated: BTreeSet<String>,
    /// Terminals no longer in the set.
    pub removed: BTreeSet<String>,
    /// Terminals with a new program to reattach to.
    pub restarted: BTreeSet<String>,
    /// Terminals whose turn ended, with the new turn. The seen decision happens here.
    pub turns: BTreeMap<String, u64>,
    /// The highest `herdr.seen` that went by for each terminal, so a raise isn't hidden by a later,
    /// lower write in the same batch. A resync can't tell; check `resynced`.
    pub seen: BTreeMap<String, u64>,
    /// Documents that changed, with the highest version announced.
    pub docs: BTreeMap<DocKey, u64>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        *self == Changes::default()
    }

    fn update(&mut self, address: &str) {
        self.removed.remove(address);
        self.updated.insert(address.to_owned());
    }

    fn remove(&mut self, address: &str) {
        self.updated.remove(address);
        self.restarted.remove(address);
        self.removed.insert(address.to_owned());
    }

    fn note_seen(&mut self, address: &str, labels: &Labels) {
        let Some(seen) = labels.get(SEEN_LABEL).and_then(|v| v.parse::<u64>().ok()) else {
            return;
        };
        let high = self.seen.entry(address.to_owned()).or_default();
        *high = (*high).max(seen);
    }
}

/// The subscription's terminals, in the core's order, and the last `seq` applied.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerminalSet {
    seq: u64,
    terminals: IndexMap<String, Terminal>,
}

impl TerminalSet {
    /// From a snapshot: the subscribe reply, or a `missed`. After a reconnect, start again from
    /// the new subscription's snapshot, since `seq` restarts with each core process.
    pub fn new(seq: u64, terminals: Vec<Terminal>) -> Self {
        let terminals = terminals
            .into_iter()
            .map(|t| (t.address.clone(), t))
            .collect();
        TerminalSet { seq, terminals }
    }

    /// The last `seq` applied.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn get(&self, address: &str) -> Option<&Terminal> {
        self.terminals.get(address)
    }

    pub fn contains(&self, address: &str) -> bool {
        self.terminals.contains_key(address)
    }

    /// Every terminal, live and finished, in the core's order.
    pub fn iter(&self) -> impl Iterator<Item = &Terminal> {
        self.terminals.values()
    }

    /// The live terminals, which is what herdr-app shows.
    pub fn live(&self) -> impl Iterator<Item = &Terminal> {
        self.iter().filter(|t| t.is_live())
    }

    pub fn len(&self) -> usize {
        self.terminals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.terminals.is_empty()
    }

    /// Applies one notification.
    pub fn apply(&mut self, notification: Notification) -> Changes {
        let mut changes = Changes::default();
        self.apply_into(notification, &mut changes);
        changes
    }

    /// Applies one notification, adding what changed to `changes`.
    pub fn apply_into(&mut self, notification: Notification, changes: &mut Changes) {
        match notification {
            Notification::Event(event) => self.apply_event(event, changes),
            Notification::Missed(snapshot) => self.resync(snapshot, changes),
            Notification::Heartbeat { .. } | Notification::Other { .. } => {}
        }
    }

    /// Replaces everything with a fresh snapshot. Events at or below its `seq` are then dropped.
    fn resync(&mut self, snapshot: Snapshot, changes: &mut Changes) {
        let fresh = TerminalSet::new(snapshot.seq, snapshot.terminals);
        for address in self.terminals.keys().filter(|a| !fresh.contains(a)) {
            changes.remove(address);
        }
        for address in fresh.terminals.keys() {
            changes.update(address);
        }
        changes.resynced = true;
        *self = fresh;
    }

    fn apply_event(&mut self, event: Event, changes: &mut Changes) {
        if event.seq <= self.seq {
            return;
        }
        self.seq = event.seq;
        let Event {
            terminal,
            machine,
            data,
            ..
        } = event;
        if let EventData::Doc(doc) = data {
            let version = changes
                .docs
                .entry(DocKey {
                    machine,
                    name: doc.name,
                })
                .or_default();
            *version = (*version).max(doc.version);
            return;
        }
        let Some(address) = terminal else { return };
        match data {
            EventData::Created(record) => {
                changes.note_seen(&address, &record.labels);
                self.put(&address, *record, changes);
            }
            EventData::Restarted(record) => {
                changes.note_seen(&address, &record.labels);
                self.put(&address, *record, changes);
                changes.restarted.insert(address);
            }
            EventData::Labels(labels) => {
                changes.note_seen(&address, &labels.labels);
                match labels.record {
                    Some(mut record) => {
                        record.labels = labels.labels;
                        self.put(&address, *record, changes);
                    }
                    None => self.edit(&address, changes, |t| t.labels = labels.labels),
                }
            }
            EventData::Removed => {
                if self.terminals.shift_remove(&address).is_some() {
                    changes.remove(&address);
                }
            }
            EventData::Lost(ended) => self.edit(&address, changes, |t| {
                set_ended(t, ended);
                t.state = TerminalState::Live;
                t.pid = None;
                t.foreground = None;
            }),
            EventData::Finished(ended) => self.edit(&address, changes, |t| {
                set_ended(t, ended);
                t.state = TerminalState::Finished;
                t.foreground = None;
            }),
            EventData::Keep(keep) => self.edit(&address, changes, |t| t.keep = keep.keep),
            EventData::Status(status) => self.edit(&address, changes, |t| {
                let status = *status;
                t.status = status.status;
                t.records = status.records;
                t.reported = status.reported;
            }),
            EventData::Turn(turn) => {
                if self.terminals.contains_key(&address) {
                    changes.turns.insert(address.clone(), turn.turn);
                }
                self.edit(&address, changes, |t| t.turn = turn.turn);
            }
            EventData::Title(title) => self.edit(&address, changes, |t| t.title = title.title),
            EventData::Size(size) => self.edit(&address, changes, |t| {
                t.cols = size.cols;
                t.rows = size.rows;
            }),
            EventData::Foreground(foreground) => {
                self.edit(&address, changes, |t| t.foreground = foreground.map(|f| *f))
            }
            EventData::Doc(_) | EventData::Other { .. } => {}
        }
    }

    /// Adds a record, or replaces it in place.
    fn put(&mut self, address: &str, mut record: Terminal, changes: &mut Changes) {
        address.clone_into(&mut record.address);
        match self.terminals.get_mut(address) {
            Some(existing) => *existing = record,
            None => {
                self.terminals.insert(address.to_owned(), record);
            }
        }
        changes.update(address);
    }

    /// Changes a terminal the set holds. Events about other terminals change nothing.
    fn edit(&mut self, address: &str, changes: &mut Changes, f: impl FnOnce(&mut Terminal)) {
        if let Some(terminal) = self.terminals.get_mut(address) {
            f(terminal);
            changes.update(address);
        }
    }
}

fn set_ended(t: &mut Terminal, ended: Ended) {
    t.reason = ended.reason;
    t.exit_code = ended.exit_code;
    t.signal = ended.signal;
    t.finished_ms = ended.finished_ms;
}

impl From<Snapshot> for TerminalSet {
    fn from(snapshot: Snapshot) -> Self {
        TerminalSet::new(snapshot.seq, snapshot.terminals)
    }
}

impl From<SubscribeResult> for TerminalSet {
    fn from(subscribed: SubscribeResult) -> Self {
        TerminalSet::new(subscribed.seq, subscribed.terminals)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Foreground, Reason, StatusState};
    use serde_json::{Value, json};

    fn record(address: &str) -> Value {
        json!({
            "address": address, "argv": ["sh"], "cwd": "/", "state": "live", "created_ms": 1,
            "screen": "exact", "cols": 80, "rows": 24, "labels": {}, "pid": 10,
            "foreground": {"pid": 11, "name": "vim", "argv": ["vim"], "cwd": "/src"}
        })
    }

    fn terminal(address: &str) -> Terminal {
        serde_json::from_value(record(address)).unwrap()
    }

    fn event(seq: u64, kind: &str, terminal: &str, data: Value) -> Notification {
        notification(
            json!({"method": "event", "params": {"seq": seq, "type": kind, "terminal": terminal, "data": data}}),
        )
    }

    fn notification(value: Value) -> Notification {
        Notification::parse(&serde_json::to_vec(&value).unwrap()).unwrap()
    }

    fn set(addresses: &[&str]) -> TerminalSet {
        TerminalSet::new(10, addresses.iter().map(|a| terminal(a)).collect())
    }

    fn addresses(set: &TerminalSet) -> Vec<&str> {
        set.iter().map(|t| t.address.as_str()).collect()
    }

    fn names(items: &BTreeSet<String>) -> Vec<&str> {
        items.iter().map(String::as_str).collect()
    }

    #[test]
    fn created_adds_at_the_end_and_replaces_in_place() {
        let mut set = set(&["t1", "t2"]);
        let changes = set.apply(event(11, "created", "t3", record("t3")));
        assert_eq!(addresses(&set), ["t1", "t2", "t3"]);
        assert_eq!(names(&changes.updated), ["t3"]);

        let mut again = record("t1");
        again["title"] = json!("reconnected");
        set.apply(event(12, "created", "t1", again));
        assert_eq!(addresses(&set), ["t1", "t2", "t3"]);
        assert_eq!(set.get("t1").unwrap().title, "reconnected");
        assert_eq!(set.seq(), 12);
    }

    #[test]
    fn lost_keeps_it_live_without_a_pid_or_foreground() {
        let mut set = set(&["t1"]);
        let lost =
            json!({"reason": "signaled", "exit_code": null, "signal": 9, "finished_ms": null});
        set.apply(event(11, "lost", "t1", lost));
        let t = set.get("t1").unwrap();
        assert!(t.is_live());
        assert_eq!(t.reason, Some(Reason::Signaled));
        assert_eq!(t.signal, Some(9));
        assert_eq!(t.pid, None);
        assert_eq!(t.foreground, None);
    }

    #[test]
    fn restarted_replaces_and_asks_for_a_reattach() {
        let mut set = set(&["t1", "t2"]);
        let mut fresh = record("t1");
        fresh["pid"] = json!(99);
        let changes = set.apply(event(11, "restarted", "t1", fresh));
        assert_eq!(set.get("t1").unwrap().pid, Some(99));
        assert_eq!(addresses(&set), ["t1", "t2"]);
        assert_eq!(names(&changes.restarted), ["t1"]);
        assert_eq!(names(&changes.updated), ["t1"]);
    }

    #[test]
    fn keep_sets_keep() {
        let mut set = set(&["t1"]);
        set.apply(event(11, "keep", "t1", json!({"keep": true})));
        assert!(set.get("t1").unwrap().keep);
        set.apply(event(12, "keep", "t1", json!({"keep": false})));
        assert!(!set.get("t1").unwrap().keep);
    }

    #[test]
    fn finished_sets_the_end_and_drops_the_foreground() {
        let mut set = set(&["t1"]);
        let finished =
            json!({"reason": "exited", "exit_code": 3, "signal": null, "finished_ms": 500});
        set.apply(event(11, "finished", "t1", finished));
        let t = set.get("t1").unwrap();
        assert_eq!(t.state, TerminalState::Finished);
        assert!(!t.is_live());
        assert_eq!(t.exit_code, Some(3));
        assert_eq!(t.finished_ms, Some(500));
        assert_eq!(t.foreground, None);
        assert_eq!(t.pid, Some(10), "finished keeps the pid");
        assert_eq!(set.live().count(), 0);
        assert_eq!(set.len(), 1, "finished terminals stay in the set");
    }

    #[test]
    fn removed_drops_it_and_keeps_the_order() {
        let mut set = set(&["t1", "t2", "t3"]);
        let changes = set.apply(event(11, "removed", "t2", json!({})));
        assert_eq!(addresses(&set), ["t1", "t3"]);
        assert_eq!(names(&changes.removed), ["t2"]);
        assert!(changes.updated.is_empty());
        assert!(set.apply(event(12, "removed", "t9", json!({}))).is_empty());
    }

    #[test]
    fn status_replaces_all_three() {
        let mut set = set(&["t1"]);
        let data = json!({"status": {"state": "blocked", "kind": "question"}, "records": {"sub": {"state": "working"}}, "reported": true});
        set.apply(event(11, "status", "t1", data));
        let t = set.get("t1").unwrap();
        assert_eq!(t.status.as_ref().unwrap().state, StatusState::Blocked);
        assert_eq!(t.records["sub"].state, StatusState::Working);
        assert!(t.reported);
        set.apply(event(
            12,
            "status",
            "t1",
            json!({"status": null, "records": {}, "reported": true}),
        ));
        let t = set.get("t1").unwrap();
        assert_eq!(t.status, None);
        assert!(t.records.is_empty());
    }

    #[test]
    fn turn_sets_the_turn_and_reports_it() {
        let mut set = set(&["t1"]);
        let changes = set.apply(event(11, "turn", "t1", json!({"turn": 4, "state": "done"})));
        assert_eq!(set.get("t1").unwrap().turn, 4);
        assert_eq!(changes.turns, BTreeMap::from([("t1".into(), 4)]));
        assert!(
            set.apply(event(12, "turn", "t9", json!({"turn": 1, "state": "idle"})))
                .is_empty()
        );
    }

    #[test]
    fn title_sets_the_title() {
        let mut set = set(&["t1"]);
        set.apply(event(11, "title", "t1", json!({"title": "vim"})));
        assert_eq!(set.get("t1").unwrap().title, "vim");
    }

    #[test]
    fn labels_replace_the_labels() {
        let mut set = set(&["t1"]);
        set.apply(event(
            11,
            "labels",
            "t1",
            json!({"labels": {"a": "1", "b": "2"}}),
        ));
        set.apply(event(12, "labels", "t1", json!({"labels": {"b": "3"}})));
        assert_eq!(
            set.get("t1").unwrap().labels,
            Labels::from([("b".into(), "3".into())])
        );
    }

    #[test]
    fn labels_with_a_record_add_the_terminal() {
        let mut set = set(&["t1"]);
        let changes = set.apply(event(
            11,
            "labels",
            "t2",
            json!({"labels": {"x": "1"}, "record": record("t2")}),
        ));
        assert_eq!(addresses(&set), ["t1", "t2"]);
        assert_eq!(set.get("t2").unwrap().label("x"), Some("1"));
        assert_eq!(names(&changes.updated), ["t2"]);
        assert!(
            set.apply(event(12, "labels", "t9", json!({"labels": {}})))
                .is_empty()
        );
    }

    #[test]
    fn size_sets_the_grid() {
        let mut set = set(&["t1"]);
        set.apply(event(11, "size", "t1", json!({"cols": 120, "rows": 40})));
        let t = set.get("t1").unwrap();
        assert_eq!((t.cols, t.rows), (120, 40));
    }

    #[test]
    fn foreground_sets_or_clears_it() {
        let mut set = set(&["t1"]);
        set.apply(event(
            11,
            "foreground",
            "t1",
            json!({"pid": 5, "name": "git", "argv": ["git"], "cwd": "/r"}),
        ));
        let expected = Foreground {
            pid: 5,
            name: "git".into(),
            argv: vec!["git".into()],
            cwd: "/r".into(),
            ..Default::default()
        };
        assert_eq!(set.get("t1").unwrap().foreground, Some(expected));
        set.apply(event(12, "foreground", "t1", Value::Null));
        assert_eq!(set.get("t1").unwrap().foreground, None);
    }

    #[test]
    fn doc_events_report_the_highest_version_per_machine() {
        let mut set = set(&["t1"]);
        let mut changes = Changes::default();
        let doc = |seq: u64, machine: Option<&str>, version: u64| {
            let mut params = json!({"seq": seq, "type": "doc", "data": {"name": "herdr-app", "version": version}});
            if let Some(machine) = machine {
                params["machine"] = json!(machine);
            }
            notification(json!({"method": "event", "params": params}))
        };
        set.apply_into(doc(11, None, 5), &mut changes);
        set.apply_into(doc(12, None, 4), &mut changes);
        set.apply_into(doc(13, Some("devbox"), 2), &mut changes);
        let local = DocKey {
            machine: None,
            name: "herdr-app".into(),
        };
        let remote = DocKey {
            machine: Some("devbox".into()),
            name: "herdr-app".into(),
        };
        assert_eq!(changes.docs, BTreeMap::from([(local, 5), (remote, 2)]));
        assert!(changes.updated.is_empty());
        assert_eq!(set.seq(), 13);
    }

    #[test]
    fn events_at_or_below_the_seq_are_dropped() {
        let mut set = set(&["t1"]);
        assert!(
            set.apply(event(10, "title", "t1", json!({"title": "old"})))
                .is_empty()
        );
        assert!(
            set.apply(event(3, "title", "t1", json!({"title": "older"})))
                .is_empty()
        );
        assert_eq!(set.get("t1").unwrap().title, "");
        assert_eq!(set.seq(), 10);
        set.apply(event(11, "title", "t1", json!({"title": "new"})));
        assert!(
            set.apply(event(11, "title", "t1", json!({"title": "again"})))
                .is_empty()
        );
        assert_eq!(set.get("t1").unwrap().title, "new");
    }

    #[test]
    fn events_about_unknown_terminals_only_move_the_seq() {
        let mut set = set(&["t1"]);
        let before = set.clone();
        for (kind, data) in [
            ("lost", json!({"reason": "signaled"})),
            ("keep", json!({"keep": true})),
            ("finished", json!({"reason": "exited"})),
            ("status", json!({"status": null})),
            ("title", json!({"title": "x"})),
            ("size", json!({"cols": 1, "rows": 1})),
            ("foreground", Value::Null),
        ] {
            assert!(
                set.apply(event(set.seq() + 1, kind, "t9", data)).is_empty(),
                "{kind}"
            );
        }
        assert_eq!(set.terminals, before.terminals);
        assert_eq!(set.seq(), 17);
    }

    #[test]
    fn heartbeats_and_unknown_notifications_change_nothing() {
        let mut set = set(&["t1"]);
        let before = set.clone();
        assert!(set.apply(Notification::Heartbeat { seq: 99 }).is_empty());
        assert!(
            set.apply(event(11, "brand_new", "t1", json!({"x": 1})))
                .is_empty()
        );
        assert!(
            set.apply(notification(json!({"method": "brand_new", "params": {}})))
                .is_empty()
        );
        assert_eq!(set.terminals, before.terminals);
    }

    #[test]
    fn missed_replaces_everything_and_drops_older_events() {
        let mut set = set(&["t1", "t2"]);
        let missed = notification(
            json!({"method": "missed", "params": {"seq": 20, "terminals": [record("t2"), record("t3")]}}),
        );
        let changes = set.apply(missed);
        assert!(changes.resynced);
        assert_eq!(addresses(&set), ["t2", "t3"]);
        assert_eq!(names(&changes.removed), ["t1"]);
        assert_eq!(names(&changes.updated), ["t2", "t3"]);
        assert_eq!(set.seq(), 20);
        assert!(
            set.apply(event(20, "title", "t2", json!({"title": "stale"})))
                .is_empty()
        );
        assert!(set.apply(event(15, "removed", "t2", json!({}))).is_empty());
        set.apply(event(21, "title", "t2", json!({"title": "fresh"})));
        assert_eq!(set.get("t2").unwrap().title, "fresh");
    }

    #[test]
    fn missed_with_a_lower_seq_still_resyncs() {
        // A machine's subscription coming back reaches local subscribers as `missed` with this
        // core's seq; the snapshot is the truth either way.
        let mut set = set(&["t1"]);
        let changes = set.apply(notification(
            json!({"method": "missed", "params": {"seq": 4, "terminals": []}}),
        ));
        assert!(changes.resynced);
        assert!(set.is_empty());
        assert_eq!(set.seq(), 4);
    }

    #[test]
    fn seen_highs_survive_a_later_lower_write() {
        let mut set = set(&["t1"]);
        let mut changes = Changes::default();
        let labels = |seen: &str| json!({"labels": {"herdr.seen": seen}});
        set.apply_into(event(11, "labels", "t1", labels("7")), &mut changes);
        set.apply_into(event(12, "labels", "t1", labels("6")), &mut changes);
        set.apply_into(
            event(13, "labels", "t1", labels("not a number")),
            &mut changes,
        );
        assert_eq!(changes.seen, BTreeMap::from([("t1".into(), 7)]));
        assert_eq!(
            set.get("t1").unwrap().label(SEEN_LABEL),
            Some("not a number")
        );

        let mut created = record("t2");
        created["labels"] = json!({"herdr.seen": "0"});
        set.apply_into(event(14, "created", "t2", created), &mut changes);
        assert_eq!(changes.seen["t2"], 0);
    }

    #[test]
    fn a_batch_that_removes_then_recreates_reports_an_update() {
        let mut set = set(&["t1"]);
        let mut changes = Changes::default();
        set.apply_into(event(11, "removed", "t1", json!({})), &mut changes);
        set.apply_into(event(12, "created", "t1", record("t1")), &mut changes);
        assert_eq!(names(&changes.updated), ["t1"]);
        assert!(changes.removed.is_empty());
        set.apply_into(event(13, "removed", "t1", json!({})), &mut changes);
        assert_eq!(names(&changes.removed), ["t1"]);
        assert!(changes.updated.is_empty());
    }

    #[test]
    fn subscribe_results_and_snapshots_build_sets() {
        let subscribed: SubscribeResult =
            serde_json::from_value(json!({"seq": 3, "terminals": [record("t1")], "channel": 1}))
                .unwrap();
        let set = TerminalSet::from(subscribed);
        assert_eq!(set.seq(), 3);
        assert_eq!(addresses(&set), ["t1"]);
        let set = TerminalSet::from(Snapshot {
            seq: 4,
            terminals: vec![terminal("t2")],
            ..Default::default()
        });
        assert_eq!(addresses(&set), ["t2"]);
    }
}
