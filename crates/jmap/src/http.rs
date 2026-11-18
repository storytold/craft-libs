//! Descriptions of one HTTP exchange, without performing it.
//!
//! The crate builds an [`HttpRequest`]; the program performs it however it can (a native HTTP client, the
//! browser's Fetch API, a test double) and describes what came back in an [`HttpResponse`]. The
//! [`HttpExchange`] trait is the shape of that service when the crate's asynchronous helpers are used.
//!
//! What the program performing a request must do:
//!
//! * send the method, address, header fields and body exactly as given;
//! * follow no redirect itself (or, if it must, check each target with
//!   [`Address::check_link`](crate::Address::check_link) and drop the credentials when the origin changes);
//! * read [`HttpRequest::max_response_bytes`] *before* reading the body (it is the kind's limit, or the lower
//!   one the caller set with [`HttpRequest::with_response_limit`]), refuse at once a `Content-Length` above it,
//!   and stop reading as soon as the body passes it, reporting [`TransportError::BodyTooLarge`]. The crate
//!   refuses a longer body anyway, but only after it has been read;
//! * copy into the response the header fields named in [`HttpResponse::FIELDS_READ`].

use std::fmt;
use std::future::Future;

use crate::address::Address;
use crate::credentials::Secret;

/// The most a session object may weigh, in octets.
pub const SESSION_RESPONSE_LIMIT: usize = 4 * 1024 * 1024;
/// The most an API response may weigh, in octets.
pub const API_RESPONSE_LIMIT: usize = 32 * 1024 * 1024;
/// The most a downloaded blob may weigh, in octets.
pub const DOWNLOAD_RESPONSE_LIMIT: usize = 256 * 1024 * 1024;
/// The most of an error response's body that is read for a problem details object, in octets.
pub const PROBLEM_BODY_LIMIT: usize = 64 * 1024;
/// The longest header field value the response keeps; longer ones are dropped.
pub const MAX_FIELD_VALUE_LEN: usize = 8 * 1024;

/// The request method.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    /// `GET`
    Get,
    /// `POST`
    Post,
}

impl Method {
    /// The method's name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

/// What a request is for; it decides the response size limit and what a successful answer must look like.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RequestKind {
    /// Fetching the session object (RFC 8620 §2).
    Session,
    /// An API request (RFC 8620 §3.1).
    Api,
    /// Downloading a blob (RFC 8620 §6.2).
    Download,
}

impl RequestKind {
    /// The largest response body accepted for this kind of request, in octets.
    pub fn max_response_bytes(self) -> usize {
        match self {
            Self::Session => SESSION_RESPONSE_LIMIT,
            Self::Api => API_RESPONSE_LIMIT,
            Self::Download => DOWNLOAD_RESPONSE_LIMIT,
        }
    }

    /// Whether a successful answer must be `application/json`.
    pub fn expects_json(self) -> bool {
        matches!(self, Self::Session | Self::Api)
    }
}

#[derive(Clone)]
enum FieldValue {
    Plain(String),
    Secret(Secret),
}

impl FieldValue {
    fn as_str(&self) -> &str {
        match self {
            Self::Plain(text) => text,
            Self::Secret(secret) => secret.expose(),
        }
    }
}

/// A request for the program to perform.
#[derive(Clone)]
pub struct HttpRequest {
    method: Method,
    address: Address,
    fields: Vec<(&'static str, FieldValue)>,
    body: Vec<u8>,
    kind: RequestKind,
    max_response: usize,
}

impl HttpRequest {
    pub(crate) fn new(method: Method, address: Address, kind: RequestKind, authorization: &Secret) -> Self {
        let mut fields = vec![("Authorization", FieldValue::Secret(authorization.clone()))];
        if kind.expects_json() {
            fields.push(("Accept", FieldValue::Plain("application/json".to_owned())));
        }
        Self { method, address, fields, body: Vec::new(), kind, max_response: kind.max_response_bytes() }
    }

    /// Lowers the largest response body accepted for this request to `bytes`. A value above the kind's
    /// limit ([`RequestKind::max_response_bytes`]) is reduced to it: the limit can only go down.
    pub fn with_response_limit(mut self, bytes: usize) -> Self {
        self.max_response = bytes.min(self.kind.max_response_bytes());
        self
    }

    pub(crate) fn with_json_body(mut self, body: Vec<u8>) -> Self {
        self.fields.push(("Content-Type", FieldValue::Plain("application/json".to_owned())));
        self.body = body;
        self
    }

    /// The method.
    pub fn method(&self) -> Method {
        self.method
    }

    /// The target address.
    pub fn address(&self) -> &Address {
        &self.address
    }

    /// The full target address as text, for the request line.
    pub fn url(&self) -> &str {
        self.address.as_str()
    }

    /// The header fields to send, `Authorization` included.
    pub fn fields(&self) -> impl Iterator<Item = (&str, &str)> {
        self.fields.iter().map(|(name, value)| (*name, value.as_str()))
    }

    /// The body (empty for `GET`).
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// What the request is for.
    pub fn kind(&self) -> RequestKind {
        self.kind
    }

