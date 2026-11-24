//! The loopback JSON-lines TCP transport: one request per line, one reply per line.

use std::io::{BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use craft_ui::runtime::motion;
use craft_ui::runtime::windows::WindowHost;
use egui::ViewportId;
use serde_json::{Value, json};

use crate::control::{ControlRequest, REPLY_DEADLINE};
use crate::protocol::write_reply;
use crate::security::{
    AUTH_DEADLINE, AuthDeadlineStream, AuthStep, ConnectionLimiter, LineRead, MAX_CONNECTIONS, MAX_REQUEST_BYTES, ServerHandshake, configure_stream,
    read_bounded_line, repeat_auth_reply,
};

/// A running control server. Dropping it stops accepting connections and returns once the port is
/// free, so a restart on the same port binds it again at once; connections already open end at their
/// next request, once the app has dropped the request channel.
#[non_exhaustive]
pub struct Listening {
    /// The bound address (with port 0, the port the system picked); the one a client needs.
    pub addr: SocketAddr,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl Drop for Listening {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // The accept loop blocks in `accept`: a connection wakes it to see the flag and close the
        // listener. Without that wake-up, joining would block, so it is skipped.
        let woke = TcpStream::connect_timeout(&self.addr, Duration::from_millis(500)).is_ok();
        if woke && let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
    }
}

impl std::fmt::Debug for Listening {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listening").field("addr", &self.addr).finish_non_exhaustive()
    }
}

/// Binds `127.0.0.1:port` (0 lets the OS pick) and [`serve`]s it.
pub fn listen(port: u16, token: String, host: WindowHost) -> std::io::Result<(Receiver<ControlRequest>, Listening)> {
    serve(TcpListener::bind(("127.0.0.1", port))?, token, host)
}

/// Serves a listener bound earlier (an app's `--control`, bound before its window opens) until the
/// returned [`Listening`] is dropped: authenticates each connection with `token`, forwards its
/// requests to the receiver and wakes the root window through `host`, which keeps each window's access tree
/// ([`WindowHost::keep_access_tree`]) for [`crate::access`]. The app prints
/// `control server listening on <addr>`.
pub fn serve(listener: TcpListener, token: String, host: WindowHost) -> std::io::Result<(Receiver<ControlRequest>, Listening)> {
    serve_with_deadline(listener, token, host, AUTH_DEADLINE)
}

/// [`serve`] with `auth_deadline` in place of [`AUTH_DEADLINE`], so a test sees a connection closed at the deadline
/// without waiting the real one.
#[doc(hidden)]
pub fn serve_with_deadline(
    listener: TcpListener,
    token: String,
    host: WindowHost,
    auth_deadline: Duration,
) -> std::io::Result<(Receiver<ControlRequest>, Listening)> {
    let (tx, rx) = channel::<ControlRequest>();
    let addr = listener.local_addr()?;
    // `ui.inspect`, `ui.click {label}` and `ui.focus` read the kept tree of the last presented frame; it is kept
    // from before the first connection, so the first request already finds it.
    host.keep_access_tree();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = Arc::clone(&stop);
    let accept = std::thread::Builder::new().name("control server".into()).spawn(move || {
        let limiter = ConnectionLimiter::new(MAX_CONNECTIONS);
        let token = Arc::new(token);
        let closer = refused_closer();
        for mut stream in listener.incoming().flatten() {
            if stopped.load(Ordering::Acquire) {
                break;
            }
            let Some(permit) = limiter.try_acquire() else {
                let _ = configure_stream(&stream);
                let _ = writeln!(stream, "{}", json!({"id": null, "ok": false, "error": "connection limit reached"}));
                // The end goes out now; the rest of the close (reading what the client still sends) happens on
                // the closer thread, so the accept loop never waits. With that thread's queue full, at once.
                let _ = stream.shutdown(Shutdown::Write);
                if let Some(closer) = &closer {
                    let _ = closer.try_send(stream);
                }
                continue;
            };
            let tx = tx.clone();
            let host = host.clone();
            let token = Arc::clone(&token);
            std::thread::spawn(move || {
                serve_connection(stream, &token, &tx, &host, auth_deadline);
                // What the last client held down would otherwise stay held with no one left to
                // release it; while another connection is live, its input is left alone.
                if permit.release() {
                    host.end_injection();
                }
            });
        }
    })?;
    Ok((rx, Listening { addr, stop, accept: Some(accept) }))
}

