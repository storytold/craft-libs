//! Local socket accept loop, auth, and the per-connection threads.
//!
//! The 17th connection receives one refusal line and does not take a slot.
//! Bulk connections count toward the same cap.

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use interprocess::TryClone;
#[cfg(unix)]
use interprocess::local_socket::GenericFilePath;
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{Listener, ListenerNonblockingMode, ListenerOptions, Stream};
use serde_json::Value;

use super::dispatch::{self, Outcome};
use super::events::{self, Subscription};
use super::tcp::BoundTcp;
use super::{Hub, lock};
use crate::discovery::{Endpoint, RuntimeDir};
use crate::error::LinkError;
use crate::wire::{self, LineReader, MAX_LINE};

const READ_POLL: Duration = Duration::from_millis(250);
const REFUSE_WRITE: Duration = Duration::from_secs(2);

pub(super) fn bind(runtime: &RuntimeDir, app: &str, pid: u32) -> Result<Listener, LinkError> {
    #[cfg(unix)]
    {
        bind_unix(runtime, app, pid)
    }
    #[cfg(windows)]
    {
        let _ = runtime;
        bind_pipe(app, pid)
    }
}

pub(super) fn endpoint(runtime: &RuntimeDir, app: &str, pid: u32) -> Result<Endpoint, LinkError> {
    #[cfg(unix)]
    {
        runtime.unix_endpoint(app, pid)
    }
    #[cfg(windows)]
    {
        let _ = runtime;
        Ok(Endpoint::Pipe { name: pipe_name(app, pid) })
    }
}

#[cfg(unix)]
fn bind_unix(runtime: &RuntimeDir, app: &str, pid: u32) -> Result<Listener, LinkError> {
    let path = runtime.socket_file(app, pid);
    let name = path.to_fs_name::<GenericFilePath>()?;
    let listener = ListenerOptions::new().name(name).nonblocking(ListenerNonblockingMode::Accept).reclaim_name(false).create_sync()?;
    Ok(listener)
}

/// Owner and SYSTEM only. A peer pid is not an allow; Windows `PeerCreds` has no user id.
#[cfg(windows)]
fn bind_pipe(app: &str, pid: u32) -> Result<Listener, LinkError> {
    use interprocess::local_socket::GenericNamespaced;
    use interprocess::os::windows::local_socket::ListenerOptionsExt;
    use interprocess::os::windows::security_descriptor::SecurityDescriptor;
    use widestring::U16CString;

    let wide = U16CString::from_str("D:P(A;;GA;;;OW)(A;;GA;;;SY)").map_err(LinkError::io)?;
    let descriptor = SecurityDescriptor::deserialize(wide.as_ucstr())?;
    let name = pipe_name(app, pid).to_ns_name::<GenericNamespaced>()?;
    let listener = ListenerOptions::new()
        .name(name)
        .nonblocking(ListenerNonblockingMode::Accept)
        .reclaim_name(false)
        .security_descriptor(descriptor)
        .create_sync()?;
    Ok(listener)
}

#[cfg(windows)]
fn pipe_name(app: &str, pid: u32) -> String {
    match windows_user() {
        Some(user) => format!("craft-{user}-{app}-{pid}"),
        None => format!("craft-{app}-{pid}"),
    }
}

/// `USERNAME` with non-alphanumeric bytes replaced. Reading the env is enough;
/// there is no dependency-free Windows user API in this crate.
#[cfg(windows)]
fn windows_user() -> Option<String> {
    let raw = std::env::var("USERNAME").ok()?;
    if raw.is_empty() {
        return None;
    }
    let mut cleaned = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            cleaned.push(ch);
        } else {
            cleaned.push('_');
        }
    }
    let trimmed = cleaned.trim_matches('_');
    if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
}

