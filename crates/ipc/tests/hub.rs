//! A hub is a peer. Two fake apps share one runtime directory:
//! `photocraft`, which owns a still and answers the asset methods, and `hub`,
//! which owns nothing, writes `caps: ["hub"]` and receives `app.activate`
//! with `open:{asset}`. A third party finds both through discovery.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use craft_ipc::asset::{Described, Refreshed, Resolved};
use craft_ipc::client::{Client, HelloClient};
use craft_ipc::moniker::Moniker;
use craft_ipc::record::AssetKind;
use craft_ipc::refresh::{Tracker, write_atomic};
use craft_ipc::server::{Activate, Activation, Assets, ControlSink, Extensions, FrameOpen, Frames, Server, ServerConfig};
use craft_ipc::{Discovery, InstanceRecord, LinkError, RuntimeDir};

const DOC: &str = "9f8e7d6c-5b4a-4c3d-8e2f-1a0b9c8d7e6f";
const LINK: &str = "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let mixed = (nanos as u64) ^ u64::from(std::process::id()).wrapping_mul(0x51ED);
        let path = PathBuf::from(format!("/tmp/cl-hub-{mixed:08x}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct RecordingSink {
    seen: Mutex<Vec<String>>,
}

impl ControlSink for RecordingSink {
    fn submit(&self, method: String, _params: Value) -> Option<std::sync::mpsc::Receiver<Value>> {
        self.seen.lock().unwrap().push(method);
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(json!({"forwarded": true}));
        Some(rx)
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

#[derive(Default)]
struct RecordingActivate {
    opens: Mutex<Vec<Option<Value>>>,
}

impl Activate for RecordingActivate {
    fn activate(&self, _token: Option<String>, _from_pid: Option<u32>, open: Option<Value>) -> Activation {
        self.opens.lock().unwrap().push(open);
        Activation::Requested
    }
}

/// A producer with one still. The real file watcher answers `link.refresh`.
struct Stills {
    asset: Moniker,
    file: PathBuf,
    thumb: PathBuf,
    tracker: Mutex<Tracker>,
    links: HashMap<String, ()>,
}

impl Assets for Stills {
    fn resolve(&self, asset: &Moniker) -> Result<Resolved, LinkError> {
        self.known(asset)?;
        let fp = craft_ipc::refresh::fingerprint_if_present(&self.file)?.ok_or_else(|| LinkError::Unknown("no result file yet".into()))?;
        let epoch = self.tracker.lock().unwrap().epoch();
        Ok(Resolved::from_fingerprint(asset.clone(), &self.file.display().to_string(), epoch, &fp))
    }

    fn describe(&self, asset: &Moniker) -> Result<Described, LinkError> {
        self.known(asset)?;
        let fp = craft_ipc::refresh::fingerprint_if_present(&self.file)?;
        Ok(Described {
            asset: asset.clone(),
            name: "Poster".into(),
            kind: AssetKind::Still,
            epoch: self.tracker.lock().unwrap().epoch(),
            w: Some(640),
            h: Some(480),
            hash: fp.as_ref().map(|f| f.hash.clone()),
            thumb_path: Some(self.thumb.display().to_string()),
            modified_ms: fp.map(|f| f.mtime_ms),
        })
    }

    fn refresh(&self, link_id: &str) -> Result<Refreshed, LinkError> {
        if !self.links.contains_key(link_id) {
            return Err(LinkError::Unknown(format!("link {link_id} is not known")));
        }
        let mut tracker = self.tracker.lock().unwrap();
        let change = tracker.poll(true)?;
        Ok(Refreshed::new(link_id, change.epoch, change.state, &self.file.display().to_string(), tracker.seen()))
    }
}

impl Stills {
    fn known(&self, asset: &Moniker) -> Result<(), LinkError> {
        if *asset == self.asset { Ok(()) } else { Err(LinkError::Unknown(format!("{asset} is not an asset of this document"))) }
    }
}

fn config(app: &str, runtime: &RuntimeDir, sink: Arc<RecordingSink>, activate: Arc<RecordingActivate>) -> ServerConfig {
    ServerConfig {
        app: app.into(),
        app_version: "0.1.0".into(),
        instance: None,
        runtime: Some(runtime.clone()),
        tcp_port: None,
        sink,
        activate,
        frames: Arc::new(NoFrames),
    }
}

fn dial(runtime: &RuntimeDir, app: &str) -> (Client, InstanceRecord) {
    let found = Discovery::scan(runtime).unwrap();
    let record = found.into_iter().find(|r| r.app == app).unwrap_or_else(|| panic!("no live {app} record"));
    let mut client = Client::connect(&record).unwrap();
    let identity = HelloClient {
        app: "filmcraft".into(),
        app_version: "0.9.1".into(),
        pid: std::process::id(),
        instance: "550e8400-e29b-41d4-a716-446655440000".into(),
        protocol_min: 1,
        protocol_max: 1,
    };
    let hello = client.hello(&identity).unwrap();
    assert_eq!(hello["app"], json!(app));
    (client, record)
}

fn caps_of(hello_or_record: &Value) -> Vec<String> {
    hello_or_record["caps"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect()
}

/// The two fake apps of the hub test and the files behind them.
struct World {
    runtime: RuntimeDir,
    asset: Moniker,
    file: PathBuf,
    producer: Server,
    producer_sink: Arc<RecordingSink>,
    hub: Server,
    hub_activate: Arc<RecordingActivate>,
}

fn start_world(tmp: &Tmp) -> World {
    let runtime = RuntimeDir::at(tmp.0.join("rt")).unwrap();
    let asset = Moniker::parse(&format!("craft://photocraft/{DOC}/image/flattened")).unwrap();
    let file = tmp.0.join("flat.png");
    write_atomic(&file, b"png-v1").unwrap();
    let stills = Arc::new(Stills {
        asset: asset.clone(),
        file: file.clone(),
        thumb: tmp.0.join("thumb.png"),
        tracker: Mutex::new(Tracker::new(&file, 3).unwrap()),
        links: HashMap::from([(LINK.to_string(), ())]),
    });

    let producer_sink = Arc::new(RecordingSink::default());
    let producer = Server::start_with(
        config("photocraft", &runtime, Arc::clone(&producer_sink), Arc::new(RecordingActivate::default())),
        Extensions { assets: Some(stills), caps: Vec::new() },
    )
    .unwrap();
    let hub_activate = Arc::new(RecordingActivate::default());
    let hub = Server::start_with(
        config("hub", &runtime, Arc::new(RecordingSink::default()), Arc::clone(&hub_activate)),
        Extensions { assets: None, caps: vec!["hub".into()] },
    )
    .unwrap();
    World { runtime, asset, file, producer, producer_sink, hub, hub_activate }
}

/// Discovery: the hub is an ordinary record with app "hub" and caps ["hub"]. `link.hello`
/// advertises the base caps plus the record caps.
fn check_discovery_and_hello(world: &World, hub_client: &mut Client, hub_record: &InstanceRecord, producer_client: &mut Client) {
    let live = Discovery::scan(&world.runtime).unwrap();
    let mut names: Vec<&str> = live.iter().map(|r| r.app.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["hub", "photocraft"]);
    assert_eq!(world.hub.record().caps(), vec!["hub".to_string()]);
    assert_eq!(world.producer.record().caps(), vec!["assets".to_string()]);

    let hello = hub_client.call("link.hello", json!({"protocol": {"min": 1, "max": 1}})).unwrap();
    assert_eq!(hello["app"], json!("hub"));
    assert!(caps_of(&hello).contains(&"hub".to_string()), "{hello}");
    assert!(caps_of(&hello).contains(&"activate".to_string()), "{hello}");
    assert_eq!(hub_record.docs.len(), 0, "a hub owns no document");

    let hello = producer_client.call("link.hello", json!({"protocol": {"min": 1, "max": 1}})).unwrap();
    assert!(caps_of(&hello).contains(&"assets".to_string()), "{hello}");
}

/// The hub browses the producer with no document open. Returns the hash of the first version.
fn check_browse(world: &World, producer_client: &mut Client) -> String {
    let asset = &world.asset;
    let described = producer_client.call("asset.describe", json!({"asset": asset.to_string()})).unwrap();
    assert_eq!(described["name"], json!("Poster"));
    assert_eq!(described["kind"], json!("still"));
    assert_eq!(described["epoch"], json!(3));
    assert_eq!(described["w"], json!(640));
    assert_eq!(described["asset"], json!(asset.to_string()));
    let resolved = producer_client.call("link.resolve", json!({"asset": asset.to_string()})).unwrap();
    assert_eq!(resolved["path"], json!(world.file.display().to_string()));
    assert_eq!(resolved["size"], json!(6));
    let v1_hash = resolved["hash"].as_str().unwrap().to_string();
    assert_eq!(described["hash"], json!(v1_hash));
    assert!(v1_hash.starts_with("sha256:") && v1_hash.len() == 71, "{v1_hash}");
    assert!(world.producer_sink.seen.lock().unwrap().is_empty(), "asset methods must not reach the UI sink");
    v1_hash
}

/// Errors map to the protocol's error codes.
fn check_error_codes(producer_client: &mut Client) {
    let other = format!("craft://photocraft/{DOC}/image/other");
    assert!(matches!(producer_client.call("asset.describe", json!({"asset": other})), Err(LinkError::Unknown(_))));
    assert!(matches!(producer_client.call("link.resolve", json!({"asset": "craft://nope"})), Err(LinkError::BadParams(_))));
    assert!(matches!(producer_client.call("link.resolve", json!({})), Err(LinkError::BadParams(_))));
    assert!(matches!(producer_client.call("link.refresh", json!({"link_id": ""})), Err(LinkError::BadParams(_))));
    assert!(matches!(producer_client.call("link.refresh", json!({"link_id": "7b1e0000-0000-4000-8000-000000000000"})), Err(LinkError::Unknown(_))));
}

/// `link.refresh` sees a new version on disk and bumps the epoch once.
fn check_refresh(world: &World, producer_client: &mut Client, v1_hash: &str) {
    let file = &world.file;
    let current = producer_client.call("link.refresh", json!({"link_id": LINK})).unwrap();
    assert_eq!(current["state"], json!("current"));
    assert_eq!(current["epoch"], json!(3));
    write_atomic(file, b"png-version-two").unwrap();
    let modified = producer_client.call("link.refresh", json!({"link_id": LINK})).unwrap();
    assert_eq!(modified["state"], json!("modified"));
    assert_eq!(modified["epoch"], json!(4));
    assert_ne!(modified["hash"], json!(v1_hash));
    let again = producer_client.call("link.refresh", json!({"link_id": LINK})).unwrap();
    assert_eq!(again["epoch"], json!(4), "no second bump without a new edit");
    std::fs::remove_file(file).unwrap();
    let missing = producer_client.call("link.refresh", json!({"link_id": LINK})).unwrap();
    assert_eq!(missing["state"], json!("missing"));
    assert!(missing.get("hash").is_none() && missing.get("result_path").is_none(), "{missing}");
    assert!(matches!(producer_client.call("link.resolve", json!({"asset": world.asset.to_string()})), Err(LinkError::Unknown(_))));
}

/// `app.activate` carries open:{asset} to the hub's Activate.
fn check_activate(world: &World, hub_client: &mut Client) {
    let asset = &world.asset;
    let reply = hub_client.call("app.activate", json!({"token": null, "from_pid": 4242, "open": {"asset": asset.to_string()}})).unwrap();
    assert_eq!(reply["activated"], json!("requested"));
    let reply = hub_client.call("app.activate", json!({"open": {"link_id": LINK, "asset": asset.to_string()}})).unwrap();
    assert_eq!(reply["activated"], json!("requested"));
    // Legacy shapes still pass through unchanged.
    hub_client.call("app.activate", json!({"open": {"link_id": LINK}})).unwrap();
    hub_client.call("app.activate", json!({})).unwrap();
    // A bad moniker never reaches the app.
    assert!(matches!(hub_client.call("app.activate", json!({"open": {"asset": "craft://bad"}})), Err(LinkError::BadParams(_))));
    assert!(matches!(hub_client.call("app.activate", json!({"open": {"asset": 7}})), Err(LinkError::BadParams(_))));
    let opens = world.hub_activate.opens.lock().unwrap().clone();
    assert_eq!(
        opens,
        vec![
            Some(json!({"asset": asset.to_string()})),
            Some(json!({"link_id": LINK, "asset": asset.to_string()})),
            Some(json!({"link_id": LINK})),
            None,
        ]
    );
}

#[test]
fn hub_is_a_peer_and_producer_answers_asset_methods() {
    let tmp = Tmp::new();
    let world = start_world(&tmp);
    let (mut hub_client, hub_record) = dial(&world.runtime, "hub");
    let (mut producer_client, _) = dial(&world.runtime, "photocraft");

    check_discovery_and_hello(&world, &mut hub_client, &hub_record, &mut producer_client);
    let v1_hash = check_browse(&world, &mut producer_client);
    check_error_codes(&mut producer_client);
    check_refresh(&world, &mut producer_client, &v1_hash);
    check_activate(&world, &mut hub_client);

    world.producer.shutdown().unwrap();
    world.hub.shutdown().unwrap();
}

#[test]
fn server_without_a_provider_forwards_asset_methods_to_the_app() {
    let tmp = Tmp::new();
    let runtime = RuntimeDir::at(tmp.0.join("rt")).unwrap();
    let sink = Arc::new(RecordingSink::default());
    let server = Server::start(config("photocraft", &runtime, Arc::clone(&sink), Arc::new(RecordingActivate::default()))).unwrap();
    assert!(server.record().caps().is_empty());
    let (mut client, _) = dial(&runtime, "photocraft");
    let asset = format!("craft://photocraft/{DOC}/image/x");
    for method in ["link.resolve", "asset.describe", "link.refresh"] {
        let reply = client.call(method, json!({"asset": asset, "link_id": LINK})).unwrap();
        assert_eq!(reply, json!({"forwarded": true}), "{method}");
    }
    assert_eq!(sink.seen.lock().unwrap().as_slice(), ["link.resolve", "asset.describe", "link.refresh"]);
    server.shutdown().unwrap();
}
