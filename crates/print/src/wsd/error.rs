//! The WS-Print layer's error type, and its mapping onto [`crate::PrintError`].

use std::fmt;
use std::time::Duration;

use super::adapter::{Stage, TransportKind, WireError, http_safety, transport_safety};
use super::fault::{Fault, FaultCode};
use crate::{ErrorKind, NativeCode, Phase, PrintError, RetrySafety};

/// Everything that can go wrong, as values an interface can show and an agent can act on.
#[derive(Debug, Clone, PartialEq)]
pub enum WsdError {
    /// The network policy refused (see `policy`): the URL, or a missing confirmation.
    Policy(String),
    /// The transport failed before a usable HTTP reply arrived. The kind says why (refused,
    /// timeout, redirect, closed without a reply, ...) and the stage how far the exchange got;
    /// the message is only for people.
    Transport { kind: TransportKind, stage: Stage, message: String },
    /// An HTTP status that is not success, with no SOAP fault in the body, and the wait the printer
    /// asked for with `Retry-After`.
    Http { status: u16, retry_after: Option<Duration> },
    /// A size cap was hit (what, the limit in bytes or items).
    TooLarge(&'static str, usize),
    /// The reply is not well-formed XML (or uses something refused, such as a DTD).
    Xml(String),
    /// Well-formed, but not the message that was expected.
    Protocol(String),
    /// The printer answered with a SOAP fault.
    Fault(Fault),
    /// An argument or option is invalid (checked before anything is sent).
    Invalid(String),
    /// The printer does not accept any format the document can be produced in.
    NoCommonFormat { offered: Vec<String>, accepted: Vec<String> },
    /// A requested option is outside what the printer lists (strict mode), or cannot be carried by
    /// the chosen ticket shape.
    Unsupported(String),
    /// The document could not be produced.
    Encode(String),
    /// The caller cancelled; nothing further was sent.
    Cancelled,
    /// A deadline passed.
    Timeout,
    /// The printer does not offer what was asked for.
    NotFound(String),
}

impl fmt::Display for WsdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Policy(m) => write!(f, "refused by policy: {m}"),
            Self::Transport { message, .. } => write!(f, "transport: {message}"),
            Self::Http { status, retry_after: Some(wait) } => write!(f, "the printer answered HTTP {status} (retry after {}s)", wait.as_secs()),
            Self::Http { status, retry_after: None } => write!(f, "the printer answered HTTP {status}"),
            Self::TooLarge(what, n) => write!(f, "{what} is larger than the limit of {n}"),
            Self::Xml(m) => write!(f, "malformed XML: {m}"),
            Self::Protocol(m) => write!(f, "unexpected reply: {m}"),
            Self::Fault(fault) => fault.fmt(f),
            Self::Invalid(m) => write!(f, "invalid {m}"),
            Self::NoCommonFormat { offered, accepted } => {
                write!(f, "the printer accepts none of the offered formats (offered: {offered:?}; printer accepts: {accepted:?})")
            }
            Self::Unsupported(m) => write!(f, "the printer does not support {m}"),
            Self::Encode(m) => write!(f, "encoding failed: {m}"),
            Self::Cancelled => f.write_str("cancelled"),
            Self::Timeout => f.write_str("the deadline passed"),
            Self::NotFound(m) => write!(f, "not found: {m}"),
        }
    }
}

impl std::error::Error for WsdError {}

impl From<WireError> for WsdError {
    /// The shared protocol errors map onto the same-named variants.
    fn from(e: WireError) -> Self {
        match e {
            WireError::Policy(m) => Self::Policy(m),
            WireError::Transport { kind, stage, message } => Self::Transport { kind, stage, message },
            WireError::Http { status, retry_after } => Self::Http { status, retry_after },
            WireError::Cancelled => Self::Cancelled,
            WireError::Timeout => Self::Timeout,
            WireError::NotFound(m) => Self::NotFound(m),
            WireError::TooLarge(what, n) => Self::TooLarge(what, n),
            WireError::Xml(m) => Self::Xml(m),
            WireError::Protocol(m) => Self::Protocol(m),
            WireError::Fault(f) => Self::Fault((*f).into()),
            WireError::Invalid(m) => Self::Invalid(m),
            // The core's error type is non-exhaustive: a future variant is at least a protocol problem.
            other => Self::Protocol(other.to_string()),
        }
    }
}

pub type Result<T, E = WsdError> = std::result::Result<T, E>;

/// Every value [`WsdError::kind`] can return.
pub const KINDS: &[&str] = &[
    "policy",
    "transport",
    "http",
    "too_large",
    "xml",
    "protocol",
    "fault",
    "invalid",
    "no_common_format",
    "unsupported",
    "encode",
    "cancelled",
    "timeout",
    "not_found",
];

/// What the printer's reply is called in [`WsdError::TooLarge`].
pub(crate) const REPLY_LABEL: &str = "the printer's reply";

/// The [`WsdError::TooLarge`] labels that name something this side sends or produces.
const REQUEST_LABELS: &[&str] = &["the request", "an inline (base64) document", "the raster stream", "a raster page"];

