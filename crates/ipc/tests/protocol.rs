//! Session behaviour. The tests own the sink, the activate trait, and frames.
//! They do not use the client module.
#![cfg(unix)]

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use craft_ipc::server::{Activate, Activation, ControlSink, FrameOpen, Frames, Server, ServerConfig};
use craft_ipc::wire::{self, MAX_LINE};
use craft_ipc::{Endpoint, LinkError, RuntimeDir};

const PAYLOAD: &[u8] = b"craft-frame";
const TICKET: &str = "00112233445566778899aabbccddeeff";

struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct ReplySink {
    result: Mutex<Value>,
    seen: Mutex<Vec<String>>,
}

impl ControlSink for ReplySink {
    fn submit(&self, method: String, _params: Value) -> Option<std::sync::mpsc::Receiver<Value>> {
        self.seen.lock().unwrap().push(method);
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(self.result.lock().unwrap().clone());
        Some(rx)
    }
}

struct SwitchActivate {
    mode: Mutex<Activation>,
    seen_token: Mutex<Option<Option<String>>>,
    seen_pid: Mutex<Option<Option<u32>>>,
}

impl Activate for SwitchActivate {
    fn activate(&self, token: Option<String>, from_pid: Option<u32>, _open: Option<Value>) -> Activation {
        *self.seen_token.lock().unwrap() = Some(token);
        *self.seen_pid.lock().unwrap() = Some(from_pid);
        *self.mode.lock().unwrap()
    }
}

struct TestFrames {
    open_count: Mutex<u32>,
    closed: Mutex<Vec<String>>,
}

impl Frames for TestFrames {
    fn open(&self, _params: &serde_json::Map<String, Value>) -> Result<FrameOpen, LinkError> {
        *self.open_count.lock().unwrap() += 1;
        Ok(FrameOpen { ticket: TICKET.into(), w: 4, h: 2, stride: 16, len: PAYLOAD.len() as u64 })
    }

    fn render(&self, ticket: &str, header: &Value) -> Result<(Value, Vec<u8>), LinkError> {
        if ticket != TICKET {
            return Err(LinkError::BadParams("unknown ticket".into()));
        }
        let req = header.get("req").cloned().unwrap_or(Value::Null);
        Ok((json!({"req": req, "epoch": 3, "w": 4, "h": 2, "stride": 16, "format": "rgba8_premul", "render_ms": 1.5}), PAYLOAD.to_vec()))
    }

    fn close_ticket(&self, ticket: &str) {
        self.closed.lock().unwrap().push(ticket.to_string());
    }
}

struct Harness {
    tmp: Tmp,
    server: Server,
    socket: PathBuf,
    token: String,
    sink: Arc<ReplySink>,
    activate: Arc<SwitchActivate>,
    frames: Arc<TestFrames>,
}

fn boot() -> Harness {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let mixed = (nanos as u64) ^ u64::from(std::process::id()).wrapping_mul(0x51ED);
    let path = PathBuf::from(format!("/tmp/cl-srv-{mixed:08x}"));
    let _ = std::fs::remove_dir_all(&path);
    let runtime = RuntimeDir::at(&path).unwrap();
    let sink = Arc::new(ReplySink { result: Mutex::new(json!({"commands": ["cut"]})), seen: Mutex::new(Vec::new()) });
    let activate = Arc::new(SwitchActivate { mode: Mutex::new(Activation::Requested), seen_token: Mutex::new(None), seen_pid: Mutex::new(None) });
    let frames = Arc::new(TestFrames { open_count: Mutex::new(0), closed: Mutex::new(Vec::new()) });
    let server = Server::start(ServerConfig {
        app: "ec".into(),
        app_version: "0.8.3".into(),
        instance: None,
        runtime: Some(runtime),
        tcp_port: None,
        sink: Arc::clone(&sink) as Arc<dyn ControlSink>,
        activate: Arc::clone(&activate) as Arc<dyn Activate>,
        frames: Arc::clone(&frames) as Arc<dyn Frames>,
    })
    .unwrap();
    let socket = match &server.record().endpoint {
        Endpoint::Unix { path } => PathBuf::from(path),
        other => panic!("expected a unix endpoint, got {other:?}"),
    };
    let token = server.record().token.clone();
    Harness { tmp: Tmp(path), server, socket, token, sink, activate, frames }
}

