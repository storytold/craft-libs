//! The protocol-level error type shared by WSD clients.
//!
//! Service-specific crates (print, scan) wrap or convert this into their own error type, adding the
//! failures that only make sense for their service. Nothing here needs to be guessed from text:
//! transport failures have a [`TransportKind`] and a [`Stage`], HTTP failures a status and the
//! device's `Retry-After`, faults their typed [`Fault`], and [`Error::retry_safety`] says whether a
//! repeat could do the work twice.

use std::fmt;
use std::time::Duration;

use crate::fault::Fault;
use crate::retry::RetrySafety;

/// Why a transport failed, as a value instead of text to be guessed at. A [`crate::Transport`] of
/// any kind (the HTTP one, an in-process one, a test double) picks the closest kind; the message
/// carries the detail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransportKind {
    /// Nothing is listening at the address (connection refused).
    Refused,
    /// The host or network cannot be reached (no route, network down, connect failed otherwise).
    Unreachable,
    /// Connecting took longer than the timeout. Nothing was sent.
    ConnectTimeout,
    /// The request was sent (or may have been) and no complete reply came within the timeout.
    Timeout,
    /// The device answered with a redirect, which the policy refuses to follow.
    Redirect,
    /// The device closed or reset the connection without a reply. What an embedded server does
    /// with a request it cannot handle right now.
    ConnectionClosed,
    /// The peer did not speak HTTP: a bad status line, bad headers or framing, an oversized header
    /// block. Not retryable: asking again gets the same nonsense.
    InvalidResponse,
    /// An OS-level I/O error that matched no other kind (a broken pipe or reset that is not
    /// recognised as a close, a permission error). Retryable as transient.
    Io,
}

impl TransportKind {
    /// Every kind, for tables and exhaustive tests.
    pub const ALL: [Self; 8] = [
        Self::Refused,
        Self::Unreachable,
        Self::ConnectTimeout,
        Self::Timeout,
        Self::Redirect,
        Self::ConnectionClosed,
        Self::InvalidResponse,
        Self::Io,
    ];

    /// A stable lowercase name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Refused => "refused",
            Self::Unreachable => "unreachable",
            Self::ConnectTimeout => "connect_timeout",
            Self::Timeout => "timeout",
            Self::Redirect => "redirect",
            Self::ConnectionClosed => "connection_closed",
            Self::InvalidResponse => "invalid_response",
            Self::Io => "io",
        }
    }

    /// Where a failure of this kind happens unless the transport knows better: the connect kinds
    /// are [`Stage::Connect`]; the rest are [`Stage::Reply`], the conservative answer ("the request
    /// may have been delivered").
    pub const fn default_stage(self) -> Stage {
        match self {
            Self::Refused | Self::Unreachable | Self::ConnectTimeout => Stage::Connect,
            _ => Stage::Reply,
        }
    }

    /// Would the same request plausibly work if tried again later? Connection and timeout problems
    /// and other I/O errors yes; a refused redirect or a peer that does not speak HTTP no.
    pub const fn is_retryable(self) -> bool {
        !matches!(self, Self::Redirect | Self::InvalidResponse)
    }
}

impl fmt::Display for TransportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// How far an exchange got when it failed. This is what makes a retry safe or unsafe
/// ([`Error::retry_safety`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Stage {
    /// Before any traffic: validation, policy, cancellation, a missed deadline.
    Prepare,
    /// While connecting. Nothing was sent.
    Connect,
    /// While writing the request. Some of it may have arrived.
    Request,
    /// The request was sent (or may have been) and the reply is missing, late, refused or unusable.
    Reply,
}

