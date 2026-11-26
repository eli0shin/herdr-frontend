//! The JSON-RPC 2.0 envelope on channel 0, and the core's errors.
//!
//! Ids are the client's own `u64`s. Calls run concurrently on the core, so replies can come in any
//! order; route them by id.

use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::event::Notification;
use crate::words::string_enum;

pub const JSONRPC: &str = "2.0";

/// A request id. The client picks them; the core echoes them.
pub type Id = u64;

/// A method's typed params. `METHOD` is its name and `Result` its reply.
pub trait Call: Serialize {
    const METHOD: &'static str;
    type Result: DeserializeOwned;
}

/// A request as it goes on the wire.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request<P> {
    pub jsonrpc: String,
    pub id: Id,
    pub method: String,
    pub params: P,
}

impl<P: Call> Request<P> {
    pub fn new(id: Id, params: P) -> Self {
        Request {
            jsonrpc: JSONRPC.into(),
            id,
            method: P::METHOD.into(),
            params,
        }
    }
}

impl<P: Serialize> Request<P> {
    /// The message, as one DATA frame's payload on channel 0 (or one line in plain mode,
    /// with a `\n` after it).
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("requests always serialize")
    }
}

/// `$/cancelRequest {id}`: ends a `machine.exec`, a `terminal.put`, or a `terminal.send` still
/// waiting on a draft. Send it when a call times out, then drop its route.
pub fn cancel_request(id: Id) -> Vec<u8> {
    serde_json::to_vec(
        &serde_json::json!({"jsonrpc": JSONRPC, "method": "$/cancelRequest", "params": {"id": id}}),
    )
    .expect("cancel always serializes")
}

string_enum! {
    /// `error.data.kind`: stable, unlike the message. Match on this, never on the message.
    pub enum ErrorKind {
        ParseError = "parse_error",
        InvalidRequest = "invalid_request",
        UnknownMethod = "unknown_method",
        InvalidParams = "invalid_params",
        Internal = "internal",
        Usage = "usage",
        NotFound = "not_found",
        MachineUnknown = "machine_unknown",
        Unavailable = "unavailable",
        Finished = "finished",
        Live = "live",
        NeedsFramed = "needs_framed",
        NameTaken = "name_taken",
        SpawnFailed = "spawn_failed",
        InvalidTemplate = "invalid_template",
        Config = "config",
        Blocked = "blocked",
        StatusMismatch = "status_mismatch",
        Cancelled = "cancelled",
        NeedsTarget = "needs_target",
        /// `doc.set` at a stale version: read the document again and replay the edit.
        Conflict = "conflict",
        NeedsUpdate = "needs_update",
        NeedsLogin = "needs_login",
        NotText = "not_text",
        Io = "io",
        EditFailed = "edit_failed",
        AlreadyAdded = "already_added",
        Unreachable = "unreachable",
        Incompatible = "incompatible",
        HcMissing = "hc_missing",
        UnsupportedGeneration = "unsupported_generation",
        Refused = "refused",
        Denied = "denied",
        Busy = "busy",
        InstallFailed = "install_failed",
    }
}

/// A JSON-RPC error from the core: `{"code","message","data":{"kind","hint"}}`.
#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub kind: ErrorKind,
    pub hint: Option<String>,
    /// Other fields of `data`, kept as sent.
    pub data: Map<String, Value>,
}

impl RpcError {
    /// An error with no `data.kind` (which the core always sends) reads as `internal`, as the
    /// core's own client reads it.
    fn from_wire(wire: WireError) -> Self {
        let mut data = match wire.data {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        let kind = match data.remove("kind") {
            Some(Value::String(kind)) => ErrorKind::from(kind),
            _ => ErrorKind::Internal,
        };
        let hint = match data.remove("hint") {
            Some(Value::String(hint)) => Some(hint),
            _ => None,
        };
        RpcError {
            code: wire.code,
            message: wire.message,
            kind,
            hint,
            data,
        }
    }

    fn to_wire(&self) -> WireError {
        let mut data = Map::new();
        data.insert("kind".into(), self.kind.as_str().into());
        if let Some(hint) = &self.hint {
            data.insert("hint".into(), hint.as_str().into());
        }
        data.extend(self.data.clone());
        WireError {
            code: self.code,
            message: self.message.clone(),
            data: Value::Object(data),
        }
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.message.is_empty() {
            "core call failed"
        } else {
            &self.message
        })?;
        match self.hint.as_deref() {
            Some(hint) if !hint.is_empty() => write!(f, " ({hint})"),
            _ => Ok(()),
        }
    }
}

impl std::error::Error for RpcError {}

impl Serialize for RpcError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_wire().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RpcError {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        WireError::deserialize(deserializer).map(RpcError::from_wire)
    }
}

#[derive(Serialize, Deserialize)]
struct WireError {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    message: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    data: Value,
}

/// A reply to one of the client's requests.
#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    /// `None` when the core couldn't read the request's id.
    pub id: Option<Id>,
    pub outcome: Result<Value, RpcError>,
}

impl Response {
    /// The typed result. A result that doesn't fit `R` is an `invalid_result` error naming the
    /// method, so a caller sees one error type.
    pub fn into_result<C: Call>(self) -> Result<C::Result, RpcError> {
        let value = self.outcome?;
        serde_json::from_value(value).map_err(|e| RpcError {
            code: -32603,
            message: format!("{} returned an unexpected result: {e}", C::METHOD),
            kind: ErrorKind::Other("invalid_result".into()),
            hint: None,
            data: Map::new(),
        })
    }
}

