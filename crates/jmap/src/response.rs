//! Reading an API response (RFC 8620 §3.4) into per-call results, where a method-level error (§3.6.2) is the
//! failure of that call only.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::{Map, Value};

use crate::classify::parse_json;
use crate::error::{Error, Unusable};
use crate::id::Id;
use crate::redact::{Excerpt, Redactor};
use crate::request::{CallId, MAX_METHOD_NAME_LEN};
use crate::session::MAX_STATE_LEN;

/// The most method responses one API response may hold.
pub const MAX_METHOD_RESPONSES: usize = 4096;
/// The most entries of `createdIds` read.
pub const MAX_CREATED_IDS: usize = 65_536;
/// The longest call id read back.
const MAX_CALL_ID_LEN: usize = 255;
/// The longest error type kept.
const MAX_ERROR_TYPE_LEN: usize = 64;

/// The type of a method-level error (RFC 8620 §3.6.2, and the method-specific ones of §5).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MethodErrorKind {
    /// `serverUnavailable`: a resource was temporarily unavailable; trying later may work.
    ServerUnavailable,
    /// `serverFail`: an unexpected error; trying again is expected to fail.
    ServerFail,
    /// `serverPartialFail`: some changes happened; resynchronise.
    ServerPartialFail,
    /// `unknownMethod`
    UnknownMethod,
    /// `invalidArguments`
    InvalidArguments,
    /// `invalidResultReference`
    InvalidResultReference,
    /// `forbidden`
    Forbidden,
    /// `accountNotFound`
    AccountNotFound,
    /// `accountNotSupportedByMethod`
    AccountNotSupportedByMethod,
    /// `accountReadOnly`
    AccountReadOnly,
    /// `requestTooLarge` (§5.1, §5.3)
    RequestTooLarge,
    /// `cannotCalculateChanges` (§5.2, §5.6)
    CannotCalculateChanges,
    /// `stateMismatch` (§5.3, §5.4)
    StateMismatch,
    /// `fromAccountNotFound` (§5.4)
    FromAccountNotFound,
    /// `fromAccountNotSupportedByMethod` (§5.4)
    FromAccountNotSupportedByMethod,
    /// `anchorNotFound` (§5.5)
    AnchorNotFound,
    /// `unsupportedSort` (§5.5)
    UnsupportedSort,
    /// `unsupportedFilter` (§5.5)
    UnsupportedFilter,
    /// `tooManyChanges` (§5.6)
    TooManyChanges,
    /// Any other type, checked to be a short identifier (empty when the server gave none or a bad one). RFC 8620
    /// §3.6.2: a client must treat a type it does not understand like `serverFail`.
    Other(String),
}

impl MethodErrorKind {
    fn from_type(kind: Option<&str>) -> Self {
        match kind.unwrap_or("") {
            "serverUnavailable" => Self::ServerUnavailable,
            "serverFail" => Self::ServerFail,
            "serverPartialFail" => Self::ServerPartialFail,
            "unknownMethod" => Self::UnknownMethod,
            "invalidArguments" => Self::InvalidArguments,
            "invalidResultReference" => Self::InvalidResultReference,
            "forbidden" => Self::Forbidden,
            "accountNotFound" => Self::AccountNotFound,
            "accountNotSupportedByMethod" => Self::AccountNotSupportedByMethod,
            "accountReadOnly" => Self::AccountReadOnly,
            "requestTooLarge" => Self::RequestTooLarge,
            "cannotCalculateChanges" => Self::CannotCalculateChanges,
            "stateMismatch" => Self::StateMismatch,
            "fromAccountNotFound" => Self::FromAccountNotFound,
            "fromAccountNotSupportedByMethod" => Self::FromAccountNotSupportedByMethod,
            "anchorNotFound" => Self::AnchorNotFound,
            "unsupportedSort" => Self::UnsupportedSort,
            "unsupportedFilter" => Self::UnsupportedFilter,
            "tooManyChanges" => Self::TooManyChanges,
            other if other.len() <= MAX_ERROR_TYPE_LEN && other.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') => {
                Self::Other(other.to_owned())
            }
            _ => Self::Other(String::new()),
        }
    }

    /// The type's name as the server sent it (empty for an unrecognisable one).
    pub fn as_str(&self) -> &str {
        match self {
            Self::ServerUnavailable => "serverUnavailable",
            Self::ServerFail => "serverFail",
            Self::ServerPartialFail => "serverPartialFail",
            Self::UnknownMethod => "unknownMethod",
            Self::InvalidArguments => "invalidArguments",
            Self::InvalidResultReference => "invalidResultReference",
            Self::Forbidden => "forbidden",
            Self::AccountNotFound => "accountNotFound",
            Self::AccountNotSupportedByMethod => "accountNotSupportedByMethod",
            Self::AccountReadOnly => "accountReadOnly",
            Self::RequestTooLarge => "requestTooLarge",
            Self::CannotCalculateChanges => "cannotCalculateChanges",
            Self::StateMismatch => "stateMismatch",
            Self::FromAccountNotFound => "fromAccountNotFound",
            Self::FromAccountNotSupportedByMethod => "fromAccountNotSupportedByMethod",
            Self::AnchorNotFound => "anchorNotFound",
            Self::UnsupportedSort => "unsupportedSort",
            Self::UnsupportedFilter => "unsupportedFilter",
            Self::TooManyChanges => "tooManyChanges",
            Self::Other(name) => name,
        }
    }

    /// Whether the same call may succeed later unchanged (only `serverUnavailable`, RFC 8620 §3.6.2).
    pub fn may_succeed_later(&self) -> bool {
        matches!(self, Self::ServerUnavailable)
    }
}

