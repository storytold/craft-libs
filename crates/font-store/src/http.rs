//! The one HTTP client: blocking HTTPS GET through ureq with rustls, timeouts, a size cap,
//! progress and cancellation.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

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

/// The User-Agent every request sends: the crate and its version, nothing about the user.
pub const USER_AGENT: &str = concat!("craft-font-store/", env!("CARGO_PKG_VERSION"));

/// The HTTP client. HTTPS only; no cookies, no credentials.
#[derive(Clone)]
pub struct Client {
    agent: ureq::Agent,
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
        let config = ureq::Agent::config_builder()
            .https_only(https_only)
            .user_agent(USER_AGENT)
            .timeout_connect(Some(t.connect))
            .timeout_recv_response(Some(t.response))
            .timeout_global(Some(t.overall))
            .max_redirects(3)
            .http_status_as_error(true)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            #[cfg(test)]
            rewrite: Vec::new(),
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
        if cancel.is_cancelled() {
            return Err(FetchError::Cancelled);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (agent, url, flag) = (self.agent.clone(), self.url(url), Arc::clone(&stop));
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
