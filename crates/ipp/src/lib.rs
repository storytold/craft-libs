//! `photocraft-ipp`: a small IPP client for CUPS, in pure Rust with no dependencies.
//!
//! Talks to the local print server the way `lp` does, but in-process: over the CUPS domain
//! socket (`/run/cups/cups.sock`, or `CUPS_SERVER` when it names a path) or TCP (`localhost:631`,
//! or `CUPS_SERVER` as `host[:port]`). Three operations, from RFC 8010 / 8011 and CUPS's own
//! extensions: list the printers, read the default, and submit a PDF as a job.
//!
//! Never panics: every read is bounded and every malformed reply is an error. Works on any Unix
//! and on Windows for TCP (CUPS rarely runs there; the Windows print path is separate).

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

/// Where the print server listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// A Unix domain socket (CUPS's default, also what a Flatpak sees with `--socket=cups`).
    Unix(PathBuf),
    /// A TCP host and port.
    Tcp(String, u16),
}

/// Why a request failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No server could be reached.
    Unreachable(String),
    /// The HTTP or IPP reply could not be read.
    Protocol(String),
    /// The server answered with an IPP error status (the code and its message).
    Status(u16, String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unreachable(m) => write!(f, "no print server: {m}"),
            Error::Protocol(m) => write!(f, "print server reply: {m}"),
            Error::Status(code, m) => write!(f, "print server refused ({code:#06x}): {m}"),
        }
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

/// A printer the server knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Printer {
    pub name: String,
    /// `printer-info`, the human description.
    pub info: String,
    /// `printer-state`: 3 idle, 4 printing, 5 stopped.
    pub state: u32,
    pub accepting_jobs: bool,
    pub is_default: bool,
    /// A CUPS class (a pool of printers) rather than one printer.
    pub is_class: bool,
}

/// Timeouts: connecting, and each request (a Print-Job carries the whole PDF).
const CONNECT: Duration = Duration::from_secs(3);
const REQUEST: Duration = Duration::from_secs(60);
/// The whole reply, however it trickles in: the timeouts above only cover each read.
const REPLY_DEADLINE: Duration = Duration::from_secs(120);
/// Largest reply read.
const MAX_REPLY: usize = 16 << 20;
/// Largest number of attributes taken from a reply.
const MAX_ATTRIBUTES: usize = 100_000;

pub const OP_PRINT_JOB: u16 = 0x0002;
pub const OP_CUPS_GET_DEFAULT: u16 = 0x4001;
pub const OP_CUPS_GET_PRINTERS: u16 = 0x4002;
pub const OP_CUPS_GET_CLASSES: u16 = 0x4005;

pub const TAG_OPERATION_GROUP: u8 = 0x01;
pub const TAG_JOB_GROUP: u8 = 0x02;
pub const TAG_END: u8 = 0x03;
pub const TAG_PRINTER_GROUP: u8 = 0x04;
pub const TAG_INTEGER: u8 = 0x21;
pub const TAG_BOOLEAN: u8 = 0x22;
pub const TAG_ENUM: u8 = 0x23;
pub const TAG_TEXT: u8 = 0x41;
pub const TAG_NAME: u8 = 0x42;
pub const TAG_KEYWORD: u8 = 0x44;
pub const TAG_URI: u8 = 0x45;
pub const TAG_CHARSET: u8 = 0x47;
pub const TAG_LANGUAGE: u8 = 0x48;
pub const TAG_MIME: u8 = 0x49;

impl Endpoint {
    /// The server this machine uses: `CUPS_SERVER` (a socket path, or `host[:port]`), else the
    /// CUPS socket, else `localhost:631`. `None` when nothing answers.
    pub fn detect() -> Option<Endpoint> {
        if let Some(server) = std::env::var_os("CUPS_SERVER").filter(|v| !v.is_empty()) {
            let ep = Endpoint::parse(&server.to_string_lossy());
            return ep.reachable().then_some(ep);
        }
        [
            Endpoint::Unix(PathBuf::from("/run/cups/cups.sock")),
            Endpoint::Unix(PathBuf::from("/var/run/cups/cups.sock")),
            // macOS
            Endpoint::Unix(PathBuf::from("/private/var/run/cupsd")),
            Endpoint::Tcp("localhost".into(), 631),
        ]
        .into_iter()
        .find(Endpoint::reachable)
    }

