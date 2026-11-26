//! Records the core sends in several places: the Terminal record, its status, machines.
//!
//! Every struct keeps the fields it doesn't know in `extra`, so a newer core's additions survive.
//! Optional fields the core always writes (as `null` when unset) serialize as `null`; those it
//! leaves out when unset are left out. A record decoded and encoded again equals the core's.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::words::string_enum;

/// Unknown fields, kept as sent.
pub type Extra = Map<String, Value>;
/// A terminal's labels: free-form `key=value`, all of them in each `labels` event.
pub type Labels = BTreeMap<String, String>;

pub(crate) fn is_false(b: &bool) -> bool {
    !*b
}

string_enum! {
    /// `Terminal.state`.
    #[derive(Default)]
    pub enum TerminalState {
        #[default]
        Live = "live",
        Finished = "finished",
    }
}

string_enum! {
    /// Why a program ended or is pending: `Terminal.reason`, `lost`/`finished` events and CLOSE.
    pub enum Reason {
        Exited = "exited",
        Killed = "killed",
        /// The terminal finished while an attach stream was open on it.
        Finished = "finished",
        ServerStop = "server_stop",
        HostShutdown = "host_shutdown",
        /// A kept program died by a signal without an exit code; a new shell follows.
        Signaled = "signaled",
        HolderLost = "holder_lost",
        StoppedByRestart = "stopped_by_restart",
        /// The initial spawn timed out; input and attach stay `unavailable` until it resolves.
        SpawnPending = "spawn_pending",
    }
}

string_enum! {
    /// `Status.state`: the program's own word (OSC 7501).
    #[derive(Default)]
    pub enum StatusState {
        #[default]
        Idle = "idle",
        Working = "working",
        Done = "done",
        Blocked = "blocked",
        Error = "error",
    }
}

string_enum! {
    /// `Status.kind`: why a program is blocked.
    pub enum BlockedKind {
        Permission = "permission",
        Question = "question",
        Auth = "auth",
    }
}

string_enum! {
    /// `MachineView.state`: a machine's line, from polling `machine.list`.
    #[derive(Default)]
    pub enum MachineState {
        #[default]
        Connecting = "connecting",
        Connected = "connected",
        NeedsLogin = "needs_login",
        NeedsUpdate = "needs_update",
        Unreachable = "unreachable",
    }
}

/// One OSC 7501 record, stored verbatim by the core.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub state: StatusState,
    /// Only when blocked, and only when the program said.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<BlockedKind>,
    /// 0..100, when the program said.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub app: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub msg: String,
    #[serde(flatten)]
    pub extra: Extra,
}

/// What runs in a terminal now: the foreground process group's leader and its folder.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Foreground {
    pub pid: i32,
    pub name: String,
    pub argv: Vec<String>,
    pub cwd: String,
    #[serde(flatten)]
    pub extra: Extra,
}

/// A template's first message: whether it was sent, and why not.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FirstMessage {
    pub sent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn: Option<u64>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// The Terminal record (`core:schema/hc.openrpc.json` `Terminal`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Terminal {
    /// `t7`, or `machine/t7` for a machine this core added.
    pub address: String,
    pub labels: Labels,
    pub argv: Vec<String>,
    /// The launch folder; `foreground.cwd` is the live one.
    pub cwd: String,
    pub state: TerminalState,
    pub created_ms: u64,
    /// `exact`, or `inexact` after a core crash.
    pub screen: String,
    pub cols: u16,
    pub rows: u16,
    pub name: Option<String>,
    pub pid: Option<u32>,
    /// The terminal that started it, set once. On remote records the machine prefix may be a
    /// host name or core id (`herdr-core-protocol.md` §5).
    pub parent: Option<String>,
    pub keep: bool,
    pub rm: bool,
    pub notify: bool,
    pub die_with_parent: bool,
    pub template: Option<String>,
    pub identity: String,
    pub title: String,
    /// The root status, `None` when the program never said or it was cleared.
    pub status: Option<Status>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub records: BTreeMap<String, Status>,
    pub reported: bool,
    pub status_ms: Option<u64>,
    /// Goes up each time the root status leaves `working`.
    pub turn: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreground: Option<Foreground>,
    pub reason: Option<Reason>,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub finished_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_message: Option<FirstMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paste_gap_ms: Option<u64>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Terminal {
    pub fn is_live(&self) -> bool {
        self.state == TerminalState::Live
    }

    pub fn label(&self, key: &str) -> Option<&str> {
        self.labels.get(key).map(String::as_str)
    }
}

