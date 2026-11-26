//! One machine's layout document, kept in step with its core under compare-and-set
//! (`herdr-core-protocol.md` §3 "Concurrency and versioning", rules 3-5).
//!
//! The `Mirror` does no I/O. Its machine's worker asks [`Mirror::next_round`] for the call to make
//! (`doc.get` or `doc.set`), makes it, and hands the reply back to [`Mirror::finish_read`] or
//! [`Mirror::finish_write`]. One round is in flight at a time.
//!
//! - It keeps the last document and version read or written.
//! - Edits are [`Intent`]s. Each applies to what the screen shows at once; the next round writes
//!   the projection at the known version, carrying every edit made since the last one.
//! - On `conflict` it reads again and replays the intents on the fresh copy. Intents whose target
//!   is gone are dropped and reported; after [`MAX_REPLAYS`] conflicts in a row the rest are too.
//! - A `doc` event announcing a newer version, a `missed`, or a reconnect makes it read again.
//! - It never writes in reaction to a read: only an edit someone made is written, and only when
//!   it changes the document.

use herdr_proto::methods::{DocGet, DocGetResult, DocSet, DocSetResult};
use herdr_proto::rpc::{ErrorKind, RpcError};

use crate::document::{DOCUMENT, Document};
use crate::intent::Intent;

/// Conflicts in a row before the edits still pending are given up.
pub const MAX_REPLAYS: u32 = 5;

/// Whether the machine's layout can be shown and edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Not read yet.
    Loading,
    Ready,
    /// The last call failed in a way the next round may not: the worker reads again at its own
    /// pace. The last document read, if any, stays shown.
    Unavailable(String),
    /// `needs_update` or `needs_login`: no rounds until a `doc` event, a `missed` or a reconnect,
    /// as in herdr-app (it doesn't retry these on a timer).
    Blocked(String),
    /// A document this crate can't read. It is never written; a `doc` event reads it again.
    Undecodable(String),
}

/// The call a round makes.
#[derive(Clone, Debug, PartialEq)]
pub enum Round {
    Read(DocGet),
    Write(DocSet),
}

/// What a finished round changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// What [`Mirror::shown`] returns changed: redraw.
    pub changed: bool,
    /// Edits given up: their target was gone on the fresh copy (or they were already in it, when
    /// a write's reply was lost), or the conflicts didn't stop.
    pub dropped: Vec<Intent>,
    /// Something to tell the person, once per reason ("Layout unavailable").
    pub notice: Option<String>,
}

/// Why an edit wasn't taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    /// The layout isn't [`Layout::Ready`].
    Unavailable,
    /// It does nothing to what is shown: its target is gone, or it is already there.
    NoEffect,
}

#[derive(Clone, Debug, PartialEq)]
enum Flight {
    Read,
    Write {
        intents: Vec<Intent>,
        projection: Document,
    },
}

/// One machine's `herdr-app` document.
#[derive(Clone, Debug, PartialEq)]
pub struct Mirror {
    machine: Option<String>,
    /// The last document read or written, as the core stores it (unqualified).
    base: Option<Document>,
    version: u64,
    /// The highest version a `doc` event announced.
    announced: u64,
    /// A `missed`, a reconnect or a failed call: read again.
    stale: bool,
    layout: Layout,
    flight: Option<Flight>,
    pending: Vec<Intent>,
    replays: u32,
    notified: Option<String>,
}

impl Mirror {
    /// The mirror of a machine's document (`None`: this core's own).
    pub fn new(machine: Option<String>) -> Self {
        Mirror {
            machine,
            base: None,
            version: 0,
            announced: 0,
            stale: true,
            layout: Layout::Loading,
            flight: None,
            pending: Vec::new(),
            replays: 0,
            notified: None,
        }
    }

    pub fn machine(&self) -> Option<&str> {
        self.machine.as_deref()
    }

    /// The version of the last document read or written.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The last document read or written, as the core stores it.
    pub fn base(&self) -> Option<&Document> {
        self.base.as_ref()
    }

    /// Edits not yet in a write.
    pub fn pending(&self) -> &[Intent] {
        &self.pending
    }

    pub fn in_flight(&self) -> bool {
        self.flight.is_some()
    }

    /// What the screen shows: the last document with the edits in flight and pending applied,
    /// qualified for the machine. `None` before the first read.
    pub fn shown(&self) -> Option<Document> {
        let base = self.base.as_ref()?;
        let mut doc = match &self.flight {
            Some(Flight::Write { projection, .. }) => projection.qualified(self.machine()),
            _ => base.qualified(self.machine()),
        };
        for intent in &self.pending {
            doc.apply(intent);
        }
        Some(doc)
    }

