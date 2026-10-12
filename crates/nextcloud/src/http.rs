//! The HTTP client of one account: trust on first use, a stall limit, retries and error mapping.

use std::fmt;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rustls::crypto::CryptoProvider;
use ureq::http::{Request, Response};
use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{Buffers, ConnectionDetails, Connector, NextTimeout, TcpConnector, Transport, time};
use ureq::{Agent, AsSendBody, Body};

use crate::account::normalize_server;
use crate::tls::{TlsState, TofuTls};
use crate::{Account, Error};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Idle connections kept for reuse.
const MAX_IDLE: usize = 16;
/// 429 and 503 are retried this many times, then the server is `RateLimited`.
const MAX_RETRIES: u32 = 3;
const MAX_RETRY_AFTER_SECS: u64 = 10;

/// What a request is for, which sets its overall time limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // `Range` and `Download` get their first callers in the next tasks
pub(crate) enum Kind {
    /// Listings, previews, small JSON: 30 s.
    Quick,
    /// `read_range`: 60 s.
    Range,
    /// Downloads: no overall limit, only the stall limit.
    Download,
}

impl Kind {
    fn timeout(self) -> Option<Duration> {
        match self {
            Kind::Quick => Some(Duration::from_secs(30)),
            Kind::Range => Some(Duration::from_secs(60)),
            Kind::Download => None,
        }
    }
}

/// HTTP for one account. Clones share the connection pool.
#[derive(Clone)]
pub(crate) struct Http {
    agent: Agent,
    base: String,
    /// The `Authorization` header value; carries the app password.
    auth: Option<String>,
    tls: TlsState,
}

impl fmt::Debug for Http {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Http").field("base", &self.base).field("auth", &self.auth.as_ref().map(|_| "<redacted>")).finish_non_exhaustive()
    }
}

impl Http {
    /// An agent for `account`; `stall` is how long the server may send nothing before the request is `Unreachable`.
    pub(crate) fn new(account: &Account, app_password: Option<&str>, provider: Arc<CryptoProvider>, stall: Duration) -> Result<Http, Error> {
        let base = normalize_server(&account.server)?;
        let tls = TlsState::new(account.trust);
        let tofu = TofuTls::new(provider, &tls, base.starts_with("https://"))?;
        let connector = ().chain(TcpConnector::default()).chain(tofu).chain(StallGuard(stall));
        let config = Agent::config_builder()
            .allow_non_standard_methods(true)
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            // Apps run around 8 workers per account; ureq keeps only 3 idle connections per host by default. One agent
            // serves one host, so the total cap matches.
            .max_idle_connections(MAX_IDLE)
            .max_idle_connections_per_host(MAX_IDLE)
            // The connector chain has no proxy support; ureq would otherwise pick one up from the environment.
            .proxy(None)
            .build();
        let auth = app_password.map(|password| format!("Basic {}", STANDARD.encode(format!("{}:{password}", account.user))));
        Ok(Http { agent: Agent::with_parts(config, connector, DefaultResolver::default()), base, auth, tls })
    }

    /// The normalized server URL, e.g. `https://cloud.example/nc`.
    pub(crate) fn base(&self) -> &str {
        &self.base
    }

    /// Sends a request, retrying 429/503 except Nextcloud's maintenance mode. 401, 404 and redirects are errors; other
    /// statuses go to the caller. The app password goes only to URLs under [`Http::base`].
    pub(crate) fn request(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
        kind: Kind,
    ) -> Result<Response<Body>, Error> {
        let mut retries = 0;
        loop {
            let res = self.send(method, url, headers, body, kind)?;
            match res.status().as_u16() {
                // Maintenance lasts minutes; waiting out the retries would only stall the caller.
                503 if res.headers().get("x-nextcloud-maintenance-mode").is_some_and(|v| v.as_bytes() == b"1") => return Err(Error::RateLimited),
                429 | 503 if retries < MAX_RETRIES => {
                    thread::sleep(retry_delay(&res, retries));
                    retries += 1;
                }
                429 | 503 => return Err(Error::RateLimited),
                401 => return Err(Error::Unauthorized),
                404 => return Err(Error::NotFound),
                300..=399 => return Err(Error::Malformed("unexpected redirect".into())),
                _ => return Ok(res),
            }
        }
    }