/// A message received on channel 0, or a line in plain mode.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    Response(Response),
    /// Events in plain mode come on the connection itself.
    Notification(Notification),
    /// A call from the core. Only another core dialing in gets these; a viewer can ignore them.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

impl Incoming {
    pub fn parse(bytes: &[u8]) -> Result<Incoming, serde_json::Error> {
        let raw: RawMessage = serde_json::from_slice(bytes)?;
        match (raw.method, raw.id) {
            (Some(method), Some(id)) if !id.is_null() => Ok(Incoming::Request {
                id,
                method,
                params: raw.params,
            }),
            (Some(method), _) => {
                Notification::from_parts(method, raw.params).map(Incoming::Notification)
            }
            (None, id) => {
                let id = id.and_then(|id| id.as_u64());
                let outcome = match (raw.error, raw.result) {
                    (Some(error), _) => Err(error),
                    (None, result) => Ok(result.unwrap_or(Value::Null)),
                };
                Ok(Incoming::Response(Response { id, outcome }))
            }
        }
    }
}

#[derive(Deserialize)]
struct RawMessage {
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    params: Value,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<RpcError>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::methods::{DocSet, TerminalGet};
    use serde_json::json;

    #[test]
    fn requests_carry_the_method_and_id() {
        let request = Request::new(
            7,
            TerminalGet {
                terminal: "t1".into(),
                ..Default::default()
            },
        );
        let value: Value = serde_json::from_slice(&request.to_bytes()).unwrap();
        assert_eq!(
            value,
            json!({"jsonrpc": "2.0", "id": 7, "method": "terminal.get", "params": {"terminal": "t1"}})
        );
    }

    #[test]
    fn cancel_is_a_notification_naming_the_id() {
        let value: Value = serde_json::from_slice(&cancel_request(9)).unwrap();
        assert_eq!(
            value,
            json!({"jsonrpc": "2.0", "method": "$/cancelRequest", "params": {"id": 9}})
        );
    }

    #[test]
    fn replies_parse_into_results_and_errors() {
        let ok = Incoming::parse(br#"{"jsonrpc":"2.0","id":3,"result":{"version":4}}"#).unwrap();
        let Incoming::Response(ok) = ok else {
            panic!("{ok:?}")
        };
        assert_eq!(ok.id, Some(3));
        assert_eq!(ok.into_result::<DocSet>().unwrap().version, 4);

        let line = br#"{"jsonrpc":"2.0","id":4,"error":{"code":-32000,"message":"document \"x\" is at version 2, not 1","data":{"kind":"conflict","hint":"read doc.get and retry with its version"}}}"#;
        let Incoming::Response(err) = Incoming::parse(line).unwrap() else {
            panic!()
        };
        let err = err.into_result::<DocSet>().unwrap_err();
        assert_eq!(err.kind, ErrorKind::Conflict);
        assert_eq!(err.code, -32000);
        assert_eq!(
            err.hint.as_deref(),
            Some("read doc.get and retry with its version")
        );
        assert_eq!(
            err.to_string(),
            "document \"x\" is at version 2, not 1 (read doc.get and retry with its version)"
        );
    }

    #[test]
    fn a_result_of_the_wrong_shape_is_an_error() {
        let Incoming::Response(r) =
            Incoming::parse(br#"{"jsonrpc":"2.0","id":3,"result":{"version":"x"}}"#).unwrap()
        else {
            panic!()
        };
        let err = r.into_result::<DocSet>().unwrap_err();
        assert_eq!(err.kind.as_str(), "invalid_result");
        assert!(
            err.message
                .starts_with("doc.set returned an unexpected result"),
            "{}",
            err.message
        );
    }

    #[test]
    fn errors_keep_unknown_kinds_and_data() {
        let raw = json!({"code": -32000, "message": "m", "data": {"kind": "brand_new", "hint": "h", "extra": [1]}});
        let err: RpcError = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(err.kind, ErrorKind::Other("brand_new".into()));
        assert_eq!(err.data["extra"], json!([1]));
        assert_eq!(serde_json::to_value(&err).unwrap(), raw);
    }

    #[test]
    fn errors_without_data_read_as_internal() {
        let err: RpcError = serde_json::from_value(json!({"code": -32603, "message": ""})).unwrap();
        assert_eq!(err.kind, ErrorKind::Internal);
        assert_eq!(err.hint, None);
        assert_eq!(err.to_string(), "core call failed");
    }

    #[test]
    fn unreadable_ids_and_empty_results_still_parse() {
        let Incoming::Response(r) = Incoming::parse(
            br#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"bad"}}"#,
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(r.id, None);
        assert_eq!(r.outcome.unwrap_err().code, -32700);
        let Incoming::Response(r) = Incoming::parse(br#"{"jsonrpc":"2.0","id":2}"#).unwrap() else {
            panic!()
        };
        assert_eq!(r.outcome, Ok(Value::Null));
    }

    #[test]
    fn notifications_and_calls_are_told_apart() {
        let n = Incoming::parse(br#"{"jsonrpc":"2.0","method":"heartbeat","params":{"seq":4}}"#)
            .unwrap();
        assert_eq!(
            n,
            Incoming::Notification(Notification::Heartbeat { seq: 4 })
        );
        let call =
            Incoming::parse(br#"{"jsonrpc":"2.0","id":"x","method":"terminal.get","params":{}}"#)
                .unwrap();
        assert!(
            matches!(call, Incoming::Request { ref method, .. } if method == "terminal.get"),
            "{call:?}"
        );
        assert!(Incoming::parse(b"not json").is_err());
    }
}
