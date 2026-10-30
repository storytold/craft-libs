//! Bulk frames over a real local socket with a fake `FrameSource`.
#![cfg(unix)]

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use craft_ipc::bulk::{BulkClient, FrameFormat, FrameLayout, FrameRequest, FrameSource, FrameSpec, FrameTickets, RenderedFrame};
use craft_ipc::client::Client;
use craft_ipc::server::{Activate, Activation, ControlSink, Frames, Server, ServerConfig};
use craft_ipc::wire;
use craft_ipc::{Endpoint, InstanceRecord, LinkError, RuntimeDir};

const W: u32 = 4;
const H: u32 = 3;
const STRIDE: u32 = 16;

struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct NoSink;

impl ControlSink for NoSink {
    fn submit(&self, _method: String, _params: Value) -> Option<std::sync::mpsc::Receiver<Value>> {
        None
    }
}

struct NoActivate;

impl Activate for NoActivate {
    fn activate(&self, _token: Option<String>, _from_pid: Option<u32>, _open: Option<Value>) -> Activation {
        Activation::No
    }
}

/// Pixels encode the request so a wrong frame cannot match by accident.
struct FakeSource {
    epoch: AtomicU64,
    closed: Mutex<Vec<String>>,
}

fn pixels_for(link_id: &str, time_s: f64, region: Option<[u32; 4]>) -> Vec<u8> {
    let seed =
        link_id.bytes().fold(0u8, |acc, b| acc.wrapping_add(b)).wrapping_add((time_s * 10.0) as u8).wrapping_add(region.map_or(0, |r| r[0] as u8));
    (0..(STRIDE * H) as usize).map(|i| seed.wrapping_add((i as u8).wrapping_mul(7))).collect()
}

impl FrameSource for FakeSource {
    fn open(&self, spec: &FrameSpec) -> Result<FrameLayout, LinkError> {
        if spec.link_id == "missing" {
            return Err(LinkError::NotReady("link_unknown".into()));
        }
        Ok(FrameLayout { w: W, h: H, stride: STRIDE, len: u64::from(STRIDE * H) })
    }

    fn render(&self, spec: &FrameSpec, request: &FrameRequest) -> Result<RenderedFrame, LinkError> {
        Ok(RenderedFrame {
            epoch: self.epoch.load(Ordering::SeqCst),
            w: W,
            h: H,
            stride: STRIDE,
            format: spec.format,
            render_ms: 0.25,
            pixels: pixels_for(&spec.link_id, request.time_s, request.region),
        })
    }

    fn close(&self, spec: &FrameSpec) {
        self.closed.lock().unwrap().push(spec.link_id.clone());
    }
}

struct Rig {
    _tmp: Tmp,
    server: Server,
    record: InstanceRecord,
    source: Arc<FakeSource>,
    tickets: Arc<FrameTickets>,
}

