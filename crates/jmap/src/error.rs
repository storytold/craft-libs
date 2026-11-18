//! The crate's error type, arranged by what the mail client can do about each case.

use std::fmt;

use crate::address::{Address, AddressError};
use crate::credentials::CredentialsError;
use crate::http::TransportError;
use crate::id::IdError;
use crate::redact::Excerpt;
use crate::retry::RetryAfter;
use crate::template::TemplateError;

/// The longest limit name kept from a server.
const MAX_LIMIT_NAME_LEN: usize = 64;

/// The name of a JMAP limit a server says was exceeded (`maxSizeRequest`, `maxCallsInRequest`, …), checked to
/// be a short identifier (letters, digits and `_`).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct LimitName(String);

impl LimitName {
    /// Checks a limit name received from a server.
    pub fn new(text: &str) -> Option<Self> {
        if text.is_empty() || text.len() > MAX_LIMIT_NAME_LEN || !text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return None;
        }
        Some(Self(text.to_owned()))
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the limit is about concurrency (`maxConcurrentRequests`, `maxConcurrentUpload`) rather than size.
    pub fn is_concurrency(&self) -> bool {
        self.0.starts_with("maxConcurrent")
    }
}

impl fmt::Display for LimitName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for LimitName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LimitName({})", self.0)
    }
}

/// A request-level JMAP problem (RFC 8620 §3.6.1), the `urn:ietf:params:jmap:error:` prefix removed.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RequestProblem {
    /// `unknownCapability`: the request used a capability the server does not support.
    UnknownCapability,
    /// `notJSON`: the request was not `application/json` or not I-JSON.
    NotJson,
    /// `notRequest`: the request did not have the shape of a Request object.
    NotRequest,
    /// `limit`: a request limit was exceeded.
    Limit,
    /// Another JMAP problem type, checked to be a short identifier.
    Other(String),
}

impl fmt::Display for RequestProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCapability => f.write_str("it uses a capability this server does not support"),
            Self::NotJson => f.write_str("the server could not read it as JSON"),
            Self::NotRequest => f.write_str("its JSON is not shaped like a JMAP request"),
            Self::Limit => f.write_str("it goes beyond one of the server's request limits"),
            Self::Other(name) if name.is_empty() => f.write_str("the server reported an unnamed JMAP problem"),
            Self::Other(name) => write!(f, "the server reported the JMAP problem {name}"),
        }
    }
}

/// Why a server's successful-looking answer cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unusable {
    /// The content type is not the one expected (`application/json`).
    WrongContentType,
    /// The body is larger than the request's limit ([`HttpRequest::max_response_bytes`](crate::HttpRequest::max_response_bytes):
    /// the kind's, or the lower one the caller set), whether the crate or the transport found it.
    BodyTooLarge,
    /// The body is not valid JSON (RFC 8259), or nests too deeply.
    InvalidJson,
    /// The body holds more JSON values or objects than the crate accepts.
    TooComplex,
    /// The JSON does not have the expected shape; the text says which part, in the crate's own words.
    Shape(&'static str),
    /// An address the server gave cannot be used; the text names the field.
    Address(&'static str, AddressError),
    /// An address template the server gave cannot be used; the text names the field.
    Template(&'static str, TemplateError),
}

impl fmt::Display for Unusable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongContentType => f.write_str("its content type is not application/json"),
            Self::BodyTooLarge => f.write_str("it is larger than the size limit set for this request"),
            Self::InvalidJson => f.write_str("it is not valid JSON, or nests more than 128 levels deep"),
            Self::TooComplex => f.write_str("it holds more JSON values or objects than this program reads at once"),
            Self::Shape(what) => write!(f, "{what}"),
            Self::Address(field, error) => write!(f, "its {field} cannot be used: {error}"),
            Self::Template(field, error) => write!(f, "its {field} cannot be used: {error}"),
        }
    }
}

