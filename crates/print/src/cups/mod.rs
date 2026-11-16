//! Based on the direct CUPS transport from contributor PR #156 (ahmedhosnypro),
//! kept separate from the broader network-printing registry in PR #116.
//! IPP/1.1 straight to the local CUPS server: printer discovery (`CUPS-Get-Printers`,
//! `CUPS-Get-Default`) and job submission (`Print-Job`) over HTTP/1.1, for when `lpstat`/`lp`
//! are missing (a sandboxed install, where the print dialog would otherwise have no printers).
//! Implemented from the public IPP encoding (RFC 2910/8010) and semantics (RFC 8011) and
//! CUPS's IPP documentation; the scheduler is reached on its socket or selected TCP endpoint.
//!
//! # Blocking and authorization
//! The caller authorizes endpoint selection. The absolute exchange deadline covers all request
//! writes and reply reads **after connection**. Standard-library DNS resolution and Unix socket
//! connection are blocking OS calls outside that deadline. TCP connection attempts have their
//! own timeout. This is not a whole-operation deadline API: do not call it on a UI thread or
//! promise bounded completion for arbitrary named hosts/Unix listeners. No detached timeout
//! thread is spawned; callers needing cancellation must supply a separately bounded transport.

#[cfg(test)]
#[cfg(unix)]
mod tests;

use super::{Duplex, Error, Job, Printer, Scaling};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Where the local scheduler listens: its socket, else its loopback port.
const CUPS_SOCKET: &str = "/run/cups/cups.sock";

/// Caps on what the server may hand back: a reply is read into memory before it is parsed.
const MAX_BODY: usize = 1 << 20;
const MAX_HEADERS: usize = 8 << 10;
const MAX_PRINTERS: usize = 256;
// Even opted-in fleet discovery has a fixed record ceiling in addition to MAX_BODY.
const MAX_DISCOVERY_QUEUES: usize = 65_536;
const MAX_NAME: usize = 256;
/// A job's title on the wire; IPP values are u16-length, and no real title is this long.
const MAX_JOB_NAME: usize = 255;

/// A wedged scheduler must not hang the dialog; a job may carry megabytes and is given longer.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(2);
const JOB_TIMEOUT: Duration = Duration::from_secs(30);

/// IPP operation codes: the standard Print-Job, and CUPS's discovery extensions.
const OPERATION_PRINT_JOB: u16 = 0x0002;
const OPERATION_CUPS_GET_DEFAULT: u16 = 0x4001;
const OPERATION_CUPS_GET_PRINTERS: u16 = 0x4002;

/// IPP value tags and group delimiters (RFC 8010 §5.1); the ones this client sends or reads.
const GROUP_OPERATION: u8 = 0x01;
const GROUP_JOB: u8 = 0x02;
const GROUP_END: u8 = 0x03;
const GROUP_PRINTER: u8 = 0x04;
const TAG_INTEGER: u8 = 0x21;
const TAG_BOOLEAN: u8 = 0x22;
const TAG_NAME: u8 = 0x42;
const TAG_KEYWORD: u8 = 0x44;
const TAG_URI: u8 = 0x45;
const TAG_CHARSET: u8 = 0x47;
const TAG_LANGUAGE: u8 = 0x48;
const TAG_MIME: u8 = 0x49;

static REQUEST_ID: AtomicU32 = AtomicU32::new(1);

/// How the scheduler is reached: its unix socket, else its loopback port.
#[derive(Clone, Debug)]
pub enum Server {
    /// The scheduler's unix socket.
    Unix(PathBuf),
    /// The scheduler's loopback port.
    Tcp(SocketAddr),
    /// A caller-authorized host. DNS is resolved once for each connection attempt.
    Named(String, u16),
}

