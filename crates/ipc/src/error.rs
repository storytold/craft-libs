//! Failures a Craft Link caller can show or turn into a protocol error object.

use serde::Serialize;
use serde_json::{Map, Value};

/// Wire error object for new methods. Existing methods keep a string `error`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ErrorObject {
    pub code: String,
    pub message: String,
    pub data: Map<String, Value>,
}

/// A link failure. The first-line auth refusal is still the string
/// `authentication required`. [`LinkError::to_object`] is for new methods.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    Io(String),
    /// The runtime directory or socket mode is not acceptable.
    Refused {
        path: String,
        reason: String,
    },
    Invalid(String),
    Busy(String),
    Auth,
    Protocol {
        server_min: u32,
        server_max: u32,
    },
    BadParams(String),
    NotInstalled {
        app: String,
    },
    Mismatch {
        field: String,
        expected: String,
        got: String,
    },
    Closed,
    Timeout,
    TooLarge,
    NotReady(String),
    /// An asset moniker, link id or file that the peer does not know.
    Unknown(String),
    /// A `link.open` payload with a bad shape or an unknown `format`.
    PayloadInvalid(String),
    /// A `link.open` payload names media files that do not exist or cannot be opened. The
    /// sender renders or relinks them and sends the call again.
    MediaMissing {
        paths: Vec<String>,
    },
}

impl LinkError {
    pub fn io(err: impl std::fmt::Display) -> Self {
        Self::Io(err.to_string())
    }

    /// Contract section 7 code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Auth => "auth_required",
            Self::Protocol { .. } => "protocol_unsupported",
            Self::BadParams(_) | Self::Invalid(_) => "bad_params",
            Self::NotReady(_) => "not_ready",
            Self::Unknown(_) => "link_unknown",
            Self::PayloadInvalid(_) => "payload_invalid",
            Self::MediaMissing { .. } => "media_missing",
            Self::TooLarge => "payload_too_large",
            Self::Io(_) => "io_error",
            Self::Busy(_) => "busy",
            Self::NotInstalled { .. } => "not_ready",
            Self::Refused { .. } | Self::Mismatch { .. } | Self::Closed | Self::Timeout => "internal",
        }
    }

    pub fn to_object(&self) -> ErrorObject {
        let mut data = Map::new();
        if let Self::Protocol { server_min, server_max } = self {
            data.insert("server_min".into(), Value::from(*server_min));
            data.insert("server_max".into(), Value::from(*server_max));
        }
        if let Self::MediaMissing { paths } = self {
            data.insert("missing".into(), Value::from(paths.clone()));
        }
        ErrorObject { code: self.code().to_string(), message: self.to_string(), data }
    }
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(msg) => write!(f, "{msg}"),
            Self::Refused { path, reason } => write!(f, "{path}: {reason}"),
            Self::Invalid(msg) => write!(f, "{msg}"),
            Self::Busy(msg) => write!(f, "{msg}"),
            Self::Auth => write!(f, "authentication required"),
            Self::Protocol { server_min, server_max } => {
                write!(f, "protocol not supported (server {server_min} to {server_max})")
            }
            Self::BadParams(msg) => write!(f, "{msg}"),
            Self::NotInstalled { app } => write!(f, "{app} is not installed"),
            Self::Mismatch { field, expected, got } => {
                write!(f, "{field} is {got}, record says {expected}")
            }
            Self::Closed => write!(f, "connection closed"),
            Self::Timeout => write!(f, "timeout"),
            Self::TooLarge => write!(f, "payload too large"),
            Self::NotReady(msg) => write!(f, "{msg}"),
            Self::Unknown(msg) | Self::PayloadInvalid(msg) => write!(f, "{msg}"),
            Self::MediaMissing { paths } => write!(f, "the payload names media that cannot be opened: {}", paths.join(", ")),
        }
    }
}

impl std::error::Error for LinkError {}

impl From<std::io::Error> for LinkError {
    /// A socket read timeout is `WouldBlock` on Linux ("Resource temporarily unavailable") and
    /// `TimedOut` elsewhere. Both mean the peer was quiet, not that the connection broke.
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => Self::Timeout,
            _ => Self::Io(err.to_string()),
        }
    }
}

impl From<serde_json::Error> for LinkError {
    fn from(err: serde_json::Error) -> Self {
        Self::Invalid(err.to_string())
    }
}