struct Client {
    reader: BufReader<UnixStream>,
}

impl Client {
    fn connect(socket: &Path, token: &str) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut reader = BufReader::new(stream);
        let req = json!({"id": 0, "method": "auth", "params": {"token": token}});
        wire::write_line(reader.get_mut(), &req).unwrap();
        let reply = read_value(&mut reader).unwrap_or_else(|| panic!("no auth reply"));
        assert_eq!(reply["ok"], json!(true), "{reply}");
        assert_eq!(reply["result"]["authenticated"], json!(true), "{reply}");
        Self { reader }
    }

    fn call(&mut self, id: i64, method: &str, params: Value) -> Value {
        let req = json!({"id": id, "method": method, "params": params});
        wire::write_line(self.reader.get_mut(), &req).unwrap();
        read_value(&mut self.reader).unwrap_or_else(|| panic!("no reply for {method}"))
    }

    fn set_timeout(&mut self, timeout: Duration) {
        self.reader.get_mut().set_read_timeout(Some(timeout)).unwrap();
    }
}

fn read_value(reader: &mut BufReader<UnixStream>) -> Option<Value> {
    match wire::read_line(reader, MAX_LINE) {
        Ok(Some(bytes)) => Some(serde_json::from_slice(&bytes).unwrap()),
        Ok(None) => None,
        Err(_) => None,
    }
}

fn read_raw_line(stream: &mut impl Read) -> Vec<u8> {
    let mut out = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).unwrap();
        if byte[0] == b'\n' {
            break;
        }
        out.push(byte[0]);
    }
    if out.last() == Some(&b'\r') {
        out.pop();
    }
    out
}

#[test]
fn legacy_command_keeps_the_request_id() {
    let harness = boot();
    let mut client = Client::connect(&harness.socket, &harness.token);
    let reply = client.call(41, "engine.commands", json!({}));
    assert_eq!(reply["id"], json!(41), "{reply}");
    assert_eq!(reply["ok"], json!(true), "{reply}");
    assert_eq!(reply["result"], json!({"commands": ["cut"]}), "{reply}");
    assert_eq!(harness.sink.seen.lock().unwrap().as_slice(), ["engine.commands"]);
    harness.server.shutdown().unwrap();
    drop(harness.tmp);
}

/// The server polls each connection with a 250 ms read timeout. On Linux that timeout surfaces as
/// `WouldBlock` ("Resource temporarily unavailable"), which must mean "idle, keep waiting". A
/// connection that stays quiet for longer than one poll must still answer the next request.
#[test]
fn a_connection_idle_for_longer_than_the_read_poll_still_answers() {
    let harness = boot();
    let mut client = Client::connect(&harness.socket, &harness.token);
    let first = client.call(1, "engine.commands", json!({}));
    assert_eq!(first["ok"], json!(true), "{first}");
    std::thread::sleep(Duration::from_millis(900));
    let second = client.call(2, "engine.commands", json!({}));
    assert_eq!(second["id"], json!(2), "the idle connection must still be open: {second}");
    assert_eq!(second["ok"], json!(true), "{second}");
    harness.server.shutdown().unwrap();
    drop(harness.tmp);
}