    /// The most body octets to read from the response: the kind's limit, or the lower one set with
    /// [`HttpRequest::with_response_limit`]. The program reads it before the body, to stop early.
    pub fn max_response_bytes(&self) -> usize {
        self.max_response
    }
}

impl fmt::Debug for HttpRequest {
    /// Method, origin, field names (the `Authorization` value hidden) and body length: no path, no query, no
    /// secret, no content.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fields: Vec<String> = self
            .fields
            .iter()
            .map(|(name, value)| match value {
                FieldValue::Plain(text) => format!("{name}: {text}"),
                FieldValue::Secret(_) => format!("{name}: <redacted>"),
            })
            .collect();
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("origin", &self.address.origin())
            .field("kind", &self.kind)
            .field("fields", &fields)
            .field("body_len", &self.body.len())
            .field("max_response_bytes", &self.max_response)
            .finish()
    }
}

/// What came back, as the program describes it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct HttpResponse {
    status: u16,
    content_type: Option<String>,
    retry_after: Option<String>,
    location: Option<String>,
    body: Vec<u8>,
}

impl HttpResponse {
    /// The header fields the crate reads; others may be left out.
    pub const FIELDS_READ: [&'static str; 3] = ["Content-Type", "Retry-After", "Location"];

    /// A response with a status code and a body.
    pub fn new(status: u16, body: Vec<u8>) -> Self {
        Self { status, body, ..Self::default() }
    }

    /// Adds a header field; names are matched without regard to case (RFC 9110 §5.1) and only those of
    /// [`HttpResponse::FIELDS_READ`] are kept. A value longer than [`MAX_FIELD_VALUE_LEN`] is dropped.
    /// When a field comes twice, the first value is kept.
    pub fn with_field(mut self, name: &str, value: &str) -> Self {
        self.set_field(name, value);
        self
    }

    /// Like [`HttpResponse::with_field`], in place.
    pub fn set_field(&mut self, name: &str, value: &str) {
        if value.len() > MAX_FIELD_VALUE_LEN {
            return;
        }
        let slot = if name.eq_ignore_ascii_case("content-type") {
            &mut self.content_type
        } else if name.eq_ignore_ascii_case("retry-after") {
            &mut self.retry_after
        } else if name.eq_ignore_ascii_case("location") {
            &mut self.location
        } else {
            return;
        };
        if slot.is_none() {
            *slot = Some(value.trim_matches([' ', '\t']).to_owned());
        }
    }

    /// The status code.
    pub fn status(&self) -> u16 {
        self.status
    }

    /// The `Content-Type` field value.
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// The `Retry-After` field value.
    pub fn retry_after(&self) -> Option<&str> {
        self.retry_after.as_deref()
    }

    /// The `Location` field value.
    pub fn location(&self) -> Option<&str> {
        self.location.as_deref()
    }

    /// The body.
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// The body, taken out of the response.
    pub fn into_body(self) -> Vec<u8> {
        self.body
    }
}

impl fmt::Debug for HttpResponse {
    /// Status, content type and body length: the body may hold someone's mail.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let short = |value: &Option<String>| value.as_deref().map(|text| text.chars().take(64).collect::<String>());
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("content_type", &short(&self.content_type))
            .field("retry_after", &short(&self.retry_after))
            .field("has_location", &self.location.is_some())
            .field("body_len", &self.body.len())
            .finish()
    }
}

/// Why the program could not complete an exchange; no free text, so nothing private can travel in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransportError {
    /// The server is overloaded at the connection level (connections refused under load, an HTTP/2
    /// `REFUSED_STREAM`, a `GOAWAY` before the request was processed, …). Classified as busy.
    Overloaded,
    /// No answer in the time the program allows.
    TimedOut,
    /// The host could not be reached (name resolution, connection).
    Unreachable,
    /// TLS failed, a certificate among the causes.
    Tls,
    /// The body was longer than [`HttpRequest::max_response_bytes`].
    BodyTooLarge,
    /// The server redirected and the program, as asked, did not follow.
    RedirectRefused,
    /// The exchange was cancelled.
    Aborted,
    /// Anything else.
    Other,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Overloaded => "the server turned the connection away because it is overloaded; try again in a little while",
            Self::TimedOut => "the server did not answer in time; check the network connection and try again",
            Self::Unreachable => "the server could not be reached; check its address and the network connection",
            Self::Tls => "no secure connection could be set up, for example because the server's certificate is invalid",
            Self::BodyTooLarge => "the answer passed the size limit set for this request and was not read further",
            Self::RedirectRefused => "the server redirected the request elsewhere and the redirect was not followed",
            Self::Aborted => "the request was cancelled before it completed",
            Self::Other => "the connection failed in a way the transport did not classify",
        })
    }
}

impl std::error::Error for TransportError {}

/// Performs one HTTP exchange for the crate's asynchronous helpers.
///
/// Before reading the response body, read [`HttpRequest::max_response_bytes`] and stop as soon as the body
/// would pass it, returning [`TransportError::BodyTooLarge`] (see the module documentation).
///
/// The returned future need not be `Send`, so that a browser transport can implement it; the crate's own
/// futures are `Send` whenever the program's are.
pub trait HttpExchange {
    /// Sends `request` and describes the response.
    fn exchange(&self, request: &HttpRequest) -> impl Future<Output = Result<HttpResponse, TransportError>>;
}

#[cfg(test)]
mod tests;
