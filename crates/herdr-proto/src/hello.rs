//! The handshake, the core's capabilities, and the compatibility check (§1.2, §6).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::methods::AttachResult;
use crate::rpc::{Call, Request};
use crate::types::{CoreInfo, Extra, is_false};

/// The protocol generation this crate speaks.
pub const GENERATION: u32 = 1;

/// The oldest core herdr-frontend supports: the first with the `docs` subscription filter and the
/// reconnect-size fix (`herdr-core-protocol.md` §6).
pub const MIN_CORE: Version = Version::new(0, 1, 22);

/// `hello.features` words (`core:crates/herdr-core/src/api.rs`).
pub mod feature {
    pub const FRAMED: &str = "framed";
    pub const ATTACH: &str = "attach";
    pub const EVENTS: &str = "events";
    pub const STATUS: &str = "status";
    pub const SHELL: &str = "shell";
    pub const RESIZE_NOTIFICATIONS: &str = "resize_notifications";
    pub const AUTOMATIC_INPUT: &str = "automatic_input";
    pub const RESTART_ENVIRONMENT: &str = "restart_environment";
    pub const PUT_CHUNKS: &str = "put_chunks";
}

/// `hello` params. `HelloParams::framed()` is what a viewer sends.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HelloParams {
    #[serde(skip_serializing_if = "is_false")]
    pub framed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<u32>,
    /// Only for another core dialing in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub core: Option<CoreInfo>,
    /// Only for another core dialing in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calls_you: Option<String>,
    /// Only for another core dialing in.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl HelloParams {
    /// `{framed: true, generation: 1}`: switch to framed mode, and fail with
    /// `unsupported_generation` rather than talk past a core of another generation.
    pub fn framed() -> Self {
        HelloParams {
            framed: true,
            generation: Some(GENERATION),
            ..Default::default()
        }
    }
}

impl Call for HelloParams {
    const METHOD: &'static str = "hello";
    type Result = Hello;
}

/// The plain-mode line that opens a framed connection: request id 0, then `\n`. The reply is
/// one line too; after it, the connection is framed.
pub fn hello_line() -> Vec<u8> {
    let mut line = Request::new(0, HelloParams::framed()).to_bytes();
    line.push(b'\n');
    line
}

/// The reply to `hello`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hello {
    pub generation: u32,
    pub version: String,
    pub core: CoreInfo,
    pub framed: bool,
    pub methods: Vec<String>,
    pub features: Vec<String>,
    pub login_session: Option<LoginSession>,
    pub keeps_running_after_logout: Option<bool>,
    pub logout_linger_user: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// The login session a core runs in.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoginSession {
    pub id: u32,
    pub user: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gui: Option<bool>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// What the core supports, from `hello.features`. A stream may use a feature only when its
/// attach reply confirms it too ([`StreamCaps`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub framed: bool,
    pub attach: bool,
    pub events: bool,
    pub resize_notifications: bool,
    pub automatic_input: bool,
}

/// What one attach stream may use: the feature in `hello` *and* in the attach reply, since every
/// hop on the path to a remote terminal must support it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamCaps {
    /// Send engine replies as AUTOMATIC_INPUT; otherwise as DATA.
    pub automatic_input: bool,
    /// The core sends RESIZE before repaints; otherwise follow `size` events.
    pub resize_notifications: bool,
}

impl Capabilities {
    pub fn stream(&self, attach: &AttachResult) -> StreamCaps {
        StreamCaps {
            automatic_input: self.automatic_input && attach.automatic_input,
            resize_notifications: self.resize_notifications && attach.resize_notifications,
        }
    }
}

/// Why a core can't serve this client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mismatch {
    /// A newer generation than this client speaks: update herdr-frontend.
    NewerGeneration { generation: u32 },
    /// An older generation: update the core.
    OlderGeneration { generation: u32 },
    /// Older than the oldest supported core, or a version that doesn't parse: update the core.
    TooOld { have: String, need: Version },
    /// Methods the client calls are missing (an unusual build): update the core.
    MissingMethods(Vec<String>),
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mismatch::NewerGeneration { generation } => {
                write!(
                    f,
                    "the core speaks protocol generation {generation}; update herdr-frontend"
                )
            }
            Mismatch::OlderGeneration { generation } => {
                write!(
                    f,
                    "the core speaks protocol generation {generation}; update hc"
                )
            }
            Mismatch::TooOld { have, need } => {
                write!(f, "hc {have} is older than {need}; update hc")
            }
            Mismatch::MissingMethods(methods) => {
                write!(f, "hc lacks {}; update hc", methods.join(", "))
            }
        }
    }
}

impl std::error::Error for Mismatch {}

impl Hello {
    pub fn has_method(&self, method: &str) -> bool {
        self.methods.iter().any(|m| m == method)
    }

    pub fn has_feature(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }

    /// The core's version, if it parses.
    pub fn version(&self) -> Option<Version> {
        Version::parse(&self.version)
    }

    pub fn capabilities(&self) -> Capabilities {
        Capabilities {
            framed: self.framed && self.has_feature(feature::FRAMED),
            attach: self.has_feature(feature::ATTACH),
            events: self.has_feature(feature::EVENTS),
            resize_notifications: self.has_feature(feature::RESIZE_NOTIFICATIONS),
            automatic_input: self.has_feature(feature::AUTOMATIC_INPUT),
        }
    }