fn boot(epoch: u64) -> Rig {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = PathBuf::from(format!("/tmp/cl-blk-{:08x}", (nanos as u64) ^ u64::from(std::process::id()).wrapping_mul(0x9E37)));
    let _ = std::fs::remove_dir_all(&path);
    let source = Arc::new(FakeSource { epoch: AtomicU64::new(epoch), closed: Mutex::new(Vec::new()) });
    let tickets = Arc::new(FrameTickets::new(Arc::clone(&source) as Arc<dyn FrameSource>));
    let server = Server::start(ServerConfig {
        app: "ec".into(),
        app_version: "0.8.3".into(),
        instance: None,
        runtime: Some(RuntimeDir::at(&path).unwrap()),
        tcp_port: None,
        sink: Arc::new(NoSink),
        activate: Arc::new(NoActivate),
        frames: Arc::clone(&tickets) as Arc<dyn Frames>,
    })
    .unwrap();
    let record = server.record().clone();
    Rig { _tmp: Tmp(path), server, record, source, tickets }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn socket_of(record: &InstanceRecord) -> PathBuf {
    match &record.endpoint {
        Endpoint::Unix { path } => PathBuf::from(path),
        other => panic!("expected a unix endpoint, got {other:?}"),
    }
}

#[test]
fn round_trip_returns_the_exact_bytes() {
    let rig = boot(3);
    let mut control = Client::connect(&rig.record).unwrap();
    let ticket = control.frames_open("link-a", FrameFormat::Rgba8Premul, 1.0).unwrap();
    assert_eq!((ticket.w, ticket.h, ticket.stride, ticket.len), (W, H, STRIDE, u64::from(STRIDE * H)));
    let mut bulk = BulkClient::attach(&rig.record, &ticket.ticket).unwrap();
    let whole = bulk.request(1.5, 3, None).unwrap();
    assert_eq!(whole.pixels, pixels_for("link-a", 1.5, None));
    assert_eq!((whole.req, whole.epoch, whole.w, whole.h, whole.stride), (1, 3, W, H, STRIDE));
    assert_eq!(whole.format, FrameFormat::Rgba8Premul);
    assert!((whole.render_ms - 0.25).abs() < f64::EPSILON);
    let part = bulk.request(2.0, 3, Some([5, 0, 2, 2])).unwrap();
    assert_eq!(part.pixels, pixels_for("link-a", 2.0, Some([5, 0, 2, 2])));
    assert_ne!(part.pixels, whole.pixels);
    assert_eq!(part.req, 2);
    drop(bulk);
    drop(control);
    rig.server.shutdown().unwrap();
}

#[test]
fn a_newer_epoch_is_returned_and_an_older_source_is_an_error() {
    let rig = boot(7);
    let mut control = Client::connect(&rig.record).unwrap();
    let ticket = control.frames_open("link-a", FrameFormat::Rgba16fPremul, 2.0).unwrap();
    let mut bulk = BulkClient::attach(&rig.record, &ticket.ticket).unwrap();
    let frame = bulk.request(0.0, 2, None).unwrap();
    assert_eq!(frame.epoch, 7, "asked for epoch 2, source is at 7");
    assert_eq!(frame.format, FrameFormat::Rgba16fPremul);
    let err = bulk.request(0.0, 9, None).unwrap_err();
    assert!(err.to_string().contains("epoch 7"), "{err}");
    // The error did not poison the connection.
    rig.source.epoch.store(9, Ordering::SeqCst);
    assert_eq!(bulk.request(0.0, 9, None).unwrap().epoch, 9);
    rig.server.shutdown().unwrap();
}

#[test]
fn a_ticket_is_refused_after_its_control_connection_closes() {
    let rig = boot(1);
    let mut control = Client::connect(&rig.record).unwrap();
    let ticket = control.frames_open("link-a", FrameFormat::Rgba8Premul, 1.0).unwrap();
    let mut live = BulkClient::attach(&rig.record, &ticket.ticket).unwrap();
    live.request(0.0, 1, None).unwrap();
    assert_eq!(rig.tickets.open_tickets(), 1);
    drop(control);
    wait_for("source.close", || !rig.source.closed.lock().unwrap().is_empty());
    assert_eq!(rig.tickets.open_tickets(), 0);
    assert!(live.request(0.0, 1, None).is_err(), "an attached connection must lose its frames");
    let again = BulkClient::attach(&rig.record, &ticket.ticket);
    assert!(again.is_err(), "reuse after control close must be refused");
    rig.server.shutdown().unwrap();
}

#[test]
fn bulk_attach_must_be_first_and_the_ticket_must_exist() {
    let rig = boot(1);
    let bogus = "0123456789abcdef0123456789abcdef";
    assert!(BulkClient::attach(&rig.record, bogus).is_err());
    let mut control = Client::connect(&rig.record).unwrap();
    assert!(control.frames_open("missing", FrameFormat::Rgba8Premul, 1.0).is_err());
    assert!(control.frames_open("a", FrameFormat::Rgba8Premul, 0.0).is_err());
    assert_eq!(rig.tickets.open_tickets(), 0);
    rig.server.shutdown().unwrap();
}

fn raw_attach(rig: &Rig, ticket: &str) -> UnixStream {
    let mut stream = UnixStream::connect(socket_of(&rig.record)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    wire::write_line(&mut stream, &json!({"id": 0, "method": "auth", "params": {"token": rig.record.token}})).unwrap();
    read_line(&mut stream);
    wire::write_line(&mut stream, &json!({"id": 1, "method": "bulk.attach", "params": {"ticket": ticket}})).unwrap();
    let reply: Value = serde_json::from_slice(&read_line(&mut stream)).unwrap();
    assert_eq!(reply["ok"], json!(true), "{reply}");
    stream
}

fn read_line(stream: &mut UnixStream) -> Vec<u8> {
    let mut out = Vec::new();
    let mut byte = [0u8; 1];
    while stream.read_exact(&mut byte).is_ok() && byte[0] != b'\n' {
        out.push(byte[0]);
    }
    out
}

#[test]
fn an_oversized_request_header_closes_that_connection_only() {
    let rig = boot(1);
    let mut control = Client::connect(&rig.record).unwrap();
    let ticket = control.frames_open("link-a", FrameFormat::Rgba8Premul, 1.0).unwrap();
    let mut raw = raw_attach(&rig, &ticket.ticket);
    raw.write_all(b"CFRQ").unwrap();
    raw.write_all(&(wire::BULK_HEADER_MAX + 1).to_le_bytes()).unwrap();
    let mut rest = Vec::new();
    // The server closes without a reply, so the read ends at EOF instead of timing out.
    let n = raw.read_to_end(&mut rest).unwrap();
    assert_eq!(n, 0, "no reply expected for an oversized header");
    // The control connection and a fresh bulk connection still work.
    let mut bulk = BulkClient::attach(&rig.record, &ticket.ticket).unwrap();
    assert!(bulk.request(0.0, 1, None).is_ok());
    rig.server.shutdown().unwrap();
}

#[test]
fn a_bad_magic_and_a_bad_header_do_not_panic_the_server() {
    let rig = boot(1);
    let mut control = Client::connect(&rig.record).unwrap();
    let ticket = control.frames_open("link-a", FrameFormat::Rgba8Premul, 1.0).unwrap();
    let mut raw = raw_attach(&rig, &ticket.ticket);
    raw.write_all(b"XXXX\0\0\0\0").unwrap();
    let mut rest = Vec::new();
    assert_eq!(raw.read_to_end(&mut rest).unwrap(), 0);
    // A well-formed record with a bad header gets an error record, not a close.
    let mut raw = raw_attach(&rig, &ticket.ticket);
    wire::write_cfrq(&mut raw, &json!({"req": 5, "epoch": 0})).unwrap();
    let (header, payload) = wire::read_cfrm(&mut raw).unwrap();
    assert_eq!(header["req"], json!(5));
    assert!(header["error"].as_str().unwrap().contains("time_s"), "{header}");
    assert!(payload.is_empty());
    rig.server.shutdown().unwrap();
}

/// A one-shot fake server. Answers auth and attach, then sends `bytes` and closes.
fn fake_server(bytes: Vec<u8>) -> (InstanceRecord, std::thread::JoinHandle<()>, Tmp) {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = PathBuf::from(format!("/tmp/cl-fk-{:08x}", (nanos as u64) ^ u64::from(std::process::id()).wrapping_mul(0x51)));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let record = InstanceRecord::new("fake", "0", std::process::id(), Endpoint::Unix { path: path.to_string_lossy().into_owned() }).unwrap();
    let handle = std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else { return };
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        read_line(&mut stream);
        wire::write_line(&mut stream, &wire::auth_success(&json!(0))).unwrap();
        read_line(&mut stream);
        wire::write_line(&mut stream, &wire::reply_ok(&json!(1), json!({}))).unwrap();
        let mut request = [0u8; 8];
        stream.read_exact(&mut request).unwrap();
        let mut header = vec![0u8; u32::from_le_bytes([request[4], request[5], request[6], request[7]]) as usize];
        stream.read_exact(&mut header).unwrap();
        stream.write_all(&bytes).unwrap();
    });
    (record, handle, Tmp(dir))
}

