//! The IPP layer's error type, and its mapping onto [`crate::PrintError`].
//!
//! The principle is the WS-Print layer's: **a failure that proves the printer did not act is
//! [`RetrySafety::Safe`]** (nothing sent, a refused address, a connect failure, a policy refusal, an
//! IPP status that declines the request); **a failure after the request may have arrived is
//! [`RetrySafety::OutcomeUnknown`] for an operation that creates something** (`Print-Job`,
//! `Create-Job`, `Send-Document`) and [`RetrySafety::Idempotent`] for a read or a cancel. The IPP
//! status table is in `docs/ipp.md` and pinned by the tests below.

use std::fmt;
use std::time::Duration;

use super::adapter::{Stage, TransportKind, WireError, http_safety, transport_safety};
use super::protocol::{Operation, StatusCode};
use crate::{ErrorKind, NativeCode, Phase, PrintError, RetrySafety};

/// Everything that can go wrong, as values an interface can show and an agent can act on.
#[derive(Debug, Clone, PartialEq)]
pub enum IppError {
    /// The network policy refused (the URL, a missing confirmation).
    Policy(String),
    /// `ipps://` or `https://`: TLS is not implemented.
    Tls(String),
    /// The transport failed before a usable HTTP reply arrived.
    Transport { kind: TransportKind, stage: Stage, message: String },
    /// An HTTP status other than 200, and the wait the printer asked for with `Retry-After`.
    Http { status: u16, retry_after: Option<Duration> },
    /// A size cap was hit (what, the limit in bytes or items).
    TooLarge(&'static str, usize),
    /// The reply is not a well-formed IPP message.
    Codec(String),
    /// A well-formed message that is not the reply that was expected.
    Protocol(String),
    /// The printer answered with an IPP status that is not a success.
    Status { code: StatusCode, message: String, retry_after: Option<Duration>, unsupported: Vec<String> },
    /// An argument or option is invalid (checked before anything is sent).
    Invalid(String),
    /// A requested setting is outside what the printer lists (strict mode).
    Unsupported(String),
    /// The printer lists formats and none of them is the document's.
    UnsupportedFormat { document: String, accepted: Vec<String> },
    /// The caller cancelled; nothing further was sent.
    Cancelled,
    /// The caller's deadline had passed before the exchange could start; nothing was sent.
    Timeout,
    /// The printer or job is not known.
    NotFound(String),
}

impl fmt::Display for IppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Policy(m) => write!(f, "refused by policy: {m}"),
            Self::Tls(m) => write!(f, "TLS is not supported: {m}"),
            Self::Transport { message, .. } => write!(f, "transport: {message}"),
            Self::Http { status, retry_after } => {
                write!(f, "the printer answered HTTP {status}")?;
                match status {
                    401 | 403 => f.write_str(" (it wants authentication, which is not implemented)")?,
                    426 => f.write_str(" (it wants TLS, which is not implemented)")?,
                    _ => {}
                }
                if let Some(wait) = retry_after {
                    write!(f, " (retry after {}s)", wait.as_secs())?;
                }
                Ok(())
            }
            Self::TooLarge(what, n) => write!(f, "{what} is larger than the limit of {n}"),
            Self::Codec(m) => write!(f, "unreadable reply: {m}"),
            Self::Protocol(m) => write!(f, "unexpected reply: {m}"),
            Self::Status { code, message, unsupported, .. } => {
                match code.name() {
                    Some(name) => write!(f, "the printer answered {name} ({:#06x})", code.0)?,
                    None => write!(f, "the printer answered status {:#06x}", code.0)?,
                }
                if !message.is_empty() {
                    write!(f, ": {message}")?;
                }
                if !unsupported.is_empty() {
                    write!(f, " (unsupported: {})", unsupported.join(", "))?;
                }
                if matches!(code.0, 0x0401..=0x0403) {
                    f.write_str(" (authentication is not implemented)")?;
                }
                Ok(())
            }
            Self::Invalid(m) => write!(f, "invalid {m}"),
            Self::Unsupported(m) => write!(f, "the printer does not support {m}"),
            Self::UnsupportedFormat { document, accepted } => {
                write!(f, "the printer does not accept {document} (it accepts: {})", accepted.join(", "))
            }
            Self::Cancelled => f.write_str("cancelled"),
            Self::Timeout => f.write_str("the deadline passed"),
            Self::NotFound(m) => write!(f, "not found: {m}"),
        }
    }
}