/// A method-level error (RFC 8620 §3.6.2).
#[derive(Clone, PartialEq)]
pub struct MethodError {
    kind: MethodErrorKind,
    description: Option<Excerpt>,
    arguments: Map<String, Value>,
}

impl MethodError {
    /// The error type.
    pub fn kind(&self) -> &MethodErrorKind {
        &self.kind
    }

    /// The server's `description`, cleaned (RFC 8620 §3.6.2: not meant for end users as is).
    pub fn description(&self) -> Option<&Excerpt> {
        self.description.as_ref()
    }

    /// All the error's arguments, for types that carry more (raw server data: do not show or log them).
    pub fn arguments(&self) -> &Map<String, Value> {
        &self.arguments
    }
}

impl fmt::Debug for MethodError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MethodError").field("kind", &self.kind).field("description", &self.description).finish_non_exhaustive()
    }
}

impl fmt::Display for MethodError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.kind.as_str().is_empty() {
            f.write_str("the server refused this call without saying why")?;
        } else {
            write!(f, "the server refused this call with the error {}", self.kind.as_str())?;
        }
        if let Some(description) = &self.description {
            write!(f, ": \"{description}\"")?;
        }
        Ok(())
    }
}

impl std::error::Error for MethodError {}

/// One response of a call (RFC 8620 §3.2).
#[derive(Clone, PartialEq)]
pub struct MethodResponse {
    name: String,
    arguments: Map<String, Value>,
}

impl MethodResponse {
    /// The response name (the method's name, for a successful call).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The response's arguments.
    pub fn arguments(&self) -> &Map<String, Value> {
        &self.arguments
    }

    /// The response's arguments, taken.
    pub fn into_arguments(self) -> Map<String, Value> {
        self.arguments
    }
}

impl fmt::Debug for MethodResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MethodResponse").field("name", &self.name).field("arguments", &self.arguments.len()).finish()
    }
}

/// Why a call has no usable result.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CallFailure {
    /// The server answered the call with an error.
    Method(MethodError),
    /// The response holds nothing for this call id (or it was already taken).
    Missing,
    /// The call's responses do not include the name asked for.
    UnexpectedResponse,
}

impl fmt::Display for CallFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Method(error) => fmt::Display::fmt(error, f),
            Self::Missing => f.write_str("the server's answer holds no result for this call; the server may have stopped before reaching it"),
            Self::UnexpectedResponse => f.write_str("the server answered this call with a response of another kind than the one asked for"),
        }
    }
}

impl std::error::Error for CallFailure {}

type CallResult = Result<Vec<MethodResponse>, MethodError>;

/// A parsed API response.
#[derive(Clone, PartialEq)]
pub struct ApiResponse {
    session_state: String,
    created_ids: Option<BTreeMap<Id, Id>>,
    calls: BTreeMap<String, CallResult>,
}

fn shape(what: &'static str) -> Error {
    Error::Unusable(Unusable::Shape(what))
}

