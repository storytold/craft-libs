//! The loopback JSON-lines TCP transport: one request per line, one reply per line.

use std::io::{BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use craft_ui::runtime::motion;
use craft_ui::runtime::windows::WindowHost;
use egui::ViewportId;
use serde_json::{Value, json};

use crate::control::{ControlRequest, REPLY_DEADLINE};
use crate::protocol::write_reply;
use crate::security::{
    AUTH_DEADLINE, AuthDeadlineStream, ConnectionLimiter, LineRead, MAX_CONNECTIONS, MAX_REQUEST_BYTES, authentication_reply, configure_stream,
    read_bounded_line, repeat_auth_reply,
};

/// Bind `127.0.0.1:port` (0 lets the OS pick; the returned address is the one a client needs),
/// authenticate each connection with `token`, forward requests and wake the root window through
/// `host`. The app prints
/// `control server listening on <addr>`.
pub fn start(port: u16, token: String, host: WindowHost) -> std::io::Result<(Receiver<ControlRequest>, SocketAddr)> {
    start_with_deadline(port, token, host, AUTH_DEADLINE)
}

/// [`start`] with `auth_deadline` in place of [`AUTH_DEADLINE`], so a test sees a connection closed at the deadline
/// without waiting the real one.
#[doc(hidden)]
pub fn start_with_deadline(
    port: u16,
    token: String,
    host: WindowHost,
    auth_deadline: Duration,
) -> std::io::Result<(Receiver<ControlRequest>, SocketAddr)> {
    let (tx, rx) = channel::<ControlRequest>();
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let addr = listener.local_addr()?;
    std::thread::spawn(move || {
        let limiter = ConnectionLimiter::new(MAX_CONNECTIONS);
        let token = Arc::new(token);
        for mut stream in listener.incoming().flatten() {
            let Some(permit) = limiter.try_acquire() else {
                let _ = configure_stream(&stream);
                let _ = writeln!(stream, "{}", json!({"id": null, "ok": false, "error": "connection limit reached"}));
                continue;
            };
            let tx = tx.clone();
            let host = host.clone();
            let token = Arc::clone(&token);
            std::thread::spawn(move || {
                let _permit = permit;
                serve(stream, &token, &tx, &host, auth_deadline);
            });
        }
    });
    Ok((rx, addr))
}

/// Serve one connection: it must complete `auth` within `auth_deadline` of connecting, however
/// much it sends meanwhile; after that only the idle timeout applies.
fn serve(stream: TcpStream, token: &str, tx: &Sender<ControlRequest>, host: &WindowHost, auth_deadline: Duration) {
    if configure_stream(&stream).is_err() {
        return;
    }
    let Ok(read) = stream.try_clone() else { return };
    let mut reader = BufReader::new(AuthDeadlineStream::new(read, auth_deadline));
    let mut out = stream;
    let mut line = String::new();
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
                break;
            }
            Ok(LineRead::Line) if line.trim().is_empty() => continue,
            Ok(LineRead::Line) => {}
        }
        if !authenticated {
            let (reply, ok) = authentication_reply(&line, token);
            authenticated = ok;
            if ok && reader.get_mut().authenticated().is_err() {
                break;
            }
            if write_reply(&mut out, &reply).is_err() || out.flush().is_err() || !authenticated {
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