impl Server {
    /// Parse an explicitly selected CUPS_SERVER value, without connecting. No URLs,
    /// credentials, control characters or HTTP authority delimiters are accepted.
    pub fn parse(value: &str) -> Result<Self, Error> {
        if value.starts_with('/') {
            return Ok(Self::Unix(value.into()));
        }
        if value.is_empty() || value.len() > 255 || value.bytes().any(|c| c.is_ascii_control() || c.is_ascii_whitespace() || b"/@?#\\".contains(&c)) {
            return Err(Error::Spool("invalid CUPS server".into()));
        }
        let (host, port) = if let Some(rest) = value.strip_prefix('[') {
            let (host, tail) = rest.split_once(']').ok_or_else(|| Error::Spool("invalid CUPS IPv6 host".into()))?;
            if !host.contains(':') {
                return Err(Error::Spool("bracketed CUPS host must be IPv6".into()));
            }
            let port = if tail.is_empty() {
                631
            } else {
                tail.strip_prefix(':')
                    .and_then(|p| p.parse::<u16>().ok())
                    .filter(|p| *p != 0)
                    .ok_or_else(|| Error::Spool("invalid CUPS port".into()))?
            };
            (host, port)
        } else if value.matches(':').count() > 1 {
            // A bare IPv6 address is a host. A colon after its scope name may
            // specify a port without cutting an address hextet in half.
            if let Some((_, zone)) = value.split_once('%')
                && zone.contains(':')
            {
                let (host, port) = value.rsplit_once(':').ok_or_else(|| Error::Spool("invalid CUPS endpoint".into()))?;
                (host, port.parse::<u16>().ok().filter(|p| *p != 0).ok_or_else(|| Error::Spool("invalid CUPS port".into()))?)
            } else {
                (value, 631)
            }
        } else {
            match value.rsplit_once(':') {
                Some((host, port)) => (host, port.parse::<u16>().ok().filter(|p| *p != 0).ok_or_else(|| Error::Spool("invalid CUPS port".into()))?),
                None => (value, 631),
            }
        };
        if host.is_empty() || host.contains(['[', ']']) {
            return Err(Error::Spool("invalid CUPS host".into()));
        }
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            return Ok(Self::Tcp(SocketAddr::new(ip, port)));
        }
        if host.contains(':') {
            let (ip, zone) = host.split_once('%').ok_or_else(|| Error::Spool("invalid CUPS IPv6 host".into()))?;
            if ip.parse::<std::net::Ipv6Addr>().is_err()
                || zone.is_empty()
                || !zone.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            {
                return Err(Error::Spool("invalid CUPS IPv6 scope".into()));
            }
        } else if host.contains('%') {
            return Err(Error::Spool("invalid CUPS host".into()));
        }
        Ok(Self::Named(host.into(), port))
    }

    fn authority(&self) -> String {
        match self {
            Self::Unix(_) => "localhost".into(),
            Self::Tcp(addr) => addr.to_string().replace('%', "%25"),
            Self::Named(host, port) if host.contains(':') => format!("[{}]:{port}", host.replace('%', "%25")),
            Self::Named(host, port) => format!("{host}:{port}"),
        }
    }
}

/// Local scheduler candidates, honoring the user's explicit CUPS_SERVER setting.
/// No requests are made by this function; apps must authorize selection before use.
pub fn configured_servers() -> Result<Vec<Server>, Error> {
    if let Some(value) = std::env::var_os("CUPS_SERVER").filter(|v| !v.is_empty()) {
        return Ok(vec![Server::parse(&value.to_string_lossy())?]);
    }
    Ok(vec![
        Server::Unix(CUPS_SOCKET.into()),
        Server::Unix("/var/run/cups/cups.sock".into()),
        Server::Unix("/private/var/run/cupsd".into()),
        Server::Tcp(([127, 0, 0, 1], 631).into()),
    ])
}

fn endpoints() -> [Server; 2] {
    [Server::Unix(PathBuf::from(CUPS_SOCKET)), Server::Tcp(([127, 0, 0, 1], 631).into())]
}

/// Whether the local CUPS server is reachable: its socket, else its loopback port. A missing
/// scheduler is a normal `false`, not an error.
pub fn cups_reachable() -> bool {
    endpoints().iter().any(reachable)
}

/// Probe a caller-authorized endpoint without sending a print request.
/// DNS and Unix connect have the blocking limitations documented on this module.
pub fn reachable(server: &Server) -> bool {
    connect(server).is_ok()
}

fn connect(server: &Server) -> std::io::Result<Stream> {
    match server {
        #[cfg(unix)]
        Server::Unix(path) => UnixStream::connect(path).map(Stream::Unix),
        #[cfg(not(unix))]
        Server::Unix(_) => Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "Unix sockets unavailable")),
        Server::Named(host, port) => {
            // Validate even directly constructed public variants before resolving or writing headers.
            let endpoint = if host.contains(':') { format!("[{host}]:{port}") } else { format!("{host}:{port}") };
            let parsed = Server::parse(&endpoint).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
            if matches!(parsed, Server::Unix(_)) {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "a named CUPS host cannot be a Unix path"));
            }
            let addr = (host.as_str(), *port)
                .to_socket_addrs()?
                .next()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "CUPS host has no address"))?;
            TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map(Stream::Tcp)
        }
        // A loopback connect either answers promptly or the server is not there.
        Server::Tcp(addr) => TcpStream::connect_timeout(addr, CONNECT_TIMEOUT).map(|s| {
            let _ = s.set_nodelay(true);
            Stream::Tcp(s)
        }),
    }
}

