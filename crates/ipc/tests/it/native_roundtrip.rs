//! Exercise the real native transport: Unix sockets or Windows named pipes.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use craft_ipc::client::{Client, HelloClient};
use craft_ipc::server::{Activate, Activation, ControlSink, FrameOpen, Frames, Server, ServerConfig};
use craft_ipc::{LinkError, RuntimeDir};
use serde_json::{Value, json};

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Echo;
impl ControlSink for Echo {
    fn submit(&self, method: String, params: Value) -> Option<mpsc::Receiver<Value>> {
        let (tx, rx) = mpsc::channel();
        tx.send(json!({"ok": true, "result": {"method": method, "params": params}})).unwrap();
        Some(rx)
    }
}

struct NoActivate;
impl Activate for NoActivate {
    fn activate(&self, _token: Option<String>, _pid: Option<u32>, _options: Option<Value>) -> Activation {
        Activation::No
    }
}

struct NoFrames;
impl Frames for NoFrames {
    fn open(&self, _params: &serde_json::Map<String, Value>) -> Result<FrameOpen, LinkError> {
        Err(LinkError::NotReady("no frames".into()))
    }
    fn render(&self, _ticket: &str, _hints: &Value) -> Result<(Value, Vec<u8>), LinkError> {
        Err(LinkError::NotReady("no frames".into()))
    }
    fn close_ticket(&self, _ticket: &str) {}
}

#[test]
fn native_transport_authenticates_and_delivers_commands() {
    let mut nonce = [0u8; 4];
    getrandom::getrandom(&mut nonce).unwrap();
    let path = std::env::temp_dir().join(format!("cl-nr-{}-{}", std::process::id(), u32::from_le_bytes(nonce)));
    let cleanup = Cleanup(path.clone());
    let server = Server::start(ServerConfig {
        app: "ec".into(),
        app_version: "test".into(),
        instance: None,
        runtime: Some(RuntimeDir::at(path).unwrap()),
        tcp_port: None,
        sink: Arc::new(Echo),
        activate: Arc::new(NoActivate),
        frames: Arc::new(NoFrames),
    })
    .unwrap();
    let mut client = Client::connect(server.record()).unwrap();
    client
        .hello(&HelloClient {
            app: "ec".into(),
            app_version: "test".into(),
            pid: std::process::id(),
            instance: "550e8400-e29b-41d4-a716-446655440000".into(),
            protocol_min: craft_ipc::wire::PROTOCOL_MIN,
            protocol_max: craft_ipc::wire::PROTOCOL_MAX,
        })
        .unwrap();
    let reply = client.call("engine.execute", json!({"command": "ping"})).unwrap();
    assert_eq!(reply, json!({"method": "engine.execute", "params": {"command": "ping"}}));

    client.subscribe(&["doc.changed"]).unwrap();
    let start = Instant::now();
    assert!(client.next_event(Some(Duration::from_millis(50))).unwrap().is_none());
    assert!(start.elapsed() < Duration::from_secs(2), "quiet native transport must respect its deadline");
    server.emit("doc.changed", json!({"after_timeout": true})).unwrap();
    let event = client.next_event(Some(Duration::from_secs(2))).unwrap().expect("event after timeout");
    assert_eq!(event.data, json!({"after_timeout": true}));

    let mut wrong = server.record().clone();
    let opposite = if wrong.token.starts_with('0') { "1" } else { "0" };
    wrong.token.replace_range(..1, opposite);
    assert!(matches!(Client::connect(&wrong), Err(LinkError::Auth)));
    drop(client);
    drop(server);
    drop(cleanup);
}
