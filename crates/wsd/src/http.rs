//! The blocking HTTP transport (built on every target except `wasm32`), on `ureq` without TLS.
//!
//! Hardened to the network policy: proxies off (even from the environment), redirects not
//! followed, a global timeout, plain `http` only (a [`DeviceUrl`] cannot be anything else), and
//! the body read through a hard size cap.
//!
//! Failures are typed ([`TransportKind`], [`Stage`]) as precisely as `ureq` allows, and the
//! device's `Retry-After` is parsed and capped onto [`Response::retry_after`].

use std::io::Read;
use std::time::{Duration, SystemTime};

use ureq::Agent;

use crate::error::{Error, Stage, TransportKind};
use crate::policy::DeviceUrl;
use crate::retry::parse_retry_after;
use crate::transport::{Request, Response, Transport};

/// The `ureq`-backed transport. Cheap to clone.
#[derive(Clone)]
pub struct HttpTransport {
    agent: Agent,
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpTransport {
    /// A transport that sends straight to the device: no proxy, no redirects followed, HTTP error
    /// statuses returned as responses rather than errors, and plain `http` allowed.
    pub fn new() -> Self {
        let config = Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            .https_only(false)
            .user_agent("craft-wsd")
            .build();
        Self { agent: Agent::new_with_config(config) }
    }
}

/// The longest a connect may take, whatever the request's overall budget: a device that is off or
/// unroutable is reported as an unsent, retry-safe connect failure after this long, not after a
/// budget meant for a slow reply (a print upload can be given minutes).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The connect limit for a request with `global` as its whole budget. A budget that is not longer
/// than [`CONNECT_TIMEOUT`] gets none, so its timeout stays the one global timeout and keeps its
/// classification (a missing reply); a longer one gets a separate connect timeout, which never
/// exceeds the budget.
fn connect_timeout(global: Duration) -> Option<Duration> {
    (global > CONNECT_TIMEOUT).then_some(CONNECT_TIMEOUT)
}

const CLOSED: &str = "the device closed the connection without replying";

/// Classify an I/O error. A connection the peer closed or reset before any reply is
/// `ConnectionClosed`; refused and unreachable happen while connecting (nothing was sent); a
/// timeout from the OS cannot say where it happened, so it counts as a missing reply.
fn from_io(io: &std::io::Error) -> Error {
    use std::io::ErrorKind as K;
    match io.kind() {
        K::ConnectionRefused => Error::transport(TransportKind::Refused, format!("connection refused ({io})")),
        K::HostUnreachable | K::NetworkUnreachable | K::NetworkDown => Error::transport(TransportKind::Unreachable, format!("unreachable ({io})")),
        K::TimedOut | K::WouldBlock => Error::transport(TransportKind::Timeout, format!("timed out ({io})")),
        K::UnexpectedEof | K::ConnectionReset | K::ConnectionAborted | K::BrokenPipe => Error::transport(TransportKind::ConnectionClosed, CLOSED),
        _ => Error::transport(TransportKind::Io, io.to_string()),
    }
}

/// Map a `ureq` error onto a typed transport failure, as precisely as `ureq` allows. Its global
/// timeout does not say which phase ran out, so that one counts as a missing reply (the request may
/// have been delivered); the per-phase timeouts keep their phase.
fn map_error(e: ureq::Error) -> Error {
    use ureq::Error as U;
    use ureq::Timeout as T;
    let message = e.to_string();
    match e {
        U::Timeout(T::Resolve | T::Connect) => Error::transport_at(TransportKind::ConnectTimeout, Stage::Connect, message),
        U::Timeout(T::SendRequest | T::SendBody) => Error::transport_at(TransportKind::Timeout, Stage::Request, message),
        U::Timeout(_) => Error::transport_at(TransportKind::Timeout, Stage::Reply, message),
        U::Io(io) => from_io(&io),
        U::HostNotFound | U::ConnectionFailed => Error::transport_at(TransportKind::Unreachable, Stage::Connect, message),
        U::TooManyRedirects | U::RedirectFailed => Error::transport_at(TransportKind::Redirect, Stage::Reply, message),
        // The peer did not speak HTTP.
        U::Protocol(_) | U::LargeResponseHeader(..) => Error::transport_at(TransportKind::InvalidResponse, Stage::Reply, message),
        // Our own request could not be built (nothing was sent; the url and headers are fixed by
        // the policy, so this is a bug or a ureq limit rather than the device).
        U::Http(_) | U::BadUri(_) | U::InvalidProxyUrl | U::RequireHttpsOnly(_) => Error::Invalid(message),
        // The reply body was larger than ureq's own limit. That is raised while the body is read, so the request was
        // delivered: a reply failure, the same as our own size cap, never "nothing was sent".
        U::BodyExceedsLimit(limit) => Error::TooLarge("the device's reply", usize::try_from(limit).unwrap_or(usize::MAX)),
        // Anything ureq adds later or that fits nothing above: a transient, unclassified failure.
        _ => Error::transport_at(TransportKind::Io, Stage::Reply, message),
    }
}

/// An error from reading the body: `ureq` wraps its own errors (timeouts) in `io::Error`.
fn map_body_error(e: std::io::Error) -> Error {
    match e.downcast::<ureq::Error>() {
        Ok(inner) => map_error(inner),
        Err(io) => from_io(&io),
    }
}

/// The `Retry-After` of a reply: the one value, parsed and capped. Several `Retry-After` headers
/// are only believed when they agree; an unreadable one voids the lot.
fn retry_after(headers: &ureq::http::HeaderMap, now: SystemTime) -> Option<Duration> {
    let mut found = None;
    for value in headers.get_all("retry-after").iter().take(8) {
        let wait = parse_retry_after(value.to_str().ok()?, now)?;
        match found {
            None => found = Some(wait),
            Some(prev) if prev == wait => {}
            Some(_) => return None,
        }
    }
    found
}

impl Transport for HttpTransport {
    /// POST `request` with only `Content-Type`, `SOAPAction` and `Accept` headers, bounded by the
    /// request's timeout. Redirects are refused and a body larger than `max_response` is
    /// [`Error::TooLarge`]; `Retry-After` is read into the response.
    fn post(&self, url: &DeviceUrl, request: Request) -> Result<Response, Error> {
        let mut builder = self
            .agent
            .post(url.as_url())
            .config()
            .timeout_global(Some(request.timeout))
            .timeout_connect(connect_timeout(request.timeout))
            .build()
            .header("Content-Type", &request.content_type)
            .header("SOAPAction", format!("\"{}\"", request.soap_action));
        // Nothing else from the caller reaches the headers.
        builder = builder.header("Accept", "application/soap+xml, multipart/related");
        let mut response = builder.send(&request.body[..]).map_err(map_error)?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            return Err(Error::transport_at(
                TransportKind::Redirect,
                Stage::Reply,
                format!("the device answered with a redirect (HTTP {status}), which is refused"),
            ));
        }
        let content_type = response.headers().get("content-type").and_then(|v| v.to_str().ok()).map(str::to_owned);
        let retry_after = retry_after(response.headers(), SystemTime::now());
        let mut body = Vec::new();
        let cap = request.max_response as u64;
        response.body_mut().as_reader().take(cap.saturating_add(1)).read_to_end(&mut body).map_err(map_body_error)?;
        if body.len() > request.max_response {
            return Err(Error::TooLarge("the device's reply", request.max_response));
        }
        Ok(Response::new(status, content_type, body).with_retry_after(retry_after))
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Error as IoError, ErrorKind as K};