    /// `server` as a socket path (starts with `/`) or `host[:port]` (port 631 by default). An IPv6
    /// server is `[address]` or `[address]:port`; written bare (more than one `:`) it keeps the
    /// default port, because its last `:` cannot be told apart from a port separator. Anything else
    /// after the bracket (`[::1]:abc`) stays the host, so a mistyped endpoint cannot resolve rather
    /// than quietly becoming the default one.
    pub fn parse(server: &str) -> Endpoint {
        if server.starts_with('/') {
            return Endpoint::Unix(PathBuf::from(server));
        }
        // `[IPv6]:port` - the brackets tell the address from the port, and are not part of the host.
        if let Some(rest) = server.strip_prefix('[')
            && let Some((host, tail)) = rest.split_once(']')
        {
            // Nothing after the bracket, or a real `:port`, is understood. Anything else (`[::1]:abc`,
            // `[::1]:70000`, `[::1]junk`) keeps the whole string as the host, so the mistyped endpoint
            // stays unresolvable instead of quietly becoming the default one.
            let port = if tail.is_empty() {
                631
            } else if let Some(port) = tail.strip_prefix(':').and_then(|p| p.parse().ok()) {
                port
            } else {
                return Endpoint::Tcp(server.to_string(), 631);
            };
            return Endpoint::Tcp(host.to_string(), port);
        }
        // A bare IPv6 literal has more than one ':', so splitting on the last one would cut the
        // address in half: it is the host, and the port stays the default. A zone id (`fe80::1%eth0`)
        // carries no ':', so there a port can still follow the address.
        let host_port = if server.matches(':').count() > 1 && !server.contains('%') { None } else { server.rsplit_once(':') };
        let (host, port) = host_port.and_then(|(h, p)| p.parse().ok().map(|p| (h.to_string(), p))).unwrap_or((server.to_string(), 631));
        Endpoint::Tcp(host, port)
    }

    /// Whether a connection opens.
    pub fn reachable(&self) -> bool {
        self.connect().is_ok()
    }

    fn connect(&self) -> Result<Stream> {
        match self {
            #[cfg(unix)]
            Endpoint::Unix(path) => {
                if !path.exists() {
                    return Err(Error::Unreachable(format!("{} does not exist", path.display())));
                }
                let s = std::os::unix::net::UnixStream::connect(path).map_err(|e| Error::Unreachable(format!("{}: {e}", path.display())))?;
                let _ = s.set_read_timeout(Some(REQUEST));
                let _ = s.set_write_timeout(Some(REQUEST));
                Ok(Stream::Unix(s))
            }
            #[cfg(not(unix))]
            Endpoint::Unix(path) => Err(Error::Unreachable(format!("{}: Unix sockets are not available here", path.display()))),
            Endpoint::Tcp(host, port) => {
                let addr = (host.as_str(), *port).to_socket_addrs().map_err(|e| Error::Unreachable(format!("{host}:{port}: {e}")))?.next();
                let addr = addr.ok_or_else(|| Error::Unreachable(format!("{host}:{port}: no address")))?;
                open_tcp(host, *port, addr, |addr| TcpStream::connect_timeout(&addr, CONNECT))
            }
        }
    }