/// A viewer's colors and cell size: attach `display`, `terminal.run` `display`, and FACTS.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// Cell width and height in pixels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell: Option<[u32; 2]>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `{id, name}` of a core.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CoreInfo {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub extra: Extra,
}

/// A machine this core added (`machine.add`, `machine.rename`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Machine {
    pub name: String,
    pub id: String,
    pub host: String,
    pub target: String,
    /// Its hc version when last seen.
    pub version: String,
    pub session: Option<String>,
    /// An agent's sandbox: herdr-app hides it and its spaces.
    pub temporary: bool,
    pub via: bool,
    pub keeps_running_after_logout: Option<bool>,
    pub logout_linger_user: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// A machine and its line state (`machine.list`, `machine.connect`, `terminal.list`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MachineView {
    pub state: MachineState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    #[serde(flatten)]
    pub machine: Machine,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_minimal_terminal_decodes_with_defaults() {
        let t: Terminal = serde_json::from_value(json!({"address": "t1"})).unwrap();
        assert_eq!(t.address, "t1");
        assert!(t.is_live());
        assert_eq!(t.status, None);
        assert!(t.extra.is_empty());
    }

    #[test]
    fn unknown_terminal_fields_and_words_survive() {
        let raw = json!({
            "address": "t1", "labels": {"herdr.seen": "3"}, "argv": ["sh"], "cwd": "/", "state": "parked",
            "created_ms": 1, "screen": "exact", "cols": 80, "rows": 24, "name": null, "pid": null,
            "parent": null, "keep": false, "rm": false, "notify": false, "die_with_parent": false,
            "template": null, "identity": "", "title": "", "reported": true, "status_ms": null, "turn": 3,
            "status": {"state": "thinking", "kind": "consent", "later": [1]},
            "reason": "vanished", "exit_code": null, "signal": null, "finished_ms": null,
            "brand_new": {"x": 1}
        });
        let t: Terminal = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(t.state, TerminalState::Other("parked".into()));
        assert!(!t.is_live());
        assert_eq!(t.label("herdr.seen"), Some("3"));
        let status = t.status.as_ref().unwrap();
        assert_eq!(status.state, StatusState::Other("thinking".into()));
        assert_eq!(status.kind, Some(BlockedKind::Other("consent".into())));
        assert_eq!(status.extra["later"], json!([1]));
        assert_eq!(t.reason, Some(Reason::Other("vanished".into())));
        assert_eq!(t.extra["brand_new"], json!({"x": 1}));
        assert_eq!(serde_json::to_value(&t).unwrap(), raw);
    }

    #[test]
    fn machine_views_flatten_the_machine() {
        let raw = json!({
            "host": "h", "id": "i", "name": "devbox", "target": "devbox", "version": "0.1.22",
            "session": null, "temporary": true, "via": false, "keeps_running_after_logout": null,
            "logout_linger_user": null, "state": "needs_login", "since_ms": 5, "why": "login", "later": 1
        });
        let m: MachineView = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(m.state, MachineState::NeedsLogin);
        assert_eq!(m.machine.name, "devbox");
        assert!(m.machine.temporary);
        assert_eq!(m.machine.extra["later"], 1);
        assert_eq!(serde_json::to_value(&m).unwrap(), raw);
    }
}