impl WsdError {
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
            Self::NoCommonFormat { .. } => "no_common_format",
            Self::Unsupported(_) => "unsupported",
            Self::Encode(_) => "encode",
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
            Self::NotFound(_) => "not_found",
        }
    }

    /// Would trying the same request again later plausibly work? The condition is transient: HTTP
    /// 408, 425, 429 and 5xx except 501 and 505, a connection that failed, timed out or that the
    /// printer closed, an I/O error that fits no other kind (and any transport kind added later),
    /// a deadline, and a fault whose code says so. A refused redirect and a peer that did not speak
    /// HTTP are not. It is exactly
    /// [`ErrorKind::is_retryable`] of [`Self::error_kind`]. Whether sending again is *safe* depends
    /// on the operation too: see [`Self::classify`].
    pub fn is_retryable(&self) -> bool {
        self.error_kind().is_retryable()
    }

    /// The fault, when the printer sent one.
    pub fn fault(&self) -> Option<&Fault> {
        match self {
            Self::Fault(f) => Some(f),
            _ => None,
        }
    }

    /// The neutral kind this error is reported as. It does not depend on the operation.
    /// `is_retryable()` of the result equals [`Self::is_retryable`] (see the table test).
    pub fn error_kind(&self) -> ErrorKind {
        self.classify(Op::Send).kind
    }

    /// The whole reading of this error for an operation: the neutral kind, where it failed, whether
    /// repeating the request is safe, and how long to wait. The table is in the crate's
    /// `docs/job-lifecycle.md` and pinned by the table tests.
    ///
    /// The principle: **a failure that proves the printer did not act is [`RetrySafety::Safe`]**
    /// (nothing sent, a refused address, a connect failure, a policy refusal, 408/425/429/503, a SOAP
    /// fault, which is a definite answer). For the transport and HTTP errors `craft-wsd` says
    /// which stage the exchange reached and whether repeating is safe; this adds what only the
    /// print service knows (a 408/425 decline, a fault being a definite answer). **A failure after the request may have arrived is
    /// [`RetrySafety::OutcomeUnknown`] for a non-idempotent operation** (create, send: a closed or
    /// reset connection, a timeout waiting for the reply, a 500/502/504, an unreadable reply) and
    /// [`RetrySafety::Idempotent`] for a query or a cancel, which may be repeated freely.
    pub fn classify(&self, op: Op) -> Class {
        use ErrorKind as K;
        use Phase as P;
        use RetrySafety as S;
        // After the request may have been delivered.
        let reached = if op.is_idempotent() { S::Idempotent } else { S::OutcomeUnknown };
        // A status poll's failures are all "poll" failures, whatever the network step was.
        let net = |phase: Phase| if op == Op::Status { P::Poll } else { phase };
        let class = |kind, phase, safety| Class { kind, phase, safety, retry_after: None };
        match self {
            Self::Policy(_) | Self::Invalid(_) | Self::Unsupported(_) => class(K::InvalidRequest, P::Prepare, S::Safe),
            Self::NoCommonFormat { .. } => class(K::UnsupportedFormat, P::Prepare, S::Safe),
            Self::Encode(_) => class(K::Failed, P::Prepare, S::Safe),
            Self::NotFound(_) => class(K::NotFound, P::Prepare, S::Safe),
            Self::Cancelled => class(K::Cancelled, P::Prepare, S::Safe),
            // A cap on something we send is the caller's document being too big; a cap on what came
            // back (the reply, a list, a multipart message) is the printer's doing.
            Self::TooLarge(what, _) if REQUEST_LABELS.contains(what) => class(K::InvalidRequest, P::Prepare, S::Safe),
            Self::TooLarge(..) | Self::Xml(_) | Self::Protocol(_) => class(K::Failed, net(P::Reply), reached),
            // The caller's deadline had already passed when the exchange was about to start (an
            // exchange that runs past it ends as a transport timeout instead): nothing was sent.
            Self::Timeout => class(K::Timeout, P::Prepare, S::Safe),
            Self::Transport { kind, stage, .. } => {
                use TransportKind as T;
                let kind_of = match kind {
                    T::ConnectTimeout | T::Timeout => K::Timeout,
                    // Not transient, like the HTTP errors that are not retryable: a refused redirect
                    // and a peer that did not speak HTTP. `craft-wsd` calls both not retryable and
                    // so does this layer.
                    T::Redirect | T::InvalidResponse => K::Failed,
                    // Refused, unreachable, closed without a reply, an I/O error that matched no
                    // other kind (transient), and any kind added later.
                    T::Refused | T::Unreachable | T::ConnectionClosed | T::Io => K::Unavailable,
                    _ => K::Unavailable,
                };
                let phase = match stage {
                    Stage::Prepare => P::Prepare,
                    Stage::Connect => P::Connect,
                    Stage::Request => P::Request,
                    _ => P::Reply,
                };
                // A status poll's failures are poll failures, whatever the network step was; a
                // failure before anything went out stays a preparation failure.
                let phase = if op == Op::Status && phase != P::Prepare { P::Poll } else { phase };
                class(kind_of, phase, transport_safety(*kind, *stage, op.is_idempotent()))
            }
            Self::Http { status, retry_after } => {
                http_class(*status, *retry_after, net(P::Request), net(P::Reply), http_safety(*status, op.is_idempotent()))
            }
            Self::Fault(fault) => {
                let kind = fault_kind(&fault.code);
                // The printer's own `Retry-After` wins over the defaults.
                let retry_after = fault.retry_after.or_else(|| fault_retry_after(&fault.code));
                // A fault is a definite answer: the printer declined, so nothing was accepted. The
                // one exception says an earlier send was in fact accepted.
                let safety = match fault.code {
                    FaultCode::ClientErrorLastDocumentAlreadySent => reached,
                    _ => S::Safe,
                };
                Class { kind, phase: net(P::Reply), safety, retry_after }
            }
        }
    }

    /// The neutral error for this one, as seen by a caller of operation `op`: the kind, the message,
    /// the native fault code, [`Phase`], [`RetrySafety`] and [`PrintError::retry_after`].
    pub fn print_error(&self, op: Op) -> PrintError {
        let class = self.classify(op);
        let mut error = PrintError::new(class.kind, self.to_string()).with_phase(class.phase).with_safety(class.safety);
        if let Some(wait) = class.retry_after {
            error = error.with_retry_after(wait);
        }
        if let Self::Fault(f) = self {
            let (code, name) = f.code.native();
            error = error.with_native(NativeCode { domain: "wsd-fault", code, name });
            // The printer answered and named the job as unknown: the printer is fine, the job is gone.
            if f.code == FaultCode::ClientErrorJobIdNotFound {
                error = error.with_subject(crate::NotFoundSubject::Job);
            }
        }
        error
    }
}

