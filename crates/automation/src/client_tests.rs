//! Behaviour tests of `client`.

use craft_ui::testing::trial;
use serde_json::json;

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