    /// The endpoint as an HTTP authority: an IPv6 host is bracketed, or its last colon reads as the
    /// port separator and the request carries a host CUPS has never heard of. A scoped address keeps
    /// its zone id and writes it escaped as "%25": "%" is not valid inside an authority (RFC 3986
    /// section 3.2.2 reserves it for escaping), and the escaped form is what CUPS, curl and the
    /// ipp:// URIs CUPS writes use for a zone. RFC 9844 (Aug 2025) obsoleted RFC 6874, which had made
    /// that form URI syntax, so this is interop rather than a standard; dropping the zone instead
    /// would name a different host on the wire than the socket was opened to. The socket still gets
    /// the raw address%zone, which is the form a resolver understands.
    fn host(&self) -> String {
        match self {
            Endpoint::Unix(_) => "localhost".into(),
            Endpoint::Tcp(host, port) if host.contains(':') => format!("[{}]:{port}", host.replace('%', "%25")),
            Endpoint::Tcp(host, port) => format!("{host}:{port}"),
        }
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Endpoint::Unix(p) => write!(f, "{}", p.display()),
            Endpoint::Tcp(h, p) if h.contains(':') => write!(f, "[{h}]:{p}"),
            Endpoint::Tcp(h, p) => write!(f, "{h}:{p}"),
        }
    }
}

/// Open one address and put the request timeouts on it. The connect call is a parameter so the
/// refusal path can be tested at all: no port is guaranteed to stay free between picking one and
/// using it, and what a refused connect reported is what the caller is supposed to hear.
fn open_tcp(host: &str, port: u16, addr: SocketAddr, open: impl FnOnce(SocketAddr) -> std::io::Result<TcpStream>) -> Result<Stream> {
    let s = open(addr).map_err(|e| Error::Unreachable(format!("{host}:{port}: {e}")))?;
    let _ = s.set_read_timeout(Some(REQUEST));
    let _ = s.set_write_timeout(Some(REQUEST));
    Ok(Stream::Tcp(s))
}

enum Stream {
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
    Tcp(TcpStream),
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Stream::Unix(s) => s.read(buf),
            Stream::Tcp(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(unix)]
            Stream::Unix(s) => s.write(buf),
            Stream::Tcp(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            #[cfg(unix)]
            Stream::Unix(s) => s.flush(),
            Stream::Tcp(s) => s.flush(),
        }
    }
}

/// Builds an IPP request body.
pub struct Request {
    buf: Vec<u8>,
}

impl Request {
    pub fn new(op: u16, id: u32) -> Self {
        let mut buf = vec![2, 0];
        buf.extend_from_slice(&op.to_be_bytes());
        buf.extend_from_slice(&id.to_be_bytes());
        buf.push(TAG_OPERATION_GROUP);
        let mut r = Request { buf };
        r.attr(TAG_CHARSET, "attributes-charset", b"utf-8");
        r.attr(TAG_LANGUAGE, "attributes-natural-language", b"en");
        r
    }

    /// Appends an attribute to the current group.
    pub fn attr(&mut self, tag: u8, name: &str, value: &[u8]) {
        // IPP lengths are 16-bit; longer names and values are cut, both the length and the bytes.
        let name = name.as_bytes().get(..name.len().min(0xffff)).unwrap_or_default();
        let value = value.get(..value.len().min(0xffff)).unwrap_or_default();
        self.buf.push(tag);
        self.buf.extend_from_slice(&(name.len() as u16).to_be_bytes());
        self.buf.extend_from_slice(name);
        self.buf.extend_from_slice(&(value.len() as u16).to_be_bytes());
        self.buf.extend_from_slice(value);
    }

    /// An attribute with several values (the extra ones carry an empty name).
    pub fn attr_many(&mut self, tag: u8, name: &str, values: &[&str]) {
        for (i, v) in values.iter().enumerate() {
            self.attr(tag, if i == 0 { name } else { "" }, v.as_bytes());
        }
    }

    pub fn integer(&mut self, name: &str, v: i32) {
        self.attr(TAG_INTEGER, name, &v.to_be_bytes());
    }

    /// Starts a new attribute group.
    pub fn group(&mut self, tag: u8) {
        self.buf.push(tag);
    }

    /// The message with its end tag.
    pub fn end(mut self) -> Vec<u8> {
        self.buf.push(TAG_END);
        self.buf
    }
}

/// One attribute of a reply: the group it sits in, its tag, name and values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub group: u8,
    pub tag: u8,
    pub name: String,
    pub values: Vec<Vec<u8>>,
}