/// The protocol operation that was in flight when an error happened. It decides whether repeating
/// the request is safe: a query or a cancel may be repeated, a create or a send may not (the first
/// one may have arrived).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    /// `GetPrinterElements`, `GetActiveJobs`, `GetJobHistory`: reads.
    Query,
    /// `CreatePrintJob`.
    Create,
    /// `SendDocument`.
    Send,
    /// `GetJobElements` while following a job.
    Status,
    /// `CancelJob`.
    Cancel,
}

impl Op {
    /// Repeating the request changes nothing the first one did not.
    pub const fn is_idempotent(self) -> bool {
        matches!(self, Self::Query | Self::Status | Self::Cancel)
    }

    pub const ALL: [Op; 5] = [Op::Query, Op::Create, Op::Send, Op::Status, Op::Cancel];
}

/// How an error reads to a caller; see [`WsdError::classify`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Class {
    pub kind: ErrorKind,
    pub phase: Phase,
    pub safety: RetrySafety,
    /// How long to wait before another try, for the conditions where the printer is merely busy.
    pub retry_after: Option<Duration>,
}

/// HTTP statuses without a SOAP fault. `request` and `reply` are the phases for "the server
/// refused before processing" and "it answered something unusable"; `reached` is what `craft-wsd`
/// says about repeating after this status (429 and 503 are a decline), and is used where the
/// request may have been processed. A wait the printer sent is kept for the busy kinds only.
fn http_class(status: u16, wait: Option<Duration>, request: Phase, reply: Phase, reached: RetrySafety) -> Class {
    use ErrorKind as K;
    // The printer's `Retry-After` wins over the default wait.
    let class = |kind: K, phase, safety, default: Option<Duration>| Class {
        kind,
        phase,
        safety,
        retry_after: if kind.is_retryable() { wait.or(default) } else { None },
    };
    match status {
        // The server says it did not take the request, or is not ready for it: nothing happened.
        408 => class(K::Timeout, request, RetrySafety::Safe, None),
        425 => class(K::Busy, request, RetrySafety::Safe, Some(Duration::from_secs(2))),
        429 => class(K::Busy, request, RetrySafety::Safe, Some(Duration::from_secs(5))),
        503 => class(K::Unavailable, request, RetrySafety::Safe, Some(Duration::from_secs(5))),
        // Not implemented and unsupported version are permanent, and nothing was done.
        501 | 505 => class(K::Failed, reply, RetrySafety::Safe, None),
        // A server-side failure or a gateway that gave up: the request may well have been processed.
        500..=599 => class(K::Unavailable, reply, reached, None),
        // Redirects and other statuses we do not follow: not transient.
        300..=399 => class(K::Failed, reply, reached, None),
        _ => class(K::Failed, reply, RetrySafety::Safe, None),
    }
}

/// The wait a busy printer is told to observe when it gives no hint of its own. These are defaults,
/// not measurements: a printer that is not taking jobs is in a mode that lasts, a temporary error
/// clears sooner, and a document still being processed is the shortest wait.
fn fault_retry_after(code: &FaultCode) -> Option<Duration> {
    match code {
        FaultCode::ServerErrorTemporaryError => Some(Duration::from_secs(5)),
        FaultCode::ServerErrorNotAcceptingJobs => Some(Duration::from_secs(15)),
        FaultCode::ServerErrorDocumentActive => Some(Duration::from_secs(2)),
        _ => None,
    }
}

/// How a fault's sub-code reads to a caller. The retryable codes ([`FaultCode::is_retryable`]) are
/// the ones that map to a retryable kind, so the two notions cannot drift apart.
fn fault_kind(code: &FaultCode) -> ErrorKind {
    use FaultCode as C;
    match code {
        C::ServerErrorTemporaryError | C::ServerErrorNotAcceptingJobs | C::ServerErrorDocumentActive => ErrorKind::Busy,
        C::ClientErrorFormatNotSupported | C::ClientErrorCompressionNotSupported => ErrorKind::UnsupportedFormat,
        C::ClientErrorJobIdNotFound => ErrorKind::NotFound,
        C::ServerErrorJobCancelled => ErrorKind::Cancelled,
        C::InvalidArgs
        | C::ClientErrorMultipleDocumentsNotSupported
        | C::ClientErrorDocumentURLSchemeNotSupported
        | C::ClientErrorDocumentURLNotFound
        | C::ClientErrorConflictingRequiredParameters => ErrorKind::InvalidRequest,
        C::ServerErrorInternalError | C::ClientErrorLastDocumentAlreadySent => ErrorKind::Failed,
        // The printer understood and declined: it is a refusal, whatever the vendor calls it.
        C::OperationFailed | C::ActionNotSupported | C::ServerErrorAddDocumentNotSupported | C::Other(..) => ErrorKind::Rejected,
    }
}