/// A connection to the scheduler.
enum Stream {
    #[cfg(unix)]
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Stream {
    fn set_timeouts(&self, timeout: Duration) -> std::io::Result<()> {
        let (read, write) = match self {
            #[cfg(unix)]
            Stream::Unix(s) => (s.set_read_timeout(Some(timeout)), s.set_write_timeout(Some(timeout))),
            Stream::Tcp(s) => (s.set_read_timeout(Some(timeout)), s.set_write_timeout(Some(timeout))),
        };
        read.and(write)
    }
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

/// The server's queues (CUPS-Get-Printers) and its default destination (CUPS-Get-Default), for
/// the Print dialog when `lpstat` is missing. A default the queue list lacks is appended, so
/// the dialog always has a selection to restore; a default that cannot be named still leaves
/// the queues usable.
pub fn printers() -> Result<Vec<Printer>, Error> {
    printers_at(&endpoints())
}

/// [`printers`] from the given scheduler endpoints.
pub fn printers_at(endpoints: &[Server]) -> Result<Vec<Printer>, Error> {
    let queues = discover(endpoints, OPERATION_CUPS_GET_PRINTERS)?;
    let default = discover(endpoints, OPERATION_CUPS_GET_DEFAULT).ok().and_then(|names| names.into_iter().next());
    let mut printers: Vec<Printer> = Vec::new();
    for name in queues {
        if printers.len() >= MAX_PRINTERS {
            break;
        }
        if !printers.iter().any(|p| p.name == name) {
            printers.push(Printer { description: String::new(), default: default.as_deref() == Some(name.as_str()), name, two_sided: None });
        }
    }
    if let Some(name) = default
        && !printers.iter().any(|p| p.name == name)
    {
        printers.push(Printer { description: String::new(), name, default: true, two_sided: None });
    }
    Ok(printers)
}

/// The printer names a discovery reply carries: every queue (CUPS-Get-Printers), or the default
/// destination (CUPS-Get-Default).
fn discover(endpoints: &[Server], operation: u16) -> Result<Vec<String>, Error> {
    let reply = post(endpoints, "/", &discovery_request(operation), DISCOVERY_TIMEOUT, true)?;
    let parsed = parse_reply(&reply)?;
    if status_ok(parsed.status) { Ok(parsed.names) } else { Err(refused(parsed.status)) }
}

/// Submit the PDF as a Print-Job to the printer (the job's, else the server's default); the
/// reply is the job message, like `lp` prints it (`request id is Office-12`). The driver's own
/// job options (PPD keywords) are not sent on this path: they have no IPP equivalents without
/// reading the printer's capabilities, which is another request.
pub fn print_job(pdf: &[u8], job: &Job) -> Result<String, Error> {
    print_job_at(&endpoints(), pdf, job)
}

pub(crate) fn print_job_at(endpoints: &[Server], pdf: &[u8], job: &Job) -> Result<String, Error> {
    let queue = match &job.printer {
        Some(name) => name.clone(),
        None => discover(endpoints, OPERATION_CUPS_GET_DEFAULT)?
            .into_iter()
            .next()
            .ok_or_else(|| Error::Spool("the print server has no default printer; choose one in the dialog".into()))?,
    };
    if !is_queue_name(&queue) {
        return Err(Error::Spool(format!("not a printer name: {queue}")));
    }
    let reply = post(endpoints, &format!("/printers/{queue}"), &print_job_request(&queue, pdf, job), JOB_TIMEOUT, false)?;
    let parsed = parse_reply(&reply)?;
    if !status_ok(parsed.status) {
        return Err(refused(parsed.status));
    }
    Ok(match parsed.job_id {
        Some(id) => format!("request id is {queue}-{id}"),
        None => format!("the job was accepted by {queue}"),
    })
}

/// A queue class can route a job to any member printer. Keep the destination kind
/// explicit rather than guessing it from a name shared by a printer and a class.
#[derive(Clone, Debug)]
pub struct Destination {
    /// The queue or class name.
    pub name: String,
    /// Whether the destination is a class rather than a printer queue.
    pub is_class: bool,
}

/// List both printer queues and classes from caller-selected CUPS endpoints.
/// The caller owns endpoint selection/authorization; this API does not discover network hosts.
pub fn destinations_at(endpoints: &[Server]) -> Result<Vec<Queue>, Error> {
    let mut out = queues_at(endpoints, false)?;
    out.extend(queues_at(endpoints, true)?);
    if out.len() > MAX_PRINTERS {
        return Err(Error::Spool("too many CUPS destinations".into()));
    }
    let default = default_at(endpoints)?;
    for queue in &mut out {
        queue.is_default = default.as_deref() == Some(queue.name.as_str());
    }
    Ok(out)
}

/// List either printer queues or classes, without resolving defaults. Applications can
/// explicitly choose whether an optional class/default query failure should hide otherwise
/// usable printer queues. No error is swallowed here except CUPS's absent/unsupported list.
pub fn queues_at(endpoints: &[Server], is_class: bool) -> Result<Vec<Queue>, Error> {
    queues_at_with_limit(endpoints, is_class, MAX_PRINTERS)
}

/// List queues or classes with an explicit fleet-size budget (1 through 65,536).
///
/// The existing [`queues_at`] API keeps its 256-queue default. Increasing this budget does
/// not increase the one-MiB response, HTTP header, name-length or exchange-time limits.
/// An excessive response is refused in full, never silently truncated. This changes only
/// local parsing: it does not invent pagination or retry a possibly submitted print job.
///
/// # Errors
/// Returns an error for an invalid budget, an over-budget or malformed response, or a
/// transport/server error, with the same absent-list handling as [`queues_at`].
pub fn queues_at_with_limit(endpoints: &[Server], is_class: bool, max_queues: usize) -> Result<Vec<Queue>, Error> {
    check_queue_limit(max_queues)?;
    let operation = if is_class { 0x4005 } else { OPERATION_CUPS_GET_PRINTERS };
    let reply = post(endpoints, "/", &discovery_request(operation), DISCOVERY_TIMEOUT, true)?;
    let parsed = parse_reply_with_limit(&reply, max_queues)?;
    if !status_ok(parsed.status) {
        if parsed.status == 0x0406 || (is_class && parsed.status == 0x0501) {
            return Ok(Vec::new());
        }
        return Err(refused(parsed.status));
    }
    let mut queues = parsed.queues;
    for queue in &mut queues {
        queue.is_class = is_class;
    }
    Ok(queues)
}

/// The scheduler's default queue; not-found means no default has been configured.
pub fn default_at(endpoints: &[Server]) -> Result<Option<String>, Error> {
    let reply = post(endpoints, "/", &discovery_request(OPERATION_CUPS_GET_DEFAULT), DISCOVERY_TIMEOUT, true)?;
    let parsed = parse_reply(&reply)?;
    if parsed.status == 0x0406 {
        return Ok(None);
    }
    if !status_ok(parsed.status) {
        return Err(refused(parsed.status));
    }
    Ok(parsed.names.into_iter().next())
}

/// Metadata from one CUPS printer group, without losing which fields belong together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Queue {
    /// The queue name (`printer-name`).
    pub name: String,
    /// The description the queue shows (`printer-info`).
    pub info: String,
    /// The IPP `printer-state` value.
    pub state: u32,
    /// Whether the queue accepts new jobs (`printer-is-accepting-jobs`).
    pub accepting_jobs: bool,
    /// Whether this is the scheduler's default destination.
    pub is_default: bool,
    /// Whether the queue is a class rather than a printer.
    pub is_class: bool,
}
impl Default for Queue {
    fn default() -> Self {
        Self { name: String::new(), info: String::new(), state: 3, accepting_jobs: true, is_default: false, is_class: false }
    }
}

/// Submit once to an explicitly selected queue/class, returning the numeric server job id.
/// An absent id is an uncertain accepted outcome, not permission to retry the document.
pub fn submit_at(endpoints: &[Server], destination: &Destination, pdf: &[u8], job: &Job) -> Result<u32, Error> {
    submit_as_at(endpoints, destination, pdf, job, "")
}

/// As [`submit_at`], with the user name supplied by the application, never inferred from
/// process credentials. Once any request bytes may have been sent, no other endpoint is tried.
pub fn submit_as_at(endpoints: &[Server], destination: &Destination, pdf: &[u8], job: &Job, user: &str) -> Result<u32, Error> {
    let path = destination_path(destination)?;
    if user.len() > MAX_JOB_NAME || user.chars().any(char::is_control) {
        return Err(Error::Spool("invalid CUPS user name".into()));
    }
    if job.copies == 0 || job.copies > 999 {
        return Err(Error::Spool("copies must be between 1 and 999".into()));
    }
    let mut last = Error::Spool("no CUPS endpoint selected".into());
    for endpoint in endpoints {
        let mut stream = match connect(endpoint) {
            Ok(stream) => stream,
            Err(error) => {
                last = Error::Spool(format!("CUPS connection failed: {error}"));
                continue;
            }
        };
        let authority = endpoint.authority();
        let request = print_job_request_for(&authority, &path, pdf, job, user);
        // Never retry exchange errors: the scheduler may already have accepted the job.
        let reply = exchange(&mut stream, &authority, &path, &request, JOB_TIMEOUT)?;
        let parsed = parse_reply(&reply)?;
        if !status_ok(parsed.status) {
            return Err(refused(parsed.status));
        }
        return parsed
            .job_id
            .filter(|id| *id != 0 && *id <= i32::MAX as u32)
            .ok_or_else(|| Error::Spool("CUPS accepted the job without a usable job id; do not resubmit".into()));
    }
    Err(last)
}

/// Encode one destination as a URI path segment, never as routing syntax. Names from
/// discovery may include spaces or Unicode. Reject separators, controls and dot segments
/// rather than relying on a server to keep an encoded traversal request harmless.
fn destination_path(destination: &Destination) -> Result<String, Error> {
    let name = &destination.name;
    if name.is_empty()
        || name.len() > MAX_NAME
        || matches!(name.as_str(), "." | "..")
        || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\'))
    {
        return Err(Error::Spool("invalid CUPS destination name".into()));
    }
    let kind = if destination.is_class { "classes" } else { "printers" };
    let mut path = format!("/{kind}/");
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            path.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            write!(&mut path, "%{byte:02X}").map_err(|_| Error::Spool("could not encode CUPS destination".into()))?;
        }
    }
    Ok(path)
}

