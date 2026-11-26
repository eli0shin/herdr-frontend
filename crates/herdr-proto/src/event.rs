//! Event-stream notifications (`herdr-core-protocol.md` §2 "Events") and the line splitter.
//!
//! In framed mode the events channel carries newline-delimited JSON-RPC notifications, and DATA
//! frame boundaries can fall anywhere inside a line.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{Extra, Foreground, Labels, Reason, Status, StatusState, Terminal};
use crate::words::string_enum;

string_enum! {
    /// An event's `type`.
    pub enum EventKind {
        Created = "created",
        Lost = "lost",
        Restarted = "restarted",
        Keep = "keep",
        Finished = "finished",
        Removed = "removed",
        Status = "status",
        Turn = "turn",
        Title = "title",
        Labels = "labels",
        Size = "size",
        Foreground = "foreground",
        Doc = "doc",
    }
}

/// One notification on an events stream.
#[derive(Clone, Debug, PartialEq)]
pub enum Notification {
    Event(Event),
    /// Every 15 s on a quiet stream, with the current `seq`.
    Heartbeat {
        seq: u64,
    },
    /// The subscriber fell behind: a fresh snapshot replaces everything. Read the documents again.
    Missed(Snapshot),
    /// A method this crate doesn't know.
    Other {
        method: String,
        params: Value,
    },
}

/// The terminals at a `seq`: the subscribe reply, or a `missed` notification.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    pub seq: u64,
    pub terminals: Vec<Terminal>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `event {seq, type, terminal?, machine?, data}`.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// Rises by one per event and restarts with each core process. Filtered subscribers see gaps.
    pub seq: u64,
    /// The terminal's address; `None` for `doc`.
    pub terminal: Option<String>,
    /// For a `doc` event from a machine this core added.
    pub machine: Option<String>,
    pub data: EventData,
}

/// An event's typed `data`.
#[derive(Clone, Debug, PartialEq)]
pub enum EventData {
    /// The whole record: add it, or replace it (a machine reconnected).
    Created(Box<Terminal>),
    /// A kept program was lost. The terminal stays live; its attach stream has closed.
    Lost(Ended),
    /// The whole record: replace it and reattach.
    Restarted(Box<Terminal>),
    Keep(KeepData),
    Finished(Ended),
    Removed,
    Status(Box<StatusData>),
    /// A turn ended. Seen and done decisions happen here.
    Turn(TurnData),
    Title(TitleData),
    Labels(LabelsData),
    /// The terminal's grid changed: the only size news a non-observe viewer gets.
    Size(SizeData),
    /// What runs in it now; `None` when nothing does.
    Foreground(Option<Box<Foreground>>),
    /// A document changed: read it when the version is newer than yours.
    Doc(DocData),
    /// A type this crate doesn't know.
    Other {
        kind: String,
        data: Value,
    },
}