impl Stage {
    /// A stable lowercase name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::Connect => "connect",
            Self::Request => "request",
            Self::Reply => "reply",
        }
    }

    /// Could the device have received (part of) the request?
    pub const fn may_have_reached_device(self) -> bool {
        matches!(self, Self::Request | Self::Reply)
    }
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Everything that can go wrong below the service layer.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The network policy refused (see `policy`): the URL, or a missing confirmation.
    #[error("refused by policy: {0}")]
    Policy(String),
    /// The transport failed before a usable HTTP reply arrived (connect, timeout, redirect refused,
    /// connection closed, ...). Build one with [`Error::transport`] or [`Error::transport_at`].
    #[error("transport: {message}")]
    Transport {
        kind: TransportKind,
        /// How far the exchange got.
        stage: Stage,
        message: String,
    },
    /// An HTTP status that is not success, with no SOAP fault in the body. Build one with
    /// [`Error::http`] or [`Error::http_retry_after`].
    #[error("{}", http_text(*status, *retry_after))]
    Http {
        status: u16,
        /// The wait the device asked for with `Retry-After`, bounded by [`crate::MAX_RETRY_AFTER`].
        retry_after: Option<Duration>,
    },
    /// A size cap was hit (what, the limit in bytes or items).
    #[error("{0} is larger than the limit of {1}")]
    TooLarge(&'static str, usize),
    /// The reply is not well-formed XML (or uses something refused, such as a DTD).
    #[error("malformed XML: {0}")]
    Xml(String),
    /// Well-formed, but not the message that was expected.
    #[error("unexpected reply: {0}")]
    Protocol(String),
    /// The device answered with a SOAP fault. Boxed so `Result<_, Error>` stays small; build one
    /// with [`Error::from_fault`] (or `Fault::into`), and read it with [`Error::fault`].
    #[error("{0}")]
    Fault(Box<Fault>),
    /// An argument or option is invalid (checked before anything is sent).
    #[error("invalid {0}")]
    Invalid(String),
    /// The caller's [`crate::CancelToken`] was set; nothing further was sent.
    #[error("cancelled")]
    Cancelled,
    /// The caller's overall deadline had passed before the exchange could start; nothing was sent.
    #[error("the deadline passed")]
    Timeout,
    /// The device does not offer what was asked for (a hosted service of some type, a usable address).
    #[error("not found: {0}")]
    NotFound(String),
}

fn http_text(status: u16, retry_after: Option<Duration>) -> String {
    match retry_after {
        Some(wait) => format!("the device answered HTTP {status} (retry after {}s)", wait.as_secs()),
        None => format!("the device answered HTTP {status}"),
    }
}

impl From<Fault> for Error {
    fn from(fault: Fault) -> Self {
        Self::from_fault(fault)
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Every value [`Error::kind`] can return.
pub const KINDS: &[&str] = &["policy", "transport", "http", "too_large", "xml", "protocol", "fault", "invalid", "cancelled", "timeout", "not_found"];

impl Error {
    /// A transport failure of `kind`, at the stage that kind usually happens in
    /// ([`TransportKind::default_stage`]). For transports outside this crate.
    pub fn transport(kind: TransportKind, message: impl Into<String>) -> Self {
        Self::transport_at(kind, kind.default_stage(), message)
    }

    /// A transport failure of `kind` that happened at `stage`.
    pub fn transport_at(kind: TransportKind, stage: Stage, message: impl Into<String>) -> Self {
        Self::Transport { kind, stage, message: message.into() }
    }

    /// An HTTP status error without a `Retry-After`.
    pub fn http(status: u16) -> Self {
        Self::Http { status, retry_after: None }
    }

    /// An HTTP status error with the device's `Retry-After`, if it sent a usable one.
    pub fn http_retry_after(status: u16, retry_after: Option<Duration>) -> Self {
        Self::Http { status, retry_after }
    }

    /// A SOAP fault as an error.
    pub fn from_fault(fault: Fault) -> Self {
        Self::Fault(Box::new(fault))
    }

    /// A stable lowercase name for the variant, one of [`KINDS`].
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Policy(_) => "policy",
            Self::Transport { .. } => "transport",
            Self::Http { .. } => "http",
            Self::TooLarge(..) => "too_large",
            Self::Xml(_) => "xml",
            Self::Protocol(_) => "protocol",
            Self::Fault(_) => "fault",
            Self::Invalid(_) => "invalid",
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
            Self::NotFound(_) => "not_found",
        }
    }

    /// Why the transport failed, for [`Self::Transport`].
    pub fn transport_kind(&self) -> Option<TransportKind> {
        match self {
            Self::Transport { kind, .. } => Some(*kind),
            _ => None,
        }
    }

