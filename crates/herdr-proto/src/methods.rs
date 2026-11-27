//! Typed params and results for the methods a desktop client uses (`herdr-core-protocol.md` §2).
//!
//! Each params struct is named after its method and implements [`Call`]. Optional params left at
//! their default are not sent. `extra` sends params this crate doesn't know yet; results keep
//! fields it doesn't know in `extra`.
//!
//! Left out on purpose: `server.stop` and `server.restart` (never sent to a person's core), and
//! `terminal.put_begin/chunk/finish/abort` (core-to-core transfers).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::rpc::Call;
use crate::types::{CoreInfo, Display, Extra, Labels, Machine, MachineView, Terminal, is_false};

fn is_zero(n: &u64) -> bool {
    *n == 0
}

macro_rules! call {
    ($params:ty, $method:literal, $result:ty) => {
        impl Call for $params {
            const METHOD: &'static str = $method;
            type Result = $result;
        }
    };
}

// Session.

/// `server.status`: version, `installed_by` and holders.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerStatus {
    #[serde(flatten)]
    pub extra: Extra,
}
call!(ServerStatus, "server.status", ServerStatusResult);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerStatusResult {
    pub version: String,
    pub core: CoreInfo,
    pub pid: u32,
    pub uptime_ms: u64,
    pub live: u64,
    pub finished: u64,
    pub holders: Vec<HolderInfo>,
    pub holder: Option<HolderInfo>,
    /// Which updater owns hc.
    pub installed_by: Option<String>,
    pub keeps_running_after_logout: Option<bool>,
    pub logout_linger_user: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HolderInfo {
    pub instance: String,
    pub pid: u32,
    pub current: bool,
    pub terminals: u64,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `rpc.discover`: the OpenRPC schema, for diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RpcDiscover {
    #[serde(flatten)]
    pub extra: Extra,
}
call!(RpcDiscover, "rpc.discover", Value);

// Terminals.

/// A terminal's initial grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitialSize {
    pub cols: u16,
    pub rows: u16,
}

/// `terminal.run`: start a program. herdr-app sends `env`, `keep: true`, the `herdr.*` labels,
/// `size` and `display`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalRun {
    /// Program and arguments, or a template name and its task.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub argv: Vec<String>,
    /// A shell line instead of `argv`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: Labels,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Another machine; the address is then `machine/tN`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub keep: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<InitialSize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<Display>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub no_parent: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub no_notify: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub outlive_parent: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub rm: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub wait: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    /// Don't pass from a UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(TerminalRun, "terminal.run", RunResult);

/// The Terminal record, plus `output` and `wait` with `wait: true`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<WaitResult>,
    #[serde(flatten)]
    pub terminal: Terminal,
}

