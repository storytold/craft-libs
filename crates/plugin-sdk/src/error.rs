//! Errors a host call can return.

use std::fmt;

use craft_plugin_abi::Status;

/// Why a `craft_host.call` produced no value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostError {
    /// `E_DENIED`: the op is not in `requires.ops`, or the grant is missing.
    Denied,
    /// `E_TOO_SMALL` after the retry rule ran out of attempts.
    TooSmall { required: usize },
    /// `E_UNSUPPORTED`: this host lists the op and does not serve it.
    Unsupported,
    /// `E_BAD_ARG`: the request did not parse, or named something unknown.
    BadArg,
    /// `E_CANCELLED`: the user cancelled the call.
    Cancelled,
    /// `E_LIMIT`: a rate limit or quota stopped the call.
    Limit,
    /// A status code this SDK does not know. A newer host may send one.
    Status(i32),
    /// The call worked and the reply was not what the op promises.
    BadReply(String),
    /// Native build with no host installed. Wasm builds never return this.
    NoHost,
}

impl HostError {
    /// Map a status code from `craft_host.call`. `count` is the byte count the host returned.
    pub fn from_status(code: i32, count: u32) -> HostError {
        match Status::from_code(code) {
            Some(Status::Denied) => HostError::Denied,
            Some(Status::TooSmall) => HostError::TooSmall { required: count as usize },
            Some(Status::Unsupported) => HostError::Unsupported,
            Some(Status::BadArg) => HostError::BadArg,
            Some(Status::Cancelled) => HostError::Cancelled,
            Some(Status::Limit) => HostError::Limit,
            // `Ok` is not an error. A caller that gets here with 0 has a bug, so keep the code visible.
            Some(Status::Ok) | None => HostError::Status(code),
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::Denied => f.write_str("E_DENIED: the op or its permission is not granted"),
            HostError::TooSmall { required } => write!(f, "E_TOO_SMALL: the host needs {required} bytes"),
            HostError::Unsupported => f.write_str("E_UNSUPPORTED: this host does not serve the op"),
            HostError::BadArg => f.write_str("E_BAD_ARG: the host rejected the request"),
            HostError::Cancelled => f.write_str("E_CANCELLED: the user cancelled"),
            HostError::Limit => f.write_str("E_LIMIT: a rate limit or quota stopped the call"),
            HostError::Status(code) => write!(f, "host status {code}"),
            HostError::BadReply(m) => write!(f, "bad host reply: {m}"),
            HostError::NoHost => f.write_str("no host: this build is not running inside a Craft app"),
        }
    }
}

impl std::error::Error for HostError {}