    /// The HTTP status, for [`Self::Http`].
    pub fn http_status(&self) -> Option<u16> {
        match self {
            Self::Http { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// The wait the device asked for (`Retry-After`), on an HTTP error or a fault that came with one.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Http { retry_after, .. } => *retry_after,
            Self::Fault(f) => f.retry_after,
            _ => None,
        }
    }

    /// Would trying the same request again later plausibly work? Transport failures other than a
    /// refused redirect, HTTP 5xx and 429. Whether a fault sub-code is retryable is a property of
    /// the service, so it is decided by the service crate. This says nothing about whether
    /// repeating is *safe*: see [`Self::retry_safety`].
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Http { status, .. } => *status == 429 || (500..600).contains(status),
            Self::Transport { kind, .. } => kind.is_retryable(),
            _ => false,
        }
    }

    /// How far the exchange got when this error happened.
    ///
    /// Errors raised before anything is sent are [`Stage::Prepare`]; a transport error says for
    /// itself; every error that comes from a reply (HTTP status, fault, XML, protocol, size) is
    /// [`Stage::Reply`]. `TooLarge` always counts as a reply, even for a request refused before
    /// sending, which errs on the side of caution.
    pub fn stage(&self) -> Stage {
        match self {
            Self::Policy(_) | Self::Invalid(_) | Self::Cancelled | Self::Timeout | Self::NotFound(_) => Stage::Prepare,
            Self::Transport { stage, .. } => *stage,
            Self::Http { .. } | Self::TooLarge(..) | Self::Xml(_) | Self::Protocol(_) | Self::Fault(_) => Stage::Reply,
        }
    }

    /// Whether repeating the request can do the work twice. `idempotent` says whether the caller
    /// knows that repeating *this request* is harmless (a query, a cancel).
    ///
    /// * Nothing was sent (policy, cancelled, connect failed, ...): [`RetrySafety::Safe`].
    /// * The device answered HTTP 429 or 503, which means it declined: [`RetrySafety::Safe`].
    /// * Otherwise the request may have been delivered: [`RetrySafety::Idempotent`] if `idempotent`,
    ///   else [`RetrySafety::OutcomeUnknown`]. A fault is in this group: whether a particular
    ///   fault means "not accepted" is for the service crate to say.
    pub fn retry_safety(&self, idempotent: bool) -> RetrySafety {
        if !self.stage().may_have_reached_device() || matches!(self, Self::Http { status: 429 | 503, .. }) {
            RetrySafety::Safe
        } else if idempotent {
            RetrySafety::Idempotent
        } else {
            RetrySafety::OutcomeUnknown
        }
    }

    /// The fault, when the device sent one.
    pub fn fault(&self) -> Option<&Fault> {
        match self {
            Self::Fault(f) => Some(f.as_ref()),
            _ => None,
        }
    }

    /// The fault by value, when the device sent one.
    pub fn into_fault(self) -> Option<Fault> {
        match self {
            Self::Fault(f) => Some(*f),
            _ => None,
        }
    }

    /// Is this a fault whose first sub-code is exactly `{ns_uri}local`?
    pub fn is_fault(&self, ns_uri: &str, local: &str) -> bool {
        self.fault().is_some_and(|f| f.code.is(ns_uri, local))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fault::{Role, SubCode};

    fn fault() -> Fault {
        Fault::new(Role::Receiver, SubCode::new(Some("urn:x"), "Busy"), "busy", "")
    }

    fn all() -> Vec<Error> {
        let mut all = vec![
            Error::Policy(String::new()),
            Error::http(500),
            Error::TooLarge("x", 1),
            Error::Xml(String::new()),
            Error::Protocol(String::new()),
            Error::Invalid(String::new()),
            Error::Cancelled,
            Error::Timeout,
            Error::NotFound(String::new()),
            Error::from_fault(fault()),
        ];
        all.extend(TransportKind::ALL.map(|k| Error::transport(k, "t")));
        all
    }

    #[test]
    fn kinds_are_listed_and_every_variant_has_one() {
        let mut seen: Vec<&str> = all().iter().map(Error::kind).collect();
        seen.sort_unstable();
        seen.dedup();
        let mut listed = KINDS.to_vec();
        listed.sort_unstable();
        assert_eq!(seen, listed);
    }

    #[test]
    fn retryability_follows_the_status_and_the_transport_kind() {
        for (s, r) in [(500, true), (502, true), (503, true), (429, true), (404, false), (400, false), (403, false), (301, false)] {
            assert_eq!(Error::http(s).is_retryable(), r, "{s}");
        }
        for k in TransportKind::ALL {
            assert_eq!(Error::transport(k, "t").is_retryable(), !matches!(k, TransportKind::Redirect | TransportKind::InvalidResponse), "{k}");
        }
        assert!(!Error::Protocol(String::new()).is_retryable());
        assert!(!Error::Cancelled.is_retryable() && !Error::Timeout.is_retryable());
        assert!(!Error::from_fault(fault()).is_retryable());
    }

    #[test]
    fn transport_errors_carry_kind_stage_and_message() {
        let e = Error::transport(TransportKind::Refused, "no listener");
        assert_eq!(e.to_string(), "transport: no listener");
        assert_eq!((e.transport_kind(), e.stage(), e.kind()), (Some(TransportKind::Refused), Stage::Connect, "transport"));
        // The default stage is the conservative one for everything that is not a connect failure.
        for k in TransportKind::ALL {
            let connect = matches!(k, TransportKind::Refused | TransportKind::Unreachable | TransportKind::ConnectTimeout);
            assert_eq!(Error::transport(k, "").stage(), if connect { Stage::Connect } else { Stage::Reply }, "{k}");
        }
        let e = Error::transport_at(TransportKind::ConnectionClosed, Stage::Request, "reset while writing");
        assert_eq!((e.stage(), e.transport_kind()), (Stage::Request, Some(TransportKind::ConnectionClosed)));
        assert_eq!(Error::http(500).transport_kind(), None);
    }

    #[test]
    fn http_errors_carry_status_and_retry_after() {
        let e = Error::http_retry_after(503, Some(Duration::from_secs(7)));
        assert_eq!((e.http_status(), e.retry_after()), (Some(503), Some(Duration::from_secs(7))));
        assert_eq!(e.to_string(), "the device answered HTTP 503 (retry after 7s)");
        assert_eq!(Error::http(503).to_string(), "the device answered HTTP 503");
        assert_eq!(Error::http(503).retry_after(), None);
        assert_ne!(Error::http(503), e, "the wait is part of the value");
        assert_eq!(Error::Cancelled.http_status(), None);
        assert_eq!(Error::from_fault(fault().with_retry_after(Some(Duration::from_secs(3)))).retry_after(), Some(Duration::from_secs(3)));
    }

    #[test]
    fn stages_and_retry_safety() {
        use RetrySafety::{Idempotent, OutcomeUnknown, Safe};
        let t = |k, s| Error::transport_at(k, s, "t");
        // (error, stage, safety for a submit, safety for a query)
        let cases: Vec<(Error, Stage, RetrySafety, RetrySafety)> = vec![
            (Error::Policy(String::new()), Stage::Prepare, Safe, Safe),
            (Error::Invalid(String::new()), Stage::Prepare, Safe, Safe),
            (Error::Cancelled, Stage::Prepare, Safe, Safe),
            (Error::Timeout, Stage::Prepare, Safe, Safe),
            (Error::NotFound(String::new()), Stage::Prepare, Safe, Safe),
            (t(TransportKind::Refused, Stage::Connect), Stage::Connect, Safe, Safe),
            (t(TransportKind::ConnectTimeout, Stage::Connect), Stage::Connect, Safe, Safe),
            (t(TransportKind::ConnectionClosed, Stage::Request), Stage::Request, OutcomeUnknown, Idempotent),
            (t(TransportKind::Timeout, Stage::Reply), Stage::Reply, OutcomeUnknown, Idempotent),
            (Error::transport(TransportKind::ConnectionClosed, ""), Stage::Reply, OutcomeUnknown, Idempotent),
            (Error::transport(TransportKind::Redirect, ""), Stage::Reply, OutcomeUnknown, Idempotent),
            (Error::transport(TransportKind::InvalidResponse, ""), Stage::Reply, OutcomeUnknown, Idempotent),
            (Error::transport(TransportKind::Io, ""), Stage::Reply, OutcomeUnknown, Idempotent),
            (Error::http(503), Stage::Reply, Safe, Safe),
            (Error::http(429), Stage::Reply, Safe, Safe),
            (Error::http(500), Stage::Reply, OutcomeUnknown, Idempotent),
            (Error::http(404), Stage::Reply, OutcomeUnknown, Idempotent),
            (Error::Xml(String::new()), Stage::Reply, OutcomeUnknown, Idempotent),
            (Error::Protocol(String::new()), Stage::Reply, OutcomeUnknown, Idempotent),
            (Error::TooLarge("x", 1), Stage::Reply, OutcomeUnknown, Idempotent),
            (Error::from_fault(fault()), Stage::Reply, OutcomeUnknown, Idempotent),
        ];
        for (e, stage, submit, query) in cases {
            assert_eq!((e.stage(), e.retry_safety(false), e.retry_safety(true)), (stage, submit, query), "{e:?}");
        }
        assert!(!Stage::Prepare.may_have_reached_device() && !Stage::Connect.may_have_reached_device());
        assert!(Stage::Request.may_have_reached_device() && Stage::Reply.may_have_reached_device());
    }

    #[test]
    fn fault_accessors() {
        let e = Error::from_fault(fault());
        assert_eq!(e.fault().map(|f| f.code.local.as_str()), Some("Busy"));
        assert!(e.is_fault("urn:x", "Busy") && !e.is_fault("urn:y", "Busy") && !e.is_fault("urn:x", "Other"));
        assert_eq!(e.clone().into_fault(), Some(fault()));
        assert!(!Error::http(500).is_fault("urn:x", "Busy") && Error::http(500).fault().is_none());
        assert_eq!(Error::http(500).into_fault(), None);
    }

    #[test]
    fn names_are_stable() {
        assert_eq!(
            TransportKind::ALL.map(TransportKind::name),
            ["refused", "unreachable", "connect_timeout", "timeout", "redirect", "connection_closed", "invalid_response", "io"]
        );
        assert_eq!([Stage::Prepare, Stage::Connect, Stage::Request, Stage::Reply].map(Stage::name), ["prepare", "connect", "request", "reply"]);
        assert_eq!(Stage::Reply.to_string(), "reply");
        assert_eq!(TransportKind::Timeout.to_string(), "timeout");
    }
}