/// A CUPS queue name: ASCII letters, digits and `_`, `-`, `.` — what the scheduler accepts, and
/// what cannot break the printer URI's path.
fn is_queue_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_NAME && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// One IPP request and its reply body. Discovery may try each endpoint in turn; a job is given
/// to the first endpoint that accepts a connection, once: a retry could print it twice.
pub(crate) fn post(endpoints: &[Server], path: &str, body: &[u8], timeout: Duration, retry: bool) -> Result<Vec<u8>, Error> {
    let mut failed: Option<Error> = None;
    for server in endpoints {
        let mut stream = match connect(server) {
            Ok(stream) => stream,
            Err(e) => {
                failed = Some(Error::Spool(format!("the print server could not be reached: {e}")));
                continue;
            }
        };
        match exchange(&mut stream, &server.authority(), path, body, timeout) {
            Ok(reply) => return Ok(reply),
            Err(e) => {
                failed = Some(e);
                if !retry {
                    break;
                }
            }
        }
    }
    Err(failed.unwrap_or_else(|| Error::Spool("the print server could not be reached".into())))
}

/// Send one IPP request and read the reply's body.
fn exchange(stream: &mut Stream, authority: &str, path: &str, body: &[u8], timeout: Duration) -> Result<Vec<u8>, Error> {
    let mut stream = DeadlineReader { stream, end: Instant::now() + timeout };
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body))
        .and_then(|()| stream.flush())
        .map_err(|e| Error::Spool(format!("the request could not be sent to the print server: {e}")))?;
    let reply = read_reply(&mut stream)?;
    validate_reply_identity(body, &reply)?;
    Ok(reply)
}

