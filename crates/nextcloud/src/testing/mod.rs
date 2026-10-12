//! A scripted, in-process Nextcloud for integration tests (feature `testing`).
//!
//! Plain HTTP or TLS with a built-in self-signed certificate; HTTP/1.1 keep-alive, `Content-Length` bodies, one
//! thread per connection. [`FakeServer::fail_next`] queues [`Fault`]s; each request consumes one.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

use crate::{Account, Fingerprint};

/// Self-signed P-256 certificate for `127.0.0.1` and `localhost`, `CA:TRUE` like many self-made server certificates.
const CERT: &[u8] = include_bytes!("cert.der");
/// Its PKCS#8 private key.
const KEY: &[u8] = include_bytes!("key.der");
/// How often blocked connection threads check whether the server was dropped.
const POLL: Duration = Duration::from_millis(50);
/// A client that stops reading for this long loses its connection.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HEAD: usize = 64 * 1024;
const MAX_BODY: usize = 16 << 20;
/// Body size sent by [`Fault::Oversize`]: just over the 64 MiB cap of the largest single response.
const OVERSIZE: u64 = (64 << 20) + 1;

/// A failure injected into one request, whatever its route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Answer with this status and an empty body; 429 and 503 carry `Retry-After: 0`.
    Status(u16),
    /// Close the connection without answering.
    Drop,
    /// Send the headers and one byte of the body, then wait until the client hangs up.
    Stall,
    /// Declare the full `Content-Length`, send half the body, close.
    Truncate,
    /// Ignore the `Range` header.
    IgnoreRange,
    /// Send more than 64 MiB of zeros as the body, ignoring any `Range`.
    Oversize,
    /// Replace the body with `<<<not xml`, keeping the status.
    Garbage,
    /// Send this `Content-Range` header instead of the real one, keeping the status and body.
    ContentRange(&'static str),
    /// Answer like Nextcloud in maintenance mode: `503`, `X-Nextcloud-Maintenance-Mode: 1`, `Retry-After: 120`.
    Maintenance,
}

/// A fake Nextcloud on `127.0.0.1`; dropping it stops it.
///
/// Every route but `/status.php` and the login flow needs Basic auth with [`FakeServer::USER`] and
/// [`FakeServer::PASSWORD`]. If the server can't start (no free port, or a provider that can't use the
/// certificate), every request to it fails as unreachable.
#[derive(Debug)]
pub struct FakeServer {
    addr: SocketAddr,
    /// Set for TLS servers.
    fingerprint: Option<Fingerprint>,
    shared: Arc<Shared>,
}

#[derive(Debug, Default)]
struct Shared {
    faults: Mutex<VecDeque<Fault>>,
    connections: AtomicUsize,
    requests: AtomicUsize,
    stopped: AtomicBool,
}

impl FakeServer {
    /// The only user.
    pub const USER: &str = "alice";
    /// The user's app password.
    pub const PASSWORD: &str = "app-password-123";

    /// Starts a plain-HTTP server.
    pub fn start() -> FakeServer {
        FakeServer::spawn(None, None)
    }

    /// Starts a TLS server presenting a self-signed certificate (see [`FakeServer::fingerprint`]).
    pub fn start_tls(provider: Arc<CryptoProvider>) -> FakeServer {
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(KEY));
        let config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .and_then(|builder| builder.with_no_client_auth().with_single_cert(vec![CertificateDer::from(CERT)], key));
        FakeServer::spawn(Some(Fingerprint::of(CERT)), config.ok().map(Arc::new))
    }

    fn spawn(fingerprint: Option<Fingerprint>, tls: Option<Arc<ServerConfig>>) -> FakeServer {
        let shared = Arc::<Shared>::default();
        let mut addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        if let Ok(listener) = TcpListener::bind(addr) {
            addr = listener.local_addr().unwrap_or(addr);
            if fingerprint.is_none() || tls.is_some() {
                let shared = shared.clone();
                let _ = thread::Builder::new().name("fake-nextcloud".into()).spawn(move || accept(&listener, &shared, tls));
            }
        }
        FakeServer { addr, fingerprint, shared }
    }

    /// `http://127.0.0.1:<port>`, or `https://…` for a TLS server.
    pub fn url(&self) -> String {
        let scheme = if self.fingerprint.is_some() { "https" } else { "http" };
        format!("{scheme}://{}", self.addr)
    }

    /// SHA-256 of the TLS certificate; `None` for plain HTTP.
    pub fn fingerprint(&self) -> Option<Fingerprint> {
        self.fingerprint
    }

    /// The account of [`FakeServer::USER`] on this server, trusting no certificate.
    pub fn account(&self) -> Account {
        Account { server: self.url(), user: FakeServer::USER.into(), trust: None }
    }

    /// Queues `fault` for the next request without one.
    pub fn fail_next(&self, fault: Fault) {
        self.shared.faults.lock().unwrap_or_else(PoisonError::into_inner).push_back(fault);
    }

    /// Connections accepted so far.
    pub fn connections(&self) -> usize {
        self.shared.connections.load(SeqCst)
    }

    /// Requests received so far, including those a fault answered.
    pub fn requests(&self) -> usize {
        self.shared.requests.load(SeqCst)
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.shared.stopped.store(true, SeqCst);
        // Wakes the accept loop so it sees the flag; connection threads see it within `POLL`.
        let _ = TcpStream::connect_timeout(&self.addr, POLL);
    }
}

