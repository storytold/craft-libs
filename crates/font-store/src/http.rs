//! The one HTTP client: blocking HTTPS GET through ureq with rustls, timeouts, a size cap,
//! progress and cancellation. Plain http only to a loopback host, and only when the caller opts
//! in for a preview service run locally.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Cancels a download from another thread. Clones share the flag.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// A flag that isn't set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks the download to stop. It stops within a few tens of milliseconds, even while a
    /// server sends nothing.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether [`Cancel::cancel`] was called.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// How long a request may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    /// Connecting: DNS, TCP and TLS.
    pub connect: Duration,
    /// From sending the request to the response's headers.
    pub response: Duration,
    /// The whole request, body included.
    pub overall: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        // Ten minutes moves the largest Google Fonts file (about 60 MB) over a 1 Mbit/s line.
        Self { connect: Duration::from_secs(15), response: Duration::from_secs(30), overall: Duration::from_secs(600) }
    }
}

impl FetchError {
    /// Whether the server couldn't be reached or didn't answer in time, as opposed to answering
    /// with an HTTP status or too much data.
    pub fn is_network(&self) -> bool {
        matches!(self, FetchError::Failed(m) if !m.starts_with("HTTP "))
    }
}

/// Why a request failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    /// [`Cancel::cancel`] was called.
    #[error("cancelled")]
    Cancelled,
    /// The body is larger than allowed.
    #[error("the response is larger than {max} bytes")]
    TooLarge {
        /// The limit.
        max: u64,
    },
    /// Anything else: connection, TLS, timeout, an HTTP status other than 2xx.
    #[error("{0}")]
    Failed(String),
}

/// How long a preview service that failed on the network is skipped.
pub const SERVICE_BACKOFF: Duration = Duration::from_secs(60);

/// `scheme://host:port` of `url`, lowercased.
fn service_origin(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    format!("{}://{}", scheme.to_ascii_lowercase(), authority.to_ascii_lowercase())
}

fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// The User-Agent every request sends: the crate and its version, nothing about the user.
pub const USER_AGENT: &str = concat!("craft-font-store/", env!("CARGO_PKG_VERSION"));

/// The HTTP client. HTTPS only, except [`Client::get_from_service`], which also speaks plain
/// http to `127.0.0.1` or `::1`; no cookies, no credentials.
#[derive(Clone)]
pub struct Client {
    agent: ureq::Agent,
    /// For `http://` to a loopback host. It follows no redirects, so it never leaves the machine.
    loopback: ureq::Agent,
    /// The origin of the preview service that failed on the network, and when it may be tried
    /// again. Another service isn't affected. Clones share it.
    service_down: Arc<Mutex<Option<(String, u64)>>>,
    https_only: bool,
    /// URL prefixes the tests serve from a local server.
    #[cfg(test)]
    pub(crate) rewrite: Vec<(String, String)>,
}

impl Default for Client {
    fn default() -> Self {
        Self::new(Timeouts::default())
    }
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Client")
    }
}

impl Client {
    /// A client with these timeouts.
    pub fn new(timeouts: Timeouts) -> Self {
        Self::build(timeouts, true)
    }

    fn build(t: Timeouts, https_only: bool) -> Self {
        let agent = |https_only: bool, redirects: u32| {
            let builder = ureq::Agent::config_builder();
            // A loopback request never goes through an environment proxy.
            let builder = if https_only || redirects > 0 { builder } else { builder.proxy(None) };
            let config = builder
                .https_only(https_only)
                .user_agent(USER_AGENT)
                .timeout_connect(Some(t.connect))
                .timeout_recv_response(Some(t.response))
                .timeout_global(Some(t.overall))
                .max_redirects(redirects)
                .http_status_as_error(true)
                .build();
            ureq::Agent::new_with_config(config)
        };
        Self {
            agent: agent(https_only, 3),
            loopback: agent(false, 0),
            service_down: Arc::new(Mutex::new(None)),
            https_only,
            #[cfg(test)]
            rewrite: Vec::new(),
        }
    }

    /// A client with other timeouts that shares this one's preview-service down state, so a
    /// pool with short timeouts (previews) and the installs agree on which service is down.
    pub fn with_timeouts(&self, timeouts: Timeouts) -> Client {
        let fresh = Self::build(timeouts, self.https_only);
        Client {
            service_down: Arc::clone(&self.service_down),
            #[cfg(test)]
            rewrite: self.rewrite.clone(),
            ..fresh
        }
    }

    /// Plain `http://` to a local test server, with `https://` prefixes mapped onto it.
    #[cfg(test)]
    pub(crate) fn for_tests(t: Timeouts, rewrite: Vec<(String, String)>) -> Self {
        Self { rewrite, ..Self::build(t, false) }
    }

    fn url(&self, url: &str) -> String {
        #[cfg(test)]
        for (from, to) in &self.rewrite {
            if let Some(rest) = url.strip_prefix(from.as_str()) {
                return format!("{to}{rest}");
            }
        }
        url.to_string()
    }