fn is_name(text: &str, max: usize) -> bool {
    !text.is_empty() && text.len() <= max && text.bytes().all(|b| (0x21..=0x7E).contains(&b))
}

impl ApiResponse {
    /// Reads a Response object (RFC 8620 §3.4). `redactor` cleans the descriptions of method errors.
    pub fn parse(body: &[u8], redactor: &Redactor) -> Result<Self, Error> {
        let Value::Object(mut object) = parse_json(body)? else {
            return Err(shape("the response is not a JSON object"));
        };
        let session_state = match object.remove("sessionState") {
            Some(Value::String(state)) if state.len() <= MAX_STATE_LEN => state,
            _ => return Err(shape("the response has no valid sessionState")),
        };
        let Some(Value::Array(responses)) = object.remove("methodResponses") else {
            return Err(shape("the response has no methodResponses array"));
        };
        if responses.len() > MAX_METHOD_RESPONSES {
            return Err(shape("the response holds too many method responses"));
        }
        let created_ids = match object.remove("createdIds") {
            None | Some(Value::Null) => None,
            Some(Value::Object(map)) if map.len() <= MAX_CREATED_IDS => {
                Some(map.iter().filter_map(|(key, value)| Some((Id::new(key).ok()?, Id::new(value.as_str()?).ok()?))).collect())
            }
            Some(_) => return Err(shape("createdIds is not an object of ids")),
        };
        let mut calls: BTreeMap<String, CallResult> = BTreeMap::new();
        for invocation in responses {
            let Value::Array(parts) = invocation else {
                return Err(shape("a method response is not an array"));
            };
            let mut parts = parts.into_iter();
            let (Some(Value::String(name)), Some(Value::Object(arguments)), Some(Value::String(call_id)), None) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                return Err(shape("a method response is not a [name, arguments, call id] triple"));
            };
            if !is_name(&name, MAX_METHOD_NAME_LEN) {
                return Err(shape("a method response has an invalid name"));
            }
            if call_id.len() > MAX_CALL_ID_LEN {
                return Err(shape("a method response has an overlong call id"));
            }
            let entry = calls.entry(call_id).or_insert_with(|| Ok(Vec::new()));
            let Ok(list) = entry else {
                // RFC 8620 §3.6.2: after an error, nothing more happens within that call.
                continue;
            };
            if name == "error" {
                let mut kind = MethodErrorKind::from_type(arguments.get("type").and_then(Value::as_str));
                if let MethodErrorKind::Other(name) = &kind
                    && redactor.mentions_secret(name)
                {
                    kind = MethodErrorKind::Other(String::new());
                }
                let description = arguments.get("description").and_then(Value::as_str).map(|text| redactor.excerpt(text));
                *entry = Err(MethodError { kind, description, arguments });
            } else {
                list.push(MethodResponse { name, arguments });
            }
        }
        Ok(Self { session_state, created_ids, calls })
    }

    /// The session state the server reported (RFC 8620 §3.4 `sessionState`).
    pub fn session_state(&self) -> &str {
        &self.session_state
    }

    /// The `createdIds` the server returned, valid entries only.
    pub fn created_ids(&self) -> Option<&BTreeMap<Id, Id>> {
        self.created_ids.as_ref()
    }

    /// Takes every response of a call; a method-level error makes the call fail.
    pub fn take(&mut self, call: &CallId) -> Result<Vec<MethodResponse>, CallFailure> {
        match self.calls.remove(call.as_str()) {
            Some(Ok(responses)) if !responses.is_empty() => Ok(responses),
            Some(Ok(_)) | None => Err(CallFailure::Missing),
            Some(Err(error)) => Err(CallFailure::Method(error)),
        }
    }

    /// Takes the arguments of the call's response named `name` (for a plain call, the method's own name).
    pub fn take_named(&mut self, call: &CallId, name: &str) -> Result<Map<String, Value>, CallFailure> {
        self.take(call)?.into_iter().find(|response| response.name == name).map(MethodResponse::into_arguments).ok_or(CallFailure::UnexpectedResponse)
    }

    /// The call ids present in the response, untaken.
    pub fn call_ids(&self) -> impl Iterator<Item = &str> {
        self.calls.keys().map(String::as_str)
    }
}

impl fmt::Debug for ApiResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiResponse").field("session_state", &self.session_state).field("calls", &self.calls.len()).finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