/// A sink value that already has `ok` is the finished reply. The server inserts `id` and does not nest it.
#[test]
fn sink_reply_with_ok_is_not_wrapped() {
    let harness = boot();
    *harness.sink.result.lock().unwrap() = json!({"ok": true, "result": null});
    let mut client = Client::connect(&harness.socket, &harness.token);
    let reply = client.call(3, "engine.commands", json!({}));
    assert_eq!(reply["id"], json!(3), "{reply}");
    assert_eq!(reply["ok"], json!(true), "{reply}");
    assert_eq!(reply["result"], Value::Null, "{reply}");
    assert!(reply.get("error").is_none(), "{reply}");

    *harness.sink.result.lock().unwrap() = json!({"ok": false, "error": "missing command"});
    let bad = client.call(4, "engine.commands", json!({}));
    assert_eq!(bad["id"], json!(4), "{bad}");
    assert_eq!(bad["ok"], json!(false), "{bad}");
    assert_eq!(bad["error"], json!("missing command"), "{bad}");
    assert!(bad.get("result").is_none(), "an error reply must stay flat: {bad}");
    harness.server.shutdown().unwrap();
}

#[test]
fn events_wait_for_subscribe_then_one_event_is_delivered() {
    let harness = boot();
    let mut client = Client::connect(&harness.socket, &harness.token);
    client.set_timeout(Duration::from_millis(200));
    harness.server.emit("doc.changed", json!({"doc_id": "a"})).unwrap();
    assert!(read_value(&mut client.reader).is_none(), "event before subscribe");
    client.set_timeout(Duration::from_secs(2));
    let sub = client.call(2, "events.subscribe", json!({"events": ["doc.changed"]}));
    assert_eq!(sub["id"], json!(2), "{sub}");
    assert_eq!(sub["result"], json!({"seq": 0}), "{sub}");
    harness.server.emit("doc.changed", json!({"doc_id": "a"})).unwrap();
    let event = read_value(&mut client.reader).unwrap_or_else(|| panic!("missing event"));
    assert_eq!(event["event"], json!("doc.changed"), "{event}");
    assert_eq!(event["seq"], json!(1), "{event}");
    assert_eq!(event["data"], json!({"doc_id": "a"}), "{event}");
    harness.server.shutdown().unwrap();
    drop(harness.tmp);
}

#[test]
fn overflow_is_one_slot_not_257_payloads() {
    let harness = boot();
    let mut client = Client::connect(&harness.socket, &harness.token);
    let sub = client.call(2, "events.subscribe", json!({"events": ["doc.changed"]}));
    assert_eq!(sub["result"]["seq"], json!(0), "{sub}");
    for n in 0..257 {
        harness.server.emit("doc.changed", json!({"n": n})).unwrap();
    }
    client.set_timeout(Duration::from_millis(500));
    let mut events = Vec::new();
    while let Some(event) = read_value(&mut client.reader) {
        events.push(event);
    }
    let mut saw_overflow = false;
    let mut since_ok = false;
    let mut payloads = HashSet::new();
    for event in &events {
        if event["event"] == json!("events.overflow") {
            saw_overflow = true;
            since_ok = event["data"]["since"].is_number();
        } else if event["event"] == json!("doc.changed") {
            payloads.insert(event["data"]["n"].clone());
        }
    }
    assert!(saw_overflow, "no overflow in {events:?}");
    assert!(since_ok, "overflow since missing in {events:?}");
    assert!(payloads.len() < 257, "got {} distinct payloads", payloads.len());
    assert!(payloads.len() <= 255, "overflow should take a queue slot, got {} payloads", payloads.len());
    harness.server.shutdown().unwrap();
    drop(harness.tmp);
}

