//! The listener: a strict, std-only, blocking HTTP/1.1 receiver for event notifications.
//!
//! One thread, one connection at a time. A connection is refused *unread* unless it comes from the
//! address of a printer we hold a subscription with; then a token bucket, then a request that must
//! be exactly a `POST /n/<token>` or `/e/<token>` with a `Content-Length` within the cap, read within
//! a total deadline. What the message may say is decided in [`ingest`]. See `docs/events.md`.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Instant;

use super::{Arc, Duration, Event, IpAddr, Ordering, SUBSCRIBED, Shared, Sub, TcpListener, backoff, bump, ct_eq, lock};
use crate::wsd::adapter::wse;
use crate::wsd::adapter::{Incoming, Notification, WSE, WireError};
use crate::wsd::ns::WPRT;

const MAX_HEAD: usize = 8 * 1024;
const MAX_HEADERS: usize = 32;
const MAX_TOKEN: usize = 128;

/// A token bucket: `burst` requests at once, `per_second` more each second.
#[derive(Debug)]
pub(super) struct Bucket {
    burst: f64,
    rate: f64,
    tokens: f64,
    last: Option<Duration>,
}

impl Default for Bucket {
    fn default() -> Self {
        Self::new(50, 20)
    }
}

impl Bucket {
    pub(super) fn new(burst: u32, per_second: u32) -> Self {
        let burst = f64::from(burst.max(1));
        Self { burst, rate: f64::from(per_second), tokens: burst, last: None }
    }

    /// Take one token at `now` (the backend's clock); `false` when the bucket is empty.
    pub(super) fn take(&mut self, now: Duration) -> bool {
        if let Some(last) = self.last {
            let elapsed = now.saturating_sub(last).as_secs_f64();
            self.tokens = (self.tokens + elapsed * self.rate).min(self.burst);
        }
        self.last = Some(now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// One request as read from the wire.
#[derive(Debug)]
pub(super) struct Request {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) content_type: Option<String>,
    pub(super) body: Vec<u8>,
}

/// The accept loop: runs until the stop flag is set, polling for connections.
pub(super) fn serve(shared: &Arc<Shared>, listener: &TcpListener) {
    while !shared.stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, peer)) => connection(shared, stream, peer.ip().to_canonical()),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(15)),
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

fn known_source(shared: &Shared, peer: IpAddr) -> bool {
    lock(&shared.state).subs.iter().any(|s| s.printer.host().to_canonical() == peer)
}

fn connection(shared: &Shared, mut stream: TcpStream, peer: IpAddr) {
    if !known_source(shared, peer) {
        // Not a printer we talk to: close without reading or answering.
        bump(&shared.counters.rejected_source);
        return;
    }
    bump(&shared.counters.connections);
    let allowed = {
        let now = shared.clock.now();
        lock(&shared.state).bucket.take(now)
    };
    let _ = stream.set_nonblocking(false);
    if !allowed {
        bump(&shared.counters.rejected_rate);
        respond(&mut stream, 429);
        return;
    }
    let status = match read_request(&mut stream, shared) {
        Ok(request) => ingest(shared, peer, &request),
        Err(code) => {
            bump(&shared.counters.malformed);
            code
        }
    };
    respond(&mut stream, status);
}

fn respond(stream: &mut TcpStream, status: u16) {
    let reason = match status {
        202 => "Accepted",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        _ => "Error",
    };
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
    let _ = stream.write_all(format!("HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes());
    let _ = stream.flush();
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Read one request within the deadline and the size caps. The error is the status to answer.
fn read_request(stream: &mut TcpStream, shared: &Shared) -> Result<Request, u16> {
    // A deadline too far away to represent (an app that sets `Duration::MAX` for "no limit") is
    // no deadline at all; adding it to `Instant::now()` would panic the listener thread.
    let deadline = Instant::now().checked_add(shared.tuning.connection_deadline);
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 2048];
    let mut fill = |buf: &mut Vec<u8>| -> Result<usize, u16> {
        let left = match deadline {
            Some(deadline) => Some(deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).ok_or(408u16)?),
            None => None,
        };
        stream.set_read_timeout(left).map_err(|_| 400u16)?;
        match stream.read(&mut chunk) {
            Ok(0) => Err(400),
            Ok(n) => {
                buf.extend_from_slice(chunk.get(..n).unwrap_or(&[]));
                Ok(n)
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Err(408),
            Err(_) => Err(400),
        }
    };
    let head_end = loop {
        if let Some(at) = find(&buf, b"\r\n\r\n") {
            break at;
        }
        if buf.len() > MAX_HEAD {
            return Err(431);
        }
        fill(&mut buf)?;
    };
    if head_end > MAX_HEAD {
        return Err(431);
    }
    let head = std::str::from_utf8(buf.get(..head_end).unwrap_or(&[])).map_err(|_| 400u16)?;
    let mut lines = head.split("\r\n");
    let line = lines.next().ok_or(400u16)?;
    let mut parts = line.split(' ');
    let (Some(method), Some(path), Some(version), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(400);
    };
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err(400);
    }
    let (mut length, mut content_type, mut count) = (None::<usize>, None::<String>, 0usize);
    for header in lines {
        count += 1;
        if count > MAX_HEADERS || header.starts_with([' ', '\t']) {
            return Err(431);
        }
        let (name, value) = header.split_once(':').ok_or(400u16)?;
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "content-length" => {
                if length.is_some() || value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(400);
                }
                length = Some(value.parse().map_err(|_| 413u16)?);
            }
            "transfer-encoding" => return Err(411),
            "content-type" => content_type = Some(value.to_owned()),
            _ => {}
        }
    }
    let length = length.ok_or(411u16)?;
    if length > shared.tuning.max_body {
        return Err(413);
    }
    let mut body: Vec<u8> = buf.get(head_end + 4..).unwrap_or(&[]).to_vec();
    while body.len() < length {
        fill(&mut body)?;
    }
    body.truncate(length);
    Ok(Request { method: method.to_owned(), path: path.to_owned(), content_type, body })
}

