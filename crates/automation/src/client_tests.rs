//! Behaviour tests of `client`.
// Unwrapping, expecting and panicking on failure is fine in tests, unlike in shipped code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_ui::testing::trial;
use serde_json::json;

#[allow(clippy::wildcard_imports, reason = "tests use most of the module under test and its helpers")]
use super::*;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Writes `reply` and its newline in one write. `writeln!` on a bare stream writes in pieces, so the client can see
/// part of a line and close the connection before the rest is sent.
fn say(out: &mut impl std::io::Write, reply: serde_json::Value) -> std::io::Result<()> {
    out.write_all(format!("{reply}\n").as_bytes())
}

// call returns Ok(result) for ok replies and Err(error) otherwise.

// A stale reply left by a timed-out call is skipped, and a transport failure closes the client.
#[trial]
fn replies_are_matched_by_id() {
    use std::io::BufRead;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut out = stream;
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        say(&mut out, json!({"id": 1, "ok": true})).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        say(&mut out, json!({"id": 99, "ok": true, "result": "stale"})).unwrap();
        say(&mut out, json!({"id": 2, "ok": true, "result": "fresh"})).unwrap();
        // Closing the connection ends the third call.
    });
    let mut client = ControlClient::connect(addr, TOKEN).unwrap();
    assert_eq!(client.call("a", json!({})), Ok(json!("fresh")));
    server.join().unwrap();
    assert!(client.call("b", json!({})).is_err(), "the closed connection fails the call");
    assert_eq!(client.call("c", json!({})), Err("connection closed".to_string()), "a failed client stays closed");
}

// A cancelled wait gives up on a reply that never comes and closes the connection, so a late reply can't answer a
// later call.
#[trial]
fn a_cancelled_call_stops_waiting_and_closes_the_connection() {
    use std::io::BufRead;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (done, wait) = std::sync::mpsc::channel::<()>();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut out = stream;
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        say(&mut out, json!({"id": 1, "ok": true})).unwrap();
        // The call is read and never answered while the client waits.
        line.clear();
        reader.read_line(&mut line).unwrap();
        wait.recv().unwrap();
        say(&mut out, json!({"id": 2, "ok": true, "result": "late"})).ok();
    });
    let mut client = ControlClient::connect(addr, TOKEN).unwrap();
    let asked = std::cell::Cell::new(0);
    let cancelled = || {
        asked.set(asked.get() + 1);
        asked.get() > 1
    };
    assert_eq!(client.call_until("slow", json!({}), &cancelled), Err("cancelled".to_string()));
    assert!(client.is_broken(), "a cancelled call closes the connection");
    done.send(()).unwrap();
    server.join().unwrap();
    assert_eq!(client.call("next", json!({})), Err("connection closed".to_string()), "the late reply answers nothing");
}

// A connection the server closed, or one holding a line no call asked for, is unusable: the next
// call sends nothing and fails, so the caller reconnects instead of reading the wrong answer.
#[trial]
fn an_unasked_line_makes_the_connection_unusable_before_sending() {
    use std::io::{BufRead, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (said, wait) = std::sync::mpsc::channel::<()>();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut out = stream;
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        say(&mut out, json!({"id": 1, "ok": true})).unwrap();
        wait.recv().unwrap();
        say(&mut out, json!({"id": null, "ok": false, "error": "idle connection closed"})).unwrap();
        out.flush().unwrap();
        // The client's close shows as an end of input or, with the unread line, a reset.
        line.clear();
        reader.read_line(&mut line).ok().filter(|&read| read > 0).map(|_| line)
    });
    let mut client = ControlClient::connect(addr, TOKEN).unwrap();
    assert!(client.is_usable(), "an idle connection with nothing pending is usable");
    said.send(()).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while client.is_usable() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!client.is_usable(), "the unasked line makes the connection unusable");
    assert_eq!(client.call("next", json!({})), Err("connection closed".to_string()));
    assert!(client.is_broken(), "the unusable connection is closed");
    assert_eq!(server.join().unwrap(), None, "the call sent nothing");
}

// A server that refuses a connection at accept writes why and closes: the sign-in reads that waiting line as its
// answer, so the caller learns the reason, and sends nothing.
#[trial]
fn a_refusal_waiting_at_sign_in_is_the_reason_connect_gives() {
    use std::io::{BufRead, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut out = stream;
        say(&mut out, json!({"id": null, "ok": false, "error": "connection limit reached"})).unwrap();
        out.flush().unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).ok().filter(|&read| read > 0).map(|_| line)
    });
    let stream = std::net::TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
    assert!(stream.peek(&mut [0]).unwrap() > 0, "the refusal is waiting before the sign-in");
    let signed = ControlClient::over(stream).unwrap().sign_in(TOKEN);
    assert_eq!(signed.err(), Some(ConnectError::Refused("connection limit reached".into())));
    assert_eq!(server.join().unwrap(), None, "the sign-in sent nothing");
}