    fn send(&self, method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>, kind: Kind) -> Result<Response<Body>, Error> {
        let mut builder = Request::builder().method(method).uri(url);
        let auth = self.auth.as_deref().filter(|_| under_base(&self.base, url));
        for (name, value) in headers.iter().copied().chain(auth.map(|auth| ("Authorization", auth))) {
            builder = builder.header(name, value);
        }
        match body {
            Some(body) => self.run(builder.body(body), kind),
            None => self.run(builder.body(()), kind),
        }
    }

    fn run<S: AsSendBody>(&self, request: Result<Request<S>, ureq::http::Error>, kind: Kind) -> Result<Response<Body>, Error> {
        let request = request.map_err(|e| Error::Malformed(e.to_string()))?;
        let request = self.agent.configure_request(request).timeout_global(kind.timeout()).build();
        self.agent.run(request).map_err(|e| map_ureq(e, &self.tls))
    }
}

/// Whether `url` is `base` or below it: the next character must end the base's path.
fn under_base(base: &str, url: &str) -> bool {
    url.strip_prefix(base).is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '?', '#']))
}

/// `Retry-After` seconds (at most 10), else 0.5 s · 2ⁿ.
fn retry_delay(res: &Response<Body>, retry: u32) -> Duration {
    let after = res.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok());
    match after {
        Some(secs) => Duration::from_secs(secs.min(MAX_RETRY_AFTER_SECS)),
        None => Duration::from_millis(500u64 << retry),
    }
}

/// Reads a whole body; more than `cap` bytes is `TooLarge`.
pub(crate) fn read_capped(body: Body, cap: u64) -> Result<Vec<u8>, Error> {
    // Reading a body can't fail on a certificate, so no TLS state is needed.
    body.into_with_config().limit(cap).read_to_vec().map_err(|e| map_ureq(e, &TlsState::default()))
}

/// Maps a ureq error: a rejected certificate per `tls`, network failures to `Unreachable`.
pub(crate) fn map_ureq(e: ureq::Error, tls: &TlsState) -> Error {
    let tls_error = match &e {
        ureq::Error::Rustls(r) => Some(r),
        // The handshake reports rustls errors wrapped in io errors.
        ureq::Error::Io(io) => io.get_ref().and_then(|inner| inner.downcast_ref::<rustls::Error>()),
        _ => None,
    };
    if let Some(r) = tls_error {
        let rejected = if matches!(r, rustls::Error::InvalidCertificate(_)) { tls.rejected() } else { None };
        return rejected.unwrap_or_else(|| Error::Unreachable(format!("TLS: {r}")));
    }
    match e {
        ureq::Error::BodyExceedsLimit(_) => Error::TooLarge,
        ureq::Error::Io(io) => Error::Unreachable(io.to_string()),
        // Includes the stall limit, which keeps the reason of the timeout it shortened.
        ureq::Error::Timeout(_) => Error::Unreachable("timed out".into()),
        e @ (ureq::Error::HostNotFound | ureq::Error::ConnectionFailed) => Error::Unreachable(e.to_string()),
        e => Error::Malformed(e.to_string()),
    }
}

/// Wraps every transport so no wait for the network is longer than the stall limit.
#[derive(Debug)]
struct StallGuard(Duration);

impl<In: Transport> Connector<In> for StallGuard {
    type Out = Stalling<In>;

    fn connect(&self, _: &ConnectionDetails, chained: Option<In>) -> Result<Option<Self::Out>, ureq::Error> {
        Ok(chained.map(|inner| Stalling { inner, limit: self.0 }))
    }
}

#[derive(Debug)]
struct Stalling<T> {
    inner: T,
    limit: Duration,
}

impl<T> Stalling<T> {
    fn clamp(&self, timeout: NextTimeout) -> NextTimeout {
        NextTimeout { after: timeout.after.min(time::Duration::Exact(self.limit)), ..timeout }
    }
}