/// One absolute deadline shared by request writes and reply reads after connection.
struct DeadlineReader<'a> {
    stream: &'a mut Stream,
    end: Instant,
}
impl Write for DeadlineReader<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let left = self
            .end
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::TimedOut, "CUPS exchange deadline exceeded"))?;
        self.stream.set_timeouts(left)?;
        self.stream.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

fn validate_reply_identity(request: &[u8], reply: &[u8]) -> Result<(), Error> {
    if reply.get(..2) != Some(&[1, 1]) || request.get(4..8).is_none() || request.get(4..8) != reply.get(4..8) {
        return Err(Error::Spool("CUPS reply version or request id does not match; do not resubmit".into()));
    }
    Ok(())
}

impl Read for DeadlineReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self
            .end
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::TimedOut, "CUPS reply deadline exceeded"))?;
        #[cfg(not(target_os = "macos"))]
        self.stream.set_timeouts(left)?;
        #[cfg(target_os = "macos")]
        if let Err(error) = self.stream.set_timeouts(left) {
            // Darwin rejects timeout updates on a disconnected Unix socket even
            // when a complete reply is still buffered. Never fall back to a
            // blocking read: drain at most this buffer, or fail on WouldBlock.
            // Every subsequent read rechecks the same absolute deadline above.
            if error.raw_os_error() == Some(22)
                && let Stream::Unix(stream) = self.stream
            {
                stream.set_nonblocking(true)?;
                let result = stream.read(buf);
                let restored = stream.set_nonblocking(false);
                return result.and_then(|count| restored.map(|()| count));
            }
            return Err(error);
        }
        self.stream.read(buf)
    }
}