/// `/n/<token>` or `/e/<token>` -> the token.
pub(super) fn token_of(path: &str) -> Option<&str> {
    let token = path.strip_prefix("/n/").or_else(|| path.strip_prefix("/e/"))?;
    (!token.is_empty() && token.len() <= MAX_TOKEN && token.bytes().all(|b| b.is_ascii_hexdigit())).then_some(token)
}

fn soap_content_type(value: Option<&str>) -> bool {
    let essence = value.and_then(|v| v.split(';').next()).unwrap_or("").trim().to_ascii_lowercase();
    matches!(essence.as_str(), "application/soap+xml" | "text/xml" | "application/xml")
}

/// The job number of a job event's body, if it has a well-formed one.
fn job_number(notification: &Notification) -> Option<u32> {
    notification
        .with_body::<Option<u32>, WireError>(|payload| {
            Ok(payload
                .descendants()
                .find(|d| d.is_element() && d.tag_name().name() == "JobId" && d.tag_name().namespace() == Some(WPRT))
                .and_then(|d| d.text())
                .and_then(|t| t.trim().parse().ok()))
        })
        .ok()
        .flatten()
}

/// Decide what a request may do. Returns the status to answer. Event content is read for exactly
/// one purpose: the job number, to choose whose watch is woken.
pub(super) fn ingest(shared: &Shared, peer: IpAddr, request: &Request) -> u16 {
    if request.method != "POST" {
        bump(&shared.counters.malformed);
        return 405;
    }
    let Some(token) = token_of(&request.path) else {
        bump(&shared.counters.malformed);
        return 404;
    };
    if !soap_content_type(request.content_type.as_deref()) {
        bump(&shared.counters.malformed);
        return 415;
    }
    let now = shared.clock.now();
    let quiet = shared.tuning.quiet_window;
    let mut state = lock(&shared.state);
    // Compare against every subscription, with no early exit.
    let mut found = None;
    for (index, sub) in state.subs.iter().enumerate() {
        if ct_eq(&sub.token, token) {
            found = Some(index);
        }
    }
    let Some(sub) = found.and_then(|i| state.subs.get_mut(i)) else {
        bump(&shared.counters.forged);
        return 404;
    };
    if sub.printer.host().to_canonical() != peer {
        bump(&shared.counters.forged);
        return 404;
    }
    let incoming = match wse::parse_incoming(&request.body) {
        Ok(incoming) => incoming,
        Err(_) => {
            bump(&shared.counters.malformed);
            return 400;
        }
    };
    match incoming {
        Incoming::Event(notification) => {
            if !notification.header(WSE, "Identifier").is_some_and(|id| ct_eq(id, &sub.token)) {
                bump(&shared.counters.forged);
                return 403;
            }
            let event = Event::from_action(&notification.action)
                .filter(|e| SUBSCRIBED.contains(e) && notification.body.ns == WPRT && notification.body.local == e.element());
            let Some(event) = event else {
                bump(&shared.counters.unsubscribed_type);
                return 400;
            };
            let job = if event.is_job_event() {
                let Some(job) = job_number(&notification) else {
                    bump(&shared.counters.malformed);
                    return 400;
                };
                Some(job)
            } else {
                None
            };
            // A verified event proves the path works, duplicate or not.
            sub.last_event = Some(now);
            if let Some(id) = &notification.message_id {
                if sub.seen.contains(id) {
                    bump(&shared.counters.duplicates);
                    return 202;
                }
                if sub.seen.len() >= super::MAX_SEEN {
                    sub.seen.pop_front();
                }
                sub.seen.push_back(id.clone());
            }
            sub.publish_health(now, quiet);
            wake(sub, job);
            bump(&shared.counters.wakes);
            202
        }
        Incoming::SubscriptionEnd(end) => {
            let mine = sub.handle.as_ref().and_then(|h| h.subscription.identifier());
            if let (Some(theirs), Some(mine)) = (end.identifier(), mine)
                && !ct_eq(theirs, mine)
            {
                bump(&shared.counters.forged);
                return 403;
            }
            sub.handle = None;
            sub.degraded = true;
            sub.failures = sub.failures.saturating_add(1);
            sub.next_attempt = now.saturating_add(backoff(sub.failures, shared.tuning));
            sub.publish_health(now, quiet);
            // Every watch on the printer polls now and carries on at the ordinary cadence.
            wake(sub, None);
            bump(&shared.counters.ended);
            202
        }
    }
}

fn wake(sub: &Sub, job: Option<u32>) {
    for rx in &sub.receivers {
        if job.is_none_or(|j| j == rx.job) {
            rx.tx.wake();
        }
    }
}