/// The conservative reading: as if the operation were a send, so that an ambiguous failure says
/// [`RetrySafety::OutcomeUnknown`]. Use [`WsdError::print_error`] with the real [`Op`] where it is
/// known (queries and cancels are [`RetrySafety::Idempotent`]).
impl From<WsdError> for PrintError {
    fn from(e: WsdError) -> Self {
        e.print_error(Op::Send)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsd::adapter::{Role, SubCode};
    use crate::wsd::ns;
    use std::time::Duration;

    fn wire_samples() -> Vec<WireError> {
        let fault = crate::wsd::adapter::WireFault::new(Role::Sender, SubCode::new(Some(ns::WPRT), "InvalidArgs"), "", "");
        vec![
            WireError::Policy("p".into()),
            WireError::transport(TransportKind::Refused, "t"),
            WireError::http_retry_after(503, Some(Duration::from_secs(7))),
            WireError::TooLarge("x", 1),
            WireError::Xml("x".into()),
            WireError::Protocol("x".into()),
            WireError::from_fault(fault),
            WireError::Invalid("x".into()),
            WireError::Cancelled,
            WireError::Timeout,
            WireError::NotFound("x".into()),
        ]
    }

    #[test]
    fn every_core_error_kind_maps_to_one_of_ours() {
        use crate::wsd::adapter::WIRE_KINDS;
        let samples = wire_samples();
        for kind in WIRE_KINDS {
            let e = samples.iter().find(|e| e.kind() == *kind).unwrap_or_else(|| panic!("no sample for the core kind {kind}"));
            let ours = WsdError::from(e.clone());
            assert_eq!(ours.kind(), *kind, "{kind} keeps its name");
            assert!(KINDS.contains(&ours.kind()));
            // And the neutral error exists, with a message.
            let neutral = PrintError::from(ours.clone());
            assert!(!neutral.message().is_empty(), "{kind}");
            assert_eq!(neutral.is_retryable(), ours.is_retryable(), "{kind}");
        }
        assert_eq!(samples.len(), WIRE_KINDS.len());
        assert!(
            !KINDS.contains(&"connection_closed") && !WIRE_KINDS.contains(&"connection_closed"),
            "a closed connection is a transport kind, not an error kind"
        );
    }

    fn fault(local: &str) -> WsdError {
        WsdError::Fault(Fault {
            role: Role::Receiver,
            code: FaultCode::from_qname(Some(ns::WPRT), local),
            reason: "r".into(),
            detail: String::new(),
            retry_after: None,
        })
    }

    fn transport(kind: TransportKind, stage: Stage) -> WsdError {
        WsdError::Transport { kind, stage, message: "m".into() }
    }

    fn http(status: u16) -> WsdError {
        WsdError::Http { status, retry_after: None }
    }

    /// One of every shape of error, with the transport texts that are classified.
    fn samples() -> Vec<WsdError> {
        let mut all = vec![
            WsdError::Policy(String::new()),
            WsdError::TooLarge(REPLY_LABEL, 1),
            WsdError::TooLarge("the device's reply", 1),
            WsdError::TooLarge("the request", 1),
            WsdError::Xml(String::new()),
            WsdError::Protocol(String::new()),
            WsdError::Invalid(String::new()),
            WsdError::NoCommonFormat { offered: vec![], accepted: vec![] },
            WsdError::Unsupported(String::new()),
            WsdError::Encode(String::new()),
            WsdError::Cancelled,
            WsdError::Timeout,
            WsdError::NotFound(String::new()),
            fault("ServerErrorNotAcceptingJobs"),
            fault("ServerErrorTemporaryError"),
            fault("ServerErrorDocumentActive"),
            fault("ServerErrorInternalError"),
            fault("ClientErrorLastDocumentAlreadySent"),
            fault("ClientErrorJobIdNotFound"),
            fault("OperationFailed"),
        ];
        for kind in TransportKind::ALL {
            for stage in [Stage::Prepare, Stage::Connect, Stage::Request, Stage::Reply] {
                // Before any traffic is not where a transient failure happens.
                if stage != Stage::Prepare || kind == TransportKind::Redirect {
                    all.push(transport(kind, stage));
                }
            }
        }
        for s in [301, 400, 401, 403, 404, 408, 425, 429, 500, 501, 502, 503, 504, 505] {
            all.push(http(s));
            all.push(WsdError::Http { status: s, retry_after: Some(Duration::from_secs(11)) });
        }
        all
    }

    #[test]
    fn every_kind_of_ours_maps_to_a_neutral_kind_and_keeps_retryability() {
        let all = [
            (WsdError::Policy(String::new()), ErrorKind::InvalidRequest),
            (transport(TransportKind::Refused, Stage::Connect), ErrorKind::Unavailable),
            (transport(TransportKind::Unreachable, Stage::Connect), ErrorKind::Unavailable),
            (transport(TransportKind::ConnectionClosed, Stage::Reply), ErrorKind::Unavailable),
            (transport(TransportKind::ConnectTimeout, Stage::Connect), ErrorKind::Timeout),
            (transport(TransportKind::Timeout, Stage::Reply), ErrorKind::Timeout),
            (transport(TransportKind::Redirect, Stage::Reply), ErrorKind::Failed),
            (transport(TransportKind::InvalidResponse, Stage::Reply), ErrorKind::Failed),
            (transport(TransportKind::Io, Stage::Reply), ErrorKind::Unavailable),
            (http(503), ErrorKind::Unavailable),
            (http(429), ErrorKind::Busy),
            (http(404), ErrorKind::Failed),
            (WsdError::TooLarge(REPLY_LABEL, 1), ErrorKind::Failed),
            (WsdError::TooLarge("the device's reply", 1), ErrorKind::Failed),
            (WsdError::TooLarge("the request", 1), ErrorKind::InvalidRequest),
            (WsdError::Xml(String::new()), ErrorKind::Failed),
            (WsdError::Protocol(String::new()), ErrorKind::Failed),
            (fault("ServerErrorNotAcceptingJobs"), ErrorKind::Busy),
            (WsdError::Invalid(String::new()), ErrorKind::InvalidRequest),
            (WsdError::NoCommonFormat { offered: vec![], accepted: vec![] }, ErrorKind::UnsupportedFormat),
            (WsdError::Unsupported(String::new()), ErrorKind::InvalidRequest),
            (WsdError::Encode(String::new()), ErrorKind::Failed),
            (WsdError::Cancelled, ErrorKind::Cancelled),
            (WsdError::Timeout, ErrorKind::Timeout),
            (WsdError::NotFound(String::new()), ErrorKind::NotFound),
        ];
        let mut seen: Vec<&str> = all.iter().map(|(e, _)| e.kind()).collect();
        seen.sort_unstable();
        seen.dedup();
        let mut listed = KINDS.to_vec();
        listed.sort_unstable();
        assert_eq!(seen, listed, "the table covers every entry of KINDS");
        for (e, kind) in &all {
            assert!(KINDS.contains(&e.kind()), "{e}");
            let neutral = PrintError::from(e.clone());
            assert_eq!(neutral.kind(), *kind, "{e}");
            assert_eq!(neutral.is_retryable(), e.is_retryable(), "{e}");
            assert_eq!(neutral.message(), e.to_string());
        }
    }

    /// The retry table: (error, op) -> (kind, phase, safety, retry_after seconds). Every kind in
    /// [`KINDS`] appears, for a non-idempotent and for an idempotent operation where they differ.
    #[test]
    fn the_retry_table_for_every_kind() {
        use ErrorKind as K;
        use Phase as P;
        use RetrySafety as S;
        use Stage as G;
        use TransportKind as T;
        let rows: Vec<(WsdError, Op, K, P, S, Option<u64>)> = vec![
            // Refused before anything is sent: not transient, safe.
            (WsdError::Policy("p".into()), Op::Create, K::InvalidRequest, P::Prepare, S::Safe, None),
            (WsdError::Invalid("i".into()), Op::Query, K::InvalidRequest, P::Prepare, S::Safe, None),
            (WsdError::Unsupported("u".into()), Op::Create, K::InvalidRequest, P::Prepare, S::Safe, None),
            (WsdError::NoCommonFormat { offered: vec![], accepted: vec![] }, Op::Query, K::UnsupportedFormat, P::Prepare, S::Safe, None),
            (WsdError::Encode("e".into()), Op::Create, K::Failed, P::Prepare, S::Safe, None),
            (WsdError::NotFound("n".into()), Op::Query, K::NotFound, P::Prepare, S::Safe, None),
            (WsdError::Cancelled, Op::Send, K::Cancelled, P::Prepare, S::Safe, None),
            (WsdError::TooLarge("the request", 1), Op::Send, K::InvalidRequest, P::Prepare, S::Safe, None),
            // The deadline passed before the exchange started: nothing was sent, whatever the operation.
            (WsdError::Timeout, Op::Create, K::Timeout, P::Prepare, S::Safe, None),
            (WsdError::Timeout, Op::Send, K::Timeout, P::Prepare, S::Safe, None),
            (WsdError::Timeout, Op::Status, K::Timeout, P::Prepare, S::Safe, None),
            // Transport, at the default stage of each kind: a connect failure sent nothing, whatever
            // the operation; the rest may have arrived (a create and a send are then unknown, a
            // query, a status poll and a cancel idempotent).
            (transport(T::Refused, G::Connect), Op::Create, K::Unavailable, P::Connect, S::Safe, None),
            (transport(T::Refused, G::Connect), Op::Send, K::Unavailable, P::Connect, S::Safe, None),
            (transport(T::Refused, G::Connect), Op::Status, K::Unavailable, P::Poll, S::Safe, None),
            (transport(T::Unreachable, G::Connect), Op::Query, K::Unavailable, P::Connect, S::Safe, None),
            (transport(T::Unreachable, G::Connect), Op::Create, K::Unavailable, P::Connect, S::Safe, None),
            (transport(T::ConnectTimeout, G::Connect), Op::Send, K::Timeout, P::Connect, S::Safe, None),
            (transport(T::ConnectTimeout, G::Connect), Op::Cancel, K::Timeout, P::Connect, S::Safe, None),
            (transport(T::Timeout, G::Reply), Op::Create, K::Timeout, P::Reply, S::OutcomeUnknown, None),
            (transport(T::Timeout, G::Reply), Op::Send, K::Timeout, P::Reply, S::OutcomeUnknown, None),
            (transport(T::Timeout, G::Reply), Op::Query, K::Timeout, P::Reply, S::Idempotent, None),
            (transport(T::Timeout, G::Reply), Op::Status, K::Timeout, P::Poll, S::Idempotent, None),
            (transport(T::Timeout, G::Reply), Op::Cancel, K::Timeout, P::Reply, S::Idempotent, None),
            (transport(T::ConnectionClosed, G::Reply), Op::Create, K::Unavailable, P::Reply, S::OutcomeUnknown, None),
            (transport(T::ConnectionClosed, G::Reply), Op::Send, K::Unavailable, P::Reply, S::OutcomeUnknown, None),
            (transport(T::ConnectionClosed, G::Reply), Op::Query, K::Unavailable, P::Reply, S::Idempotent, None),
            (transport(T::ConnectionClosed, G::Reply), Op::Status, K::Unavailable, P::Poll, S::Idempotent, None),
            (transport(T::ConnectionClosed, G::Reply), Op::Cancel, K::Unavailable, P::Reply, S::Idempotent, None),
            // A reset while the request was being written: it may have partly arrived.
            (transport(T::ConnectionClosed, G::Request), Op::Send, K::Unavailable, P::Request, S::OutcomeUnknown, None),
            (transport(T::ConnectionClosed, G::Request), Op::Query, K::Unavailable, P::Request, S::Idempotent, None),
            // A refused redirect is not transient; the request may have reached a server that answered.
            (transport(T::Redirect, G::Reply), Op::Send, K::Failed, P::Reply, S::OutcomeUnknown, None),
            (transport(T::Redirect, G::Reply), Op::Create, K::Failed, P::Reply, S::OutcomeUnknown, None),
            (transport(T::Redirect, G::Reply), Op::Query, K::Failed, P::Reply, S::Idempotent, None),
            (transport(T::Redirect, G::Reply), Op::Status, K::Failed, P::Poll, S::Idempotent, None),
            // A peer that did not speak HTTP: not retryable, the request may have arrived.
            (transport(T::InvalidResponse, G::Reply), Op::Create, K::Failed, P::Reply, S::OutcomeUnknown, None),
            (transport(T::InvalidResponse, G::Reply), Op::Send, K::Failed, P::Reply, S::OutcomeUnknown, None),
            (transport(T::InvalidResponse, G::Reply), Op::Query, K::Failed, P::Reply, S::Idempotent, None),
            (transport(T::InvalidResponse, G::Reply), Op::Status, K::Failed, P::Poll, S::Idempotent, None),
            (transport(T::InvalidResponse, G::Reply), Op::Cancel, K::Failed, P::Reply, S::Idempotent, None),
            // An I/O error that matched no other kind: transient, the request may have arrived.
            (transport(T::Io, G::Reply), Op::Create, K::Unavailable, P::Reply, S::OutcomeUnknown, None),
            (transport(T::Io, G::Reply), Op::Send, K::Unavailable, P::Reply, S::OutcomeUnknown, None),
            (transport(T::Io, G::Reply), Op::Query, K::Unavailable, P::Reply, S::Idempotent, None),
            (transport(T::Io, G::Reply), Op::Status, K::Unavailable, P::Poll, S::Idempotent, None),
            (transport(T::Io, G::Reply), Op::Cancel, K::Unavailable, P::Reply, S::Idempotent, None),
            // An error from before the wire (a transport that refuses to start) is a preparation failure.
            (transport(T::Io, G::Prepare), Op::Send, K::Unavailable, P::Prepare, S::Safe, None),
            // A request the HTTP client could not even build arrives as `Invalid`: nothing was sent.
            (WsdError::Invalid("bad uri".into()), Op::Create, K::InvalidRequest, P::Prepare, S::Safe, None),
            (WsdError::from(WireError::Invalid("bad uri".into())), Op::Send, K::InvalidRequest, P::Prepare, S::Safe, None),
            (WsdError::from(WireError::Invalid("bad uri".into())), Op::Status, K::InvalidRequest, P::Prepare, S::Safe, None),
            // HTTP.
            (http(408), Op::Send, K::Timeout, P::Request, S::Safe, None),
            (http(425), Op::Create, K::Busy, P::Request, S::Safe, Some(2)),
            (http(429), Op::Send, K::Busy, P::Request, S::Safe, Some(5)),
            (http(429), Op::Status, K::Busy, P::Poll, S::Safe, Some(5)),
            (http(503), Op::Create, K::Unavailable, P::Request, S::Safe, Some(5)),
            // The printer's own Retry-After replaces the default wait, and only for a transient status.
            (WsdError::Http { status: 503, retry_after: Some(Duration::from_secs(30)) }, Op::Send, K::Unavailable, P::Request, S::Safe, Some(30)),
            (WsdError::Http { status: 429, retry_after: Some(Duration::from_secs(1)) }, Op::Create, K::Busy, P::Request, S::Safe, Some(1)),
            (
                WsdError::Http { status: 500, retry_after: Some(Duration::from_secs(9)) },
                Op::Send,
                K::Unavailable,
                P::Reply,
                S::OutcomeUnknown,
                Some(9),
            ),
            (WsdError::Http { status: 404, retry_after: Some(Duration::from_secs(9)) }, Op::Query, K::Failed, P::Reply, S::Safe, None),
            (http(500), Op::Send, K::Unavailable, P::Reply, S::OutcomeUnknown, None),
            (http(502), Op::Send, K::Unavailable, P::Reply, S::OutcomeUnknown, None),
            (http(504), Op::Query, K::Unavailable, P::Reply, S::Idempotent, None),
            (http(504), Op::Status, K::Unavailable, P::Poll, S::Idempotent, None),
            (http(501), Op::Send, K::Failed, P::Reply, S::Safe, None),
            (http(505), Op::Query, K::Failed, P::Reply, S::Safe, None),
            (http(400), Op::Send, K::Failed, P::Reply, S::Safe, None),
            (http(404), Op::Query, K::Failed, P::Reply, S::Safe, None),
            (http(301), Op::Send, K::Failed, P::Reply, S::OutcomeUnknown, None),
            // A reply that cannot be used: the request was probably processed.
            (WsdError::Xml("x".into()), Op::Send, K::Failed, P::Reply, S::OutcomeUnknown, None),
            (WsdError::Xml("x".into()), Op::Query, K::Failed, P::Reply, S::Idempotent, None),
            (WsdError::Protocol("x".into()), Op::Create, K::Failed, P::Reply, S::OutcomeUnknown, None),
            (WsdError::TooLarge(REPLY_LABEL, 1), Op::Send, K::Failed, P::Reply, S::OutcomeUnknown, None),
            (WsdError::TooLarge("the job list", 1), Op::Query, K::Failed, P::Reply, S::Idempotent, None),
            // Faults are definite answers: nothing was accepted.
            (fault("ServerErrorNotAcceptingJobs"), Op::Create, K::Busy, P::Reply, S::Safe, Some(15)),
            (fault("ServerErrorTemporaryError"), Op::Send, K::Busy, P::Reply, S::Safe, Some(5)),
            (fault("ServerErrorDocumentActive"), Op::Send, K::Busy, P::Reply, S::Safe, Some(2)),
            (fault("ServerErrorInternalError"), Op::Send, K::Failed, P::Reply, S::Safe, None),
            (fault("ClientErrorJobIdNotFound"), Op::Status, K::NotFound, P::Poll, S::Safe, None),
            (fault("OperationFailed"), Op::Cancel, K::Rejected, P::Reply, S::Safe, None),
            (fault("ClientErrorFormatNotSupported"), Op::Send, K::UnsupportedFormat, P::Reply, S::Safe, None),
            (fault("InvalidArgs"), Op::Create, K::InvalidRequest, P::Reply, S::Safe, None),
            (fault("ServerErrorJobCancelled"), Op::Status, K::Cancelled, P::Poll, S::Safe, None),
            // An earlier send was in fact accepted: the job may print.
            (
                WsdError::Fault(Fault {
                    role: Role::Receiver,
                    code: FaultCode::ServerErrorNotAcceptingJobs,
                    reason: String::new(),
                    detail: String::new(),
                    retry_after: Some(Duration::from_secs(40)),
                }),
                Op::Create,
                K::Busy,
                P::Reply,
                S::Safe,
                Some(40),
            ),
            (fault("ClientErrorLastDocumentAlreadySent"), Op::Send, K::Failed, P::Reply, S::OutcomeUnknown, None),
        ];
        let mut kinds_seen: Vec<&str> = Vec::new();
        for (e, op, kind, phase, safety, wait) in &rows {
            let c = e.classify(*op);
            let what = format!("{e} [{op:?}]");
            assert_eq!(c.kind, *kind, "{what}");
            assert_eq!(c.phase, *phase, "{what}");
            assert_eq!(c.safety, *safety, "{what}");
            assert_eq!(c.retry_after.map(|d| d.as_secs()), *wait, "{what}");
            let neutral = e.print_error(*op);
            assert_eq!(
                (neutral.kind(), neutral.phase(), neutral.retry_safety(), neutral.retry_after()),
                (c.kind, Some(c.phase), c.safety, c.retry_after),
                "{what}"
            );
            kinds_seen.push(e.kind());
        }
        kinds_seen.sort_unstable();
        kinds_seen.dedup();
        let mut listed = KINDS.to_vec();
        listed.sort_unstable();
        assert_eq!(kinds_seen, listed, "the retry table has a row for every kind");
    }

    /// Every typed transport kind x every operation, at the stage the kind usually happens in: the
    /// neutral kind, the phase and the safety follow from the typed values alone, and the
    /// retryability agrees with `craft-wsd`.
    #[test]
    fn every_transport_kind_for_every_operation() {
        use ErrorKind as K;
        use Phase as P;
        use RetrySafety as S;
        for kind in TransportKind::ALL {
            let stage = kind.default_stage();
            let (want_kind, want_net_phase, retryable) = match kind {
                TransportKind::Refused | TransportKind::Unreachable => (K::Unavailable, P::Connect, true),
                TransportKind::ConnectTimeout => (K::Timeout, P::Connect, true),
                TransportKind::Timeout => (K::Timeout, P::Reply, true),
                TransportKind::ConnectionClosed => (K::Unavailable, P::Reply, true),
                TransportKind::Redirect => (K::Failed, P::Reply, false),
                TransportKind::InvalidResponse => (K::Failed, P::Reply, false),
                TransportKind::Io => (K::Unavailable, P::Reply, true),
                _ => panic!("a new transport kind needs a row here"),
            };
            assert_eq!(kind.is_retryable(), retryable, "{kind}: agrees with craft-wsd");
            let sent = stage.may_have_reached_device();
            for op in Op::ALL {
                let e = transport(kind, stage);
                let c = e.classify(op);
                let what = format!("{kind} [{op:?}]");
                assert_eq!(c.kind, want_kind, "{what}");
                assert_eq!(c.phase, if op == Op::Status { P::Poll } else { want_net_phase }, "{what}");
                let want_safety = match (sent, op.is_idempotent()) {
                    (false, _) => S::Safe,
                    (true, true) => S::Idempotent,
                    (true, false) => S::OutcomeUnknown,
                };
                assert_eq!(c.safety, want_safety, "{what}");
                assert_eq!(c.retry_after, None, "{what}");
                assert_eq!(e.is_retryable(), retryable, "{what}");
                assert_eq!(e.print_error(op).retry_safety(), want_safety, "{what}");
            }
        }
    }

    #[test]
    fn the_retry_invariants_hold_for_every_error_and_operation() {
        for e in samples() {
            for op in Op::ALL {
                let c = e.classify(op);
                let what = format!("{e} [{op:?}]");
                assert_eq!(c.kind.is_retryable(), e.is_retryable(), "{what}: retryability does not depend on the operation");
                assert_eq!(c.kind, e.error_kind(), "{what}: neither does the kind");
                if op.is_idempotent() {
                    assert_ne!(c.safety, RetrySafety::OutcomeUnknown, "{what}: a query or a cancel can always be repeated");
                } else {
                    assert_ne!(c.safety, RetrySafety::Idempotent, "{what}: a create or a send is not idempotent");
                }
                if c.retry_after.is_some() {
                    assert!(c.kind.is_retryable(), "{what}: only a transient condition has a wait");
                }
                if c.kind.is_retryable() {
                    // The one retryable error that never reached the wire: the deadline had passed
                    // before the exchange started, so a fresh deadline can try again.
                    if matches!(e, WsdError::Timeout) {
                        assert_eq!((c.phase, c.safety), (Phase::Prepare, RetrySafety::Safe), "{what}: nothing was sent");
                    } else {
                        assert_ne!(c.phase, Phase::Prepare, "{what}: a transient failure happened on the wire");
                    }
                }
                if c.phase == Phase::Poll {
                    assert_eq!(op, Op::Status, "{what}");
                }
                if op == Op::Status && c.phase != Phase::Prepare {
                    assert_eq!(c.phase, Phase::Poll, "{what}: a status poll's wire failures are poll failures");
                }
                // The conversion keeps it all.
                let neutral = e.print_error(op);
                assert_eq!((neutral.is_retryable(), neutral.retry_safety()), (e.is_retryable(), c.safety), "{what}");
            }
        }
        // Refusals and the redirect are not transient.
        for e in [WsdError::Policy("p".into()), transport(TransportKind::Redirect, Stage::Reply)] {
            assert!(!e.is_retryable(), "{e}");
        }
        // The conversion without an operation is the careful one.
        assert_eq!(PrintError::from(transport(TransportKind::ConnectionClosed, Stage::Reply)).retry_safety(), RetrySafety::OutcomeUnknown);
    }

    #[test]
    fn every_fault_code_maps_and_the_retryable_ones_stay_retryable() {
        for (name, code) in crate::wsd::fault::WPRT_CODES.iter() {
            let e =
                WsdError::Fault(Fault { role: Role::Sender, code: code.clone(), reason: String::new(), detail: String::new(), retry_after: None });
            let neutral = PrintError::from(e.clone());
            let native = neutral.native().expect("a fault carries its code");
            assert_eq!((native.domain, native.name), ("wsd-fault", Some(*name)));
            assert!(native.code >= 1, "{name}");
            assert_eq!(neutral.is_retryable(), code.is_retryable(), "{name}");
            assert_eq!(neutral.retry_after().is_some(), code.is_retryable(), "{name}: a busy fault carries a wait, others do not");
        }
        let action = PrintError::from(WsdError::Fault(Fault {
            role: Role::Sender,
            code: FaultCode::ActionNotSupported,
            reason: String::new(),
            detail: String::new(),
            retry_after: None,
        }));
        assert_eq!((action.kind(), action.native().map(|n| n.name)), (ErrorKind::Rejected, Some(Some("ActionNotSupported"))));
        let vendor = PrintError::from(WsdError::Fault(Fault {
            role: Role::Receiver,
            code: FaultCode::Other(Some("urn:v".into()), "Odd".into()),
            reason: String::new(),
            detail: String::new(),
            retry_after: None,
        }));
        assert_eq!((vendor.kind(), vendor.native().map(|n| (n.code, n.name))), (ErrorKind::Rejected, Some((0, None))));
        // Codes are distinct.
        let mut codes: Vec<i64> = crate::wsd::fault::WPRT_CODES.iter().map(|(_, c)| c.native().0).collect();
        codes.push(FaultCode::ActionNotSupported.native().0);
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), crate::wsd::fault::WPRT_CODES.len() + 1);
    }

    #[test]
    fn http_retryability_follows_the_status() {
        for (s, r) in [
            (500, true),
            (502, true),
            (503, true),
            (504, true),
            (429, true),
            (408, true),
            (425, true),
            (501, false),
            (505, false),
            (404, false),
            (400, false),
            (403, false),
            (301, false),
        ] {
            assert_eq!(http(s).is_retryable(), r, "{s}");
            assert_eq!(PrintError::from(http(s)).is_retryable(), r, "{s}");
        }
        assert!(!WsdError::Protocol(String::new()).is_retryable());
        assert!(transport(TransportKind::ConnectionClosed, Stage::Reply).is_retryable());
    }
}