fn read_reply(stream: &mut impl Read) -> Result<Vec<u8>, Error> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if head.len() >= MAX_HEADERS {
            return Err(Error::Spool("the print server sent a header block that is too large".into()));
        }
        match stream.read(&mut byte) {
            Ok(0) => return Err(Error::Spool("the print server closed the connection before sending its reply".into())),
            Ok(_) => head.push(byte[0]),
            Err(e) => return Err(Error::Spool(format!("the print server's reply could not be read: {e}"))),
        }
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&head);
    let Some(status_line) = head.lines().next() else { return Err(Error::Spool("the print server sent an unreadable reply".into())) };
    let code = status_line
        .strip_prefix("HTTP/")
        .and_then(|rest| rest.split_once(' '))
        .and_then(|(_, code)| code.split_whitespace().next())
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| Error::Spool("the print server sent an unreadable reply".into()))?;
    match code {
        200..=299 => {}
        401 => return Err(Error::Spool("the print server requires authentication".into())),
        426 => return Err(Error::Spool("the print server only accepts encrypted connections".into())),
        _ => return Err(Error::Spool(format!("the print server refused the request (HTTP {code})"))),
    }
    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    for line in head.lines().skip(1) {
        // The empty line ends the header block; any other line without a colon is malformed.
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(Error::Spool("the print server sent an unreadable header".into()));
        };
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => {
                let len = value.trim().parse::<usize>().map_err(|_| Error::Spool("invalid Content-Length".into()))?;
                if content_length.replace(len).is_some() {
                    return Err(Error::Spool("duplicate Content-Length".into()));
                }
            }
            // Only `identity` is not a transfer encoding to be decoded.
            "transfer-encoding" => {
                if chunked || !value.trim().eq_ignore_ascii_case("chunked") {
                    return Err(Error::Spool("unsupported Transfer-Encoding".into()));
                }
                chunked = true;
            }
            _ => {}
        }
    }
    if chunked {
        if content_length.is_some() {
            return Err(Error::Spool("ambiguous reply framing".into()));
        }
        return read_chunks(stream);
    }
    let Some(len) = content_length else { return Err(Error::Spool("the print server sent no Content-Length".into())) };
    if len > MAX_BODY {
        return Err(Error::Spool(format!("the print server's reply is too large ({len} bytes)")));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).map_err(|e| Error::Spool(format!("the print server's reply was cut short: {e}")))?;
    Ok(body)
}

fn read_chunks(stream: &mut impl Read) -> Result<Vec<u8>, Error> {
    fn line(stream: &mut impl Read) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        for _ in 0..MAX_HEADERS {
            let mut byte = [0];
            stream.read_exact(&mut byte).map_err(|e| Error::Spool(format!("truncated chunk: {e}")))?;
            out.push(byte[0]);
            if out.ends_with(b"\r\n") {
                out.truncate(out.len() - 2);
                return Ok(out);
            }
        }
        Err(Error::Spool("chunk line exceeds limit".into()))
    }
    let mut body = Vec::new();
    for _ in 0..100_000 {
        let header = line(stream)?;
        let size = header.split(|b| *b == b';').next().unwrap_or_default();
        if size.is_empty() || !size.iter().all(u8::is_ascii_hexdigit) {
            return Err(Error::Spool("invalid chunk size".into()));
        }
        let len = std::str::from_utf8(size)
            .ok()
            .and_then(|s| usize::from_str_radix(s, 16).ok())
            .ok_or_else(|| Error::Spool("invalid chunk size".into()))?;
        if len == 0 {
            let mut total = 0usize;
            loop {
                let trailer = line(stream)?;
                total = total.saturating_add(trailer.len() + 2);
                if total > MAX_HEADERS {
                    return Err(Error::Spool("chunk trailers exceed limit".into()));
                }
                if trailer.is_empty() {
                    return Ok(body);
                }
                if !trailer.contains(&b':') {
                    return Err(Error::Spool("invalid chunk trailer".into()));
                }
            }
        }
        let end = body.len().checked_add(len).filter(|n| *n <= MAX_BODY).ok_or_else(|| Error::Spool("chunked reply exceeds limit".into()))?;
        let start = body.len();
        body.resize(end, 0);
        stream.read_exact(&mut body[start..]).map_err(|e| Error::Spool(format!("truncated chunk: {e}")))?;
        let mut crlf = [0; 2];
        stream.read_exact(&mut crlf).map_err(|e| Error::Spool(format!("truncated chunk: {e}")))?;
        if crlf != *b"\r\n" {
            return Err(Error::Spool("invalid chunk terminator".into()));
        }
    }
    Err(Error::Spool("too many chunks".into()))
}

/// An IPP request being built: a version, an operation and a request id, then attributes, then
/// the document.
struct Request {
    bytes: Vec<u8>,
}

impl Request {
    fn new(operation: u16) -> Self {
        // IPP 1.1: version, operation, a request id (any nonzero value).
        let mut bytes = vec![1, 1];
        bytes.extend(operation.to_be_bytes());
        bytes.extend(REQUEST_ID.fetch_add(1, Ordering::Relaxed).max(1).to_be_bytes());
        Request { bytes }
    }

    fn group(&mut self, tag: u8) {
        self.bytes.push(tag);
    }

    fn end(&mut self) {
        self.bytes.push(GROUP_END);
    }

    fn attribute(&mut self, tag: u8, name: &str, value: &[u8]) {
        let (Ok(name_len), Ok(value_len)) = (u16::try_from(name.len()), u16::try_from(value.len())) else {
            return;
        };
        self.bytes.push(tag);
        self.bytes.extend(name_len.to_be_bytes());
        self.bytes.extend(name.as_bytes());
        self.bytes.extend(value_len.to_be_bytes());
        self.bytes.extend(value);
    }