fn cfrm(header: &Value, declared_payload: u64, payload: &[u8]) -> Vec<u8> {
    let header = serde_json::to_vec(header).unwrap();
    let mut out = b"CFRM".to_vec();
    out.extend((header.len() as u32).to_le_bytes());
    out.extend(declared_payload.to_le_bytes());
    out.extend(header);
    out.extend(payload);
    out
}

#[test]
fn truncated_replies_are_errors_not_panics() {
    let good_header = json!({"req": 1, "epoch": 1, "w": 4, "h": 3, "stride": 16, "format": "rgba8_premul", "render_ms": 1.0});
    let full = cfrm(&good_header, 48, &[7u8; 48]);
    for cut in [2, 6, 10, 20, full.len() - 1] {
        let (record, handle, _tmp) = fake_server(full[..cut].to_vec());
        let mut bulk = BulkClient::attach(&record, &"a".repeat(32)).unwrap();
        let err = bulk.request(0.0, 0, None).unwrap_err();
        assert!(matches!(err, LinkError::Io(_)), "cut {cut}: {err:?}");
        handle.join().unwrap();
    }
    // The uncut record is accepted by the same fake, so the cuts above are the cause.
    let (record, handle, _tmp) = fake_server(full);
    let mut bulk = BulkClient::attach(&record, &"a".repeat(32)).unwrap();
    assert_eq!(bulk.request(0.0, 0, None).unwrap().pixels, vec![7u8; 48]);
    handle.join().unwrap();
}

#[test]
fn oversized_or_inconsistent_replies_are_rejected_by_the_client() {
    let good = json!({"req": 1, "epoch": 1, "w": 4, "h": 3, "stride": 16, "format": "rgba8_premul", "render_ms": 1.0});
    let mut huge_header = b"CFRM".to_vec();
    huge_header.extend((wire::BULK_HEADER_MAX + 1).to_le_bytes());
    huge_header.extend(0u64.to_le_bytes());
    let huge_payload = cfrm(&good, wire::FRAME_PAYLOAD_MAX + 1, &[]);
    let short_payload = cfrm(&good, 47, &[1u8; 47]);
    let wrong_req = cfrm(&json!({"req": 99, "epoch": 1, "w": 4, "h": 3, "stride": 16, "format": "rgba8_premul", "render_ms": 1.0}), 48, &[1u8; 48]);
    for (name, bytes, want_too_large) in
        [("header", huge_header, true), ("payload", huge_payload, true), ("short", short_payload, false), ("req", wrong_req, false)]
    {
        let (record, handle, _tmp) = fake_server(bytes);
        let mut bulk = BulkClient::attach(&record, &"b".repeat(32)).unwrap();
        let err = bulk.request(0.0, 0, None).unwrap_err();
        assert_eq!(matches!(err, LinkError::TooLarge), want_too_large, "{name}: {err:?}");
        handle.join().unwrap();
    }
}