    use super::*;

    #[test]
    fn a_reply_body_over_ureqs_limit_is_a_reply_failure_not_an_unsent_request() {
        let e = map_error(ureq::Error::BodyExceedsLimit(1024));
        assert!(matches!(e, Error::TooLarge("the device's reply", 1024)), "{e:?}");
        assert_eq!(e.stage(), Stage::Reply);
        // The request was delivered, so a send is outcome-unknown, never safe to repeat (it would print twice).
        assert_ne!(e.retry_safety(false), crate::retry::RetrySafety::Safe);
        // The same limit reached while reading the body takes the same road.
        let from_body = map_body_error(IoError::other(ureq::Error::BodyExceedsLimit(7)));
        assert!(matches!(from_body, Error::TooLarge(..)), "{from_body:?}");
    }

    #[test]
    fn a_long_budget_gets_a_ten_second_connect_limit_and_a_short_one_none() {
        let secs = Duration::from_secs;
        assert_eq!(connect_timeout(secs(300)), Some(secs(10)));
        assert_eq!(connect_timeout(secs(11)), Some(secs(10)));
        // At or under ten seconds the global timeout alone applies, as before.
        assert_eq!(connect_timeout(secs(10)), None);
        assert_eq!(connect_timeout(secs(2)), None);
        // The connect limit never exceeds the budget.
        for g in [1u64, 5, 10, 11, 60] {
            assert!(connect_timeout(secs(g)).is_none_or(|c| c <= secs(g)));
        }
    }

    #[test]
    fn a_connect_timeout_is_an_unsent_retry_safe_connect_failure() {
        let e = map_error(ureq::Error::Timeout(ureq::Timeout::Connect));
        assert_eq!(kind_stage(&e), (Some(TransportKind::ConnectTimeout), Stage::Connect));
        // Nothing was sent, so even a send is safe to repeat.
        assert_eq!(e.retry_safety(false), crate::retry::RetrySafety::Safe);
        assert_eq!(e.retry_safety(true), crate::retry::RetrySafety::Safe);
    }

    /// The transport kind and stage an error reports, for compact assertions.
    fn kind_stage(e: &Error) -> (Option<TransportKind>, Stage) {
        (e.transport_kind(), e.stage())
    }