    /// The check to make at connect, for the local core and every machine: the generation, the
    /// minimum version, and the methods the client calls.
    pub fn check(&self, min: Version, uses: &[&str]) -> Result<(), Mismatch> {
        if self.generation > GENERATION {
            return Err(Mismatch::NewerGeneration {
                generation: self.generation,
            });
        }
        if self.generation < GENERATION {
            return Err(Mismatch::OlderGeneration {
                generation: self.generation,
            });
        }
        if self.version().is_none_or(|have| have < min) {
            return Err(Mismatch::TooOld {
                have: self.version.clone(),
                need: min,
            });
        }
        let missing: Vec<String> = uses
            .iter()
            .filter(|m| !self.has_method(m))
            .map(|m| (*m).to_owned())
            .collect();
        if !missing.is_empty() {
            return Err(Mismatch::MissingMethods(missing));
        }
        Ok(())
    }
}

/// A numeric `major.minor.patch`, compared as numbers. Pre-release and build suffixes are ignored,
/// as herdr-app ignores them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Version {
            major,
            minor,
            patch,
        }
    }

    /// Parses `0.1.22`, `hc 0.1.22` or `0.1.22-rc.1`. A missing minor or patch is 0.
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        let text = text.strip_prefix("hc ").unwrap_or(text).trim();
        let core = text.split(['-', '+']).next()?;
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().map_or(Some(0), |p| p.parse().ok())?;
        let patch = parts.next().map_or(Some(0), |p| p.parse().ok())?;
        if parts.next().is_some() {
            return None;
        }
        Some(Version {
            major,
            minor,
            patch,
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn hello(generation: u32, version: &str, methods: &[&str], features: &[&str]) -> Hello {
        Hello {
            generation,
            version: version.into(),
            framed: true,
            methods: methods.iter().map(|m| (*m).into()).collect(),
            features: features.iter().map(|f| (*f).into()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn the_hello_line_asks_for_framed_generation_one() {
        let line = hello_line();
        assert_eq!(line.last(), Some(&b'\n'));
        let value: Value = serde_json::from_slice(&line).unwrap();
        assert_eq!(
            value,
            json!({"jsonrpc": "2.0", "id": 0, "method": "hello", "params": {"framed": true, "generation": 1}})
        );
    }

    #[test]
    fn versions_parse_and_compare_as_numbers() {
        assert_eq!(Version::parse("0.1.22"), Some(Version::new(0, 1, 22)));
        assert_eq!(Version::parse("hc 0.1.22\n"), Some(Version::new(0, 1, 22)));
        assert_eq!(Version::parse("0.1.22-rc.1"), Some(Version::new(0, 1, 22)));
        assert_eq!(Version::parse("1.2"), Some(Version::new(1, 2, 0)));
        assert_eq!(Version::parse(""), None);
        assert_eq!(Version::parse("x.1.2"), None);
        assert_eq!(Version::parse("1.2.3.4"), None);
        assert!(Version::new(0, 1, 9) < Version::new(0, 1, 22));
        assert_eq!(Version::new(0, 1, 22).to_string(), "0.1.22");
    }

    #[test]
    fn the_check_covers_generation_version_and_methods() {
        let uses = ["terminal.attach", "doc.set"];
        let ok = hello(1, "0.1.22", &uses, &[]);
        assert_eq!(ok.check(MIN_CORE, &uses), Ok(()));
        assert_eq!(
            hello(2, "0.1.22", &uses, &[]).check(MIN_CORE, &uses),
            Err(Mismatch::NewerGeneration { generation: 2 })
        );
        assert_eq!(
            hello(0, "0.1.22", &uses, &[]).check(MIN_CORE, &uses),
            Err(Mismatch::OlderGeneration { generation: 0 })
        );
        assert_eq!(
            hello(1, "0.1.21", &uses, &[]).check(MIN_CORE, &uses),
            Err(Mismatch::TooOld {
                have: "0.1.21".into(),
                need: MIN_CORE
            })
        );
        assert_eq!(
            hello(1, "garbage", &uses, &[]).check(MIN_CORE, &uses),
            Err(Mismatch::TooOld {
                have: "garbage".into(),
                need: MIN_CORE
            })
        );
        assert_eq!(
            hello(1, "0.2.0", &["doc.set"], &[]).check(MIN_CORE, &uses),
            Err(Mismatch::MissingMethods(vec!["terminal.attach".into()]))
        );
    }

    #[test]
    fn stream_caps_need_the_feature_and_the_attach_reply() {
        let all = hello(
            1,
            "0.1.22",
            &[],
            &[
                feature::FRAMED,
                feature::AUTOMATIC_INPUT,
                feature::RESIZE_NOTIFICATIONS,
            ],
        );
        let none = hello(1, "0.1.22", &[], &[feature::FRAMED]);
        let yes = AttachResult {
            automatic_input: true,
            resize_notifications: true,
            ..Default::default()
        };
        let no = AttachResult::default();
        let caps = all.capabilities();
        assert!(caps.framed && caps.automatic_input && caps.resize_notifications && !caps.attach);
        assert_eq!(
            caps.stream(&yes),
            StreamCaps {
                automatic_input: true,
                resize_notifications: true
            }
        );
        assert_eq!(caps.stream(&no), StreamCaps::default());
        assert_eq!(none.capabilities().stream(&yes), StreamCaps::default());
    }
}