    /// Takes an edit someone made. It shows at once and goes out with the next write.
    pub fn edit(&mut self, intent: Intent) -> Result<(), EditError> {
        if self.layout != Layout::Ready {
            return Err(EditError::Unavailable);
        }
        let mut doc = self.shown().ok_or(EditError::Unavailable)?;
        if !doc.apply(&intent) {
            return Err(EditError::NoEffect);
        }
        self.pending.push(intent);
        Ok(())
    }

    /// A `doc` event for this machine's `herdr-app` document.
    pub fn note_doc(&mut self, version: u64) {
        self.announced = self.announced.max(version);
        if matches!(self.layout, Layout::Blocked(_) | Layout::Undecodable(_)) {
            self.stale = true;
        }
    }

    /// A `missed`: read again.
    pub fn note_missed(&mut self) {
        self.stale = true;
    }

    /// A new connection to the core. A round in flight on the old one is forgotten; its edits are
    /// pending again (replaying them is harmless if the write went through: see [`Intent`]).
    pub fn reconnected(&mut self) {
        if let Some(Flight::Write { intents, .. }) = self.flight.take() {
            self.requeue(intents);
        }
        self.flight = None;
        self.stale = true;
        self.announced = 0;
    }

    fn needs_read(&self) -> bool {
        self.base.is_none() || self.stale || self.announced > self.version
    }

    /// The next call to make, if any. At most one is in flight.
    pub fn next_round(&mut self) -> Option<Round> {
        if self.flight.is_some() {
            return None;
        }
        let waits = matches!(self.layout, Layout::Blocked(_) | Layout::Undecodable(_));
        if waits && !self.stale && self.announced <= self.version {
            return None;
        }
        if self.needs_read() {
            self.flight = Some(Flight::Read);
            return Some(Round::Read(DocGet {
                name: DOCUMENT.into(),
                on: self.machine.clone(),
                ..Default::default()
            }));
        }
        if self.layout != Layout::Ready || self.pending.is_empty() {
            return None;
        }
        let projection = self.shown()?.localized(self.machine());
        if self.base.as_ref() == Some(&projection) {
            // The edits cancel out: nothing to write.
            self.pending.clear();
            self.replays = 0;
            return None;
        }
        let round = Round::Write(DocSet {
            name: DOCUMENT.into(),
            value: projection.to_value(),
            version: self.version,
            on: self.machine.clone(),
            ..Default::default()
        });
        self.flight = Some(Flight::Write {
            intents: std::mem::take(&mut self.pending),
            projection,
        });
        Some(round)
    }

    /// The reply to a [`Round::Read`].
    pub fn finish_read(&mut self, result: Result<DocGetResult, RpcError>) -> Report {
        if self.flight != Some(Flight::Read) {
            return Report::default();
        }
        self.flight = None;
        let read = match result {
            Ok(read) => read,
            Err(error) => return self.failed(&error),
        };
        self.stale = false;
        self.version = read.version;
        let doc = match Document::from_value(read.value) {
            Ok(doc) => doc,
            Err(error) => return self.unusable(Layout::Undecodable(error.to_string())),
        };
        let before = self.shown();
        self.base = Some(doc);
        self.layout = Layout::Ready;
        self.notified = None;
        let dropped = self.rebase();
        if self.pending.is_empty() {
            // Nothing left to replay: the next conflict starts a new count.
            self.replays = 0;
        }
        Report {
            changed: self.shown() != before,
            dropped,
            notice: None,
        }
    }

    /// The reply to a [`Round::Write`].
    pub fn finish_write(&mut self, result: Result<DocSetResult, RpcError>) -> Report {
        let Some(Flight::Write {
            intents,
            projection,
        }) = self.flight.take()
        else {
            return Report::default();
        };
        match result {
            Ok(written) => {
                self.base = Some(projection);
                self.version = written.version;
                self.replays = 0;
                Report::default()
            }
            Err(error) if error.kind == ErrorKind::Conflict => {
                self.requeue(intents);
                self.stale = true;
                self.replays += 1;
                if self.replays <= MAX_REPLAYS {
                    return Report::default();
                }
                self.replays = 0;
                let before = self.shown();
                let dropped = std::mem::take(&mut self.pending);
                Report {
                    changed: self.shown() != before,
                    dropped,
                    notice: Some("Layout kept changing elsewhere; edits were not saved".into()),
                }
            }
            Err(error) => {
                self.requeue(intents);
                self.failed(&error)
            }
        }
    }