    /// An additional value of the previous attribute (a `1setOf`): the name is empty.
    fn additional(&mut self, tag: u8, value: &str) {
        self.attribute(tag, "", value.as_bytes());
    }
}

/// A discovery request: the operation's required attributes, then the names to be answered with.
fn discovery_request(operation: u16) -> Vec<u8> {
    let mut r = Request::new(operation);
    r.group(GROUP_OPERATION);
    r.attribute(TAG_CHARSET, "attributes-charset", b"utf-8");
    r.attribute(TAG_LANGUAGE, "attributes-natural-language", b"en");
    r.attribute(TAG_URI, "printer-uri", b"ipp://localhost/");
    r.attribute(TAG_KEYWORD, "requested-attributes", b"printer-name");
    r.additional(TAG_KEYWORD, "printer-info");
    r.additional(TAG_KEYWORD, "printer-state");
    r.additional(TAG_KEYWORD, "printer-is-accepting-jobs");
    r.end();
    r.bytes
}

/// A Print-Job request for `queue`: the job's options as IPP attributes, the PDF as the document
/// data that follows the end of attributes.
fn print_job_request(queue: &str, pdf: &[u8], job: &Job) -> Vec<u8> {
    print_job_request_path(&format!("/printers/{queue}"), pdf, job)
}

fn print_job_request_path(path: &str, pdf: &[u8], job: &Job) -> Vec<u8> {
    print_job_request_for("localhost", path, pdf, job, "")
}

fn print_job_request_for(authority: &str, path: &str, pdf: &[u8], job: &Job, user: &str) -> Vec<u8> {
    let mut r = Request::new(OPERATION_PRINT_JOB);
    r.group(GROUP_OPERATION);
    r.attribute(TAG_CHARSET, "attributes-charset", b"utf-8");
    r.attribute(TAG_LANGUAGE, "attributes-natural-language", b"en");
    r.attribute(TAG_URI, "printer-uri", format!("ipp://{authority}{path}").as_bytes());
    if !user.is_empty() {
        r.attribute(TAG_NAME, "requesting-user-name", user.as_bytes());
    }
    r.attribute(TAG_NAME, "job-name", truncate_utf8(&job.title, MAX_JOB_NAME).as_bytes());
    r.attribute(TAG_MIME, "document-format", b"application/pdf");
    r.group(GROUP_JOB);
    r.attribute(TAG_INTEGER, "copies", &job.copies.clamp(1, 999).to_be_bytes());
    r.attribute(TAG_BOOLEAN, "collate", &[u8::from(job.collate)]);
    if let Some(duplex) = job.duplex {
        r.attribute(
            TAG_KEYWORD,
            "sides",
            match duplex {
                Duplex::Off => "one-sided",
                Duplex::LongEdge => "two-sided-long-edge",
                Duplex::ShortEdge => "two-sided-short-edge",
            }
            .as_bytes(),
        );
    }
    if job.grayscale {
        r.attribute(TAG_KEYWORD, "print-color-mode", b"monochrome");
    }
    // PWG 5100.13 §6.2.5: `none` keeps every sheet at its size, `auto-fit` shrinks only
    // oversized ones — the same choices `lp`'s fit-to-page and print-scaling options make.
    r.attribute(
        TAG_KEYWORD,
        "print-scaling",
        match job.scaling {
            Scaling::Exact => "none",
            Scaling::ShrinkToFit => "auto-fit",
        }
        .as_bytes(),
    );
    r.end();
    r.bytes.extend(pdf);
    r.bytes
}

/// The string cut to at most `max` bytes, at a character boundary.
fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    // `end` is a checked boundary; nothing is the only name that cannot be cut.
    s.get(..end).unwrap_or_default()
}

/// The parsed parts of a reply: the IPP status, the printer names in its printer groups, and a
/// submission's job id.
struct Reply {
    status: u16,
    names: Vec<String>,
    queues: Vec<Queue>,
    job_id: Option<u32>,
}

/// A cursor over a reply: `None` at the end or past a length, never an over-read.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(len)?;
        let taken = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(taken)
    }

    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        <[u8; N]>::try_from(self.take(N)?).ok()
    }

    fn u16(&mut self) -> Option<u16> {
        self.array::<2>().map(u16::from_be_bytes)
    }
}

fn check_queue_limit(max_queues: usize) -> Result<(), Error> {
    if !(1..=MAX_DISCOVERY_QUEUES).contains(&max_queues) {
        return Err(Error::Spool("CUPS discovery queue limit must be 1 through 65536".into()));
    }
    Ok(())
}

fn parse_reply(reply: &[u8]) -> Result<Reply, Error> {
    parse_reply_with_limit(reply, MAX_PRINTERS)
}

