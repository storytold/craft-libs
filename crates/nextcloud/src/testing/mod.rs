//! A scripted, in-process Nextcloud for integration tests (feature `testing`).
//!
//! Plain HTTP or TLS with a built-in self-signed certificate; HTTP/1.1 keep-alive, `Content-Length` bodies, one
//! thread per connection. [`FakeServer::fail_next`] queues [`Fault`]s; each request consumes one.

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use percent_encoding::percent_decode_str;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

use crate::path::encode_segment;
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
    downloads: AtomicUsize,
    user_agent: Mutex<Option<String>>,
    stopped: AtomicBool,
    files: Mutex<Files>,
    /// Source of file ids and etags.
    counter: AtomicU64,
    /// `http(s)://127.0.0.1:<port>`, for URLs in login responses.
    base: Mutex<String>,
    approved: AtomicBool,
    poll_endpoint: Mutex<Option<String>>,
    /// Put before every listing href, like a path-rewriting proxy.
    href_prefix: Mutex<String>,
}

/// The user's files: absolute path (`/a/b`, root is `""`) to node, plus listing entries added raw.
#[derive(Debug, Default)]
struct Files {
    nodes: BTreeMap<String, Node>,
    /// Preview images by absolute path.
    previews: BTreeMap<String, Vec<u8>>,
    raw: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
struct Node {
    dir: bool,
    size: usize,
    data: Vec<u8>,
    id: u64,
    etag: u64,
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
        let server = FakeServer { addr, fingerprint, shared };
        *server.shared.base.lock().unwrap_or_else(PoisonError::into_inner) = server.url();
        server
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

    /// Makes the next login poll succeed with [`FakeServer::USER`] and [`FakeServer::PASSWORD`].
    pub fn approve_login(&self) {
        self.shared.approved.store(true, SeqCst);
    }

    /// Makes the login flow start answer with this poll endpoint instead of the server's own.
    pub fn set_poll_endpoint(&self, url: &str) {
        *self.shared.poll_endpoint.lock().unwrap_or_else(PoisonError::into_inner) = Some(url.to_string());
    }

    /// Puts `prefix` (e.g. `/nextcloud`) before every href in listings, like a reverse proxy that rewrites paths.
    pub fn set_href_prefix(&self, prefix: &str) {
        *self.shared.href_prefix.lock().unwrap_or_else(PoisonError::into_inner) = prefix.to_string();
    }

    /// Queues `fault` for the next request without one.
    pub fn fail_next(&self, fault: Fault) {
        self.shared.faults.lock().unwrap_or_else(PoisonError::into_inner).push_back(fault);
    }

    /// Adds a directory and its parents. `path` is `/`-separated under the user's files root, e.g. `/Photos`.
    pub fn add_dir(&self, path: &str) {
        let mut files = self.shared.files.lock().unwrap_or_else(PoisonError::into_inner);
        self.shared.insert(&mut files, &norm(path), Vec::new(), true);
    }

    /// Adds or replaces a file (creating parents) and returns its file id. Its etag changes on every write.
    pub fn add_file(&self, path: &str, bytes: Vec<u8>) -> u64 {
        let mut files = self.shared.files.lock().unwrap_or_else(PoisonError::into_inner);
        self.shared.insert(&mut files, &norm(path), bytes, false)
    }

    /// Sets the image the preview endpoint returns for `path` (same form as in [`FakeServer::add_file`]).
    pub fn set_preview(&self, path: &str, bytes: Vec<u8>) {
        self.shared.files.lock().unwrap_or_else(PoisonError::into_inner).previews.insert(norm(path), bytes);
    }

    /// Removes `path` and everything below it.
    pub fn remove(&self, path: &str) {
        let path = norm(path);
        let below = format!("{path}/");
        let mut files = self.shared.files.lock().unwrap_or_else(PoisonError::into_inner);
        files.nodes.retain(|p, _| *p != path && !p.starts_with(&below));
    }

    /// Adds a listing entry to `dir` whose href ends in `encoded_name` as given (no encoding applied), however
    /// hostile. It is not a real node.
    pub fn add_raw_entry(&self, dir: &str, encoded_name: &str) {
        self.shared.files.lock().unwrap_or_else(PoisonError::into_inner).raw.push((norm(dir), encoded_name.to_string()));
    }

    /// Connections accepted so far.
    pub fn connections(&self) -> usize {
        self.shared.connections.load(SeqCst)
    }

    /// Requests received so far, including those a fault answered.
    pub fn requests(&self) -> usize {
        self.shared.requests.load(SeqCst)
    }

    /// Whole-file downloads so far: `GET`s below the user's WebDAV files root without a `Range` header
    /// ([`Client::download_to`](crate::Client::download_to) and [`OriginalsCache::fetch`](crate::OriginalsCache::fetch)
    /// misses). Listings and `stat` (`PROPFIND` on the same paths), previews and `read_range` don't count.
    pub fn downloads(&self) -> usize {
        self.shared.downloads.load(SeqCst)
    }