impl std::error::Error for IppError {}

impl From<WireError> for IppError {
    fn from(e: WireError) -> Self {
        match e {
            WireError::Policy(m) => Self::Policy(m),
            WireError::Transport { kind, stage, message } => Self::Transport { kind, stage, message },
            WireError::Http { status, retry_after } => Self::Http { status, retry_after },
            WireError::Cancelled => Self::Cancelled,
            WireError::Timeout => Self::Timeout,
            WireError::NotFound(m) => Self::NotFound(m),
            WireError::TooLarge(what, n) => Self::TooLarge(what, n),
            WireError::Xml(m) | WireError::Protocol(m) => Self::Protocol(m),
            WireError::Invalid(m) => Self::Invalid(m),
            other => Self::Protocol(other.to_string()),
        }
    }
}

impl From<super::codec::CodecError> for IppError {
    fn from(e: super::codec::CodecError) -> Self {
        Self::Codec(e.to_string())
    }
}

pub type Result<T, E = IppError> = std::result::Result<T, E>;

/// The protocol step that was in flight; it decides whether repeating is safe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    /// `Get-Printer-Attributes`, `Get-Jobs`, `Validate-Job`: reads.
    Query,
    /// `Create-Job`.
    Create,
    /// `Print-Job` and `Send-Document`.
    Send,
    /// `Get-Job-Attributes` while following a job.
    Status,
    /// `Cancel-Job`.
    Cancel,
}

impl Op {
    /// Repeating the request changes nothing the first one did not.
    pub const fn is_idempotent(self) -> bool {
        matches!(self, Self::Query | Self::Status | Self::Cancel)
    }

    pub const ALL: [Op; 5] = [Op::Query, Op::Create, Op::Send, Op::Status, Op::Cancel];

    /// The step an operation is, when it is not a status poll.
    pub const fn of(operation: Operation) -> Self {
        match operation {
            Operation::PrintJob | Operation::SendDocument => Self::Send,
            Operation::CreateJob => Self::Create,
            Operation::CancelJob => Self::Cancel,
            Operation::ValidateJob | Operation::GetJobAttributes | Operation::GetJobs | Operation::GetPrinterAttributes => Self::Query,
        }
    }
}

/// How an error reads to a caller; see [`IppError::classify`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Class {
    pub kind: ErrorKind,
    pub phase: Phase,
    pub safety: RetrySafety,
    /// How long to wait before another try, for the conditions where the printer is merely busy.
    pub retry_after: Option<Duration>,
}

/// Every value [`IppError::kind`] can return.
pub const KINDS: &[&str] = &[
    "policy",
    "tls",
    "transport",
    "http",
    "too_large",
    "codec",
    "protocol",
    "status",
    "invalid",
    "unsupported",
    "unsupported_format",
    "cancelled",
    "timeout",
    "not_found",
];

/// The [`IppError::TooLarge`] labels that name something this side sends.
const REQUEST_LABELS: &[&str] = &["the request", "the document"];