    #[test]
    fn io_errors_map_to_precise_kinds() {
        use TransportKind as T;
        for (k, want, stage) in [
            (K::ConnectionRefused, T::Refused, Stage::Connect),
            (K::HostUnreachable, T::Unreachable, Stage::Connect),
            (K::NetworkUnreachable, T::Unreachable, Stage::Connect),
            (K::NetworkDown, T::Unreachable, Stage::Connect),
            (K::TimedOut, T::Timeout, Stage::Reply),
            (K::WouldBlock, T::Timeout, Stage::Reply),
            (K::UnexpectedEof, T::ConnectionClosed, Stage::Reply),
            (K::ConnectionReset, T::ConnectionClosed, Stage::Reply),
            (K::ConnectionAborted, T::ConnectionClosed, Stage::Reply),
            (K::BrokenPipe, T::ConnectionClosed, Stage::Reply),
            (K::PermissionDenied, T::Io, Stage::Reply),
            (K::InvalidData, T::Io, Stage::Reply),
        ] {
            assert_eq!(kind_stage(&map_error(ureq::Error::Io(IoError::from(k)))), (Some(want), stage), "{k:?}");
            assert_eq!(kind_stage(&map_body_error(IoError::from(k))), (Some(want), stage), "{k:?} while reading the body");
        }
    }

    #[test]
    fn ureq_errors_map_to_precise_kinds() {
        use TransportKind as T;
        use ureq::Timeout as UT;
        let cases = [
            (ureq::Error::Timeout(UT::Connect), T::ConnectTimeout, Stage::Connect),
            (ureq::Error::Timeout(UT::Resolve), T::ConnectTimeout, Stage::Connect),
            (ureq::Error::Timeout(UT::SendRequest), T::Timeout, Stage::Request),
            (ureq::Error::Timeout(UT::SendBody), T::Timeout, Stage::Request),
            (ureq::Error::Timeout(UT::RecvResponse), T::Timeout, Stage::Reply),
            (ureq::Error::Timeout(UT::RecvBody), T::Timeout, Stage::Reply),
            (ureq::Error::Timeout(UT::Global), T::Timeout, Stage::Reply),
            (ureq::Error::HostNotFound, T::Unreachable, Stage::Connect),
            (ureq::Error::ConnectionFailed, T::Unreachable, Stage::Connect),
            (ureq::Error::TooManyRedirects, T::Redirect, Stage::Reply),
            (ureq::Error::RedirectFailed, T::Redirect, Stage::Reply),
            (ureq::Error::LargeResponseHeader(70000, 65536), T::InvalidResponse, Stage::Reply),
            (ureq::Error::TlsRequired, T::Io, Stage::Reply),
        ];
        for (e, want, stage) in cases {
            let text = e.to_string();
            let got = map_error(e);
            assert_eq!(kind_stage(&got), (Some(want), stage), "{text}");
            assert!(got.to_string().contains(&text), "{got} / {text}");
        }
        // Our own malformed request is not a transport failure and nothing was sent.
        let bad = map_error(ureq::Error::BadUri("x".into()));
        assert!(matches!(bad, Error::Invalid(_)) && bad.stage() == Stage::Prepare, "{bad:?}");
        // A ureq timeout that arrives wrapped in an io::Error while the body is read keeps its kind.
        let wrapped = IoError::other(ureq::Error::Timeout(UT::RecvBody));
        assert_eq!(kind_stage(&map_body_error(wrapped)), (Some(T::Timeout), Stage::Reply));
    }

    /// A header map with one `Retry-After` per value (an unrepresentable value becomes empty).
    fn headers(values: &[&str]) -> ureq::http::HeaderMap {
        let mut map = ureq::http::HeaderMap::new();
        for v in values {
            map.append("retry-after", ureq::http::HeaderValue::from_str(v).unwrap_or_else(|_| ureq::http::HeaderValue::from_static("")));
        }
        map
    }

    #[test]
    fn retry_after_headers_are_believed_only_when_unambiguous() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let secs = |v: &[&str]| retry_after(&headers(v), now).map(|d| d.as_secs());
        assert_eq!(secs(&[]), None);
        assert_eq!(secs(&["30"]), Some(30));
        assert_eq!(secs(&["30", "30"]), Some(30), "agreeing duplicates");
        assert_eq!(secs(&["30", "31"]), None, "conflicting duplicates");
        assert_eq!(secs(&["30", "soon"]), None, "an unreadable one voids the rest");
        assert_eq!(secs(&["soon"]), None);
        assert_eq!(secs(&["99999999999999999999999"]), Some(3600));
        assert_eq!(secs(&["Thu, 01 Jan 1970 00:17:30 GMT"]), Some(50), "an HTTP-date is measured from now");
        assert_eq!(secs(&["1"; 20]), Some(1));
        // A value that is not text is ignored, not a panic.
        let mut raw = ureq::http::HeaderMap::new();
        raw.append("retry-after", ureq::http::HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap_or_else(|_| ureq::http::HeaderValue::from_static("")));
        assert_eq!(retry_after(&raw, now), None);
    }
}
