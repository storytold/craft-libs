//! `ControlChannel` deadlines and the loopback server's limits.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::mpsc::channel;
use std::time::Duration;

use craft_automation::control::{ControlChannel, ControlRequest, expired_error, timeout_error};
use craft_automation::security::MAX_REQUEST_BYTES;
use craft_automation::server;
use craft_ui::runtime::motion;
use craft_ui::runtime::windows::WindowHost;
use craft_ui::testing::trial;
use egui::Context;
use serde_json::{Value, json};

/// An expired queued request is replied `expired_error` and never dispatched; a dispatched one that times out gets
/// `timeout_error`.
#[trial]
fn deadlines_distinguish_expired_from_timed_out() {
    let (tx, rx) = channel();
    let mut channel = ControlChannel::new(rx);

    let (mut stale, stale_reply) = ControlRequest::new("ui.inspect", json!({}));
    stale.deadline = Some(motion::now() - Duration::from_secs(1));
    let stale_dispatch = stale.dispatch.clone();
    let (live, _live_reply) = ControlRequest::new("ui.key", json!({}));
    let live_dispatch = live.dispatch.clone();
    tx.send(stale).expect("sent");
    tx.send(live).expect("sent");

    let next = channel.next_request().expect("the live request is dispatched");
    assert_eq!(next.method, "ui.key");
    assert!(channel.next_request().is_none());
    assert_eq!(stale_reply.try_recv().expect("a reply"), json!({"ok": false, "error": expired_error()}));
    assert_eq!(stale_dispatch.give_up(), expired_error());
    assert_eq!(live_dispatch.give_up(), timeout_error());
    assert!(timeout_error().starts_with("timeout:") && expired_error().starts_with("timeout:"));

    // A request the transport abandoned before the UI thread claimed it is not run.
    let (abandoned, abandoned_reply) = ControlRequest::new("ui.click", json!({}));
    let _ = abandoned.dispatch.give_up();
    tx.send(abandoned).expect("sent");
    assert!(channel.next_request().is_none());
    assert_eq!(abandoned_reply.try_recv().expect("a reply")["error"], json!(expired_error()));
}

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn read_json(reader: &mut impl BufRead) -> Option<Value> {
    let mut line = String::new();
    (reader.read_line(&mut line).ok()? > 0).then(|| serde_json::from_str(&line).expect("a JSON line"))
}

fn host() -> WindowHost {
    WindowHost::of(&Context::default())
}

/// A connection that keeps sending without authenticating is closed at the deadline, whatever it sends, and nothing it
/// sent is dispatched.
#[trial]
fn an_unauthenticated_connection_is_closed_at_the_deadline_whatever_it_sends() {
    // A short deadline, so the test does not wait out the real one.
    let deadline = Duration::from_millis(400);
    let (rx, addr) = server::start_with_deadline(0, TOKEN.into(), host(), deadline).expect("started");
    // Blank lines and partial frames, one byte every 20 ms: each read succeeds long before the idle timeout.
    let mut stream = TcpStream::connect(addr).expect("connected");
    let started = motion::now();
    let mut closed = false;
    for i in 0..1000 {
        let byte: &[u8] = if i % 5 == 0 { b"\n" } else { b" " };
        if stream.write_all(byte).is_err() {
            closed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let served = started.elapsed();
    assert!(closed, "the trickle did not keep the connection open");
    assert!(served < deadline + Duration::from_secs(3), "the trickle did not extend the deadline: {served:?}");
    assert!(rx.try_recv().is_err(), "nothing was dispatched");
}

/// A request line over `MAX_REQUEST_BYTES` without a newline gets an error reply and the connection is closed.
#[trial]
fn an_over_long_request_is_refused_and_closes_the_connection() {
    let (rx, addr) = server::start(0, TOKEN.into(), host()).expect("started");
    let mut stream = TcpStream::connect(addr).expect("connected");
    stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
    let mut reader = BufReader::new(stream.try_clone().expect("cloned"));
    writeln!(stream, "{}", json!({"id": 1, "method": "auth", "params": {"token": TOKEN}})).expect("written");
    assert_eq!(read_json(&mut reader).expect("a reply")["ok"], true);
    // The server may close while this is still being written.
    let _ = stream.write_all(&vec![b'x'; MAX_REQUEST_BYTES + 16]);
    let reply = read_json(&mut reader).expect("a reply");
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].as_str().expect("an error").starts_with("request exceeds"), "{reply}");
    assert!(read_json(&mut reader).is_none(), "the connection is closed");
    assert!(rx.try_recv().is_err());
}
