//! Connection limits and partial-line handling. Each test asserts the secure behavior: silent
//! peers cannot fill the server, and a line that arrives in pieces is not lost.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use craft_ipc::server::{Activate, Activation, ControlSink, Extensions, FrameOpen, Frames, Limits, Server, ServerConfig};
use craft_ipc::{LinkError, RuntimeDir};

struct Sink;
impl ControlSink for Sink {
    fn submit(&self, _method: String, _params: Value) -> Option<std::sync::mpsc::Receiver<Value>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(json!({}));
        Some(rx)
    }
}
struct NoActivate;
impl Activate for NoActivate {
    fn activate(&self, _t: Option<String>, _p: Option<u32>, _o: Option<Value>) -> Activation {
        Activation::No
    }
}
struct NoFrames;
impl Frames for NoFrames {
    fn open(&self, _p: &serde_json::Map<String, Value>) -> Result<FrameOpen, LinkError> {
        Err(LinkError::NotReady("no frames".into()))
    }
    fn render(&self, _t: &str, _h: &Value) -> Result<(Value, Vec<u8>), LinkError> {
        Err(LinkError::NotReady("no frames".into()))
    }
    fn close_ticket(&self, _t: &str) {}
}

fn boot_with(label: &str, limits: Limits) -> (Server, PathBuf, String, PathBuf) {
    let dir = PathBuf::from(format!("/tmp/cl-ipc-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let runtime = RuntimeDir::at(&dir).unwrap();
    let server = Server::start_with_limits(
        ServerConfig {
            app: "ec".into(),
            app_version: "0.0.1".into(),
            instance: None,
            runtime: Some(runtime),
            tcp_port: None,
            sink: Arc::new(Sink),
            activate: Arc::new(NoActivate),
            frames: Arc::new(NoFrames),
        },
        Extensions::default(),
        limits,
    )
    .unwrap();
    let socket = match &server.record().endpoint {
        craft_ipc::Endpoint::Unix { path } => PathBuf::from(path),
        other => panic!("unix endpoint expected, got {other:?}"),
    };
    let token = server.record().token.clone();
    (server, socket, token, dir)
}

fn boot(label: &str) -> (Server, PathBuf, String, PathBuf) {
    boot_with(label, Limits::default())
}

fn auth_line(token: &str) -> String {
    format!("{{\"id\":0,\"method\":\"auth\",\"params\":{{\"token\":\"{token}\"}}}}\n")
}

const HELLO: &str = "{\"id\":1,\"method\":\"link.hello\",\"params\":{\"protocol\":{\"min\":1,\"max\":1}}}\n";

fn read_text(stream: &mut UnixStream, secs: u64) -> String {
    stream.set_read_timeout(Some(Duration::from_secs(secs))).unwrap();
    let mut text = String::new();
    let _ = stream.read_to_string(&mut text);
    text
}

fn next_line(reader: &mut BufReader<UnixStream>) -> String {
    let mut line = String::new();
    reader.get_ref().set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    reader.read_line(&mut line).unwrap();
    line
}

/// 16 silent connections must not lock the legitimate client out. At most 4 unauthenticated
/// connections are admitted, the rest are refused at once, and the admitted ones are closed at the
/// auth deadline. A client that authenticated before the flood keeps working.
#[test]
fn silent_connections_cannot_lock_out_clients() {
    let (server, socket, token, dir) = boot("slots");
    let early = UnixStream::connect(&socket).unwrap();
    let mut early_writer = early.try_clone().unwrap();
    let mut early_reader = BufReader::new(early);
    early_writer.write_all(auth_line(&token).as_bytes()).unwrap();
    assert!(next_line(&mut early_reader).contains("authenticated"));

    let mut hogs = Vec::new();
    for _ in 0..16 {
        hogs.push(UnixStream::connect(&socket).unwrap());
    }
    // The authenticated client still gets its answer while the flood is open.
    early_writer.write_all(HELLO.as_bytes()).unwrap();
    let line = next_line(&mut early_reader);
    assert!(line.contains("\"ok\":true"), "authenticated client lost service: {line:?}");

    // Past the auth deadline every silent connection is closed and a new client is served.
    std::thread::sleep(Duration::from_millis(3500));
    for hog in &mut hogs {
        let text = read_text(hog, 1);
        assert!(!text.contains("\"authenticated\""), "{text:?}");
    }
    let mut legit = UnixStream::connect(&socket).unwrap();
    legit.write_all(auth_line(&token).as_bytes()).unwrap();
    let mut reader = BufReader::new(legit);
    let line = next_line(&mut reader);
    assert!(line.contains("authenticated"), "legitimate client was not served: {line:?}");
    drop(hogs);
    drop(server);
    let _ = std::fs::remove_dir_all(dir);
}

/// the unauthenticated pool is small, and the refusal for an extra peer is immediate.
#[test]
fn unauthenticated_peers_beyond_the_cap_are_refused_at_once() {
    let limits = Limits { max_unauthenticated: 2, ..Limits::default() };
    let (server, socket, token, dir) = boot_with("unauth-cap", limits);
    let a = UnixStream::connect(&socket).unwrap();
    let b = UnixStream::connect(&socket).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let mut third = UnixStream::connect(&socket).unwrap();
    let text = read_text(&mut third, 1);
    assert!(text.contains("too many unauthenticated connections"), "{text:?}");
    // Authenticating frees an unauthenticated slot.
    let mut a = a;
    a.write_all(auth_line(&token).as_bytes()).unwrap();
    let mut reader = BufReader::new(a);
    assert!(next_line(&mut reader).contains("authenticated"));
    let mut fourth = UnixStream::connect(&socket).unwrap();
    fourth.write_all(auth_line(&token).as_bytes()).unwrap();
    let mut reader = BufReader::new(fourth);
    assert!(next_line(&mut reader).contains("authenticated"));
    drop(b);
    drop(server);
    let _ = std::fs::remove_dir_all(dir);
}

/// a connection that sends no auth line is closed after the deadline (default 2 s).
#[test]
fn silent_connection_is_closed_at_the_auth_deadline() {
    let (server, socket, _token, dir) = boot("deadline");
    let mut s = UnixStream::connect(&socket).unwrap();
    let start = std::time::Instant::now();
    let text = read_text(&mut s, 6);
    let waited = start.elapsed();
    assert!(text.contains("authentication required"), "{text:?}");
    assert!(waited < Duration::from_secs(4), "closed after {waited:?}");
    assert!(waited >= Duration::from_millis(1500), "closed too early: {waited:?}");
    drop(server);
    let _ = std::fs::remove_dir_all(dir);
}

/// a peer that starts a line and stalls is cut at the partial-line deadline.
#[test]
fn a_stalled_partial_line_is_closed() {
    let limits = Limits { partial_line: Duration::from_millis(800), ..Limits::default() };
    let (server, socket, token, dir) = boot_with("stall", limits);
    let mut s = UnixStream::connect(&socket).unwrap();
    s.write_all(auth_line(&token).as_bytes()).unwrap();
    s.write_all(b"{\"id\":1,\"method\":\"link.he").unwrap();
    let start = std::time::Instant::now();
    let text = read_text(&mut s, 6);
    assert!(text.contains("authenticated"), "{text:?}");
    assert!(start.elapsed() < Duration::from_secs(4), "the stalled line stayed open for {:?}", start.elapsed());
    drop(server);
    let _ = std::fs::remove_dir_all(dir);
}

/// an authenticated connection with no subscription and no traffic is closed at the idle limit.
/// A subscribed connection stays open.
#[test]
fn idle_connections_are_closed_but_subscribers_stay() {
    let limits = Limits { idle: Duration::from_millis(900), ..Limits::default() };
    let (server, socket, token, dir) = boot_with("idle", limits);
    let mut idle = UnixStream::connect(&socket).unwrap();
    idle.write_all(auth_line(&token).as_bytes()).unwrap();
    let mut sub = UnixStream::connect(&socket).unwrap();
    sub.write_all(auth_line(&token).as_bytes()).unwrap();
    sub.write_all(b"{\"id\":2,\"method\":\"events.subscribe\",\"params\":{\"events\":[\"doc.changed\"]}}\n").unwrap();
    let start = std::time::Instant::now();
    let text = read_text(&mut idle, 6);
    assert!(text.contains("authenticated"), "{text:?}");
    assert!(start.elapsed() < Duration::from_secs(4), "idle connection stayed open for {:?}", start.elapsed());
    // The subscriber is still served after the idle limit passed.
    std::thread::sleep(Duration::from_millis(300));
    sub.write_all(HELLO.as_bytes()).unwrap();
    let mut reader = BufReader::new(sub);
    let mut seen = String::new();
    for _ in 0..3 {
        seen.push_str(&next_line(&mut reader));
        if seen.contains("\"id\":1") {
            break;
        }
    }
    assert!(seen.contains("\"id\":1"), "subscriber was cut: {seen:?}");
    drop(server);
    let _ = std::fs::remove_dir_all(dir);
}

/// a request split at several points, with a pause longer than the 250 ms read poll, is served.
#[test]
fn a_line_split_across_the_250ms_read_poll_is_kept() {
    let (server, socket, token, dir) = boot("split");
    for cut in [1usize, 23, HELLO.len() - 2, HELLO.len() - 1] {
        let mut s = UnixStream::connect(&socket).unwrap();
        s.write_all(auth_line(&token).as_bytes()).unwrap();
        s.write_all(&HELLO.as_bytes()[..cut]).unwrap();
        std::thread::sleep(Duration::from_millis(600));
        s.write_all(&HELLO.as_bytes()[cut..]).unwrap();
        let mut reader = BufReader::new(s);
        assert!(next_line(&mut reader).contains("authenticated"), "cut {cut}");
        let line = next_line(&mut reader);
        assert!(line.contains("\"ok\":true"), "cut {cut}: split request was not served: {line:?}");
    }
    drop(server);
    let _ = std::fs::remove_dir_all(dir);
}

/// the auth line itself may arrive in pieces.
#[test]
fn an_auth_line_split_across_the_read_poll_is_kept() {
    let (server, socket, token, dir) = boot("authsplit");
    let mut s = UnixStream::connect(&socket).unwrap();
    let line = auth_line(&token);
    s.write_all(&line.as_bytes()[..20]).unwrap();
    std::thread::sleep(Duration::from_millis(600));
    s.write_all(&line.as_bytes()[20..]).unwrap();
    let mut reader = BufReader::new(s);
    assert!(next_line(&mut reader).contains("authenticated"));
    drop(server);
    let _ = std::fs::remove_dir_all(dir);
}
