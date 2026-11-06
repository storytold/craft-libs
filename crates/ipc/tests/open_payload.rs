//! `link.open` payload validation over a real server and a real client.
//! A bad payload gets `payload_invalid` and never reaches the app sink.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use craft_ipc::client::{Client, HelloClient};
use craft_ipc::server::{Activate, Activation, ControlSink, FrameOpen, Frames, Server, ServerConfig};
use craft_ipc::{Discovery, LinkError, RuntimeDir};

const ASSET: &str = "craft://photocraft/550e8400-e29b-41d4-a716-446655440000/image/flat";
const LINK: &str = "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = PathBuf::from(format!("/tmp/cl-open-{}-{nanos:x}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Records every forwarded request and answers with a finished reply payload.
#[derive(Default)]
struct RecordingSink {
    seen: Mutex<Vec<(String, Value)>>,
}

impl ControlSink for RecordingSink {
    fn submit(&self, method: String, params: Value) -> Option<std::sync::mpsc::Receiver<Value>> {
        self.seen.lock().unwrap().push((method, params));
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(json!({"comp": "c1"}));
        Some(rx)
    }
}

struct NoActivate;

impl Activate for NoActivate {
    fn activate(&self, _token: Option<String>, _from_pid: Option<u32>, _open: Option<Value>) -> Activation {
        Activation::No
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

fn open_params(payload: Value) -> Value {
    json!({"link_id": LINK, "payload": payload, "result_path": "/tmp/x.png"})
}

#[test]
fn bad_payloads_get_payload_invalid_and_good_ones_reach_the_sink() {
    let tmp = Tmp::new();
    let runtime = RuntimeDir::at(tmp.0.join("rt")).unwrap();
    let sink = Arc::new(RecordingSink::default());
    let server = Server::start(ServerConfig {
        app: "effectcraft".into(),
        app_version: "0.1.0".into(),
        instance: None,
        runtime: Some(runtime.clone()),
        tcp_port: None,
        sink: Arc::clone(&sink) as Arc<dyn ControlSink>,
        activate: Arc::new(NoActivate),
        frames: Arc::new(NoFrames),
    })
    .unwrap();
    let record = Discovery::scan(&runtime).unwrap().into_iter().next().unwrap();
    let mut client = Client::connect(&record).unwrap();
    client
        .hello(&HelloClient {
            app: "filmcraft".into(),
            app_version: "0.9.1".into(),
            pid: std::process::id(),
            instance: "550e8400-e29b-41d4-a716-446655440001".into(),
            protocol_min: 1,
            protocol_max: 1,
        })
        .unwrap();

    let bad = [
        ("unknown format", json!({"format": "svg", "data": "<svg/>"})),
        ("no format", json!({"data": "{}"})),
        ("payload is a string", json!("otio")),
        ("otio without data", json!({"format": "otio-json"})),
        ("otio empty data", json!({"format": "otio-json", "data": ""})),
        ("image-doc relative path", json!({"format": "image-doc", "path": "rel.png", "asset": ASSET})),
        ("image-doc no path", json!({"format": "image-doc", "asset": ASSET})),
        ("image-doc bad moniker", json!({"format": "image-doc", "path": "/tmp/a.png", "asset": "craft://bad"})),
        ("image-doc no asset", json!({"format": "image-doc", "path": "/tmp/a.png"})),
    ];
    for (name, payload) in bad {
        let err = client.call("link.open", open_params(payload)).unwrap_err();
        assert!(matches!(err, LinkError::PayloadInvalid(_)), "{name}: {err:?}");
    }
    let err = client.call("link.open", json!({"link_id": LINK})).unwrap_err();
    assert!(matches!(err, LinkError::PayloadInvalid(_)), "missing payload: {err:?}");
    // A payload that names a file that is not there gets media_missing with the paths.
    let missing = json!({"OTIO_SCHEMA": "ExternalReference.1", "target_url": "file:///nowhere/clip.mov"}).to_string();
    let err = client.call("link.open", open_params(json!({"format": "otio-json", "data": missing}))).unwrap_err();
    assert_eq!(err, LinkError::MediaMissing { paths: vec!["/nowhere/clip.mov".to_string()] });
    // A path that climbs, is relative or is not a string never reaches the app.
    let good = json!({"format": "otio-json", "data": "{}"});
    for result_path in [json!("../x.mov"), json!("/tmp/../etc/x.mov"), json!("rel/x.mov"), json!(7), json!("/tmp/a\u{0}b")] {
        let params = json!({"link_id": LINK, "payload": good, "result_path": result_path});
        let err = client.call("link.open", params).unwrap_err();
        assert!(matches!(err, LinkError::PayloadInvalid(_)), "{result_path}: {err:?}");
    }
    let climbing = json!({"OTIO_SCHEMA": "ExternalReference.1", "target_url": "file:///tmp/../etc/passwd"}).to_string();
    let err = client.call("link.open", open_params(json!({"format": "otio-json", "data": climbing}))).unwrap_err();
    assert!(matches!(err, LinkError::PayloadInvalid(_)), "{err:?}");
    // Params that are not an object are still bad_params.
    let err = client.call("link.open", json!("nope")).unwrap_err();
    assert!(matches!(err, LinkError::BadParams(_)), "{err:?}");
    assert!(sink.seen.lock().unwrap().is_empty(), "a bad payload must not reach the app: {:?}", sink.seen.lock().unwrap());

    let otio = open_params(json!({"format": "otio-json", "data": "{\"OTIO_SCHEMA\":\"Timeline.1\"}"}));
    let still = open_params(json!({"format": "image-doc", "path": "/tmp/a.photocraft", "asset": ASSET}));
    for params in [otio, still] {
        let reply = client.call("link.open", params.clone()).unwrap();
        assert_eq!(reply, json!({"comp": "c1"}));
        let seen = sink.seen.lock().unwrap();
        let (method, got) = seen.last().unwrap();
        assert_eq!(method, "link.open");
        assert_eq!(got, &params, "the app gets the request unchanged");
    }
    assert_eq!(sink.seen.lock().unwrap().len(), 2);

    // The wire carries the contract code, not bad_params.
    let err = client.call("link.open", open_params(json!({"format": "svg"}))).unwrap_err();
    assert_eq!(err.code(), "payload_invalid");
    // The connection stays usable after a refusal.
    client.call("link.open", open_params(json!({"format": "otio-json", "data": "{}"}))).unwrap();
    assert_eq!(sink.seen.lock().unwrap().len(), 3);

    server.shutdown().unwrap();
}