impl IppError {
    /// A stable lowercase name for the variant, one of [`KINDS`].
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Policy(_) => "policy",
            Self::Tls(_) => "tls",
            Self::Transport { .. } => "transport",
            Self::Http { .. } => "http",
            Self::TooLarge(..) => "too_large",
            Self::Codec(_) => "codec",
            Self::Protocol(_) => "protocol",
            Self::Status { .. } => "status",
            Self::Invalid(_) => "invalid",
            Self::Unsupported(_) => "unsupported",
            Self::UnsupportedFormat { .. } => "unsupported_format",
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
            Self::NotFound(_) => "not_found",
        }
    }

    /// The printer's status code, when it sent one.
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            Self::Status { code, .. } => Some(*code),
            _ => None,
        }
    }

    /// Would trying the same request again later plausibly work (exactly
    /// [`ErrorKind::is_retryable`] of [`Self::classify`]'s kind)? Whether it is *safe* depends on the
    /// step too.
    pub fn is_retryable(&self) -> bool {
        self.classify(Op::Send).kind.is_retryable()
    }

    /// The whole reading of this error for a step: kind, phase, safety and wait.
    pub fn classify(&self, op: Op) -> Class {
        use ErrorKind as K;
        use Phase as P;
        use RetrySafety as S;
        let reached = if op.is_idempotent() { S::Idempotent } else { S::OutcomeUnknown };
        let net = |phase: Phase| if op == Op::Status { P::Poll } else { phase };
        let class = |kind, phase, safety| Class { kind, phase, safety, retry_after: None };
        match self {
            Self::Policy(_) | Self::Tls(_) | Self::Invalid(_) | Self::Unsupported(_) => class(K::InvalidRequest, P::Prepare, S::Safe),
            Self::UnsupportedFormat { .. } => class(K::UnsupportedFormat, P::Prepare, S::Safe),
            Self::NotFound(_) => class(K::NotFound, P::Prepare, S::Safe),
            Self::Cancelled => class(K::Cancelled, P::Prepare, S::Safe),
            Self::TooLarge(what, _) if REQUEST_LABELS.contains(what) => class(K::InvalidRequest, P::Prepare, S::Safe),
            Self::TooLarge(..) | Self::Codec(_) | Self::Protocol(_) => class(K::Failed, net(P::Reply), reached),
            // The deadline had already passed when the exchange was about to start (an exchange that
            // runs past it ends as a transport timeout instead): nothing was sent.
            Self::Timeout => class(K::Timeout, P::Prepare, S::Safe),
            Self::Transport { kind, stage, .. } => {
                use TransportKind as T;
                let kind_of = match kind {
                    T::ConnectTimeout | T::Timeout => K::Timeout,
                    T::Redirect | T::InvalidResponse => K::Failed,
                    _ => K::Unavailable,
                };
                let phase = match stage {
                    Stage::Prepare => P::Prepare,
                    Stage::Connect => P::Connect,
                    Stage::Request => P::Request,
                    _ => P::Reply,
                };
                let phase = if op == Op::Status && phase != P::Prepare { P::Poll } else { phase };
                class(kind_of, phase, transport_safety(*kind, *stage, op.is_idempotent()))
            }
            Self::Http { status, retry_after } => {
                http_class(*status, *retry_after, net(P::Request), net(P::Reply), http_safety(*status, op.is_idempotent()))
            }
            Self::Status { code, retry_after, .. } => {
                let (kind, reaches, default_wait) = status_row(*code);
                let wait = if kind.is_retryable() { retry_after.or(default_wait) } else { None };
                // A status is a definite answer: the printer declined, so nothing was accepted. The
                // rows that say otherwise (internal and device errors) may have followed a created job.
                Class { kind, phase: net(P::Reply), safety: if reaches { reached } else { S::Safe }, retry_after: wait }
            }
        }
    }

    /// The neutral error for this one, as seen by a caller of step `op`.
    pub fn print_error(&self, op: Op) -> PrintError {
        let class = self.classify(op);
        let mut error = PrintError::new(class.kind, self.to_string()).with_phase(class.phase).with_safety(class.safety);
        if let Some(wait) = class.retry_after {
            error = error.with_retry_after(wait);
        }
        match self {
            Self::Status { code, .. } => error = error.with_native(NativeCode { domain: "ipp", code: i64::from(code.0), name: code.name() }),
            Self::Http { status, .. } => error = error.with_native(NativeCode { domain: "http", code: i64::from(*status), name: None }),
            _ => {}
        }
        error
    }
}

/// The conservative reading: as if the step were a send, so that an ambiguous failure says
/// [`RetrySafety::OutcomeUnknown`].
impl From<IppError> for PrintError {
    fn from(e: IppError) -> Self {
        e.print_error(Op::Send)
    }
}