/// A mistake in a request being built, found before anything is sent.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BuildError {
    /// A method name that is empty, too long, or not printable ASCII.
    InvalidMethodName,
    /// A capability identifier that is empty, too long, or not printable ASCII.
    InvalidCapability,
    /// A capability the session does not advertise.
    UnsupportedCapability,
    /// An argument given both plainly and as a result reference (RFC 8620 §3.7).
    DuplicateArgument,
    /// An argument name that is empty or starts with `#`.
    InvalidArgumentName,
    /// A result reference to a call that is not earlier in this request (RFC 8620 §3.7).
    UnknownCall,
    /// A result reference path that is not a JSON Pointer (RFC 6901, with JMAP's `*`).
    InvalidPath,
    /// More calls than the server accepts (`maxCallsInRequest`) or than the crate builds.
    TooManyCalls {
        /// The limit that applies.
        limit: u64,
    },
    /// A request body larger than the server accepts (`maxSizeRequest`).
    TooLarge {
        /// The limit that applies, in octets.
        limit: u64,
    },
    /// A request without any method call.
    NoCalls,
    /// The request could not be written as JSON.
    Serialisation,
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMethodName => f.write_str("the method name is empty, longer than 255 characters, or holds a space or non-ASCII character"),
            Self::InvalidCapability => {
                f.write_str("the capability identifier is empty, longer than 256 characters, or holds a space or non-ASCII character")
            }
            Self::UnsupportedCapability => {
                f.write_str("the request uses a capability this server's session does not offer; remove it or check the server")
            }
            Self::DuplicateArgument => f.write_str("an argument is given both directly and as a result reference (#name); keep only one"),
            Self::InvalidArgumentName => {
                f.write_str("a result reference argument name is empty or starts with #; give the plain name, the # is added")
            }
            Self::UnknownCall => f.write_str("a result reference names a call that is not earlier in this same request"),
            Self::InvalidPath => {
                f.write_str("a result reference path is not a JSON pointer: it must be empty or start with /, and use ~ only as ~0 or ~1")
            }
            Self::TooManyCalls { limit } => {
                write!(f, "the request holds more than {limit} method calls, the server's limit; send it in several requests")
            }
            Self::TooLarge { limit } => {
                write!(f, "the request body is larger than the server's limit of {limit} octets; send it in several requests")
            }
            Self::NoCalls => f.write_str("the request holds no method call, so there is nothing to send"),
            Self::Serialisation => f.write_str("the request could not be written out as JSON"),
        }
    }
}

/// Everything that can go wrong, arranged by what the mail client can do about it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The program's HTTP exchange failed before a response came (offline, timeout, TLS, …).
    Transport(TransportError),
    /// The server is busy or limiting the rate (429, 503, connection-level overload, a concurrency limit):
    /// try again later, after at least the delay it asked for when there is one.
    Busy {
        /// What the server asked for, if anything usable.
        retry_after: Option<RetryAfter>,
    },
    /// The credentials were refused (401, or 403 when the problem details say so): ask for them again.
    CredentialsRefused,
    /// The request was too large for the server (413, or a JMAP `limit` problem): split it.
    TooLarge {
        /// The limit the server named, if any.
        limit: Option<LimitName>,
    },
    /// The server refuses access to this resource (403).
    Forbidden {
        /// What the server said, cleaned.
        detail: Option<Excerpt>,
    },
    /// Nothing at that address (404, 410).
    NotFound,
    /// The server rejected the request as malformed or unsupported (400 and other client errors, or a JMAP
    /// request-level problem).
    Refused {
        /// The JMAP problem, if one was recognised.
        problem: Option<RequestProblem>,
        /// What the server said, cleaned.
        detail: Option<Excerpt>,
    },
    /// The server redirected; the crate follows nothing. The target has passed the transport rule and the
    /// loopback check; whether to follow (and whether to send the credentials to another origin) is the
    /// caller's decision.
    Redirected {
        /// Where the server points.
        target: Address,
        /// Whether the target has the same origin as the request.
        same_origin: bool,
    },
    /// The server redirected to an address that may not be used (plain http, loopback from a remote host,
    /// malformed).
    RedirectRefused(AddressError),
    /// The server answered something that cannot be used.
    Unusable(Unusable),
    /// Any other HTTP status.
    Http {
        /// The status code.
        status: u16,
        /// What the server said, cleaned.
        detail: Option<Excerpt>,
    },
    /// An address given to the crate is not usable.
    Address(AddressError),
    /// An address template could not be expanded.
    Template(TemplateError),
    /// Credentials given to the crate are not usable.
    Credentials(CredentialsError),
    /// An id given to the crate is not valid.
    Id(IdError),
    /// A request being built is wrong.
    Build(BuildError),
    /// The session offers no account for mail (RFC 8621 `urn:ietf:params:jmap:mail`).
    NoMailAccount,
}

impl Error {
    /// The delay before trying again, for [`Error::Busy`].
    pub fn retry_after(&self) -> Option<&RetryAfter> {
        match self {
            Self::Busy { retry_after } => retry_after.as_ref(),
            _ => None,
        }
    }
}