#[test]
fn bulk_round_trip_returns_the_frame_bytes() {
    let harness = boot();
    let mut client = Client::connect(&harness.socket, &harness.token);
    let hello = client.call(
        1,
        "link.hello",
        json!({"client": {"app": "fc", "app_version": "0.9.1", "pid": 7, "instance": "00000000-0000-4000-8000-000000000001"}, "protocol": {"min": 1, "max": 1}}),
    );
    assert_eq!(hello["ok"], json!(true), "{hello}");
    assert_eq!(hello["result"]["app"], json!("ec"), "{hello}");
    assert_eq!(hello["result"]["app_version"], json!("0.8.3"), "{hello}");
    assert_eq!(hello["result"]["protocol"], json!(1), "{hello}");
    assert_eq!(hello["result"]["caps"], json!(["events", "activate", "link.receive", "frames"]), "{hello}");
    let opened = client.call(8, "frames.open", json!({"link_id": "L", "format": "rgba8_premul", "scale": 1.0}));
    assert_eq!(opened["ok"], json!(true), "{opened}");
    assert_eq!(opened["result"]["ticket"], json!(TICKET), "{opened}");
    assert_eq!(opened["result"]["w"], json!(4), "{opened}");
    assert_eq!(opened["result"]["len"], json!(PAYLOAD.len()), "{opened}");

    let mut bulk = UnixStream::connect(&harness.socket).unwrap();
    bulk.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    bulk.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
    let auth = json!({"id": 0, "method": "auth", "params": {"token": harness.token}});
    wire::write_line(&mut bulk, &auth).unwrap();
    let auth_reply: Value = serde_json::from_slice(&read_raw_line(&mut bulk)).unwrap();
    assert_eq!(auth_reply["ok"], json!(true), "{auth_reply}");
    let attach = json!({"id": 9, "method": "bulk.attach", "params": {"ticket": TICKET}});
    wire::write_line(&mut bulk, &attach).unwrap();
    let attach_reply: Value = serde_json::from_slice(&read_raw_line(&mut bulk)).unwrap();
    assert_eq!(attach_reply["ok"], json!(true), "{attach_reply}");
    assert_eq!(attach_reply["id"], json!(9), "{attach_reply}");

    let header = json!({"req": 7, "time_s": 1.5, "epoch": 3, "region": null});
    wire::write_cfrq(&mut bulk, &header).unwrap();
    let (got, payload) = wire::read_cfrm(&mut bulk).unwrap();
    assert_eq!(got["req"], json!(7), "{got}");
    assert_eq!(payload, PAYLOAD);

    drop(bulk);
    drop(client);
    harness.server.shutdown().unwrap();
    assert_eq!(harness.frames.closed.lock().unwrap().as_slice(), [TICKET]);
    drop(harness.tmp);
}

#[test]
fn activate_returns_requested_from_the_trait() {
    let harness = boot();
    let mut client = Client::connect(&harness.socket, &harness.token);
    let params = json!({"token": "wayland-token", "from_pid": 4242, "open": {"link_id": "abc"}});
    let reply = client.call(3, "app.activate", params);
    assert_eq!(reply["ok"], json!(true), "{reply}");
    assert_eq!(reply["result"]["activated"], json!("requested"), "{reply}");
    assert_eq!(harness.activate.seen_token.lock().unwrap().clone().unwrap().as_deref(), Some("wayland-token"));
    assert_eq!(harness.activate.seen_pid.lock().unwrap().unwrap(), Some(4242));

    *harness.activate.mode.lock().unwrap() = Activation::No;
    let reply = client.call(4, "app.activate", json!({"token": null}));
    assert_eq!(reply["result"]["activated"], json!(false), "{reply}");

    *harness.activate.mode.lock().unwrap() = Activation::Yes;
    let reply = client.call(5, "app.activate", json!({}));
    assert_eq!(reply["result"]["activated"], json!(true), "{reply}");
    harness.server.shutdown().unwrap();
    drop(harness.tmp);
}

#[test]
fn the_17th_connection_is_refused_and_does_not_stay_open() {
    let harness = boot();
    let mut held = Vec::new();
    // Authenticated connections fill the cap. Unauthenticated ones are limited to a smaller pool.
    for _ in 0..wire::MAX_CONNECTIONS {
        held.push(Client::connect(&harness.socket, &harness.token));
    }
    let mut extra = UnixStream::connect(&harness.socket).unwrap();
    extra.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut line = String::new();
    BufReader::new(&mut extra).read_line(&mut line).unwrap();
    let value: Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(value["ok"], json!(false), "{line}");
    assert_eq!(value["error"], json!("too many control connections"), "{line}");
    drop(held);
    harness.server.shutdown().unwrap();
    drop(harness.tmp);
}