impl Attribute {
    /// The first value as text.
    pub fn text(&self) -> String {
        self.values.first().map(|v| String::from_utf8_lossy(v).into_owned()).unwrap_or_default()
    }
    /// The first value as a big-endian integer.
    pub fn int(&self) -> Option<i32> {
        self.values.first().and_then(|v| <[u8; 4]>::try_from(v.as_slice()).ok()).map(i32::from_be_bytes)
    }
}

/// A decoded reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub request_id: u32,
    pub attributes: Vec<Attribute>,
}

/// The attribute range for one group delimiter in a parsed message. Ranges are half-open and
/// include empty groups, so repeated group tags remain distinct.
#[derive(Debug, Clone, Copy)]
struct GroupRange {
    tag: u8,
    start: usize,
    end: usize,
}

/// Parsed message details kept private so the public flat `Reply` API stays source-compatible.
struct ParsedMessage {
    reply: Reply,
    end: usize,
    groups: Vec<GroupRange>,
}

impl Reply {
    /// Parses an IPP message (RFC 8010 §3): header, then groups of attributes until the end tag.
    pub fn parse(b: &[u8]) -> Result<Reply> {
        parse_message(b).map(|parsed| parsed.reply)
    }

    /// Parses a message and returns where its attributes end (document data may follow).
    fn parse_at(b: &[u8]) -> Result<(Reply, usize)> {
        parse_message(b).map(|parsed| (parsed.reply, parsed.end))
    }

    /// The server's own message for an error status, or a name for the code.
    fn status_message(&self) -> String {
        self.attributes.iter().find(|a| a.name == "status-message").map(Attribute::text).filter(|m| !m.is_empty()).unwrap_or_else(|| {
            match self.status {
                0x0400 => "bad request",
                0x0401 => "forbidden",
                0x0402 => "not authenticated",
                0x0403 => "not authorized",
                0x0406 => "not found",
                0x0409 => "unsupported document format",
                0x0500 => "internal server error",
                0x0506 => "the printer is not accepting jobs",
                _ => "error",
            }
            .to_string()
        })
    }
}

/// Parses one IPP message while preserving each group instance for private consumers that need
/// object boundaries. The public reply remains a flat list of attributes.
fn parse_message(b: &[u8]) -> Result<ParsedMessage> {
    let bad = |m: &str| Error::Protocol(m.to_string());
    if b.len() < 9 {
        return Err(bad("reply shorter than an IPP header"));
    }
    let status = u16::from_be_bytes([b[2], b[3]]);
    let request_id = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
    let mut pos = 8;
    let mut group = 0u8;
    let mut active_group = None;
    let mut last_attribute_group = None;
    let mut attributes: Vec<Attribute> = Vec::new();
    let mut groups: Vec<GroupRange> = Vec::new();
    loop {
        let tag = *b.get(pos).ok_or_else(|| bad("reply ends before the end-of-attributes tag"))?;
        pos += 1;
        if tag == TAG_END {
            if let Some(last) = groups.last_mut() {
                last.end = attributes.len();
            }
            return Ok(ParsedMessage { reply: Reply { status, request_id, attributes }, end: pos, groups });
        }
        if tag < 0x10 {
            if let Some(last) = groups.last_mut() {
                last.end = attributes.len();
            }
            if groups.len() >= MAX_ATTRIBUTES {
                return Err(bad("too many attribute groups"));
            }
            group = tag;
            let group_index = groups.len();
            groups.push(GroupRange { tag, start: attributes.len(), end: attributes.len() });
            active_group = Some(group_index);
            continue;
        }
        let name_len =
            usize::from(u16::from_be_bytes([*b.get(pos).ok_or_else(|| bad("cut attribute"))?, *b.get(pos + 1).ok_or_else(|| bad("cut attribute"))?]));
        pos += 2;
        let name = b.get(pos..pos + name_len).ok_or_else(|| bad("cut attribute name"))?;
        pos += name_len;
        let value_len =
            usize::from(u16::from_be_bytes([*b.get(pos).ok_or_else(|| bad("cut attribute"))?, *b.get(pos + 1).ok_or_else(|| bad("cut attribute"))?]));
        pos += 2;
        let value = b.get(pos..pos + value_len).ok_or_else(|| bad("cut attribute value"))?.to_vec();
        pos += value_len;
        if name_len == 0 {
            // Additional values can continue an attribute only within this exact group instance.
            match attributes.last_mut() {
                Some(last) if last_attribute_group == active_group => last.values.push(value),
                _ => return Err(bad("an additional value with no attribute before it")),
            }
        } else {
            attributes.push(Attribute { group, tag, name: String::from_utf8_lossy(name).into_owned(), values: vec![value] });
            last_attribute_group = active_group;
        }
        if attributes.len() > MAX_ATTRIBUTES {
            return Err(bad("too many attributes"));
        }
    }
}