fn accept(listener: &TcpListener, shared: &Arc<Shared>, tls: Option<Arc<ServerConfig>>) {
    for stream in listener.incoming() {
        if shared.stopped.load(SeqCst) {
            return;
        }
        let Ok(stream) = stream else {
            thread::sleep(POLL); // e.g. out of file descriptors
            continue;
        };
        shared.connections.fetch_add(1, SeqCst);
        let (shared, tls) = (shared.clone(), tls.clone());
        let _ = thread::Builder::new().name("fake-nextcloud-conn".into()).spawn(move || serve(&shared, stream, tls));
    }
}

/// A plain or TLS connection.
trait Conn: Read + Write {}
impl<T: Read + Write> Conn for T {}

/// Serves one connection until the client hangs up, sends something unparsable, a fault closes it, or the
/// server is dropped.
fn serve(shared: &Shared, stream: TcpStream, tls: Option<Arc<ServerConfig>>) {
    let _ = stream.set_nodelay(true);
    if stream.set_read_timeout(Some(POLL)).is_err() || stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err() {
        return;
    }
    let mut conn: Box<dyn Conn> = match tls {
        None => Box::new(stream),
        Some(config) => match ServerConnection::new(config) {
            Ok(session) => Box::new(StreamOwned::new(session, stream)),
            Err(_) => return,
        },
    };
    let mut buf = Vec::new();
    while let Some(req) = read_request(shared, conn.as_mut(), &mut buf) {
        shared.requests.fetch_add(1, SeqCst);
        let fault = shared.faults.lock().unwrap_or_else(PoisonError::into_inner).pop_front();
        let res = respond(&req);
        if !matches!(send(shared, conn.as_mut(), &req, res, fault), Ok(true)) {
            return;
        }
    }
}

/// A parsed request. Its body is read and dropped: no route needs one yet.
struct Request {
    method: String,
    /// Path and query, still percent-encoded.
    target: String,
    /// Names are lowercase.
    headers: Vec<(String, String)>,
}