fn with_detail(f: &mut fmt::Formatter<'_>, text: &str, detail: Option<&Excerpt>) -> fmt::Result {
    f.write_str(text)?;
    if let Some(detail) = detail {
        write!(f, " (server said: \"{detail}\")")?;
    }
    Ok(())
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => write!(f, "the request did not complete: {error}"),
            Self::Busy {
                retry_after: Some(RetryAfter::Delay(delay)),
            } => write!(
                f,
                "the server is busy and asks to wait {} s before the next attempt",
                delay.as_secs()
            ),
            Self::Busy {
                retry_after: Some(RetryAfter::At(_)),
            } => f.write_str("the server is busy and named a time before which not to try again"),
            Self::Busy { retry_after: None } => {
                f.write_str("the server is busy or limiting requests; wait a little before trying again")
            }
            Self::CredentialsRefused => f.write_str(
                "the server rejected the user name and password (or the access token) for this account; check them, or sign in again if they were changed or have expired",
            ),
            Self::TooLarge { limit: Some(limit) } => write!(
                f,
                "the request went beyond the server's limit {limit}; send it in smaller parts"
            ),
            Self::TooLarge { limit: None } => {
                f.write_str("the request was larger than the server accepts; send it in smaller parts")
            }
            Self::Forbidden { detail } => with_detail(
                f,
                "the server does not let this account reach that resource",
                detail.as_ref(),
            ),
            Self::NotFound => f.write_str(
                "the server has nothing at that address; it may have been moved or deleted, so refresh and try again",
            ),
            Self::Refused {
                problem: Some(problem),
                detail,
            } => with_detail(
                f,
                &format!("the server rejected the request: {problem}"),
                detail.as_ref(),
            ),
            Self::Refused {
                problem: None,
                detail,
            } => with_detail(f, "the server rejected the request as invalid", detail.as_ref()),
            Self::Redirected {
                target,
                same_origin: true,
            } => write!(
                f,
                "the server moved this resource to another path on {}; it was not followed automatically",
                target.origin()
            ),
            Self::Redirected { target, .. } => write!(
                f,
                "the server sent this request on to {}; confirm before following, since the credentials would go there too",
                target.origin()
            ),
            Self::RedirectRefused(error) => write!(
                f,
                "the server redirected to an address that cannot be used: {error}"
            ),
            Self::Unusable(why) => write!(f, "the server's answer cannot be used: {why}"),
            Self::Http { status, detail } => with_detail(
                f,
                &format!("the server answered with HTTP status {status}, which this program cannot act on"),
                detail.as_ref(),
            ),
            Self::Address(error) => fmt::Display::fmt(error, f),
            Self::Template(error) => fmt::Display::fmt(error, f),
            Self::Credentials(error) => fmt::Display::fmt(error, f),
            Self::Id(error) => fmt::Display::fmt(error, f),
            Self::Build(error) => fmt::Display::fmt(error, f),
            Self::NoMailAccount => f.write_str(
                "the server offers no mail account to these credentials; check that mail is enabled for this account",
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::RedirectRefused(error) | Self::Address(error) => Some(error),
            Self::Template(error) => Some(error),
            Self::Credentials(error) => Some(error),
            Self::Id(error) => Some(error),
            _ => None,
        }
    }
}

impl From<TransportError> for Error {
    /// A connection-level overload is the server being busy; a body cut at the request's limit is the same
    /// failure as one the crate refuses after reading; anything else stays a transport failure.
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::Overloaded => Self::Busy { retry_after: None },
            TransportError::BodyTooLarge => Self::Unusable(Unusable::BodyTooLarge),
            other => Self::Transport(other),
        }
    }
}

impl From<AddressError> for Error {
    fn from(error: AddressError) -> Self {
        Self::Address(error)
    }
}

impl From<TemplateError> for Error {
    fn from(error: TemplateError) -> Self {
        Self::Template(error)
    }
}

impl From<CredentialsError> for Error {
    fn from(error: CredentialsError) -> Self {
        Self::Credentials(error)
    }
}

impl From<IdError> for Error {
    fn from(error: IdError) -> Self {
        Self::Id(error)
    }
}

impl From<BuildError> for Error {
    fn from(error: BuildError) -> Self {
        Self::Build(error)
    }
}

impl From<Unusable> for Error {
    fn from(why: Unusable) -> Self {
        Self::Unusable(why)
    }
}

#[cfg(test)]
mod tests;