impl<T: Transport> Transport for Stalling<T> {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        let timeout = self.clamp(timeout);
        self.inner.transmit_output(amount, timeout)
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        let timeout = self.clamp(timeout);
        self.inner.await_input(timeout)
    }

    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }

    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    use super::*;
    use crate::Fingerprint;
    use crate::testing::{FakeServer, Fault};

    const STALL: Duration = Duration::from_secs(30);

    fn provider() -> Arc<CryptoProvider> {
        Arc::new(rustls_rustcrypto::provider())
    }

    fn http(account: &Account) -> Http {
        Http::new(account, Some(FakeServer::PASSWORD), provider(), STALL).unwrap()
    }

    /// GETs `path` with optional extra headers, reads the whole body (64 MiB cap).
    fn fetch(http: &Http, path: &str, headers: &[(&str, &str)]) -> Result<(u16, Vec<u8>), Error> {
        let res = http.request("GET", &format!("{}{path}", http.base()), headers, None, Kind::Quick)?;
        let status = res.status().as_u16();
        Ok((status, read_capped(res.into_body(), 64 << 20)?))
    }

    fn status(account: &Account) -> Result<u16, Error> {
        fetch(&http(account), "/status.php", &[]).map(|(s, _)| s)
    }

    #[test]
    fn plain_http_get_status() {
        let server = FakeServer::start();
        let (status, body) = fetch(&http(&server.account()), "/status.php", &[]).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, br#"{"installed":true,"version":"35.0.0"}"#);
    }

    #[test]
    fn unknown_certificate_reports_fingerprint() {
        let server = FakeServer::start_tls(provider());
        match status(&server.account()) {
            Err(Error::UnknownCertificate { fingerprint }) => assert_eq!(fingerprint, server.fingerprint().unwrap()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn pinned_ca_true_self_signed_cert_connects() {
        let server = FakeServer::start_tls(provider());
        let account = Account { trust: server.fingerprint(), ..server.account() };
        assert_eq!(status(&account).unwrap(), 200);
    }

    #[test]
    fn changed_certificate_is_hard_error() {
        let server = FakeServer::start_tls(provider());
        let account = Account { trust: Some(Fingerprint([0; 32])), ..server.account() };
        assert!(matches!(status(&account), Err(Error::CertificateChanged)));
    }

    #[test]
    fn wrong_password_is_unauthorized() {
        let server = FakeServer::start();
        let dav = "/remote.php/dav/files/alice/";
        for password in [Some("nope"), None] {
            let http = Http::new(&server.account(), password, provider(), STALL).unwrap();
            assert!(matches!(fetch(&http, dav, &[]), Err(Error::Unauthorized)), "{password:?}");
        }
        // Auth is checked before routing: the right password reaches the (not yet served) route.
        assert!(matches!(fetch(&http(&server.account()), dav, &[]), Err(Error::NotFound)));
    }

    #[test]
    fn password_only_goes_to_the_account_server() {
        let (ours, other) = (FakeServer::start(), FakeServer::start());
        let http = http(&ours.account());
        assert!(matches!(fetch(&http, "/remote.php/dav/files/alice/", &[]), Err(Error::NotFound)));
        // `other` accepts the same password, so a leaked header would reach the route (404) instead of 401.
        let foreign = format!("{}/remote.php/dav/files/alice/", other.url());
        assert!(matches!(http.request("GET", &foreign, &[], None, Kind::Quick), Err(Error::Unauthorized)));
    }

    #[test]
    fn auth_url_must_be_under_base() {
        let base = "http://127.0.0.1:8080";
        for url in ["", "/", "/status.php", "?a=1", "#f"] {
            assert!(under_base(base, &format!("{base}{url}")), "{url:?}");
        }
        for url in
            ["http://127.0.0.1:80801/", "http://127.0.0.1:8080.evil.example/", "http://127.0.0.1:8080@evil.example/", "https://127.0.0.1:8080/"]
        {
            assert!(!under_base(base, url), "{url:?}");
        }
        assert!(under_base("https://h/nc", "https://h/nc/remote.php"));
        assert!(!under_base("https://h/nc", "https://h/nc2/remote.php"));
        assert!(!under_base("https://h/nc", "https://h/index.php"));
    }

    #[test]
    fn certificate_error_without_a_seen_leaf_is_not_unknown_certificate() {
        let invalid = || ureq::Error::Rustls(rustls::Error::InvalidCertificate(rustls::CertificateError::BadSignature));
        assert!(matches!(map_ureq(invalid(), &TlsState::new(None)), Error::Unreachable(d) if d.starts_with("TLS: ")));
        assert!(matches!(map_ureq(invalid(), &TlsState::new(Some(Fingerprint([0; 32])))), Error::CertificateChanged));
    }

    #[test]
    fn retries_429_then_succeeds() {
        let server = FakeServer::start();
        server.fail_next(Fault::Status(429));
        server.fail_next(Fault::Status(429));
        assert_eq!(status(&server.account()).unwrap(), 200);
        assert_eq!(server.requests(), 3);
        for _ in 0..4 {
            server.fail_next(Fault::Status(429));
        }
        assert!(matches!(status(&server.account()), Err(Error::RateLimited)));
    }

    #[test]
    fn maintenance_mode_is_rate_limited_without_retries() {
        let server = FakeServer::start();
        server.fail_next(Fault::Maintenance);
        assert!(matches!(status(&server.account()), Err(Error::RateLimited)));
        assert_eq!(server.requests(), 1);
    }

    #[test]
    fn dropped_connection_is_unreachable() {
        let server = FakeServer::start();
        for _ in 0..4 {
            server.fail_next(Fault::Drop);
        }
        assert!(matches!(status(&server.account()), Err(Error::Unreachable(_))));
    }

    #[test]
    fn stall_limit_trips() {
        let server = FakeServer::start();
        server.fail_next(Fault::Stall);
        let http = Http::new(&server.account(), Some(FakeServer::PASSWORD), provider(), Duration::from_millis(300)).unwrap();
        let start = Instant::now();
        assert!(matches!(fetch(&http, "/status.php", &[]), Err(Error::Unreachable(_))));
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn connection_is_reused() {
        let server = FakeServer::start();
        let http = http(&server.account());
        for _ in 0..10 {
            assert_eq!(fetch(&http, "/status.php", &[]).unwrap().0, 200);
        }
        assert_eq!(server.connections(), 1);
    }

    #[test]
    fn password_not_in_debug() {
        let server = FakeServer::start();
        let debug = format!("{:?}", http(&server.account()));
        assert!(!debug.contains(FakeServer::PASSWORD), "{debug}");
        assert!(!debug.contains(&STANDARD.encode(format!("alice:{}", FakeServer::PASSWORD))), "{debug}");
        assert!(debug.contains("<redacted>"), "{debug}");
    }

    #[test]
    fn faults_apply_to_any_response() {
        let server = FakeServer::start();
        let http = http(&server.account());
        let get = |fault: Option<Fault>, headers: &[(&str, &str)]| {
            if let Some(fault) = fault {
                server.fail_next(fault);
            }
            fetch(&http, "/status.php", headers)
        };
        let range = [("Range", "bytes=1-3")];
        assert_eq!(get(None, &range).unwrap(), (206, br#""in"#.to_vec()));
        assert_eq!(get(None, &[("Range", "bytes=100-")]).unwrap().0, 416);
        assert_eq!(get(Some(Fault::IgnoreRange), &range).unwrap().0, 200);
        assert_eq!(get(Some(Fault::Garbage), &[]).unwrap(), (200, b"<<<not xml".to_vec()));
        assert_eq!(get(Some(Fault::Status(503)), &[]).unwrap().0, 200);
        assert!(matches!(get(Some(Fault::Status(302)), &[]), Err(Error::Malformed(_))));
        assert!(matches!(get(Some(Fault::Truncate), &[]), Err(Error::Unreachable(_))));
        assert!(matches!(get(Some(Fault::Oversize), &[]), Err(Error::TooLarge)));
    }
}