/// `templates.list`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplatesList {
    #[serde(flatten)]
    pub extra: Extra,
}
call!(TemplatesList, "templates.list", TemplatesResult);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplatesResult {
    pub templates: Vec<TemplateInfo>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateInfo {
    pub name: String,
    pub description: String,
    pub command: String,
    /// Set when the template file can't be used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `terminal.list`: a one-shot list. Not needed once subscribed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalList {
    /// Include finished terminals.
    #[serde(skip_serializing_if = "is_false")]
    pub all: bool,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: Labels,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(TerminalList, "terminal.list", ListResult);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListResult {
    pub terminals: Vec<Terminal>,
    pub machines: Vec<MachineView>,
    pub hidden_finished: u64,
    #[serde(flatten)]
    pub extra: Extra,
}

/// Defines a `{terminal, caller?}` params struct.
macro_rules! terminal_ref {
    ($(#[$meta:meta])* $name:ident, $method:literal, $result:ty) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
        #[serde(default)]
        pub struct $name {
            /// `t7`, a name, or `machine/t7`.
            pub terminal: String,
            /// Don't pass from a UI: it marks a child's news as seen by that parent.
            #[serde(skip_serializing_if = "Option::is_none")]
            pub caller: Option<String>,
            #[serde(flatten)]
            pub extra: Extra,
        }
        call!($name, $method, $result);
    };
}

terminal_ref!(
    /// `terminal.get`: one record, live or finished.
    TerminalGet, "terminal.get", Terminal
);
terminal_ref!(
    /// `terminal.kill`: close a pane's program.
    TerminalKill, "terminal.kill", Terminal
);
terminal_ref!(
    /// `terminal.rm`: clear a finished record.
    TerminalRm, "terminal.rm", RemoveResult
);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoveResult {
    pub removed: String,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `terminal.keep`: replace a lost program with a shell in place.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalKeep {
    pub terminal: String,
    pub keep: bool,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(TerminalKeep, "terminal.keep", Terminal);

/// `terminal.label`: set `k=v`; `k=""` removes `k`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalLabel {
    pub terminal: String,
    pub labels: Labels,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(TerminalLabel, "terminal.label", Terminal);

/// `terminal.read`: the screen plus history as text, without attaching.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalRead {
    pub terminal: String,
    /// Lines of history above the screen.
    #[serde(skip_serializing_if = "is_zero")]
    pub lines: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(TerminalRead, "terminal.read", ReadResult);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadResult {
    pub text: String,
    pub complete: bool,
    pub how: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `terminal.attach` (framed mode only). See §4 for when to pass `resumed` or `observe`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalAttach {
    pub terminal: String,
    pub cols: u16,
    pub rows: u16,
    /// Scrollback lines in the first paint; herdr-app asks for 2000.
    #[serde(skip_serializing_if = "is_zero")]
    pub history: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<Display>,
    /// Watch only: never type or resize.
    #[serde(skip_serializing_if = "is_false")]
    pub observe: bool,
    /// An observer that understands core-to-viewer RESIZE.
    #[serde(skip_serializing_if = "is_false")]
    pub resize_notifications: bool,
    /// Take the size only once active.
    #[serde(skip_serializing_if = "is_false")]
    pub resumed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(TerminalAttach, "terminal.attach", AttachResult);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AttachResult {
    /// The stream's channel. Register its route before reading further frames.
    pub channel: u32,
    pub terminal: Terminal,
    /// Confirmed by every hop: see [`crate::Capabilities::stream`].
    pub resize_notifications: bool,
    pub automatic_input: bool,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `terminal.put`: a file lands beside the terminal on its machine; it types nothing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalPut {
    pub terminal: String,
    pub name: String,
    /// The bytes, base64; at most 8 MB raw.
    pub data: String,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(TerminalPut, "terminal.put", PutResult);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PutResult {
    pub path: String,
    pub bytes: u64,
    #[serde(flatten)]
    pub extra: Extra,
}

// Agents.

/// `terminal.send {if}`: type only when the status is exactly this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdleOrWorking {
    Idle,
    Working,
}

/// `terminal.send`: agent-to-agent messaging. Not for human typing, which is DATA.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalSend {
    pub terminal: String,
    pub text: String,
    #[serde(skip_serializing_if = "is_false")]
    pub force: bool,
    #[serde(rename = "if", skip_serializing_if = "Option::is_none")]
    pub only_if: Option<IdleOrWorking>,
    #[serde(skip_serializing_if = "is_false")]
    pub wait: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(TerminalSend, "terminal.send", SendResult);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SendResult {
    pub turn: u64,
    pub started: Option<bool>,
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait: Option<WaitResult>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `terminal.keys`: named keys (`enter`, `ctrl+c`, …).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalKeys {
    pub terminal: String,
    pub keys: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(TerminalKeys, "terminal.keys", KeysResult);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeysResult {
    pub turn: u64,
    pub status: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `terminal.interrupt`: the stop key. A "stop agent" button.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalInterrupt {
    pub terminal: String,
    /// Seconds to watch for an effect (default 10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(TerminalInterrupt, "terminal.interrupt", InterruptResult);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InterruptResult {
    pub key: String,
    pub outcome: String,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `terminal.wait {children}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Children {
    Any,
    All,
}

/// `terminal.wait`: for agents and scripts. A UI watches events instead.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalWait {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Children>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(TerminalWait, "terminal.wait", WaitResult);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WaitResult {
    pub terminal: String,
    pub ended: bool,
    pub reason: String,
    pub status: Option<String>,
    pub turn: u64,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub children: Vec<WaitResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

// Machines.

/// `machine.list`: line state, by polling.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MachineList {
    #[serde(flatten)]
    pub extra: Extra,
}
call!(MachineList, "machine.list", MachineListResult);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MachineListResult {
    pub machines: Vec<MachineView>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `machine.add`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MachineAdd {
    /// How ssh reaches it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// A command to reach it instead of ssh.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub temporary: bool,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(MachineAdd, "machine.add", Machine);

/// `machine.connect`: dial a machine now.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MachineConnect {
    pub machine: String,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(MachineConnect, "machine.connect", MachineView);

/// `machine.rename`: addresses with the old name stop working.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MachineRename {
    pub machine: String,
    pub new_name: String,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(MachineRename, "machine.rename", Machine);

/// `machine.rm`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MachineRm {
    pub machine: String,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(MachineRm, "machine.rm", RemoveResult);

/// `machine.exec`: a command without a terminal, on any machine.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MachineExec {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub argv: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Its stdin, base64.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(MachineExec, "machine.exec", ExecResult);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecResult {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    /// base64.
    pub stdout: String,
    /// base64.
    pub stderr: String,
    pub ms: u64,
    pub timed_out: bool,
    pub truncated: bool,
    #[serde(flatten)]
    pub extra: Extra,
}

impl ExecResult {
    /// What it wrote to stdout, as text (invalid UTF-8 replaced); empty when it isn't base64.
    pub fn stdout_text(&self) -> String {
        text(&self.stdout)
    }

    /// What it wrote to stderr, as [`ExecResult::stdout_text`] reads stdout.
    pub fn stderr_text(&self) -> String {
        text(&self.stderr)
    }

    /// It ran to the end and exited 0.
    pub fn succeeded(&self) -> bool {
        self.code == Some(0) && !self.timed_out
    }
}

/// Base64 output as text (invalid UTF-8 replaced); empty when it isn't base64.
fn text(encoded: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(encoded.as_bytes())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

// Files.

/// `file.read`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileRead {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The first line, from 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(FileRead, "file.read", FileReadResult);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileReadResult {
    pub text: String,
    pub first: u64,
    pub last: u64,
    pub lines: u64,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `file.write`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileWrite {
    pub path: String,
    /// The new content, base64.
    pub data: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(FileWrite, "file.write", FileResult);

/// One `file.edit` replacement.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Edit {
    pub old: String,
    pub new: String,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `file.edit`: replacements, all made or none.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileEdit {
    pub path: String,
    pub edits: Vec<Edit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(FileEdit, "file.edit", FileResult);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileResult {
    pub path: String,
    pub bytes: u64,
    #[serde(flatten)]
    pub extra: Extra,
}

// Documents.

/// `doc.get`. An unwritten name reads as `{value: null, version: 0}`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DocGet {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(DocGet, "doc.get", DocGetResult);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DocGetResult {
    pub value: Value,
    pub version: u64,
    #[serde(flatten)]
    pub extra: Extra,
}

/// `doc.set`: succeeds only at the current version, else `conflict`. `value: null` deletes the
/// value and keeps the version.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DocSet {
    pub name: String,
    pub value: Value,
    pub version: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(DocSet, "doc.set", DocSetResult);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DocSetResult {
    pub version: u64,
    #[serde(flatten)]
    pub extra: Extra,
}

// Events.

/// `events.subscribe`. A full client sends no filter: then one subscription carries every
/// terminal, local and remote, and every document.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EventsSubscribe {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children_of: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: Labels,
    /// Documents by name; a trailing `*` is a prefix. Naming documents alone hears only them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub docs: Vec<String>,
    #[serde(flatten)]
    pub extra: Extra,
}
call!(EventsSubscribe, "events.subscribe", SubscribeResult);

/// The snapshot, and in framed mode the events channel.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SubscribeResult {
    pub seq: u64,
    pub terminals: Vec<Terminal>,
    pub channel: Option<u32>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exec_output_reads_as_text() {
        let r: ExecResult =
            serde_json::from_value(json!({"code": 0, "stdout": "L2EKL2IK"})).unwrap();
        assert_eq!(r.stdout_text(), "/a\n/b\n");
        assert_eq!(r.stderr_text(), "");
        assert!(r.succeeded());
        let bad = ExecResult {
            stdout: "not base64!".into(),
            code: Some(1),
            ..Default::default()
        };
        assert_eq!(bad.stdout_text(), "");
        assert!(!bad.succeeded());
    }

    #[test]
    fn defaults_are_not_sent() {
        let run = TerminalRun {
            argv: vec!["sh".into()],
            keep: true,
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&run).unwrap(),
            json!({"argv": ["sh"], "keep": true})
        );
        assert_eq!(
            serde_json::to_value(EventsSubscribe::default()).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(ServerStatus::default()).unwrap(),
            json!({})
        );
        let attach = TerminalAttach {
            terminal: "t1".into(),
            cols: 80,
            rows: 24,
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&attach).unwrap(),
            json!({"terminal": "t1", "cols": 80, "rows": 24})
        );
    }

    #[test]
    fn required_params_are_always_sent() {
        let keep = TerminalKeep {
            terminal: "t1".into(),
            keep: false,
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&keep).unwrap(),
            json!({"terminal": "t1", "keep": false})
        );
        let delete = DocSet {
            name: "herdr-frontend".into(),
            value: Value::Null,
            version: 3,
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&delete).unwrap(),
            json!({"name": "herdr-frontend", "value": null, "version": 3})
        );
    }

    #[test]
    fn renamed_and_extra_params_go_out_as_named() {
        let mut send = TerminalSend {
            terminal: "t1".into(),
            text: "hi".into(),
            only_if: Some(IdleOrWorking::Idle),
            ..Default::default()
        };
        send.extra.insert("newer".into(), json!(true));
        assert_eq!(
            serde_json::to_value(&send).unwrap(),
            json!({"terminal": "t1", "text": "hi", "if": "idle", "newer": true})
        );
        let wait = TerminalWait {
            children: Some(Children::All),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&wait).unwrap(),
            json!({"children": "all"})
        );
    }

    #[test]
    fn method_names_match_the_schema() {
        assert_eq!(<TerminalRun as Call>::METHOD, "terminal.run");
        assert_eq!(<TerminalRm as Call>::METHOD, "terminal.rm");
        assert_eq!(<MachineRename as Call>::METHOD, "machine.rename");
        assert_eq!(<EventsSubscribe as Call>::METHOD, "events.subscribe");
        assert_eq!(<RpcDiscover as Call>::METHOD, "rpc.discover");
    }

    #[test]
    fn run_results_flatten_the_terminal() {
        let raw = json!({"address": "t4", "argv": ["echo", "hi"], "output": "hi", "later": 1});
        let run: RunResult = serde_json::from_value(raw).unwrap();
        assert_eq!(run.output.as_deref(), Some("hi"));
        assert_eq!(run.terminal.address, "t4");
        assert_eq!(run.terminal.extra["later"], 1);
        assert!(!run.terminal.extra.contains_key("output"));
    }
}