/// POSTs `body` (IPP, with any document data after it) to `path` and returns the parsed reply with
/// its private group boundaries.
fn post(endpoint: &Endpoint, path: &str, body: &[u8]) -> Result<ParsedMessage> {
    let mut stream = endpoint.connect()?;
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        endpoint.host(),
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body))
        .and_then(|()| stream.flush())
        .map_err(|e| Error::Protocol(format!("sending: {e}")))?;
    let raw = read_response(&mut stream)?;
    let (status, headers, body) = http_response(&raw)?;
    if status != 200 {
        return Err(Error::Protocol(match status {
            401 => "the print server asks for authentication (HTTP 401)".to_string(),
            403 => "the print server forbids this request (HTTP 403)".to_string(),
            s => format!("HTTP {s}"),
        }));
    }
    let body = if headers.iter().any(|(k, v)| k == "transfer-encoding" && v.contains("chunked")) { dechunk(body)? } else { body.to_vec() };
    let reply = parse_message(&body)?;
    if reply.reply.status >= 0x0400 {
        return Err(Error::Status(reply.reply.status, reply.reply.status_message()));
    }
    Ok(reply)
}

/// Lowercase HTTP headers.
type Headers = Vec<(String, String)>;

/// Reads one HTTP response: the headers, then as much body as `Content-Length` says, or up to
/// the last chunk, or to the end of the stream when the server says neither. A server that keeps
/// the connection open would otherwise hold the reply until the read times out.
fn read_response(stream: &mut Stream) -> Result<Vec<u8>> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    let mut body_at = None;
    let mut expected: Option<usize> = None;
    let mut chunked = false;
    // Where the chunk walk resumes, so each read costs its own bytes, not the whole body again.
    let mut chunk_at = 0usize;
    // Where the header-end search resumes (three bytes back, for a marker split across reads).
    let mut head_at = 0usize;
    let started = std::time::Instant::now();
    loop {
        if started.elapsed() > REPLY_DEADLINE {
            return Err(Error::Protocol("the print server took too long to reply".into()));
        }
        let n = stream.read(&mut buf).map_err(|e| Error::Protocol(format!("reading: {e}")))?;
        if n == 0 {
            return Ok(raw);
        }
        raw.extend_from_slice(buf.get(..n).unwrap_or_default());
        if raw.len() > MAX_REPLY {
            return Err(Error::Protocol("the reply is larger than 16 MiB".into()));
        }
        if body_at.is_none()
            && let Some(end) = raw.get(head_at..).and_then(|r| r.windows(4).position(|w| w == b"\r\n\r\n")).map(|p| head_at + p)
        {
            body_at = Some(end + 4);
            let (_, headers, _) = http_response(&raw)?;
            expected = headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse().ok());
            chunked = headers.iter().any(|(k, v)| k == "transfer-encoding" && v.contains("chunked"));
        }
        let Some(at) = body_at else {
            head_at = raw.len().saturating_sub(3);
            continue;
        };
        let body = raw.get(at..).unwrap_or_default();
        let complete = match expected {
            Some(len) if !chunked => body.len() >= len,
            _ if chunked => chunked_complete(body, &mut chunk_at),
            _ => false,
        };
        if complete {
            if let Some(len) = expected.filter(|_| !chunked) {
                raw.truncate(at + len);
            }
            return Ok(raw);
        }
    }
}

