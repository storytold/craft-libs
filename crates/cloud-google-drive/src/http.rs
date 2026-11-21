//! The HTTP seam. [`Transport`] sends one request and returns the whole (bounded) response.

use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Patch,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Patch => "PATCH",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// Shared, so a retry after a refreshed token doesn't copy a large upload again.
    pub body: Arc<[u8]>,
    /// The most response bytes to accept, after decompression.
    pub limit: u64,
    /// The longest the whole call may take, body included: a server trickling bytes can't hold
    /// the backend's lock for ever.
    pub deadline: Duration,
}

impl std::fmt::Debug for Request {
    // Headers carry bearer tokens and bodies carry codes and refresh tokens: log neither.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request").field("method", &self.method).field("url", &self.url).field("body_len", &self.body.len()).finish_non_exhaustive()
    }
}

impl Request {
    pub fn new(method: Method, url: impl Into<String>) -> Self {
        Self { method, url: url.into(), headers: Vec::new(), body: Arc::from(Vec::new()), limit: JSON_LIMIT, deadline: CALL_DEADLINE }
    }
    pub fn header(mut self, k: &str, v: impl Into<String>) -> Self {
        self.headers.push((k.to_owned(), v.into()));
        self
    }
    pub fn body(mut self, content_type: &str, body: impl Into<Arc<[u8]>>) -> Self {
        self.body = body.into();
        self.header("Content-Type", content_type)
    }
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = limit;
        self
    }
    pub fn deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }
    pub fn bearer(self, token: &str) -> Self {
        self.header("Authorization", format!("Bearer {token}"))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl std::fmt::Debug for Response {
    // A token endpoint's answer carries access and refresh tokens: never print the body.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Response").field("status", &self.status).field("body_len", &self.body.len()).finish()
    }
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
    /// The response as JSON when the status is 2xx, otherwise the API error it describes.
    pub fn json(&self) -> Result<serde_json::Value> {
        if !self.ok() {
            return Err(self.api_error());
        }
        serde_json::from_slice(&self.body).map_err(|e| Error::Protocol(format!("not JSON: {e}")))
    }
    /// Google's error JSON (`{"error":{"message":…}}` or `{"error":"…","error_description":…}`)
    /// turned into [`Error::Api`]; falls back to a short prefix of the body.
    pub fn api_error(&self) -> Error {
        let v: serde_json::Value = serde_json::from_slice(&self.body).unwrap_or_default();
        let err = v.get("error");
        let message = err
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .or_else(|| v.get("error_description").and_then(|m| m.as_str()))
            .or_else(|| err.and_then(|e| e.as_str()))
            .map(str::to_owned)
            .unwrap_or_else(|| String::from_utf8_lossy(self.body.get(..200).unwrap_or(&self.body)).into_owned());
        Error::Api { status: self.status, message }
    }
}

/// Default limit for JSON responses.
pub const JSON_LIMIT: u64 = 4 << 20;
/// Default deadline for one call (OAuth, metadata, a listing page).
pub const CALL_DEADLINE: Duration = Duration::from_secs(120);
/// Deadline for a document download or upload: long enough for the largest document on a slow
/// link, short enough that a stalled one ends.
pub const TRANSFER_DEADLINE: Duration = Duration::from_secs(60 * 60);

pub trait Transport: Send + Sync {
    fn send(&self, req: Request) -> Result<Response>;
}

/// `ureq` with rustls and the platform's certificate verifier; HTTP error statuses are returned
/// as responses so [`Response::api_error`] can read Google's message.
///
/// `ureq_proto` dumps every byte on the wire at `TRACE` level, unredacted: bearer tokens, the
/// token endpoint's form bodies and its answers. While a host's logger has that enabled,
/// [`Ureq`] sends nothing and says why, so no credential reaches a log.
pub struct Ureq {
    agent: ureq::Agent,
}

impl Ureq {
    /// A transport that names `user_agent` (the app, and its version) to Google.
    pub fn new(user_agent: &str) -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            // Limits on a stalled connection, not on the whole call: a large document on a slow
            // link takes as long as it takes while bytes keep moving.
            .timeout_connect(Some(std::time::Duration::from_secs(30)))
            .timeout_per_read(Some(std::time::Duration::from_secs(60)))
            .timeout_per_write(Some(std::time::Duration::from_secs(60)))
            .timeout_recv_response(Some(std::time::Duration::from_secs(120)))
            .user_agent(user_agent)
            // The operating system's trust store, so a CA a managed machine installed is trusted.
            .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
            .build();
        Self { agent: config.into() }
    }
}

/// Whether the host's logger would record `ureq_proto`'s wire dump.
pub fn wire_traced() -> bool {
    WIRE_TARGETS.iter().any(|t| log::log_enabled!(target: t, log::Level::Trace))
}

/// Where `ureq_proto` logs the bytes it sends and receives (its `util` module), and the crate as
/// a whole, which a filter can name instead.
const WIRE_TARGETS: [&str; 2] = ["ureq_proto::util", "ureq_proto"];

impl Transport for Ureq {
    fn send(&self, req: Request) -> Result<Response> {
        // Checked on every call: a host can change its log level while running.
        if wire_traced() {
            return Err(Error::Invalid(
                "Google Drive is off while TRACE logging is on for ureq_proto: it would record sign-in tokens. Lower that log level to use Google Drive"
                    .into(),
            ));
        }
        let mut b = ureq::http::Request::builder().method(req.method.as_str()).uri(&req.url);
        for (k, v) in &req.headers {
            b = b.header(k.as_str(), v.as_str());
        }
        let r = b.body(&*req.body).map_err(|e| Error::Invalid(format!("bad request: {e}")))?;
        let r = self.agent.configure_request(r).timeout_global(Some(req.deadline)).build();
        let mut resp = self.agent.run(r).map_err(|e| Error::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        // Bounded after decompression, with one byte of room to tell "exactly the limit" from
        // "over it".
        let mut body = Vec::new();
        resp.body_mut().as_reader().take(req.limit.saturating_add(1)).read_to_end(&mut body).map_err(|e| Error::Network(e.to_string()))?;
        bounded(body, req.limit).map(|body| Response { status, body })
    }
}

/// `body` when it holds at most `limit` bytes.
pub(crate) fn bounded(body: Vec<u8>, limit: u64) -> Result<Vec<u8>> {
    if body.len() as u64 > limit {
        return Err(Error::Protocol(format!("response larger than {limit} bytes")));
    }
    Ok(body)
}
