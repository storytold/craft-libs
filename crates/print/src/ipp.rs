//! IPP/1.1 straight to the local CUPS server: printer discovery (`CUPS-Get-Printers`,
//! `CUPS-Get-Default`) and job submission (`Print-Job`) over HTTP/1.1, for when `lpstat`/`lp`
//! are missing (a sandboxed install, where the print dialog would otherwise have no printers).
//! Implemented from the public IPP encoding (RFC 2910/8010) and semantics (RFC 8011) and
//! CUPS's IPP documentation; the scheduler is reached on its socket or loopback port.

use super::{Duplex, Error, Job, Printer, Scaling};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

/// Where the local scheduler listens: its socket, else its loopback port.
const CUPS_SOCKET: &str = "/run/cups/cups.sock";

/// Caps on what the server may hand back: a reply is read into memory before it is parsed.
const MAX_BODY: usize = 1 << 20;
const MAX_HEADERS: usize = 8 << 10;
const MAX_PRINTERS: usize = 256;
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
pub(crate) enum Server {
    Unix(PathBuf),
    Tcp(SocketAddr),
}

fn endpoints() -> [Server; 2] {
    [Server::Unix(PathBuf::from(CUPS_SOCKET)), Server::Tcp(([127, 0, 0, 1], 631).into())]
}

/// Whether the local CUPS server is reachable: its socket, else its loopback port. A missing
/// scheduler is a normal `false`, not an error.
pub fn cups_reachable() -> bool {
    endpoints().iter().any(reachable)
}

pub(crate) fn reachable(server: &Server) -> bool {
    connect(server).is_ok()
}

fn connect(server: &Server) -> std::io::Result<Stream> {
    match server {
        Server::Unix(path) => UnixStream::connect(path).map(Stream::Unix),
        // A loopback connect either answers promptly or the server is not there.
        Server::Tcp(addr) => TcpStream::connect_timeout(addr, CONNECT_TIMEOUT).map(|s| {
            let _ = s.set_nodelay(true);
            Stream::Tcp(s)
        }),
    }
}

/// A connection to the scheduler.
enum Stream {
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Stream {
    fn set_timeouts(&self, timeout: Duration) -> std::io::Result<()> {
        let (read, write) = match self {
            Stream::Unix(s) => (s.set_read_timeout(Some(timeout)), s.set_write_timeout(Some(timeout))),
            Stream::Tcp(s) => (s.set_read_timeout(Some(timeout)), s.set_write_timeout(Some(timeout))),
        };
        read.and(write)
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Unix(s) => s.read(buf),
            Stream::Tcp(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Stream::Unix(s) => s.write(buf),
            Stream::Tcp(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
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

pub(crate) fn printers_at(endpoints: &[Server]) -> Result<Vec<Printer>, Error> {
    let queues = discover(endpoints, OPERATION_CUPS_GET_PRINTERS)?;
    let default = discover(endpoints, OPERATION_CUPS_GET_DEFAULT).ok().and_then(|names| names.into_iter().next());
    let mut printers: Vec<Printer> = Vec::new();
    for name in queues {
        if printers.len() >= MAX_PRINTERS {
            break;
        }
        if !printers.iter().any(|p| p.name == name) {
            printers.push(Printer { default: default.as_deref() == Some(name.as_str()), name, two_sided: None });
        }
    }
    if let Some(name) = default
        && !printers.iter().any(|p| p.name == name)
    {
        printers.push(Printer { name, default: true, two_sided: None });
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
        match exchange(&mut stream, path, body, timeout) {
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
fn exchange(stream: &mut Stream, path: &str, body: &[u8], timeout: Duration) -> Result<Vec<u8>, Error> {
    stream.set_timeouts(timeout).map_err(|e| Error::Spool(format!("the print server could not be set up: {e}")))?;
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body))
        .and_then(|()| stream.flush())
        .map_err(|e| Error::Spool(format!("the request could not be sent to the print server: {e}")))?;
    read_reply(stream)
}

/// The reply's body: a strict status line and headers, then exactly `Content-Length` bytes. CUPS
/// answers with Content-Length; a chunked or unterminated reply is refused, not guessed.
fn read_reply(stream: &mut Stream) -> Result<Vec<u8>, Error> {
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
    for line in head.lines().skip(1) {
        // The empty line ends the header block; any other line without a colon is malformed.
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(Error::Spool("the print server sent an unreadable header".into()));
        };
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => content_length = value.trim().parse::<usize>().ok(),
            // Only `identity` is not a transfer encoding to be decoded.
            "transfer-encoding" if !value.trim().eq_ignore_ascii_case("identity") => {
                return Err(Error::Spool("the print server sent a chunked reply".into()));
            }
            _ => {}
        }
    }
    let Some(len) = content_length else { return Err(Error::Spool("the print server sent no Content-Length".into())) };
    if len > MAX_BODY {
        return Err(Error::Spool(format!("the print server's reply is too large ({len} bytes)")));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).map_err(|e| Error::Spool(format!("the print server's reply was cut short: {e}")))?;
    Ok(body)
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
    r.additional(TAG_KEYWORD, "printer-state");
    r.additional(TAG_KEYWORD, "printer-is-accepting-jobs");
    r.end();
    r.bytes
}

/// A Print-Job request for `queue`: the job's options as IPP attributes, the PDF as the document
/// data that follows the end of attributes.
fn print_job_request(queue: &str, pdf: &[u8], job: &Job) -> Vec<u8> {
    let mut r = Request::new(OPERATION_PRINT_JOB);
    r.group(GROUP_OPERATION);
    r.attribute(TAG_CHARSET, "attributes-charset", b"utf-8");
    r.attribute(TAG_LANGUAGE, "attributes-natural-language", b"en");
    r.attribute(TAG_URI, "printer-uri", format!("ipp://localhost/printers/{queue}").as_bytes());
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

fn parse_reply(reply: &[u8]) -> Result<Reply, Error> {
    let malformed = || Error::Spool("the print server sent a malformed reply".into());
    let mut r = Reader { bytes: reply, pos: 0 };
    let _version = r.array::<2>().ok_or_else(malformed)?;
    let status = r.u16().ok_or_else(malformed)?;
    let _request_id = r.array::<4>().ok_or_else(malformed)?;
    let (mut names, mut job_id, mut group) = (Vec::new(), None, 0u8);
    loop {
        let Some(tag) = r.array::<1>().map(|t| t[0]) else { return Err(malformed()) };
        match tag {
            GROUP_END => break,
            GROUP_OPERATION | GROUP_JOB | GROUP_PRINTER | 0x05 => {
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
                        if names.len() < MAX_PRINTERS
                            && let Some(printer) = std::str::from_utf8(value).ok().filter(|n| !n.is_empty() && n.len() <= MAX_NAME)
                        {
                            names.push(printer.to_string());
                        }
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
    Ok(Reply { status, names, job_id })
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