/// Splits an HTTP/1.1 response into status, lowercase headers and body.
fn http_response(raw: &[u8]) -> Result<(u16, Headers, &[u8])> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| Error::Protocol("no HTTP header".into()))?;
    let head = String::from_utf8_lossy(raw.get(..end).unwrap_or(&[]));
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status: u16 =
        status_line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or_else(|| Error::Protocol(format!("bad status line `{status_line}`")))?;
    let headers = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_ascii_lowercase())).collect();
    Ok((status, headers, raw.get(end + 4..).unwrap_or(&[])))
}

/// Whether a chunked body has reached its last (zero-size) chunk, walking the chunk sizes so a
/// `0\r\n\r\n` inside a chunk's payload (an attribute value can hold it) does not count. A
/// malformed or impossible size counts as complete, so [`dechunk`] reports it instead of
/// waiting for more.
fn chunked_complete(body: &[u8], resume: &mut usize) -> bool {
    let mut b = body.get(*resume..).unwrap_or_default();
    loop {
        // Chunks already walked are not walked again: `resume` marks the start of this one.
        *resume = body.len() - b.len();
        let Some(line_end) = b.windows(2).position(|w| w == b"\r\n") else { return false };
        let size_str = String::from_utf8_lossy(b.get(..line_end).unwrap_or(&[]));
        let Ok(size) = usize::from_str_radix(size_str.split(';').next().unwrap_or_default().trim(), 16) else { return true };
        b = b.get(line_end + 2..).unwrap_or(&[]);
        if size == 0 {
            return true;
        }
        // A size past the reply cap can never be satisfied: complete, so `dechunk` rejects it
        // (checked, so `ffffffffffffffff` cannot overflow the slice arithmetic).
        let Some(end) = size.checked_add(2).filter(|_| size <= MAX_REPLY) else { return true };
        let Some(rest) = b.get(end..) else { return false };
        b = rest;
    }
}

/// Joins the chunks of a chunked transfer encoding.
fn dechunk(mut b: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = b.windows(2).position(|w| w == b"\r\n").ok_or_else(|| Error::Protocol("cut chunk size".into()))?;
        let size_str = String::from_utf8_lossy(b.get(..line_end).unwrap_or(&[]));
        let size = usize::from_str_radix(size_str.split(';').next().unwrap_or_default().trim(), 16).map_err(|_| Error::Protocol("bad chunk size".into()))?;
        b = b.get(line_end + 2..).unwrap_or(&[]);
        if size == 0 {
            return Ok(out);
        }
        let chunk = b.get(..size).ok_or_else(|| Error::Protocol("cut chunk".into()))?;
        out.extend_from_slice(chunk);
        if out.len() > MAX_REPLY {
            return Err(Error::Protocol("the reply is larger than 16 MiB".into()));
        }
        b = b.get(size + 2..).ok_or_else(|| Error::Protocol("cut chunk end".into()))?;
    }
}

/// Where `/path` lands on the server: `/printers/<name>` or `/classes/<name>`.
fn queue_path(name: &str, is_class: bool) -> Result<String> {
    if name.is_empty() || name.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return Err(Error::Protocol("printer name contains an invalid control character".into()));
    }
    let mut encoded = String::with_capacity(name.len());
    for &b in name.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(b));
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    Ok(format!("/{}/{encoded}", if is_class { "classes" } else { "printers" }))
}

/// The printers the server knows (CUPS-Get-Printers), then its classes (CUPS-Get-Classes, which
/// `lp -d` accepts just the same), the default one flagged.
pub fn printers(endpoint: &Endpoint) -> Result<Vec<Printer>> {
    let mut out = queues(endpoint, OP_CUPS_GET_PRINTERS, false)?;
    // A server without classes answers "not found"; an older server may not know the operation.
    out.extend(queues(endpoint, OP_CUPS_GET_CLASSES, true).unwrap_or_default());
    let default = default_printer(endpoint).unwrap_or(None);
    for p in &mut out {
        p.is_default = default.as_deref() == Some(p.name.as_str());
    }
    Ok(out)
}

