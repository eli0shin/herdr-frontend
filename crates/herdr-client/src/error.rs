//! What can go wrong talking to a core.

use std::fmt;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use herdr_proto::{ErrorKind, Mismatch, RpcError};
use serde::Deserialize;

/// A failed connect, call or stream. `Clone`, because one lost connection fails every call and
/// stream waiting on it.
#[derive(Clone, Debug)]
pub enum Error {
    /// The core answered with an error. Match on `kind`, never on the message.
    Rpc(RpcError),
    /// No reply in time. The route was dropped and `$/cancelRequest` sent.
    Timeout { method: String, after: Duration },
    /// The connection dropped, or was already gone. The supervisor reconnects.
    Lost,
    /// This side closed the connection.
    Closed,
    /// `hc connect` could not reach or start the core, and said why on stderr.
    Startup(StartupError),
    /// The core can't serve this client (rule 10): update one side.
    Incompatible(Mismatch),
    /// The core lacks `hello.features` this client can't work without.
    MissingFeatures(Vec<String>),
    /// Starting `hc`, the socket, or a read or write failed.
    Io(Arc<io::Error>),
    /// The core sent something that breaks the protocol.
    Protocol(String),
}

impl Error {
    /// The core's `data.kind`, for errors the core sent.
    pub fn kind(&self) -> Option<&ErrorKind> {
        match self {
            Error::Rpc(e) => Some(&e.kind),
            Error::Startup(e) => Some(&e.kind),
            _ => None,
        }
    }

    /// Whether the connection is gone, so a call can't succeed until the next one.
    pub fn is_lost(&self) -> bool {
        matches!(self, Error::Lost | Error::Closed)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Rpc(e) => e.fmt(f),
            Error::Timeout { method, after } => {
                write!(f, "{method}: no reply from the core in {after:?}")
            }
            Error::Lost => f.write_str("lost the core"),
            Error::Closed => f.write_str("the connection to the core was closed"),
            Error::Startup(e) => e.fmt(f),
            Error::Incompatible(m) => m.fmt(f),
            Error::MissingFeatures(features) => {
                write!(f, "hc lacks {}; update hc", features.join(", "))
            }
            Error::Io(e) => e.fmt(f),
            Error::Protocol(what) => write!(f, "the core broke the protocol: {what}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Rpc(e) => Some(e),
            Error::Startup(e) => Some(e),
            Error::Incompatible(m) => Some(m),
            Error::Io(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(Arc::new(e))
    }
}

impl From<RpcError> for Error {
    fn from(e: RpcError) -> Self {
        Error::Rpc(e)
    }
}

impl From<Mismatch> for Error {
    fn from(m: Mismatch) -> Self {
        Error::Incompatible(m)
    }
}

/// The one JSON object `hc connect` prints on stderr when it can't start:
/// `{"error":{"kind","message","hint"}}` (`app:src/hc.rs` `connect_error`).
#[derive(Clone, Debug, PartialEq)]
pub struct StartupError {
    pub kind: ErrorKind,
    pub message: String,
    pub hint: Option<String>,
}

impl StartupError {
    /// Parses what `hc connect` wrote on stderr. `None` unless it is exactly that object.
    pub fn parse(stderr: &str) -> Option<StartupError> {
        #[derive(Deserialize)]
        struct Wire {
            error: Inner,
        }
        #[derive(Deserialize)]
        struct Inner {
            #[serde(default)]
            kind: Option<String>,
            message: String,
            #[serde(default)]
            hint: Option<String>,
        }
        let wire: Wire = serde_json::from_str(stderr.trim()).ok()?;
        Some(StartupError {
            kind: wire.error.kind.map_or(ErrorKind::Internal, ErrorKind::from),
            message: wire.error.message,
            hint: wire.error.hint.filter(|h| !h.is_empty()),
        })
    }
}

impl fmt::Display for StartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        match &self.hint {
            Some(hint) => write!(f, " ({hint})"),
            None => Ok(()),
        }
    }
}

impl std::error::Error for StartupError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_errors_keep_the_kind_message_and_hint() {
        let err = StartupError::parse(
            "{\"error\":{\"kind\":\"usage\",\"message\":\"the socket path is too long\",\"hint\":\"use a shorter HC_SESSION\"}}\n",
        )
        .unwrap();
        assert_eq!(err.kind, ErrorKind::Usage);
        assert_eq!(
            err.to_string(),
            "the socket path is too long (use a shorter HC_SESSION)"
        );
        let bare = StartupError::parse(r#"{"error":{"message":"m","hint":""}}"#).unwrap();
        assert_eq!(bare.kind, ErrorKind::Internal);
        assert_eq!(bare.hint, None);
        assert_eq!(StartupError::parse("warning: something\n"), None);
        assert_eq!(StartupError::parse(""), None);
    }

    #[test]
    fn errors_say_whether_the_connection_is_gone() {
        assert!(Error::Lost.is_lost());
        assert!(Error::Closed.is_lost());
        assert!(!Error::Protocol("x".into()).is_lost());
        assert_eq!(Error::Lost.kind(), None);
    }
}