pub(super) fn accept_loop(listener: Listener, tcp: Option<BoundTcp>, hub: Arc<Hub>) {
    while !hub.stop.load(Ordering::SeqCst) {
        reap(&hub);
        let mut busy = false;
        match listener.accept() {
            Ok(stream) => {
                busy = true;
                accept_local(stream, &hub);
            }
            Err(err) if idle_accept(&err) => {}
            Err(_) => {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
        }
        if let Some(bound) = tcp.as_ref()
            && let Ok(Some(stream)) = super::tcp::poll_accept(&bound.listener)
        {
            busy = true;
            accept_tcp(stream, &hub);
        }
        if !busy {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn idle_accept(err: &std::io::Error) -> bool {
    matches!(err.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted)
}

fn reap(hub: &Hub) {
    let mut threads = lock(&hub.threads);
    let pending = std::mem::take(&mut *threads);
    for handle in pending {
        if handle.is_finished() {
            let _ = handle.join();
        } else {
            threads.push(handle);
        }
    }
}

/// One connection slot. A connection that has to authenticate also holds a share of the small
/// unauthenticated pool until it does. The slots outside that pool are only for peers that proved
/// the token, so silent peers cannot fill the server.
struct Slot {
    hub: Arc<Hub>,
    pending: bool,
}

/// Why [`Slot::admit`] said no.
enum Refusal {
    Full,
    Unauthenticated,
}

impl Refusal {
    fn text(&self) -> &'static str {
        match self {
            Self::Full => "too many control connections",
            Self::Unauthenticated => "too many unauthenticated connections",
        }
    }
}

/// Add one to `counter` unless it is at `cap`.
fn take_one(counter: &AtomicUsize, cap: usize) -> bool {
    loop {
        let cur = counter.load(Ordering::SeqCst);
        if cur >= cap {
            return false;
        }
        if counter.compare_exchange(cur, cur + 1, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            return true;
        }
    }
}

impl Slot {
    fn admit(hub: &Arc<Hub>, needs_auth: bool) -> Result<Self, Refusal> {
        if !take_one(&hub.open, wire::MAX_CONNECTIONS) {
            return Err(Refusal::Full);
        }
        if needs_auth && !take_one(&hub.pending, hub.limits.max_unauthenticated.min(wire::MAX_CONNECTIONS)) {
            hub.open.fetch_sub(1, Ordering::SeqCst);
            return Err(Refusal::Unauthenticated);
        }
        Ok(Slot { hub: Arc::clone(hub), pending: needs_auth })
    }

    /// The peer sent the right token. It leaves the unauthenticated pool and keeps its slot.
    fn authenticated(&mut self) {
        if self.pending {
            self.pending = false;
            self.hub.pending.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.authenticated();
        self.hub.open.fetch_sub(1, Ordering::SeqCst);
    }
}

fn accept_local(mut stream: Stream, hub: &Arc<Hub>) {
    let _ = stream.set_send_timeout(Some(REFUSE_WRITE));
    let slot = match Slot::admit(hub, true) {
        Ok(slot) => slot,
        Err(refusal) => {
            refuse(&mut stream, &refusal);
            return;
        }
    };
    if crate::ensure_peer_user(&stream).is_err() {
        let _ = wire::write_line(&mut stream, &wire::auth_failure());
        drop(slot);
        return;
    }
    let _ = stream.set_recv_timeout(Some(READ_POLL));
    let Ok(writer_stream) = stream.try_clone() else {
        return;
    };
    let reader: Box<dyn Read + Send> = Box::new(stream);
    let writer: Box<dyn Write + Send> = Box::new(writer_stream);
    spawn(slot, Arc::clone(hub), BufReader::new(reader), Arc::new(std::sync::Mutex::new(writer)), true);
}

fn accept_tcp(mut stream: std::net::TcpStream, hub: &Arc<Hub>) {
    let _ = stream.set_write_timeout(Some(REFUSE_WRITE));
    let slot = match Slot::admit(hub, true) {
        Ok(slot) => slot,
        Err(refusal) => {
            refuse(&mut stream, &refusal);
            return;
        }
    };
    // The listener is bound to 127.0.0.1. Refuse anything else anyway.
    if !stream.peer_addr().is_ok_and(|addr| addr.ip().is_loopback()) {
        let _ = wire::write_line(&mut stream, &wire::auth_failure());
        return;
    }
    let _ = stream.set_read_timeout(Some(READ_POLL));
    let Ok(writer_stream) = stream.try_clone() else {
        return;
    };
    let reader: Box<dyn Read + Send> = Box::new(stream);
    let writer: Box<dyn Write + Send> = Box::new(writer_stream);
    spawn(slot, Arc::clone(hub), BufReader::new(reader), Arc::new(std::sync::Mutex::new(writer)), true);
}

fn refuse(stream: &mut impl Write, refusal: &Refusal) {
    let line = serde_json::json!({"ok": false, "error": refusal.text()});
    let _ = wire::write_line(stream, &line);
}

fn spawn(slot: Slot, hub: Arc<Hub>, reader: BufReader<Box<dyn Read + Send>>, writer: Arc<std::sync::Mutex<Box<dyn Write + Send>>>, auth: bool) {
    let thread_hub = Arc::clone(&hub);
    let started = std::thread::Builder::new().name("craft-ipc-conn".into()).spawn(move || {
        let mut slot = slot;
        connection(thread_hub, reader, writer, auth, &mut slot);
    });
    if let Ok(handle) = started {
        lock(&hub.threads).push(handle);
    }
}

struct Cleanup {
    hub: Arc<Hub>,
    conn_id: u64,
    sub: Arc<Subscription>,
    writer: Option<JoinHandle<()>>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        events::close_sub(&self.sub);
        if let Some(handle) = self.writer.take() {
            let _ = handle.join();
        }
        events::remove_sub(&self.hub, self.conn_id);
        dispatch::release_tickets(&self.hub, self.conn_id);
    }
}

fn connection(
    hub: Arc<Hub>,
    mut reader: BufReader<Box<dyn Read + Send>>,
    writer: Arc<std::sync::Mutex<Box<dyn Write + Send>>>,
    auth: bool,
    slot: &mut Slot,
) {
    let first_patience = if auth { hub.limits.partial_line.min(hub.limits.auth_deadline) } else { hub.limits.partial_line };
    let mut lines = LineReader::new(MAX_LINE).with_patience(first_patience);
    if auth && authenticate(&hub, &mut reader, &mut lines, &writer).is_err() {
        return;
    }
    slot.authenticated();
    lines.set_patience(hub.limits.partial_line);
    let conn_id = hub.next_id.fetch_add(1, Ordering::SeqCst);
    let sub = Subscription::new(conn_id);
    lock(&hub.subs).push(Arc::clone(&sub));
    let write_sub = Arc::clone(&sub);
    let write_io = Arc::clone(&writer);
    let write_stop = Arc::clone(&hub);
    let writer_thread = std::thread::Builder::new().name("craft-ipc-write".into()).spawn(move || {
        events::write_loop(write_sub, write_io, &write_stop.stop);
    });
    let Ok(writer_thread) = writer_thread else {
        events::remove_sub(&hub, conn_id);
        return;
    };
    let mut cleanup = Cleanup { hub: Arc::clone(&hub), conn_id, sub: Arc::clone(&sub), writer: Some(writer_thread) };
    let mut first = true;
    let mut last_input = Instant::now();
    loop {
        if hub.stop.load(Ordering::SeqCst) {
            break;
        }
        let line = match next_line(&hub, &mut reader, &mut lines) {
            Incoming::Again => {
                if lines.partial_age().is_some_and(|age| age >= hub.limits.partial_line) {
                    let _ = write_value(&writer, &wire::reply_err_text(&Value::Null, "request line not finished in time; closing the connection"));
                    break;
                }
                if lines.partial_age().is_none() && last_input.elapsed() >= hub.limits.idle && !events::is_subscribed(&sub) {
                    let _ = write_value(&writer, &wire::reply_err_text(&Value::Null, "idle connection closed"));
                    break;
                }
                continue;
            }
            Incoming::Stop => break,
            Incoming::TooLarge => {
                let _ = write_value(&writer, &wire::reply_err_text(&Value::Null, &overlong()));
                break;
            }
            Incoming::Line(bytes) => bytes,
        };
        last_input = Instant::now();
        let req = match wire::parse_request(&line) {
            Ok(None) => continue,
            Ok(Some(req)) => req,
            Err(text) => {
                let _ = write_value(&writer, &wire::reply_err_text(&Value::Null, &text));
                break;
            }
        };
        let outcome = dispatch::dispatch(&hub, conn_id, &sub, first, &req);
        first = false;
        match outcome {
            Outcome::Continue(reply) => {
                if write_value(&writer, &reply).is_err() {
                    break;
                }
            }
            Outcome::Close(reply) => {
                if let Some(reply) = reply {
                    let _ = write_value(&writer, &reply);
                }
                break;
            }
            Outcome::Bulk { reply, ticket } => {
                events::close_sub(&sub);
                if let Some(handle) = cleanup.writer.take() {
                    let _ = handle.join();
                }
                if write_value(&writer, &reply).is_err() {
                    break;
                }
                bulk_loop(&mut reader, &writer, &hub, &ticket);
                break;
            }
        }
    }
    drop(cleanup);
}

fn authenticate(
    hub: &Hub,
    reader: &mut BufReader<Box<dyn Read + Send>>,
    lines: &mut LineReader,
    writer: &Arc<std::sync::Mutex<Box<dyn Write + Send>>>,
) -> Result<(), ()> {
    let deadline = Instant::now() + hub.limits.auth_deadline;
    loop {
        if hub.stop.load(Ordering::SeqCst) {
            return Err(());
        }
        let line = match next_line(hub, reader, lines) {
            Incoming::Again if Instant::now() >= deadline => {
                let _ = write_value(writer, &wire::auth_failure());
                return Err(());
            }
            Incoming::Again => continue,
            Incoming::Stop | Incoming::TooLarge => {
                let _ = write_value(writer, &wire::auth_failure());
                return Err(());
            }
            Incoming::Line(bytes) => bytes,
        };
        match wire::parse_request(&line) {
            Ok(None) if Instant::now() < deadline => continue,
            Ok(None) => {
                let _ = write_value(writer, &wire::auth_failure());
                return Err(());
            }
            Ok(Some(req)) if req.method == "auth" && token_ok(&req.params, &hub.record.token) => {
                return write_value(writer, &wire::auth_success(&req.id)).map_err(|_| ());
            }
            Ok(Some(_)) | Err(_) => {
                let _ = write_value(writer, &wire::auth_failure());
                return Err(());
            }
        }
    }
}

fn token_ok(params: &Value, expected: &str) -> bool {
    params.get("token").and_then(Value::as_str).is_some_and(|token| crate::tokens_equal(expected, token))
}

enum Incoming {
    Line(Vec<u8>),
    Again,
    TooLarge,
    Stop,
}

fn next_line(hub: &Hub, reader: &mut BufReader<Box<dyn Read + Send>>, lines: &mut LineReader) -> Incoming {
    match lines.read(reader) {
        Ok(Some(bytes)) => Incoming::Line(bytes),
        Ok(None) => Incoming::Stop,
        Err(LinkError::TooLarge) => Incoming::TooLarge,
        Err(err) if is_timeout(&err) => {
            if hub.stop.load(Ordering::SeqCst) {
                Incoming::Stop
            } else {
                Incoming::Again
            }
        }
        Err(_) => Incoming::Stop,
    }
}

fn is_timeout(err: &LinkError) -> bool {
    if matches!(err, LinkError::Timeout) {
        return true;
    }
    let LinkError::Io(msg) = err else {
        return false;
    };
    let lower = msg.to_ascii_lowercase();
    lower.contains("timed out") || lower.contains("timeout")
}

fn overlong() -> String {
    format!("request line longer than {MAX_LINE} bytes; closing the connection")
}

fn write_value(writer: &Arc<std::sync::Mutex<Box<dyn Write + Send>>>, value: &Value) -> Result<(), LinkError> {
    let mut guard = lock(writer);
    wire::write_line(&mut *guard, value)
}

/// Retries reads that time out until `deadline`, so a record split across the read poll is whole.
struct Patient<'a, R> {
    inner: &'a mut R,
    deadline: Instant,
    stop: &'a std::sync::atomic::AtomicBool,
}

impl<R: Read> Read for Patient<'_, R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        loop {
            match self.inner.read(out) {
                Err(err) if idle_accept(&err) && Instant::now() < self.deadline && !self.stop.load(Ordering::SeqCst) => {}
                other => return other,
            }
        }
    }
}