fn queues(endpoint: &Endpoint, op: u16, is_class: bool) -> Result<Vec<Printer>> {
    let mut r = Request::new(op, 1);
    r.attr_many(TAG_KEYWORD, "requested-attributes", &["printer-name", "printer-info", "printer-state", "printer-is-accepting-jobs"]);
    let parsed = match post(endpoint, "/", &r.end()) {
        Ok(reply) => reply,
        // CUPS answers "not found" ("No destinations added.") when no printer is set up.
        Err(Error::Status(0x0406, _)) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    let attributes = &parsed.reply.attributes;
    for group in parsed.groups.iter().filter(|group| group.tag == TAG_PRINTER_GROUP) {
        let Some(group_attributes) = attributes.get(group.start..group.end) else {
            return Err(Error::Protocol("invalid printer attribute group range".into()));
        };
        let mut name = None;
        let mut info = String::new();
        let mut state = 3;
        let mut accepting_jobs = true;
        for a in group_attributes {
            match a.name.as_str() {
                "printer-name" if name.is_none() => name = Some(a.text()),
                "printer-info" => info = a.text(),
                "printer-state" => state = a.int().unwrap_or(3).clamp(0, 5) as u32,
                "printer-is-accepting-jobs" => accepting_jobs = a.values.first().is_none_or(|v| v.first() != Some(&0)),
                _ => {}
            }
        }
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            out.push(Printer { name, info, state, accepting_jobs, is_default: false, is_class });
        }
    }
    Ok(out)
}

/// The server's default printer (CUPS-Get-Default), `None` when there is none.
pub fn default_printer(endpoint: &Endpoint) -> Result<Option<String>> {
    let mut r = Request::new(OP_CUPS_GET_DEFAULT, 2);
    r.attr_many(TAG_KEYWORD, "requested-attributes", &["printer-name"]);
    match post(endpoint, "/", &r.end()) {
        Ok(reply) => Ok(reply.reply.attributes.iter().find(|a| a.name == "printer-name").map(Attribute::text).filter(|n| !n.is_empty())),
        // CUPS answers "not found" when no default is set.
        Err(Error::Status(0x0406, _)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// What to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job<'a> {
    pub printer: &'a str,
    /// `printer` names a class.
    pub is_class: bool,
    /// Shown in the queue.
    pub title: &'a str,
    pub user: &'a str,
    pub copies: u32,
    /// The document, as `format` (`application/pdf`).
    pub document: &'a [u8],
    pub format: &'a str,
}

/// Submits `job` (Print-Job) and returns the job id.
pub fn print_job(endpoint: &Endpoint, job: &Job<'_>) -> Result<u32> {
    let mut r = Request::new(OP_PRINT_JOB, 3);
    let path = queue_path(job.printer, job.is_class)?;
    r.attr(TAG_URI, "printer-uri", format!("ipp://{}{path}", endpoint.host()).as_bytes());
    r.attr(TAG_NAME, "requesting-user-name", job.user.as_bytes());
    r.attr(TAG_NAME, "job-name", job.title.as_bytes());
    r.attr(TAG_MIME, "document-format", job.format.as_bytes());
    if job.copies > 1 {
        r.buf.push(TAG_JOB_GROUP);
        r.integer("copies", job.copies.clamp(1, 999) as i32);
    }
    let mut body = r.end();
    body.extend_from_slice(job.document);
    let reply = post(endpoint, &path, &body)?.reply;
    reply
        .attributes
        .iter()
        .find(|a| a.name == "job-id")
        .and_then(Attribute::int)
        .map(|id| id.max(0) as u32)
        .ok_or_else(|| Error::Protocol("no job id in the reply".into()))
}

/// A request as a server sees it: the operation (the request's status field), its attributes
/// and the document data after the end tag. For fakes in tests and tools.
pub fn parse_request(b: &[u8]) -> Result<(u16, Vec<Attribute>, Vec<u8>)> {
    let (reply, end) = Reply::parse_at(b)?;
    Ok((reply.status, reply.attributes, b.get(end..).unwrap_or(&[]).to_vec()))
}

#[cfg(test)]
mod tests;
