//! Behaviour tests of `client`.
// Unwrapping, expecting and panicking on failure is fine in tests, unlike in shipped code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_ui::testing::trial;
use serde_json::json;

#[allow(clippy::wildcard_imports, reason = "tests use most of the module under test and its helpers")]
use super::*;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

// call returns Ok(result) for ok replies and Err(error) otherwise.

// A stale reply left by a timed-out call is skipped, and a transport failure closes the client.
#[trial]
fn replies_are_matched_by_id() {
    use std::io::{BufRead, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut out = stream;
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        writeln!(out, "{}", json!({"id": 1, "ok": true})).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        writeln!(out, "{}", json!({"id": 99, "ok": true, "result": "stale"})).unwrap();
        writeln!(out, "{}", json!({"id": 2, "ok": true, "result": "fresh"})).unwrap();
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
    use std::io::{BufRead, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (done, wait) = std::sync::mpsc::channel::<()>();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut out = stream;
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        writeln!(out, "{}", json!({"id": 1, "ok": true})).unwrap();
        // The call is read and never answered while the client waits.
        line.clear();
        reader.read_line(&mut line).unwrap();
        wait.recv().unwrap();
        writeln!(out, "{}", json!({"id": 2, "ok": true, "result": "late"})).ok();
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