fn bulk_loop(reader: &mut BufReader<Box<dyn Read + Send>>, writer: &Arc<std::sync::Mutex<Box<dyn Write + Send>>>, hub: &Hub, ticket: &str) {
    loop {
        if hub.stop.load(Ordering::SeqCst) {
            return;
        }
        // Wait for the first byte of a request. A timeout here loses nothing.
        match reader.fill_buf() {
            Ok([]) => return,
            Ok(_) => {}
            Err(err) if idle_accept(&err) => continue,
            Err(_) => return,
        }
        // The record has started. A pause inside it is not a new request, so keep
        // reading until it ends, the server stops, or the peer takes too long.
        let mut patient = Patient { inner: &mut *reader, deadline: Instant::now() + hub.limits.partial_line, stop: &hub.stop };
        let header = match wire::read_cfrq(&mut patient) {
            Ok(header) => header,
            Err(_) => return,
        };
        match hub.frames.render(ticket, &header) {
            Ok((hdr, payload)) => {
                if send_frame(writer, &header, &hdr, &payload).is_err() {
                    return;
                }
            }
            Err(err) => {
                let fail = error_header(&header, &err.to_string());
                if write_frame(writer, &fail, &[]).is_err() {
                    return;
                }
            }
        }
    }
}

fn send_frame(writer: &Arc<std::sync::Mutex<Box<dyn Write + Send>>>, request: &Value, header: &Value, payload: &[u8]) -> Result<(), LinkError> {
    match write_frame(writer, header, payload) {
        Ok(()) => Ok(()),
        Err(err) => {
            let fail = error_header(request, &err.to_string());
            write_frame(writer, &fail, &[])?;
            Err(err)
        }
    }
}

fn error_header(request: &Value, message: &str) -> Value {
    let req = request.get("req").cloned().unwrap_or(Value::Null);
    serde_json::json!({"req": req, "error": message})
}

fn write_frame(writer: &Arc<std::sync::Mutex<Box<dyn Write + Send>>>, header: &Value, payload: &[u8]) -> Result<(), LinkError> {
    let mut guard = lock(writer);
    wire::write_cfrm(&mut *guard, header, payload)
}