/// HTTP statuses. 401 and 403 are an authentication demand (not implemented), 413 is a document that
/// is too large for the printer, 426 asks for TLS.
fn http_class(status: u16, wait: Option<Duration>, request: Phase, reply: Phase, reached: RetrySafety) -> Class {
    use ErrorKind as K;
    let class = |kind: K, phase, safety, default: Option<Duration>| Class {
        kind,
        phase,
        safety,
        retry_after: if kind.is_retryable() { wait.or(default) } else { None },
    };
    match status {
        401 | 403 => class(K::Rejected, request, RetrySafety::Safe, None),
        408 => class(K::Timeout, request, RetrySafety::Safe, None),
        413 => class(K::InvalidRequest, request, RetrySafety::Safe, None),
        425 => class(K::Busy, request, RetrySafety::Safe, Some(Duration::from_secs(2))),
        429 => class(K::Busy, request, RetrySafety::Safe, Some(Duration::from_secs(5))),
        503 => class(K::Unavailable, request, RetrySafety::Safe, Some(Duration::from_secs(5))),
        501 | 505 | 426 => class(K::Failed, reply, RetrySafety::Safe, None),
        500..=599 => class(K::Unavailable, reply, reached, None),
        300..=399 => class(K::Failed, reply, reached, None),
        _ => class(K::Failed, reply, RetrySafety::Safe, None),
    }
}