    /// GETs `url`, at most `max` bytes, calling `on_bytes` with the bytes read so far. The
    /// request runs on a helper thread, so `cancel` stops it even while the server sends nothing.
    pub fn get(&self, url: &str, max: u64, cancel: &Cancel, on_bytes: &mut dyn FnMut(u64)) -> Result<Vec<u8>, FetchError> {
        self.request(url, max, cancel, on_bytes, false)
    }

    /// [`Client::get`] for a URL built from the validated preview-service setting: plain `http://`
    /// to a loopback host is allowed, without following redirects. Never call it with a URL from
    /// an index or a user source.
    ///
    /// After a network failure (no connection, a timeout; not an HTTP status) that service (its
    /// scheme, host and port) counts as down for [`SERVICE_BACKOFF`]: calls in that time fail at once, so a service that is off
    /// doesn't make every request wait out the timeouts. Clones of the client share this.
    pub fn get_from_service(&self, url: &str, max: u64, cancel: &Cancel, on_bytes: &mut dyn FnMut(u64)) -> Result<Vec<u8>, FetchError> {
        if self.service_is_down(url) {
            return Err(FetchError::Failed("the preview service is down".into()));
        }
        let got = self.request(url, max, cancel, on_bytes, true);
        if got.as_ref().is_err_and(FetchError::is_network) {
            self.mark_service_down(url);
        }
        got
    }

    /// Whether a network failure of the preview service at `service` (its URL or any URL on it)
    /// is recent enough to skip it. Another service is not down.
    pub fn service_is_down(&self, service: &str) -> bool {
        let down = self.service_down.lock().unwrap_or_else(|e| e.into_inner());
        down.as_ref().is_some_and(|(origin, until)| *origin == service_origin(service) && now_millis() < *until)
    }

    /// Marks the preview service at `service` as down for [`SERVICE_BACKOFF`].
    pub fn mark_service_down(&self, service: &str) {
        let until = now_millis() + SERVICE_BACKOFF.as_millis() as u64;
        *self.service_down.lock().unwrap_or_else(|e| e.into_inner()) = Some((service_origin(service), until));
    }

    fn request(&self, url: &str, max: u64, cancel: &Cancel, on_bytes: &mut dyn FnMut(u64), allow_loopback_http: bool) -> Result<Vec<u8>, FetchError> {
        if cancel.is_cancelled() {
            return Err(FetchError::Cancelled);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let url = self.url(url);
        let agent = if allow_loopback_http && is_loopback_http(&url) { self.loopback.clone() } else { self.agent.clone() };
        let flag = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("font-store-fetch".into())
            .spawn(move || {
                let progress = tx.clone();
                let result = download(&agent, &url, max, &flag, &mut |n| {
                    let _ = progress.send(Msg::Bytes(n));
                });
                // After a cancel nobody receives this.
                let _ = tx.send(Msg::Done(result));
            })
            .map_err(|e| FetchError::Failed(format!("can't start the download: {e}")))?;
        loop {
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(Msg::Bytes(n)) => on_bytes(n),
                Ok(Msg::Done(r)) => return r,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(FetchError::Failed("the download stopped unexpectedly".into())),
            }
            if cancel.is_cancelled() {
                stop.store(true, Ordering::Relaxed);
                return Err(FetchError::Cancelled);
            }
        }
    }
}

/// Whether `url` is `http://` to `127.0.0.1` or `[::1]`.
pub(crate) fn is_loopback_http(url: &str) -> bool {
    let Some(rest) = url.get(..7).filter(|s| s.eq_ignore_ascii_case("http://")).and_then(|_| url.get(7..)) else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return false;
    }
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => authority.split(':').next().unwrap_or_default(),
    };
    matches!(host.to_ascii_lowercase().as_str(), "127.0.0.1" | "::1")
}

enum Msg {
    Bytes(u64),
    Done(Result<Vec<u8>, FetchError>),
}

fn describe(e: &ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(code) => format!("HTTP {code}"),
        ureq::Error::Timeout(_) => "timed out".into(),
        other => other.to_string(),
    }
}

fn download(agent: &ureq::Agent, url: &str, max: u64, stop: &AtomicBool, on_bytes: &mut dyn FnMut(u64)) -> Result<Vec<u8>, FetchError> {
    let mut resp = agent.get(url).call().map_err(|e| FetchError::Failed(describe(&e)))?;
    if resp.body().content_length().is_some_and(|n| n > max) {
        return Err(FetchError::TooLarge { max });
    }
    // One byte over the cap is an error, not a truncated file.
    let mut reader = resp.body_mut().with_config().limit(max.saturating_add(1)).reader();
    let mut out: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err(FetchError::Cancelled);
        }
        let n = reader.read(&mut buf).map_err(|e| FetchError::Failed(format!("reading the response failed: {e}")))?;
        if n == 0 {
            return Ok(out);
        }
        if (out.len() as u64).saturating_add(n as u64) > max {
            return Err(FetchError::TooLarge { max });
        }
        out.extend_from_slice(buf.get(..n).unwrap_or_default());
        on_bytes(out.len() as u64);
    }
}