/// Serve one connection: it must authenticate ([`ServerHandshake`]) within `auth_deadline` of
/// connecting, however much it sends meanwhile; after that only the idle timeout applies.
fn serve_connection(stream: TcpStream, token: &str, tx: &Sender<ControlRequest>, host: &WindowHost, auth_deadline: Duration) {
    if configure_stream(&stream).is_err() {
        return;
    }
    let Ok(read) = stream.try_clone() else { return };
    let mut reader = BufReader::new(AuthDeadlineStream::new(read, auth_deadline));
    let mut out = stream;
    let mut line = String::new();
    let mut handshake = ServerHandshake::new();
    let mut authenticated = false;
    loop {
        match read_bounded_line(&mut reader, &mut line) {
            Ok(LineRead::Eof) | Err(_) => break,
            Ok(LineRead::TooLong) => {
                let reply = json!({
                    "id": null,
                    "ok": false,
                    "error": format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
                });
                let _ = write_reply(&mut out, &reply);
                let _ = out.flush();
                close_after_refusal(&out);
                break;
            }
            Ok(LineRead::Line) if line.trim().is_empty() => continue,
            Ok(LineRead::Line) => {}
        }
        if !authenticated {
            let (reply, step) = handshake.reply(&line, token);
            authenticated = step == AuthStep::Authenticated;
            if authenticated && reader.get_mut().authenticated().is_err() {
                break;
            }
            if write_reply(&mut out, &reply).is_err() || out.flush().is_err() {
                break;
            }
            if step == AuthStep::Rejected {
                close_after_refusal(&out);
                break;
            }
            continue;
        }
        if let Some(reply) = repeat_auth_reply(&line) {
            if write_reply(&mut out, &reply).is_err() || out.flush().is_err() {
                break;
            }
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => {
                let id = msg.get("id").cloned().unwrap_or(Value::Null);
                let method = msg.get("method").and_then(Value::as_str).unwrap_or("").to_string();
                let params = msg.get("params").cloned().unwrap_or(json!({}));
                let deadline = motion::now() + REPLY_DEADLINE;
                let (mut req, rrx) = ControlRequest::new(method, params);
                req.deadline = Some(deadline);
                let dispatch = req.dispatch.clone();
                if tx.send(req).is_err() {
                    break;
                }
                host.repaint_in(ViewportId::ROOT, -1.0);
                let mut r = rrx
                    .recv_timeout(deadline.saturating_duration_since(motion::now()))
                    .unwrap_or_else(|_| json!({"ok": false, "error": dispatch.give_up()}));
                if let Some(o) = r.as_object_mut() {
                    o.insert("id".into(), id);
                }
                r
            }
            Err(e) => json!({"id": null, "ok": false, "error": format!("bad JSON: {e}")}),
        };
        if write_reply(&mut out, &reply).is_err() || out.flush().is_err() {
            break;
        }
    }
}

/// How long a refused connection is read and thrown away before it is closed.
const REFUSAL_DRAIN: Duration = Duration::from_secs(1);
/// The most a refused connection is read before it is closed regardless.
const REFUSAL_DRAIN_BYTES: usize = 64 * 1024;

/// End a connection whose refusal was just written so the client can still read it. Dropping a socket with unread
/// input makes the system reset the connection, and a reset can discard the refusal before the client reads it: a
/// client that wrote its request before reading the answer would see the connection reset instead. So the write side
/// is shut first (the client reads the refusal and then the end), and what the client still sends is read and thrown
/// away for at most [`REFUSAL_DRAIN`] and [`REFUSAL_DRAIN_BYTES`]. A client that keeps sending past those bounds,
/// or a refusal at the limit while the closer thread's queue is full, still ends with a reset; that is the bound.
fn close_after_refusal(stream: &TcpStream) {
    let _ = stream.shutdown(Shutdown::Write);
    let started = Instant::now();
    let mut buf = [0u8; 4096];
    let mut read = 0;
    let mut reader = stream;
    while read < REFUSAL_DRAIN_BYTES {
        let Some(left) = REFUSAL_DRAIN.checked_sub(started.elapsed()).filter(|left| !left.is_zero()) else { break };
        if reader.set_read_timeout(Some(left)).is_err() {
            break;
        }
        let want = buf.len().min(REFUSAL_DRAIN_BYTES - read);
        match reader.read(buf.get_mut(..want).unwrap_or_default()) {
            Ok(0) | Err(_) => break,
            Ok(n) => read += n,
        }
    }
}

/// A thread that closes connections refused at the limit with [`close_after_refusal`], one at a time, so the accept
/// loop never waits on one. Its queue holds [`MAX_CONNECTIONS`]; `None` when the thread can't be started.
fn refused_closer() -> Option<SyncSender<TcpStream>> {
    let (send, receive) = sync_channel::<TcpStream>(MAX_CONNECTIONS);
    std::thread::Builder::new()
        .name("control server closer".into())
        .spawn(move || {
            for stream in receive {
                close_after_refusal(&stream);
            }
        })
        .ok()?;
    Some(send)
}