/// One row of the status table: the kind, whether the request may have been acted on (so a send is
/// outcome-unknown), and the wait suggested when the printer gives none.
fn status_row(code: StatusCode) -> (ErrorKind, bool, Option<Duration>) {
    use ErrorKind as K;
    let s = Duration::from_secs;
    match code {
        StatusCode::BAD_REQUEST => (K::InvalidRequest, false, None),
        StatusCode::FORBIDDEN | StatusCode::NOT_AUTHENTICATED | StatusCode::NOT_AUTHORIZED | StatusCode::NOT_POSSIBLE => (K::Rejected, false, None),
        StatusCode::TIMEOUT => (K::Timeout, false, Some(s(2))),
        StatusCode::NOT_FOUND | StatusCode::GONE => (K::NotFound, false, None),
        StatusCode::REQUEST_ENTITY_TOO_LARGE | StatusCode::REQUEST_VALUE_TOO_LONG => (K::InvalidRequest, false, None),
        StatusCode::DOCUMENT_FORMAT_NOT_SUPPORTED | StatusCode::COMPRESSION_NOT_SUPPORTED => (K::UnsupportedFormat, false, None),
        StatusCode::ATTRIBUTES_OR_VALUES_NOT_SUPPORTED
        | StatusCode::URI_SCHEME_NOT_SUPPORTED
        | StatusCode::CHARSET_NOT_SUPPORTED
        | StatusCode::CONFLICTING_ATTRIBUTES
        | StatusCode::ATTRIBUTES_NOT_SETTABLE
        | StatusCode::DOCUMENT_ACCESS_ERROR => (K::InvalidRequest, false, None),
        StatusCode::COMPRESSION_ERROR
        | StatusCode::DOCUMENT_FORMAT_ERROR
        | StatusCode::DOCUMENT_PASSWORD_ERROR
        | StatusCode::DOCUMENT_PERMISSION_ERROR
        | StatusCode::DOCUMENT_SECURITY_ERROR
        | StatusCode::DOCUMENT_UNPRINTABLE_ERROR => (K::Rejected, false, None),
        StatusCode::INTERNAL_ERROR | StatusCode::DEVICE_ERROR => (K::Failed, true, None),
        StatusCode::OPERATION_NOT_SUPPORTED | StatusCode::VERSION_NOT_SUPPORTED => (K::Failed, false, None),
        StatusCode::SERVICE_UNAVAILABLE => (K::Unavailable, false, Some(s(5))),
        StatusCode::TEMPORARY_ERROR | StatusCode::BUSY => (K::Busy, false, Some(s(5))),
        StatusCode::NOT_ACCEPTING_JOBS => (K::Busy, false, Some(s(15))),
        StatusCode::TOO_MANY_JOBS => (K::Busy, false, Some(s(10))),
        StatusCode::JOB_CANCELED => (K::Cancelled, false, None),
        StatusCode::MULTIPLE_DOCUMENT_JOBS_NOT_SUPPORTED => (K::InvalidRequest, false, None),
        StatusCode::PRINTER_IS_DEACTIVATED | StatusCode::TOO_MANY_DOCUMENTS => (K::Rejected, false, None),
        other if other.is_client_error() => (K::Rejected, false, None),
        // An unknown server error, an informational or redirection code, a success code in an error
        // position: the printer's word is not understood, so the request may have been acted on.
        _ => (K::Failed, true, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(code: StatusCode) -> IppError {
        IppError::Status { code, message: String::new(), retry_after: None, unsupported: Vec::new() }
    }

    /// The status table: (code, kind, retryable, may have been acted on, default wait seconds).
    #[test]
    fn every_ipp_status_code_has_a_row() {
        use ErrorKind as K;
        let rows: &[(StatusCode, K, bool, Option<u64>)] = &[
            (StatusCode::BAD_REQUEST, K::InvalidRequest, false, None),
            (StatusCode::FORBIDDEN, K::Rejected, false, None),
            (StatusCode::NOT_AUTHENTICATED, K::Rejected, false, None),
            (StatusCode::NOT_AUTHORIZED, K::Rejected, false, None),
            (StatusCode::NOT_POSSIBLE, K::Rejected, false, None),
            (StatusCode::TIMEOUT, K::Timeout, false, Some(2)),
            (StatusCode::NOT_FOUND, K::NotFound, false, None),
            (StatusCode::GONE, K::NotFound, false, None),
            (StatusCode::REQUEST_ENTITY_TOO_LARGE, K::InvalidRequest, false, None),
            (StatusCode::REQUEST_VALUE_TOO_LONG, K::InvalidRequest, false, None),
            (StatusCode::DOCUMENT_FORMAT_NOT_SUPPORTED, K::UnsupportedFormat, false, None),
            (StatusCode::ATTRIBUTES_OR_VALUES_NOT_SUPPORTED, K::InvalidRequest, false, None),
            (StatusCode::URI_SCHEME_NOT_SUPPORTED, K::InvalidRequest, false, None),
            (StatusCode::CHARSET_NOT_SUPPORTED, K::InvalidRequest, false, None),
            (StatusCode::CONFLICTING_ATTRIBUTES, K::InvalidRequest, false, None),
            (StatusCode::COMPRESSION_NOT_SUPPORTED, K::UnsupportedFormat, false, None),
            (StatusCode::COMPRESSION_ERROR, K::Rejected, false, None),
            (StatusCode::DOCUMENT_FORMAT_ERROR, K::Rejected, false, None),
            (StatusCode::DOCUMENT_ACCESS_ERROR, K::InvalidRequest, false, None),
            (StatusCode::ATTRIBUTES_NOT_SETTABLE, K::InvalidRequest, false, None),
            (StatusCode::IGNORED_ALL_SUBSCRIPTIONS, K::Rejected, false, None),
            (StatusCode::TOO_MANY_SUBSCRIPTIONS, K::Rejected, false, None),
            (StatusCode::IGNORED_ALL_NOTIFICATIONS, K::Rejected, false, None),
            (StatusCode::PRINT_SUPPORT_FILE_NOT_FOUND, K::Rejected, false, None),
            (StatusCode::DOCUMENT_PASSWORD_ERROR, K::Rejected, false, None),
            (StatusCode::DOCUMENT_PERMISSION_ERROR, K::Rejected, false, None),
            (StatusCode::DOCUMENT_SECURITY_ERROR, K::Rejected, false, None),
            (StatusCode::DOCUMENT_UNPRINTABLE_ERROR, K::Rejected, false, None),
            (StatusCode::INTERNAL_ERROR, K::Failed, true, None),
            (StatusCode::OPERATION_NOT_SUPPORTED, K::Failed, false, None),
            (StatusCode::SERVICE_UNAVAILABLE, K::Unavailable, false, Some(5)),
            (StatusCode::VERSION_NOT_SUPPORTED, K::Failed, false, None),
            (StatusCode::DEVICE_ERROR, K::Failed, true, None),
            (StatusCode::TEMPORARY_ERROR, K::Busy, false, Some(5)),
            (StatusCode::NOT_ACCEPTING_JOBS, K::Busy, false, Some(15)),
            (StatusCode::BUSY, K::Busy, false, Some(5)),
            (StatusCode::JOB_CANCELED, K::Cancelled, false, None),
            (StatusCode::MULTIPLE_DOCUMENT_JOBS_NOT_SUPPORTED, K::InvalidRequest, false, None),
            (StatusCode::PRINTER_IS_DEACTIVATED, K::Rejected, false, None),
            (StatusCode::TOO_MANY_JOBS, K::Busy, false, Some(10)),
            (StatusCode::TOO_MANY_DOCUMENTS, K::Rejected, false, None),
            (StatusCode(0x04FF), K::Rejected, false, None),
            (StatusCode(0x05FF), K::Failed, true, None),
            (StatusCode(0x0600), K::Failed, true, None),
            (StatusCode(0x0300), K::Failed, true, None),
        ];
        let failures: Vec<_> = StatusCode::KNOWN.iter().filter(|c| !c.is_success()).collect();
        for code in failures {
            assert!(rows.iter().any(|(c, ..)| c == code), "{code:?} has no row");
        }
        for (code, kind, reaches, wait) in rows {
            let e = status(*code);
            for op in Op::ALL {
                let c = e.classify(op);
                assert_eq!(c.kind, *kind, "{code:?} {op:?}");
                let want_safety =
                    if *reaches { if op.is_idempotent() { RetrySafety::Idempotent } else { RetrySafety::OutcomeUnknown } } else { RetrySafety::Safe };
                assert_eq!(c.safety, want_safety, "{code:?} {op:?}");
                assert_eq!(c.retry_after.map(|d| d.as_secs()), *wait, "{code:?}");
                assert_eq!(c.phase, if op == Op::Status { Phase::Poll } else { Phase::Reply });
            }
            let neutral = e.print_error(Op::Send);
            assert_eq!(neutral.is_retryable(), kind.is_retryable());
            let native = neutral.native().unwrap();
            assert_eq!((native.domain, native.code), ("ipp", i64::from(code.0)));
            assert_eq!(native.name, code.name());
        }
        // The retryable statuses are exactly the busy ones, all safe to resend.
        for code in [StatusCode::BUSY, StatusCode::SERVICE_UNAVAILABLE, StatusCode::NOT_ACCEPTING_JOBS, StatusCode::TEMPORARY_ERROR] {
            let c = status(code).classify(Op::Send);
            assert!(c.kind.is_retryable() && c.safety == RetrySafety::Safe && c.retry_after.is_some(), "{code:?}");
        }
        // Every client-error-* is not retryable except the client timeout.
        for code in StatusCode::KNOWN.iter().filter(|c| c.is_client_error() && **c != StatusCode::TIMEOUT) {
            assert!(!status(*code).classify(Op::Send).kind.is_retryable(), "{code:?}");
        }
    }

    #[test]
    fn the_printers_retry_after_beats_the_default() {
        let e = IppError::Status { code: StatusCode::BUSY, message: "busy".into(), retry_after: Some(Duration::from_secs(42)), unsupported: vec![] };
        assert_eq!(e.classify(Op::Create).retry_after, Some(Duration::from_secs(42)));
        let not_retryable = IppError::Status {
            code: StatusCode::BAD_REQUEST,
            message: String::new(),
            retry_after: Some(Duration::from_secs(9)),
            unsupported: vec![],
        };
        assert_eq!(not_retryable.classify(Op::Send).retry_after, None);
    }

    #[test]
    fn transport_failures_follow_the_step() {
        use RetrySafety as S;
        let closed = IppError::Transport { kind: TransportKind::ConnectionClosed, stage: Stage::Reply, message: "m".into() };
        assert_eq!(closed.classify(Op::Send).safety, S::OutcomeUnknown);
        assert_eq!(closed.classify(Op::Create).safety, S::OutcomeUnknown);
        assert_eq!(closed.classify(Op::Query).safety, S::Idempotent);
        assert_eq!(closed.classify(Op::Cancel).safety, S::Idempotent);
        assert_eq!(closed.classify(Op::Status).phase, Phase::Poll);
        let refused = IppError::Transport { kind: TransportKind::Refused, stage: Stage::Connect, message: "m".into() };
        for op in Op::ALL {
            assert_eq!(refused.classify(op).safety, S::Safe);
            assert_eq!(refused.classify(op).kind, ErrorKind::Unavailable);
        }
        let timeout = IppError::Transport { kind: TransportKind::Timeout, stage: Stage::Reply, message: "m".into() };
        assert_eq!(timeout.classify(Op::Send).kind, ErrorKind::Timeout);
        assert!(timeout.print_error(Op::Send).outcome_unknown());
        assert!(!timeout.print_error(Op::Query).outcome_unknown());
    }

    #[test]
    fn http_statuses_have_their_readings() {
        let h = |status| IppError::Http { status, retry_after: None };
        assert_eq!(h(401).classify(Op::Send).kind, ErrorKind::Rejected);
        assert!(h(401).to_string().contains("authentication"));
        assert!(h(426).to_string().contains("TLS"));
        assert_eq!(h(413).classify(Op::Send).kind, ErrorKind::InvalidRequest);
        assert_eq!(h(503).classify(Op::Send).safety, RetrySafety::Safe);
        assert_eq!(h(503).classify(Op::Send).retry_after, Some(Duration::from_secs(5)));
        assert_eq!(h(500).classify(Op::Send).safety, RetrySafety::OutcomeUnknown);
        assert_eq!(h(500).classify(Op::Query).safety, RetrySafety::Idempotent);
        assert_eq!(h(404).classify(Op::Send).kind, ErrorKind::Failed);
        assert_eq!(h(429).classify(Op::Send).kind, ErrorKind::Busy);
        let n = h(503).print_error(Op::Send).native().unwrap();
        assert_eq!((n.domain, n.code), ("http", 503));
    }

    #[test]
    fn every_kind_is_listed_and_maps() {
        let samples = [
            IppError::Policy(String::new()),
            IppError::Tls(String::new()),
            IppError::Transport { kind: TransportKind::Refused, stage: Stage::Connect, message: String::new() },
            IppError::Http { status: 500, retry_after: None },
            IppError::TooLarge("the printer's reply", 1),
            IppError::Codec(String::new()),
            IppError::Protocol(String::new()),
            status(StatusCode::BUSY),
            IppError::Invalid(String::new()),
            IppError::Unsupported(String::new()),
            IppError::UnsupportedFormat { document: String::new(), accepted: vec![] },
            IppError::Cancelled,
            IppError::Timeout,
            IppError::NotFound(String::new()),
        ];
        let mut kinds: Vec<_> = samples.iter().map(IppError::kind).collect();
        kinds.sort_unstable();
        let mut listed = KINDS.to_vec();
        listed.sort_unstable();
        assert_eq!(kinds, listed);
        for e in &samples {
            let p = PrintError::from(e.clone());
            assert_eq!(p.is_retryable(), e.is_retryable());
        }
        assert_eq!(IppError::TooLarge("the request", 1).classify(Op::Send).kind, ErrorKind::InvalidRequest);
        assert_eq!(IppError::TooLarge("the printer's reply", 1).classify(Op::Send).kind, ErrorKind::Failed);
        assert_eq!(IppError::Tls("x".into()).classify(Op::Send).kind, ErrorKind::InvalidRequest);
    }

    #[test]
    fn an_expired_deadline_never_reached_the_wire() {
        // Like the WSD error: the deadline passed before anything was sent, so it is safe to try
        // again with a fresh one, for a create and a status poll alike.
        for op in [Op::Send, Op::Status] {
            let c = IppError::Timeout.classify(op);
            assert_eq!((c.kind, c.phase, c.safety), (ErrorKind::Timeout, Phase::Prepare, RetrySafety::Safe), "{op:?}");
        }
    }
}