    fn requeue(&mut self, mut intents: Vec<Intent>) {
        intents.append(&mut self.pending);
        self.pending = intents;
    }

    fn failed(&mut self, error: &RpcError) -> Report {
        self.stale = true;
        let reason = match &error.hint {
            Some(hint) if !hint.is_empty() => format!("{}: {hint}", error.message),
            _ => error.message.clone(),
        };
        match error.kind {
            ErrorKind::NeedsUpdate | ErrorKind::NeedsLogin => {
                self.stale = false;
                self.unusable(Layout::Blocked(reason))
            }
            _ => {
                self.layout = Layout::Unavailable(reason);
                Report::default()
            }
        }
    }

    /// Blocked or undecodable: say so once per reason.
    fn unusable(&mut self, layout: Layout) -> Report {
        let reason = match &layout {
            Layout::Blocked(r) | Layout::Undecodable(r) => r.clone(),
            _ => unreachable!("only blocked and undecodable layouts are unusable"),
        };
        self.layout = layout;
        let notice = (self.notified.as_ref() != Some(&reason)).then(|| reason.clone());
        self.notified = Some(reason);
        Report {
            changed: false,
            dropped: Vec::new(),
            notice,
        }
    }

    /// Replays the pending edits on a new base; those that no longer apply are dropped.
    fn rebase(&mut self) -> Vec<Intent> {
        let Some(base) = &self.base else {
            return Vec::new();
        };
        let mut doc = base.qualified(self.machine());
        let (kept, dropped) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|intent| doc.apply(intent));
        self.pending = kept;
        dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Axis, Content};
    use serde_json::{Value, json};

    fn conflict() -> RpcError {
        error(
            ErrorKind::Conflict,
            "document \"herdr-app\" is at version 9, not 8",
        )
    }

    fn error(kind: ErrorKind, message: &str) -> RpcError {
        RpcError {
            code: -32000,
            message: message.into(),
            kind,
            hint: None,
            data: Default::default(),
        }
    }

    fn one_space() -> Value {
        json!({"spaces": [{"id": "s1", "name": "", "folder": "/w", "tabs": [{"id": "c1", "name": "", "root":
            {"id": "c1", "type": "row", "children": [{"id": "x1", "type": "block", "content": {"terminal": "t1"}}]}}]}]})
    }

    fn read(value: Value, version: u64) -> Result<DocGetResult, RpcError> {
        Ok(DocGetResult {
            value,
            version,
            ..Default::default()
        })
    }

    fn written(version: u64) -> Result<DocSetResult, RpcError> {
        Ok(DocSetResult {
            version,
            ..Default::default()
        })
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

    fn rename(tab: &str, name: &str) -> Intent {
        Intent::RenameTab {
            tab: tab.into(),
            name: name.into(),
        }
    }

    fn ready(value: Value, version: u64) -> Mirror {
        let mut m = Mirror::new(None);
        assert!(matches!(m.next_round(), Some(Round::Read(_))));
        m.finish_read(read(value, version));
        assert_eq!(m.layout(), &Layout::Ready);
        m
    }

    fn write(m: &mut Mirror) -> DocSet {
        match m.next_round() {
            Some(Round::Write(set)) => set,
            other => panic!("expected a write, got {other:?}"),
        }
    }

    #[test]
    fn the_first_round_reads_and_a_read_never_writes() {
        let mut m = Mirror::new(None);
        assert_eq!(m.edit(rename("c1", "x")), Err(EditError::Unavailable));
        let Some(Round::Read(get)) = m.next_round() else {
            panic!()
        };
        assert_eq!((get.name.as_str(), get.on), ("herdr-app", None));
        assert_eq!(m.next_round(), None, "one round in flight");
        let report = m.finish_read(read(Value::Null, 0));
        assert!(report.changed);
        assert_eq!(m.shown(), Some(Document::default()));
        assert_eq!(m.next_round(), None, "nothing to write");
        // Another screen's copy, read after a doc event: shown, never written back.
        m.note_doc(3);
        assert!(matches!(m.next_round(), Some(Round::Read(_))));
        assert!(m.finish_read(read(one_space(), 3)).changed);
        assert_eq!(m.next_round(), None);
        assert_eq!(m.version(), 3);
    }

    #[test]
    fn an_edit_writes_the_projection_at_the_known_version() {
        let mut m = ready(one_space(), 7);
        m.edit(split("x1", "x2", "t2")).unwrap();
        assert_eq!(
            m.shown().unwrap().addresses(),
            ["t1", "t2"],
            "shown at once"
        );
        let set = write(&mut m);
        assert_eq!((set.name.as_str(), set.version), ("herdr-app", 7));
        assert_eq!(
            Document::from_value(set.value.clone()).unwrap().addresses(),
            ["t1", "t2"]
        );
        assert!(m.finish_write(written(8)).dropped.is_empty());
        assert_eq!(m.version(), 8);
        assert_eq!(m.base().unwrap().to_value(), set.value);
        // Our own write's doc event reads nothing.
        m.note_doc(8);
        assert_eq!(m.next_round(), None);
    }

    #[test]
    fn edits_made_during_a_write_go_out_together_in_the_next() {
        let mut m = ready(one_space(), 1);
        m.edit(split("x1", "x2", "t2")).unwrap();
        let _ = write(&mut m);
        m.edit(split("x2", "x3", "t3")).unwrap();
        m.edit(rename("c1", "review")).unwrap();
        assert_eq!(m.next_round(), None, "one round in flight");
        assert_eq!(m.shown().unwrap().addresses(), ["t1", "t2", "t3"]);
        m.finish_write(written(2));
        let set = write(&mut m);
        assert_eq!(set.version, 2);
        let doc = Document::from_value(set.value).unwrap();
        assert_eq!(doc.addresses(), ["t1", "t2", "t3"]);
        assert_eq!(doc.spaces[0].tabs[0].name, "review");
    }

    #[test]
    fn a_conflict_reads_again_and_replays_the_edit_on_the_fresh_copy() {
        let mut m = ready(one_space(), 1);
        m.edit(split("x1", "x2", "t2")).unwrap();
        let _ = write(&mut m);
        assert_eq!(m.finish_write(Err(conflict())), Report::default());
        assert!(matches!(m.next_round(), Some(Round::Read(_))));
        // The other screen renamed the space and added a tab.
        let mut fresh = one_space();
        fresh["spaces"][0]["name"] = json!("api");
        fresh["spaces"][0]["tabs"].as_array_mut().unwrap().push(json!({"id": "c2", "name": "", "root":
            {"id": "c2", "type": "row", "children": [{"id": "x9", "type": "block", "content": {"terminal": "t9"}}]}}));
        let report = m.finish_read(read(fresh, 2));
        assert!(report.changed && report.dropped.is_empty());
        let set = write(&mut m);
        assert_eq!(set.version, 2);
        let doc = Document::from_value(set.value).unwrap();
        assert_eq!(doc.spaces[0].name, "api", "theirs kept");
        assert_eq!(doc.addresses(), ["t1", "t2", "t9"], "ours replayed");
    }

    #[test]
    fn an_edit_whose_target_is_gone_is_dropped_on_replay() {
        let mut m = ready(one_space(), 1);
        m.edit(split("x1", "x2", "t2")).unwrap();
        m.edit(rename("c1", "mine")).unwrap();
        let _ = write(&mut m);
        m.finish_write(Err(conflict()));
        m.next_round();
        // The other screen closed the tab.
        let report = m.finish_read(read(json!({"spaces": []}), 2));
        assert_eq!(
            report.dropped,
            [split("x1", "x2", "t2"), rename("c1", "mine")]
        );
        assert_eq!(m.next_round(), None, "nothing left to write");
        assert_eq!(m.shown(), Some(Document::default()));
    }

    #[test]
    fn a_terminal_both_screens_placed_is_not_placed_twice() {
        let mut m = ready(one_space(), 1);
        m.edit(split("x1", "x2", "t2")).unwrap();
        let _ = write(&mut m);
        m.finish_write(Err(conflict()));
        m.next_round();
        // herdr-app adopted t2 first, in a pane of its own.
        let mut fresh = one_space();
        fresh["spaces"][0]["tabs"][0]["root"]["children"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": "y2", "type": "block", "content": {"terminal": "t2"}}));
        let report = m.finish_read(read(fresh, 2));
        assert_eq!(report.dropped.len(), 1);
        assert_eq!(m.shown().unwrap().addresses(), ["t1", "t2"]);
        assert_eq!(m.next_round(), None);
    }

    #[test]
    fn conflicts_that_never_stop_give_the_edits_up() {
        let mut m = ready(one_space(), 1);
        m.edit(rename("c1", "mine")).unwrap();
        for round in 1..=MAX_REPLAYS {
            let _ = write(&mut m);
            assert!(
                m.finish_write(Err(conflict())).dropped.is_empty(),
                "round {round}"
            );
            m.next_round();
            m.finish_read(read(one_space(), 1 + u64::from(round)));
        }
        let _ = write(&mut m);
        let report = m.finish_write(Err(conflict()));
        assert_eq!(report.dropped, [rename("c1", "mine")]);
        assert!(report.notice.is_some());
        assert!(m.pending().is_empty());
    }

    /// `MAX_REPLAYS` conflicts, each ending with nothing left to write because the other screen
    /// made the same edit; then a fresh edit's first conflict must not give it up.
    fn conflicts_then_a_fresh_edit(same_edit: impl Fn(&mut Mirror, &mut Value, u32)) {
        let mut m = ready(one_space(), 1);
        let mut version = 1;
        for episode in 0..MAX_REPLAYS {
            let mut fresh = one_space();
            same_edit(&mut m, &mut fresh, episode);
            let _ = write(&mut m);
            assert_eq!(m.finish_write(Err(conflict())), Report::default());
            m.next_round();
            version += 1;
            m.finish_read(read(fresh, version));
            assert_eq!(m.next_round(), None);
        }
        m.edit(rename("c1", "later")).unwrap();
        let _ = write(&mut m);
        assert_eq!(m.finish_write(Err(conflict())), Report::default());
        assert_eq!(m.pending(), [rename("c1", "later")]);
    }

    #[test]
    fn conflicts_count_again_after_an_edit_dropped_on_replay() {
        conflicts_then_a_fresh_edit(|m, fresh, n| {
            let address = format!("t{}", n + 2);
            m.edit(split("x1", &format!("x{}", n + 2), &address))
                .unwrap();
            fresh["spaces"][0]["tabs"][0]["root"]["children"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id": "y", "type": "block", "content": {"terminal": address}}));
        });
    }

    #[test]
    fn conflicts_count_again_after_an_edit_that_cancelled_out() {
        conflicts_then_a_fresh_edit(|m, fresh, n| {
            let name = format!("mine {n}");
            m.edit(rename("c1", &name)).unwrap();
            fresh["spaces"][0]["tabs"][0]["name"] = json!(name);
        });
    }

    #[test]
    fn a_swap_lost_to_a_reconnect_is_not_swapped_back() {
        let mut fresh = one_space();
        fresh["spaces"][0]["tabs"][0]["root"]["children"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": "x2", "type": "block", "content": {"terminal": "t2"}}));
        let mut m = ready(fresh, 1);
        let swap = Intent::swap_panes(&m.shown().unwrap(), "x1", "x2").unwrap();
        m.edit(swap).unwrap();
        let set = write(&mut m);
        m.reconnected();
        m.next_round();
        m.finish_read(read(set.value, 2));
        assert_eq!(m.shown().unwrap().addresses(), ["t2", "t1"]);
        assert_eq!(m.next_round(), None);
    }

    #[test]
    fn a_newer_doc_event_a_missed_or_a_reconnect_reads_again() {
        let mut m = ready(one_space(), 4);
        m.note_doc(4);
        assert_eq!(m.next_round(), None, "not newer");
        m.note_doc(5);
        assert!(matches!(m.next_round(), Some(Round::Read(_))));
        m.finish_read(read(one_space(), 5));
        m.note_missed();
        assert!(matches!(m.next_round(), Some(Round::Read(_))));
        m.finish_read(read(one_space(), 5));
        m.reconnected();
        assert!(matches!(m.next_round(), Some(Round::Read(_))));
    }

    #[test]
    fn a_write_lost_to_a_reconnect_is_replayed_harmlessly() {
        let mut m = ready(one_space(), 1);
        m.edit(split("x1", "x2", "t2")).unwrap();
        let set = write(&mut m);
        // The write landed, but its reply was lost with the connection.
        m.reconnected();
        assert_eq!(m.pending().len(), 1);
        assert!(matches!(m.next_round(), Some(Round::Read(_))));
        m.finish_read(read(set.value, 2));
        assert_eq!(m.shown().unwrap().addresses(), ["t1", "t2"]);
        assert_eq!(m.next_round(), None, "already there: nothing to write");
        // A reply from the old connection changes nothing.
        assert_eq!(m.finish_write(written(9)), Report::default());
        assert_eq!(m.version(), 2);
    }

    #[test]
    fn edits_that_change_nothing_are_not_written() {
        let mut m = ready(one_space(), 1);
        m.edit(rename("c1", "x")).unwrap();
        m.edit(rename("c1", "")).unwrap();
        assert_eq!(m.next_round(), None);
        assert!(m.pending().is_empty());
        assert_eq!(
            m.edit(Intent::RemovePane {
                pane: "gone".into()
            }),
            Err(EditError::NoEffect)
        );
    }

    #[test]
    fn login_and_update_errors_wait_for_news() {
        let mut m = Mirror::new(Some("devbox".into()));
        m.next_round();
        let mut e = error(ErrorKind::NeedsLogin, "cannot use machine");
        e.hint = Some("run hc machine login devbox".into());
        let report = m.finish_read(Err(e.clone()));
        assert_eq!(
            report.notice.as_deref(),
            Some("cannot use machine: run hc machine login devbox")
        );
        assert!(matches!(m.layout(), Layout::Blocked(_)));
        assert_eq!(m.next_round(), None, "no retry on a timer");
        m.note_doc(1);
        assert!(matches!(m.next_round(), Some(Round::Read(_))));
        assert_eq!(m.finish_read(Err(e)).notice, None, "said once");
        m.reconnected();
        assert!(matches!(m.next_round(), Some(Round::Read(_))));
        m.finish_read(read(one_space(), 1));
        assert_eq!(m.layout(), &Layout::Ready);
    }

    #[test]
    fn other_errors_read_again_and_keep_what_is_shown() {
        let mut m = ready(one_space(), 1);
        m.edit(rename("c1", "x")).unwrap();
        let _ = write(&mut m);
        let report = m.finish_write(Err(error(ErrorKind::Unavailable, "machine is connecting")));
        assert_eq!(report, Report::default());
        assert_eq!(
            m.layout(),
            &Layout::Unavailable("machine is connecting".into())
        );
        assert_eq!(m.shown().unwrap().spaces[0].tabs[0].name, "x");
        assert_eq!(m.edit(rename("c1", "y")), Err(EditError::Unavailable));
        assert!(matches!(m.next_round(), Some(Round::Read(_))));
        m.finish_read(read(one_space(), 1));
        let set = write(&mut m);
        assert_eq!(set.value["spaces"][0]["tabs"][0]["name"], "x");
    }

    #[test]
    fn an_undecodable_document_is_never_written() {
        let mut m = ready(one_space(), 1);
        m.note_doc(2);
        m.next_round();
        let report = m.finish_read(read(
            json!({"spaces": [{"id": "s", "tabs": [{"id": "t", "root": {"id": "t"}}]}]}),
            2,
        ));
        assert!(report.notice.unwrap().contains("cannot read"));
        assert!(matches!(m.layout(), Layout::Undecodable(_)));
        assert_eq!(m.edit(rename("c1", "x")), Err(EditError::Unavailable));
        assert_eq!(m.next_round(), None);
        m.note_doc(3);
        assert!(matches!(m.next_round(), Some(Round::Read(_))));
        m.finish_read(read(one_space(), 3));
        assert_eq!(m.layout(), &Layout::Ready);
    }

    #[test]
    fn a_remote_machine_is_shown_qualified_and_written_bare() {
        let mut m = Mirror::new(Some("devbox".into()));
        let Some(Round::Read(get)) = m.next_round() else {
            panic!()
        };
        assert_eq!(get.on.as_deref(), Some("devbox"));
        m.finish_read(read(one_space(), 1));
        let shown = m.shown().unwrap();
        assert_eq!(shown.addresses(), ["devbox/t1"]);
        m.edit(split("devbox/x1", "devbox/x2", "devbox/t2"))
            .unwrap();
        let set = write(&mut m);
        assert_eq!(set.on.as_deref(), Some("devbox"));
        assert_eq!(
            Document::from_value(set.value).unwrap().addresses(),
            ["t1", "t2"]
        );
    }

    #[test]
    fn unknown_fields_survive_an_edit_and_a_write() {
        let mut raw = one_space();
        raw["later"] = json!(1);
        raw["spaces"][0]["color"] = json!("teal");
        raw["spaces"][0]["tabs"][0]["root"]["children"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": "w", "type": "web", "url": "x"}));
        let mut m = ready(raw.clone(), 1);
        m.edit(rename("c1", "x")).unwrap();
        let set = write(&mut m);
        let mut expected = raw;
        expected["spaces"][0]["tabs"][0]["name"] = json!("x");
        assert_eq!(set.value, expected);
    }
}