impl Request {
    fn path(&self) -> &str {
        self.target.split_once('?').map_or(&self.target, |(path, _)| path)
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

/// A response before faults are applied.
struct Response {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

impl Response {
    fn empty(status: u16) -> Response {
        Response { status, headers: Vec::new(), body: Vec::new() }
    }

    fn json(body: &str) -> Response {
        Response { status: 200, headers: vec![("Content-Type", "application/json".into())], body: body.into() }
    }
}

/// Basic auth on everything but `status.php` and the login flow, then the routes.
fn respond(req: &Request) -> Response {
    let path = req.path();
    let public = path == "/status.php" || path.starts_with("/index.php/login/v2");
    let auth = format!("Basic {}", STANDARD.encode(format!("{}:{}", FakeServer::USER, FakeServer::PASSWORD)));
    if !public && req.header("authorization") != Some(auth.as_str()) {
        return Response::empty(401);
    }
    match (req.method.as_str(), path) {
        ("GET", "/status.php") => Response::json(r#"{"installed":true,"version":"35.0.0"}"#),
        _ => Response::empty(404),
    }
}

/// Writes `res` as `fault` alters it; `Ok(false)` when the connection must close.
fn send(shared: &Shared, conn: &mut dyn Conn, req: &Request, mut res: Response, fault: Option<Fault>) -> io::Result<bool> {
    match fault {
        Some(Fault::Drop) => return Ok(false),
        Some(Fault::Status(status)) => {
            res = Response::empty(status);
            if matches!(status, 429 | 503) {
                res.headers.push(("Retry-After", "0".into()));
            }
        }
        Some(Fault::Garbage) => res.body = b"<<<not xml".to_vec(),
        Some(Fault::Maintenance) => {
            res = Response::empty(503);
            res.headers.extend([("X-Nextcloud-Maintenance-Mode", "1".into()), ("Retry-After", "120".into())]);
        }
        _ => {}
    }
    if !matches!(fault, Some(Fault::IgnoreRange | Fault::Oversize)) {
        res = apply_range(req, res);
    }
    if let Some(Fault::ContentRange(value)) = fault {
        res.headers.retain(|(name, _)| *name != "Content-Range");
        res.headers.push(("Content-Range", value.into()));
    }
    let length = if fault == Some(Fault::Oversize) { OVERSIZE } else { res.body.len() as u64 };
    let reason = ureq::http::StatusCode::from_u16(res.status).ok().and_then(|s| s.canonical_reason()).unwrap_or("");
    let mut head = format!("HTTP/1.1 {} {reason}\r\nContent-Length: {length}\r\n", res.status);
    for (name, value) in &res.headers {
        let _ = write!(head, "{name}: {value}\r\n");
    }
    head.push_str("\r\n");
    conn.write_all(head.as_bytes())?;
    let body = res.body.as_slice();
    match fault {
        Some(Fault::Stall) => {
            conn.write_all(body.get(..1).unwrap_or_default())?;
            conn.flush()?;
            let mut sink = Vec::new();
            while fill(shared, conn, &mut sink).is_some() {
                sink.clear();
            }
            return Ok(false);
        }
        Some(Fault::Truncate) => {
            conn.write_all(body.get(..body.len() / 2).unwrap_or_default())?;
            conn.flush()?;
            return Ok(false);
        }
        Some(Fault::Oversize) => {
            io::copy(&mut io::repeat(0).take(OVERSIZE), conn)?;
        }
        _ => conn.write_all(body)?,
    }
    conn.flush()?;
    Ok(!req.header("connection").is_some_and(|v| v.eq_ignore_ascii_case("close")))
}

/// Serves `Range: bytes=a-b`, `a-` or `-n` from a full `200` GET body: `206`, or `416` past the end.
/// Other range forms get the full body.
fn apply_range(req: &Request, mut res: Response) -> Response {
    let spec = req.header("range").and_then(|r| r.strip_prefix("bytes="));
    let (Some(spec), "GET", 200) = (spec, req.method.as_str(), res.status) else { return res };
    let len = res.body.len() as u64;
    let last = len.saturating_sub(1);
    let bounds = match spec.split_once('-') {
        Some(("", n)) => n.parse::<u64>().ok().map(|n| (len.saturating_sub(n), last)),
        Some((a, "")) => a.parse().ok().map(|a| (a, last)),
        Some((a, b)) => a.parse().ok().zip(b.parse::<u64>().ok()).map(|(a, b)| (a, b.min(last))),
        None => None,
    };
    let Some((start, end)) = bounds else { return res };
    let part = usize::try_from(start).ok().zip(usize::try_from(end).ok()).filter(|(s, e)| s <= e).and_then(|(s, e)| res.body.get(s..=e));
    match part {
        Some(part) => {
            let body = part.to_vec();
            res.headers.push(("Content-Range", format!("bytes {start}-{end}/{len}")));
            Response { status: 206, headers: res.headers, body }
        }
        None => Response { status: 416, headers: vec![("Content-Range", format!("bytes */{len}"))], body: Vec::new() },
    }
}

/// Reads the next request; `None` when the client hung up or sent something this server doesn't parse.
fn read_request(shared: &Shared, conn: &mut dyn Conn, buf: &mut Vec<u8>) -> Option<Request> {
    let end = loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break end;
        }
        if buf.len() > MAX_HEAD {
            return None;
        }
        fill(shared, conn, buf)?;
    };
    let head = std::str::from_utf8(buf.get(..end)?).ok()?;
    let mut lines = head.split("\r\n");
    let mut start = lines.next()?.split(' ');
    let (method, target) = (start.next()?.to_string(), start.next()?.to_string());
    let headers = lines.map(|l| l.split_once(':').map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))).collect::<Option<_>>()?;
    let req = Request { method, target, headers };
    if req.header("transfer-encoding").is_some() {
        return None;
    }
    let length = req.header("content-length").map_or(Some(0), |v| v.parse::<usize>().ok()).filter(|l| *l <= MAX_BODY)?;
    buf.drain(..end + 4);
    while buf.len() < length {
        fill(shared, conn, buf)?;
    }
    buf.drain(..length);
    Some(req)
}

/// Appends what the client sends next; `None` when it hung up or the server was dropped.
fn fill(shared: &Shared, conn: &mut dyn Conn, buf: &mut Vec<u8>) -> Option<()> {
    let mut chunk = [0u8; 16 * 1024];
    while !shared.stopped.load(SeqCst) {
        match conn.read(&mut chunk) {
            Ok(0) => return None,
            Ok(n) => {
                buf.extend_from_slice(chunk.get(..n)?);
                return Some(());
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {}
            Err(_) => return None,
        }
    }
    None
}