fn parse_reply_with_limit(reply: &[u8], max_queues: usize) -> Result<Reply, Error> {
    check_queue_limit(max_queues)?;
    if reply.len() > MAX_BODY {
        return Err(Error::Spool("CUPS reply exceeds limit".into()));
    }
    let malformed = || Error::Spool("the print server sent a malformed reply".into());
    let mut r = Reader { bytes: reply, pos: 0 };
    let _version = r.array::<2>().ok_or_else(malformed)?;
    let status = r.u16().ok_or_else(malformed)?;
    let _request_id = r.array::<4>().ok_or_else(malformed)?;
    let (mut names, mut job_id, mut group) = (Vec::new(), None, 0u8);
    let mut queues = Vec::new();
    let mut queue = Queue::default();
    loop {
        let Some(tag) = r.array::<1>().map(|t| t[0]) else { return Err(malformed()) };
        match tag {
            GROUP_END => {
                if !queue.name.is_empty() {
                    queues.push(queue);
                }
                break;
            }
            GROUP_OPERATION | GROUP_JOB | GROUP_PRINTER | 0x05 => {
                if !queue.name.is_empty() {
                    if queues.len() >= max_queues {
                        return Err(malformed());
                    }
                    queues.push(std::mem::take(&mut queue));
                } else {
                    queue = Queue::default();
                }
                group = tag;
            }
            // An attribute: name and value, each u16-length. An empty name is an additional
            // value of the previous one (the attributes read here are single-valued, so those
            // are ignored). Group tags not in the registry and collections (the attributes
            // requested here are never collections) are refused, not guessed.
            0x10..=0x33 | 0x35..=0x4A => {
                let name_len = r.u16().ok_or_else(malformed)?;
                if usize::from(name_len) > MAX_NAME {
                    return Err(malformed());
                }
                let name_bytes = r.take(usize::from(name_len)).ok_or_else(malformed)?;
                let name = std::str::from_utf8(name_bytes).map_err(|_| malformed())?;
                let value_len = r.u16().ok_or_else(malformed)?;
                let value = r.take(usize::from(value_len)).ok_or_else(malformed)?;
                match (group, name, tag) {
                    (GROUP_PRINTER, "printer-name", TAG_NAME) => {
                        if let Some(printer) = std::str::from_utf8(value).ok().filter(|n| !n.is_empty() && n.len() <= MAX_NAME) {
                            if names.len() >= max_queues {
                                return Err(Error::Spool("too many CUPS destinations".into()));
                            }
                            if !queue.name.is_empty() {
                                return Err(malformed());
                            }
                            queue.name = printer.to_string();
                            names.push(printer.to_string());
                        }
                    }
                    (GROUP_PRINTER, "printer-info", 0x41 | TAG_NAME) => {
                        // Human descriptions are text, not queue identifiers. Their u16
                        // wire length and the whole-response MAX_BODY cap bound allocation.
                        queue.info = std::str::from_utf8(value).map_err(|_| malformed())?.to_owned();
                    }
                    (GROUP_PRINTER, "printer-state", 0x23 | TAG_INTEGER) => {
                        queue.state = u32::from_be_bytes(value.try_into().map_err(|_| malformed())?);
                    }
                    (GROUP_PRINTER, "printer-is-accepting-jobs", TAG_BOOLEAN) => {
                        queue.accepting_jobs = match value {
                            [0] => false,
                            [1] => true,
                            _ => return Err(malformed()),
                        };
                    }
                    (GROUP_JOB, "job-id", TAG_INTEGER) => {
                        if let Ok(id) = <[u8; 4]>::try_from(value) {
                            job_id = Some(u32::from_be_bytes(id));
                        }
                    }
                    _ => {}
                }
            }
            _ => return Err(malformed()),
        }
    }
    Ok(Reply { status, names, queues, job_id })
}

/// Successful IPP status codes are `0x00xx` (RFC 8011 Appendix B).
fn status_ok(status: u16) -> bool {
    status < 0x0100
}

/// The IPP status code's name, for the error the user sees.
fn status_name(status: u16) -> Option<&'static str> {
    Some(match status {
        0x0400 => "bad request",
        0x0401 => "forbidden",
        0x0402 => "not authenticated",
        0x0403 => "not authorized",
        0x0404 => "not possible",
        0x0406 => "printer not found",
        0x0408 => "request too large",
        0x040A => "PDF is not a supported document format for this printer",
        0x0412 => "the document could not be accessed",
        0x0500 => "internal error",
        0x0501 => "operation not supported",
        0x0502 => "service unavailable",
        0x0503 => "IPP version not supported",
        0x0504 => "device error",
        0x0505 => "temporary error",
        0x0506 => "the printer is not accepting jobs",
        0x0507 => "the printer is busy",
        _ => return None,
    })
}

fn refused(status: u16) -> Error {
    match status_name(status) {
        Some(name) => Error::Spool(format!("the print server refused the request ({name})")),
        None => Error::Spool(format!("the print server refused the request (IPP status 0x{status:04X})")),
    }
}