    /// The `User-Agent` of the last request, if it had one.
    pub fn last_user_agent(&self) -> Option<String> {
        self.shared.user_agent.lock().unwrap_or_else(PoisonError::into_inner).clone()
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
        if req.method == "GET" && req.header("range").is_none() && req.path().strip_prefix(DAV_ROOT).is_some_and(|rest| rest.starts_with('/')) {
            shared.downloads.fetch_add(1, SeqCst);
        }
        *shared.user_agent.lock().unwrap_or_else(PoisonError::into_inner) = req.header("user-agent").map(str::to_string);
        let fault = shared.faults.lock().unwrap_or_else(PoisonError::into_inner).pop_front();
        let res = respond(shared, &req);
        if !matches!(send(shared, conn.as_mut(), &req, res, fault), Ok(true)) {
            return;
        }
    }
}

/// A parsed request.
struct Request {
    method: String,
    /// Path and query, still percent-encoded.
    target: String,
    /// Names are lowercase.
    headers: Vec<(String, String)>,
    body: Vec<u8>,
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
fn respond(shared: &Shared, req: &Request) -> Response {
    let path = req.path();
    let public = path == "/status.php" || path.starts_with("/index.php/login/v2");
    let auth = format!("Basic {}", STANDARD.encode(format!("{}:{}", FakeServer::USER, FakeServer::PASSWORD)));
    if !public && req.header("authorization") != Some(auth.as_str()) {
        return Response::empty(401);
    }
    match (req.method.as_str(), path) {
        ("POST", "/index.php/login/v2") => login_start(shared),
        ("POST", "/index.php/login/v2/poll") => login_poll(shared, req),
        ("GET", "/status.php") => Response::json(r#"{"installed":true,"version":"35.0.0"}"#),
        ("GET", "/index.php/core/preview.png") => preview(shared, req),
        ("GET", _) => path.strip_prefix(DAV_ROOT).map_or_else(|| Response::empty(404), |rest| get_file(shared, rest)),
        ("PROPFIND", _) => path.strip_prefix(DAV_ROOT).map_or_else(|| Response::empty(404), |rest| propfind(shared, req, rest)),
        _ => Response::empty(404),
    }
}

const LOGIN_TOKEN: &str = "login-token-123";

fn login_start(shared: &Shared) -> Response {
    let base = shared.base.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let endpoint =
        shared.poll_endpoint.lock().unwrap_or_else(PoisonError::into_inner).clone().unwrap_or_else(|| format!("{base}/index.php/login/v2/poll"));
    Response::json(&format!(r#"{{"poll":{{"token":"{LOGIN_TOKEN}","endpoint":"{endpoint}"}},"login":"{base}/login/v2/flow/{LOGIN_TOKEN}"}}"#))
}

/// 404 until [`FakeServer::approve_login`], and for any other token.
fn login_poll(shared: &Shared, req: &Request) -> Response {
    if req.body != format!("token={LOGIN_TOKEN}").as_bytes() || !shared.approved.load(SeqCst) {
        return Response::empty(404);
    }
    let base = shared.base.lock().unwrap_or_else(PoisonError::into_inner).clone();
    Response::json(&format!(r#"{{"server":"{base}","loginName":"{}","appPassword":"{}"}}"#, FakeServer::USER, FakeServer::PASSWORD))
}

const DAV_ROOT: &str = "/remote.php/dav/files/alice";
const MTIME: &str = "Sun, 06 Nov 1994 08:49:37 GMT";

/// `/a/b` for `a/b/`, `//a`, etc.; the root is `""`.
fn norm(path: &str) -> String {
    path.split('/').filter(|s| !s.is_empty()).fold(String::new(), |acc, s| acc + "/" + s)
}

impl Shared {
    /// Inserts `path` with its parents; a new etag and, for a new path, a new id.
    fn insert(&self, files: &mut Files, path: &str, data: Vec<u8>, dir: bool) -> u64 {
        for (i, _) in path.match_indices('/').skip(1) {
            let parent = path.get(..i).unwrap_or_default();
            if !files.nodes.contains_key(parent) {
                let id = self.counter.fetch_add(1, SeqCst);
                let etag = self.counter.fetch_add(1, SeqCst);
                files.nodes.insert(parent.to_string(), Node { dir: true, size: 0, data: Vec::new(), id, etag });
            }
        }
        let id = files.nodes.get(path).map_or_else(|| self.counter.fetch_add(1, SeqCst), |n| n.id);
        let etag = self.counter.fetch_add(1, SeqCst);
        files.nodes.insert(path.to_string(), Node { dir, size: data.len(), data, id, etag });
        id
    }
}

/// `GET` of a file's bytes (`Range` is applied later, to every `200`).
fn get_file(shared: &Shared, rest: &str) -> Response {
    let Ok(path) = percent_decode_str(rest).decode_utf8().map(|p| norm(&p)) else { return Response::empty(400) };
    let files = shared.files.lock().unwrap_or_else(PoisonError::into_inner);
    match files.nodes.get(&path) {
        Some(node) if !node.dir => Response { status: 200, headers: Vec::new(), body: node.data.clone() },
        _ => Response::empty(404),
    }
}

/// The preview of `file=<path>`; 404 unless [`FakeServer::set_preview`] was called for it.
fn preview(shared: &Shared, req: &Request) -> Response {
    let query = req.target.split_once('?').map_or("", |(_, q)| q);
    let file = query.split('&').find_map(|kv| kv.strip_prefix("file="));
    let Some(Ok(file)) = file.map(|f| percent_decode_str(f).decode_utf8()) else { return Response::empty(404) };
    let files = shared.files.lock().unwrap_or_else(PoisonError::into_inner);
    files.previews.get(&norm(&file)).map_or_else(
        || Response::empty(404),
        |bytes| Response { status: 200, headers: vec![("Content-Type", "image/png".into())], body: bytes.clone() },
    )
}

/// `PROPFIND` at `Depth` 0 or 1; the full property set whatever the body asks for.
fn propfind(shared: &Shared, req: &Request, rest: &str) -> Response {
    let depth = req.header("depth").unwrap_or("infinity");
    if !matches!(depth, "0" | "1") {
        return Response::empty(403);
    }
    let Ok(path) = percent_decode_str(rest).decode_utf8().map(|p| norm(&p)) else { return Response::empty(400) };
    let dav_root = format!("{}{DAV_ROOT}", shared.href_prefix.lock().unwrap_or_else(PoisonError::into_inner));
    let files = shared.files.lock().unwrap_or_else(PoisonError::into_inner);
    let root = Node { dir: true, size: 0, data: Vec::new(), id: 0, etag: 0 };
    let Some(node) = (if path.is_empty() { Some(&root) } else { files.nodes.get(&path) }) else { return Response::empty(404) };
    let mut body =
        String::from(r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns" xmlns:nc="http://nextcloud.org/ns">"#);
    entry_xml(&mut body, &dav_root, &path, node);
    if depth == "1" && node.dir {
        let prefix = format!("{path}/");
        for (p, n) in files.nodes.range(prefix.clone()..).take_while(|(p, _)| p.starts_with(&prefix)) {
            if !p.get(prefix.len()..).is_some_and(|name| name.contains('/')) {
                entry_xml(&mut body, &dav_root, p, n);
            }
        }
        for (_, name) in files.raw.iter().filter(|(dir, _)| *dir == path) {
            let _ = write!(
                body,
                "<d:response><d:href>{dav_root}{path}/{name}</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>0</d:getcontentlength><d:getetag>\"raw\"</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"
            );
        }
    }
    body.push_str("</d:multistatus>");
    Response { status: 207, headers: vec![("Content-Type", "application/xml; charset=utf-8".into())], body: body.into() }
}

fn entry_xml(body: &mut String, dav_root: &str, path: &str, n: &Node) {
    let href: String = path.split('/').filter(|s| !s.is_empty()).map(|s| format!("/{}", encode_segment(s))).collect();
    let slash = if n.dir { "/" } else { "" };
    let _ = write!(body, "<d:response><d:href>{dav_root}{href}{}</d:href><d:propstat><d:prop>", if href.is_empty() { "/" } else { slash });
    let _ =
        write!(body, "<d:getlastmodified>{MTIME}</d:getlastmodified><d:getetag>&quot;{:x}&quot;</d:getetag><oc:fileid>{}</oc:fileid>", n.etag, n.id);
    if n.dir {
        let _ = write!(
            body,
            "<d:resourcetype><d:collection/></d:resourcetype><oc:size>{}</oc:size></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>",
            n.size
        );
        body.push_str(
            "<d:propstat><d:prop><d:getcontentlength/><d:getcontenttype/></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat>",
        );
    } else {
        let _ = write!(
            body,
            "<d:resourcetype/><d:getcontentlength>{}</d:getcontentlength><d:getcontenttype>application/octet-stream</d:getcontenttype><nc:has-preview>true</nc:has-preview></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>",
            n.size
        );
    }
    body.push_str("</d:response>");
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
    let mut req = Request { method, target, headers, body: Vec::new() };
    if req.header("transfer-encoding").is_some() {
        return None;
    }
    let length = req.header("content-length").map_or(Some(0), |v| v.parse::<usize>().ok()).filter(|l| *l <= MAX_BODY)?;
    buf.drain(..end + 4);
    while buf.len() < length {
        fill(shared, conn, buf)?;
    }
    req.body = buf.drain(..length).collect();
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