/// `lost` and `finished`: `{reason, exit_code, signal, finished_ms}`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ended {
    pub reason: Option<Reason>,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub finished_ms: Option<u64>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeepData {
    pub keep: bool,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `{status, records, reported}`: replace all three.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusData {
    pub status: Option<Status>,
    pub records: std::collections::BTreeMap<String, Status>,
    pub reported: bool,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `{turn, state}`: the turn counter after a turn ended, and the word it ended on.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TurnData {
    pub turn: u64,
    pub state: Option<StatusState>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TitleData {
    pub title: String,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `{labels, record?}`: all labels now; `record` when the terminal newly matches a label filter.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LabelsData {
    pub labels: Labels,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<Box<Terminal>>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SizeData {
    pub cols: u16,
    pub rows: u16,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DocData {
    pub name: String,
    pub version: u64,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Event {
    pub fn kind(&self) -> EventKind {
        match &self.data {
            EventData::Created(_) => EventKind::Created,
            EventData::Lost(_) => EventKind::Lost,
            EventData::Restarted(_) => EventKind::Restarted,
            EventData::Keep(_) => EventKind::Keep,
            EventData::Finished(_) => EventKind::Finished,
            EventData::Removed => EventKind::Removed,
            EventData::Status(_) => EventKind::Status,
            EventData::Turn(_) => EventKind::Turn,
            EventData::Title(_) => EventKind::Title,
            EventData::Labels(_) => EventKind::Labels,
            EventData::Size(_) => EventKind::Size,
            EventData::Foreground(_) => EventKind::Foreground,
            EventData::Doc(_) => EventKind::Doc,
            EventData::Other { kind, .. } => EventKind::from(kind.as_str()),
        }
    }

    /// The `data` as JSON again.
    pub fn data_value(&self) -> Value {
        let value = match &self.data {
            EventData::Created(t) | EventData::Restarted(t) => serde_json::to_value(t),
            EventData::Lost(d) | EventData::Finished(d) => serde_json::to_value(d),
            EventData::Keep(d) => serde_json::to_value(d),
            EventData::Removed => Ok(Value::Object(Default::default())),
            EventData::Status(d) => serde_json::to_value(d),
            EventData::Turn(d) => serde_json::to_value(d),
            EventData::Title(d) => serde_json::to_value(d),
            EventData::Labels(d) => serde_json::to_value(d),
            EventData::Size(d) => serde_json::to_value(d),
            EventData::Foreground(d) => serde_json::to_value(d),
            EventData::Doc(d) => serde_json::to_value(d),
            EventData::Other { data, .. } => Ok(data.clone()),
        };
        value.expect("event data always serializes")
    }
}

#[derive(Deserialize)]
struct RawMessage {
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Deserialize)]
struct RawEvent {
    seq: u64,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    terminal: Option<String>,
    #[serde(default)]
    machine: Option<String>,
    #[serde(default)]
    data: Value,
}

#[derive(Deserialize)]
struct RawHeartbeat {
    seq: u64,
}

impl Notification {
    /// Parses one line (without its `\n`).
    pub fn parse(line: &[u8]) -> Result<Notification, serde_json::Error> {
        let raw: RawMessage = serde_json::from_slice(line)?;
        Notification::from_parts(raw.method, raw.params)
    }

    pub(crate) fn from_parts(
        method: String,
        params: Value,
    ) -> Result<Notification, serde_json::Error> {
        Ok(match method.as_str() {
            "event" => Notification::Event(Event::from_raw(serde_json::from_value(params)?)?),
            "heartbeat" => Notification::Heartbeat {
                seq: serde_json::from_value::<RawHeartbeat>(params)?.seq,
            },
            "missed" => Notification::Missed(serde_json::from_value(params)?),
            _ => Notification::Other { method, params },
        })
    }
}

impl Event {
    fn from_raw(raw: RawEvent) -> Result<Event, serde_json::Error> {
        let data = raw.data;
        let data = match EventKind::from(raw.kind.as_str()) {
            EventKind::Created => EventData::Created(serde_json::from_value(data)?),
            EventKind::Lost => EventData::Lost(serde_json::from_value(data)?),
            EventKind::Restarted => EventData::Restarted(serde_json::from_value(data)?),
            EventKind::Keep => EventData::Keep(serde_json::from_value(data)?),
            EventKind::Finished => EventData::Finished(serde_json::from_value(data)?),
            EventKind::Removed => EventData::Removed,
            EventKind::Status => EventData::Status(serde_json::from_value(data)?),
            EventKind::Turn => EventData::Turn(serde_json::from_value(data)?),
            EventKind::Title => EventData::Title(serde_json::from_value(data)?),
            EventKind::Labels => EventData::Labels(serde_json::from_value(data)?),
            EventKind::Size => EventData::Size(serde_json::from_value(data)?),
            EventKind::Foreground => EventData::Foreground(serde_json::from_value(data)?),
            EventKind::Doc => EventData::Doc(serde_json::from_value(data)?),
            EventKind::Other(kind) => EventData::Other { kind, data },
        };
        Ok(Event {
            seq: raw.seq,
            terminal: raw.terminal,
            machine: raw.machine,
            data,
        })
    }
}

/// Splits a byte stream into lines, however the frames split it.
#[derive(Debug, Default)]
pub struct LineSplitter {
    buf: Vec<u8>,
    /// Where the next unreturned byte is in `buf`.
    start: usize,
    /// How far `buf[start..]` is known to hold no `\n`.
    scanned: usize,
}

impl LineSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        if self.start > 0 && self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
            self.scanned = 0;
        } else if self.start > self.buf.len() / 2 {
            self.buf.drain(..self.start);
            self.scanned -= self.start;
            self.start = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole line, without its `\n` (or `\r\n`).
    pub fn next_line(&mut self) -> Option<&[u8]> {
        let from = self.scanned.max(self.start);
        match self.buf[from..].iter().position(|b| *b == b'\n') {
            Some(at) => {
                let end = from + at;
                let line_start = self.start;
                self.start = end + 1;
                self.scanned = self.start;
                let line = &self.buf[line_start..end];
                Some(line.strip_suffix(b"\r").unwrap_or(line))
            }
            None => {
                self.scanned = self.buf.len();
                None
            }
        }
    }

    /// Bytes of an unfinished line.
    pub fn buffered(&self) -> usize {
        self.buf.len() - self.start
    }

    /// What follows the lines read so far. After the plain-mode `hello` reply, these bytes are
    /// already framed.
    pub fn into_rest(mut self) -> Vec<u8> {
        self.buf.drain(..self.start);
        self.buf
    }
}

/// An events stream: lines in, notifications out.
#[derive(Debug, Default)]
pub struct EventLines {
    lines: LineSplitter,
}

impl EventLines {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one DATA frame's payload.
    pub fn push(&mut self, bytes: &[u8]) {
        self.lines.push(bytes);
    }

    /// The next notification. Blank lines are skipped; a line that doesn't parse is an error for
    /// that line only, and the stream goes on.
    pub fn next_notification(&mut self) -> Option<Result<Notification, serde_json::Error>> {
        loop {
            let line = self.lines.next_line()?;
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            return Some(Notification::parse(line));
        }
    }

    /// Bytes of an unfinished line.
    pub fn buffered(&self) -> usize {
        self.lines.buffered()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lines(splitter: &mut LineSplitter) -> Vec<Vec<u8>> {
        std::iter::from_fn(|| splitter.next_line().map(<[u8]>::to_vec)).collect()
    }

    #[test]
    fn lines_survive_every_split() {
        let text = b"one\ntwo\r\n\nthree\n";
        for split in 0..=text.len() {
            let mut splitter = LineSplitter::new();
            splitter.push(&text[..split]);
            let mut got = lines(&mut splitter);
            splitter.push(&text[split..]);
            got.extend(lines(&mut splitter));
            assert_eq!(
                got,
                [&b"one"[..], b"two", b"", b"three"],
                "split at {split}"
            );
            assert_eq!(splitter.buffered(), 0);
        }
    }

    #[test]
    fn a_partial_line_waits() {
        let mut splitter = LineSplitter::new();
        splitter.push(b"{\"method\":");
        assert_eq!(splitter.next_line(), None);
        assert_eq!(splitter.buffered(), 10);
        splitter.push(b"\"heartbeat\"}\nrest");
        assert_eq!(
            splitter.next_line(),
            Some(&b"{\"method\":\"heartbeat\"}"[..])
        );
        assert_eq!(splitter.next_line(), None);
        assert_eq!(splitter.into_rest(), b"rest");
    }

    #[test]
    fn the_rest_after_a_hello_line_is_kept() {
        let mut splitter = LineSplitter::new();
        splitter.push(b"{\"id\":0}\n\x00\x00\x00\x00\x00\x02\x00\x00\x00{}");
        assert_eq!(splitter.next_line(), Some(&b"{\"id\":0}"[..]));
        assert_eq!(
            splitter.into_rest(),
            b"\x00\x00\x00\x00\x00\x02\x00\x00\x00{}"
        );
    }

    #[test]
    fn notifications_parse() {
        let heartbeat =
            Notification::parse(br#"{"jsonrpc":"2.0","method":"heartbeat","params":{"seq":42}}"#)
                .unwrap();
        assert_eq!(heartbeat, Notification::Heartbeat { seq: 42 });

        let missed = Notification::parse(
            br#"{"jsonrpc":"2.0","method":"missed","params":{"seq":57,"terminals":[{"address":"t1"}]}}"#,
        )
        .unwrap();
        let Notification::Missed(snapshot) = missed else {
            panic!("{missed:?}")
        };
        assert_eq!(snapshot.seq, 57);
        assert_eq!(snapshot.terminals[0].address, "t1");

        let other =
            Notification::parse(br#"{"jsonrpc":"2.0","method":"brand_new","params":{"x":1}}"#)
                .unwrap();
        assert_eq!(
            other,
            Notification::Other {
                method: "brand_new".into(),
                params: json!({"x": 1})
            }
        );
    }

    #[test]
    fn events_parse_by_type() {
        let parse = |params: Value| {
            let line =
                serde_json::to_vec(&json!({"jsonrpc": "2.0", "method": "event", "params": params}))
                    .unwrap();
            match Notification::parse(&line).unwrap() {
                Notification::Event(event) => event,
                other => panic!("{other:?}"),
            }
        };
        let doc = parse(
            json!({"seq": 3, "type": "doc", "machine": "devbox", "data": {"name": "herdr-app", "version": 9}}),
        );
        assert_eq!(doc.kind(), EventKind::Doc);
        assert_eq!(doc.machine.as_deref(), Some("devbox"));
        assert_eq!(doc.terminal, None);
        assert_eq!(
            doc.data,
            EventData::Doc(DocData {
                name: "herdr-app".into(),
                version: 9,
                ..Default::default()
            })
        );

        let gone = parse(json!({"seq": 4, "type": "foreground", "terminal": "t1", "data": null}));
        assert_eq!(gone.data, EventData::Foreground(None));

        let removed = parse(json!({"seq": 5, "type": "removed", "terminal": "t1", "data": {}}));
        assert_eq!(removed.data, EventData::Removed);
        assert_eq!(removed.data_value(), json!({}));

        let newer =
            parse(json!({"seq": 6, "type": "brand_new", "terminal": "t1", "data": {"x": 1}}));
        assert_eq!(newer.kind(), EventKind::Other("brand_new".into()));
        assert_eq!(newer.data_value(), json!({"x": 1}));

        let turn = parse(
            json!({"seq": 7, "type": "turn", "terminal": "t1", "data": {"turn": 4, "state": "done"}}),
        );
        assert_eq!(
            turn.data,
            EventData::Turn(TurnData {
                turn: 4,
                state: Some(StatusState::Done),
                ..Default::default()
            })
        );
    }

    #[test]
    fn malformed_lines_are_errors_and_the_stream_goes_on() {
        let mut events = EventLines::new();
        events.push(b"not json\n\n{\"method\":\"event\",\"params\":{\"type\":\"doc\"}}\n");
        events.push(b"{\"method\":\"heartbeat\",\"params\":{\"seq\":1}}\n");
        assert!(events.next_notification().unwrap().is_err(), "not json");
        assert!(
            events.next_notification().unwrap().is_err(),
            "an event without seq"
        );
        assert_eq!(
            events.next_notification().unwrap().unwrap(),
            Notification::Heartbeat { seq: 1 }
        );
        assert!(events.next_notification().is_none());
        assert_eq!(events.buffered(), 0);
    }

    #[test]
    fn a_known_type_with_the_wrong_data_is_an_error() {
        let line = br#"{"method":"event","params":{"seq":1,"type":"size","terminal":"t1","data":{"cols":"wide"}}}"#;
        assert!(Notification::parse(line).is_err());
    }
}
