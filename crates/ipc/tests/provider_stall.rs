//! A provider that never returns must not hold up shutdown. `Assets` and `Activate` calls run on
//! helper threads with a deadline: the caller gets a timeout reply, the helper is detached, and a
//! normal call still works afterwards.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use craft_ipc::asset::{Described, Refreshed, Resolved};
use craft_ipc::moniker::Moniker;
use craft_ipc::record::AssetKind;
use craft_ipc::refresh::FileState;
use craft_ipc::server::{Activate, Activation, Assets, ControlSink, Extensions, FrameOpen, Frames, Limits, Server, ServerConfig};
use craft_ipc::{Endpoint, LinkError, RuntimeDir};

const DEADLINE: Duration = Duration::from_millis(400);
/// `link.refresh` with this link id blocks until the test releases it. `app.activate` with this
/// token blocks the same way.
const STUCK: &str = "stuck";

/// Blocks while `release` is false, so a test can end its helper threads when it is done.
struct Gate {
    release: AtomicBool,
    entered: AtomicUsize,
}

impl Gate {
    fn wait(&self) {
        self.entered.fetch_add(1, Ordering::SeqCst);
        while !self.release.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

struct StuckAssets(Arc<Gate>);

impl Assets for StuckAssets {
    fn resolve(&self, _asset: &Moniker) -> Result<Resolved, LinkError> {
        Err(LinkError::Unknown("no such asset".into()))
    }

    fn describe(&self, asset: &Moniker) -> Result<Described, LinkError> {
        Ok(Described {
            asset: asset.clone(),
            name: "x".into(),
            kind: AssetKind::Still,
            epoch: 1,
            w: None,
            h: None,
            hash: None,
            thumb_path: None,
            modified_ms: None,
        })
    }

    fn refresh(&self, link_id: &str) -> Result<Refreshed, LinkError> {
        if link_id == STUCK {
            self.0.wait();
        }
        Ok(Refreshed::new(link_id, 1, FileState::Current, "/tmp/x", None))
    }
}

struct StuckActivate(Arc<Gate>);

impl Activate for StuckActivate {
    fn activate(&self, token: Option<String>, _from_pid: Option<u32>, _open: Option<Value>) -> Activation {
        if token.as_deref() == Some(STUCK) {
            self.0.wait();
        }
        Activation::Yes
    }
}

struct Sink;

impl ControlSink for Sink {
    fn submit(&self, _method: String, _params: Value) -> Option<mpsc::Receiver<Value>> {
        None
    }
}

struct NoFrames;

impl Frames for NoFrames {
    fn open(&self, _params: &serde_json::Map<String, Value>) -> Result<FrameOpen, LinkError> {
        Err(LinkError::NotReady("no frames".into()))
    }
    fn render(&self, _ticket: &str, _header: &Value) -> Result<(Value, Vec<u8>), LinkError> {
        Err(LinkError::NotReady("no frames".into()))
    }
    fn close_ticket(&self, _ticket: &str) {}
}

struct Rig {
    server: Server,
    gate: Arc<Gate>,
    reader: BufReader<UnixStream>,
    dir: PathBuf,
}

impl Rig {
    fn boot(label: &str, deadline: Duration) -> Self {
        let dir = PathBuf::from(format!("/tmp/cl-stall-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let gate = Arc::new(Gate { release: AtomicBool::new(false), entered: AtomicUsize::new(0) });
        let server = Server::start_with_limits(
            ServerConfig {
                app: "ec".into(),
                app_version: "0.0.1".into(),
                instance: None,
                runtime: Some(RuntimeDir::at(&dir).unwrap()),
                tcp_port: None,
                sink: Arc::new(Sink),
                activate: Arc::new(StuckActivate(Arc::clone(&gate))),
                frames: Arc::new(NoFrames),
            },
            Extensions { assets: Some(Arc::new(StuckAssets(Arc::clone(&gate)))), caps: Vec::new() },
            Limits { provider_deadline: deadline, ..Limits::default() },
        )
        .unwrap();
        let Endpoint::Unix { path } = &server.record().endpoint else { panic!("unix endpoint expected") };
        let stream = UnixStream::connect(path).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut reader = BufReader::new(stream);
        let token = server.record().token.clone();
        send(&mut reader, &json!({"id": 0, "method": "auth", "params": {"token": token}}));
        assert_eq!(read(&mut reader)["ok"], json!(true));
        Self { server, gate, reader, dir }
    }

    fn call(&mut self, id: u64, method: &str, params: Value) -> Value {
        send(&mut self.reader, &json!({"id": id, "method": method, "params": params}));
        read(&mut self.reader)
    }

    /// Shut down on a thread and return how long it took. A hang fails the test after `limit`.
    fn shutdown_within(self, limit: Duration) -> Duration {
        let Self { server, gate, reader, dir } = self;
        let (done, finished) = mpsc::channel();
        let started = Instant::now();
        std::thread::spawn(move || {
            let result = server.shutdown();
            let _ = done.send(result.is_ok());
        });
        let waited = finished.recv_timeout(limit);
        // Let the detached helpers end, so the test process leaves no spinning threads.
        gate.release.store(true, Ordering::SeqCst);
        drop(reader);
        let _ = std::fs::remove_dir_all(dir);
        assert!(waited.expect("shutdown did not finish: a stuck provider held it up"), "shutdown failed");
        started.elapsed()
    }
}

fn send(reader: &mut BufReader<UnixStream>, value: &Value) {
    let mut bytes = serde_json::to_vec(value).unwrap();
    bytes.push(b'\n');
    reader.get_mut().write_all(&bytes).unwrap();
}

fn read(reader: &mut BufReader<UnixStream>) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap_or_else(|err| panic!("reply {line:?}: {err}"))
}

fn timed_out(reply: &Value) -> bool {
    reply["ok"] == json!(false) && reply["error"]["message"] == json!("timeout")
}

#[test]
fn a_stuck_asset_call_times_out_and_the_connection_keeps_working() {
    let mut rig = Rig::boot("assets", DEADLINE);
    let started = Instant::now();
    let reply = rig.call(1, "link.refresh", json!({"link_id": STUCK}));
    let waited = started.elapsed();
    assert!(timed_out(&reply), "{reply}");
    assert_eq!(reply["id"], json!(1));
    assert!(waited >= DEADLINE && waited < DEADLINE * 5, "{waited:?}");
    // Other calls on the same connection still work, asset calls included.
    let fine = rig.call(2, "link.refresh", json!({"link_id": "ok"}));
    assert_eq!(fine["ok"], json!(true), "{fine}");
    assert_eq!(fine["result"]["link_id"], json!("ok"));
    let hello = rig.call(3, "link.hello", json!({"protocol": {"min": 1, "max": 1}}));
    assert_eq!(hello["ok"], json!(true), "{hello}");
    assert_eq!(rig.gate.entered.load(Ordering::SeqCst), 1);
    let took = rig.shutdown_within(Duration::from_secs(3));
    assert!(took < Duration::from_secs(2), "{took:?}");
}

#[test]
fn shutdown_does_not_wait_for_a_stuck_asset_call() {
    // The deadline is long, so shutdown meets the call still in flight.
    let mut rig = Rig::boot("assets-stop", Duration::from_secs(60));
    send(&mut rig.reader, &json!({"id": 1, "method": "link.refresh", "params": {"link_id": STUCK}}));
    while rig.gate.entered.load(Ordering::SeqCst) == 0 {
        std::thread::sleep(Duration::from_millis(10));
    }
    let took = rig.shutdown_within(Duration::from_secs(3));
    assert!(took < Duration::from_secs(2), "{took:?}");
}

#[test]
fn a_stuck_activate_call_times_out_and_shutdown_does_not_wait_for_it() {
    let mut rig = Rig::boot("activate", DEADLINE);
    let reply = rig.call(1, "app.activate", json!({"token": STUCK}));
    assert!(timed_out(&reply), "{reply}");
    let fine = rig.call(2, "app.activate", json!({"token": "ok"}));
    assert_eq!(fine["result"]["activated"], json!(true), "{fine}");
    // A second stuck call, then shut down while it is in flight.
    send(&mut rig.reader, &json!({"id": 3, "method": "app.activate", "params": {"token": STUCK}}));
    while rig.gate.entered.load(Ordering::SeqCst) < 2 {
        std::thread::sleep(Duration::from_millis(10));
    }
    let took = rig.shutdown_within(Duration::from_secs(3));
    assert!(took < Duration::from_secs(2), "{took:?}");
}

#[test]
fn stuck_helpers_are_capped_and_the_server_says_busy() {
    let mut rig = Rig::boot("cap", Duration::from_millis(60));
    let mut busy_at = None;
    for id in 1..=40u64 {
        let reply = rig.call(id, "link.refresh", json!({"link_id": STUCK}));
        if reply["error"]["code"] == json!("busy") {
            busy_at = Some(id);
            break;
        }
        assert!(timed_out(&reply), "call {id}: {reply}");
    }
    let busy_at = busy_at.expect("the server kept starting helper threads past its cap");
    assert_eq!(busy_at, 33, "32 stuck helpers are allowed, the next call is refused");
    assert_eq!(rig.gate.entered.load(Ordering::SeqCst), 32, "a refused call starts no thread");
    // A call that needs no helper still works while the cap is full.
    let hello = rig.call(100, "link.hello", json!({"protocol": {"min": 1, "max": 1}}));
    assert_eq!(hello["ok"], json!(true), "{hello}");
    let took = rig.shutdown_within(Duration::from_secs(3));
    assert!(took < Duration::from_secs(2), "{took:?}");
}
