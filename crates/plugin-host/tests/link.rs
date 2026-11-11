//! `link.call` and `link.apps` against real `craft_ipc::server::Server` peers in a temporary
//! runtime directory, reached through `LinkClient` by a wasmi guest (`tests/fixtures/relay.wat`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::services::*;
use common::*;
use craft_command::Owner;
use craft_ipc::server::{Activate, Activation, ControlSink, FrameOpen, Frames, Server, ServerConfig};
use craft_ipc::{LinkError, RuntimeDir};
use craft_plugin_abi::{Op, Permission, Status};
use craft_plugin_host::dispatch::{DispatchConfig, HostDispatcher, PluginContext};
use craft_plugin_host::engine::{EngineId, HostError};
use craft_plugin_host::invoke::CallError;
use craft_plugin_host::lifecycle::{Decision, InstallOutcome};
use craft_plugin_host::link::LinkClient;
use craft_plugin_host::services::LinkFailure;
use craft_plugin_host::store::{MemoryStore, Source};
use serde_json::{Value, json};

struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The app side of a peer. It records every method it is given and answers with `reply`.
/// With `hang` set it never answers, so a caller can be cancelled.
struct Sink {
    seen: Mutex<Vec<(String, Value)>>,
    reply: Mutex<Value>,
    hang: AtomicBool,
    held: Mutex<Vec<Sender<Value>>>,
}

impl Sink {
    fn new(reply: Value) -> Arc<Sink> {
        Arc::new(Sink { seen: Mutex::new(Vec::new()), reply: Mutex::new(reply), hang: AtomicBool::new(false), held: Mutex::new(Vec::new()) })
    }

    fn seen(&self) -> Vec<(String, Value)> {
        self.seen.lock().unwrap().clone()
    }
}

impl ControlSink for Sink {
    fn submit(&self, method: String, params: Value) -> Option<Receiver<Value>> {
        self.seen.lock().unwrap().push((method, params));
        let (tx, rx) = channel();
        if self.hang.load(Ordering::Relaxed) {
            self.held.lock().unwrap().push(tx);
        } else {
            tx.send(self.reply.lock().unwrap().clone()).unwrap();
        }
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
        Err(LinkError::NotReady("no frames in this test".into()))
    }
    fn render(&self, _ticket: &str, _header: &Value) -> Result<(Value, Vec<u8>), LinkError> {
        Err(LinkError::NotReady("no frames in this test".into()))
    }
    fn close_ticket(&self, _ticket: &str) {}
}

fn runtime() -> (Tmp, RuntimeDir) {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = PathBuf::from(format!("/tmp/phl-{:08x}", (nanos as u64) ^ u64::from(std::process::id()).wrapping_mul(0x9E37)));
    let _ = std::fs::remove_dir_all(&path);
    let dir = RuntimeDir::at(&path).unwrap();
    (Tmp(path), dir)
}

fn peer(dir: &RuntimeDir, app: &str, version: &str, sink: &Arc<Sink>) -> Server {
    Server::start(ServerConfig {
        app: app.into(),
        app_version: version.into(),
        instance: None,
        runtime: Some(dir.clone()),
        tcp_port: None,
        sink: sink.clone() as Arc<dyn ControlSink>,
        activate: Arc::new(NoActivate),
        frames: Arc::new(NoFrames),
    })
    .unwrap()
}

const RELAY: &str = "org.example.relay";

fn relay_manifest(required: &[&str], optional: &[&str]) -> Value {
    json!({
        "schema": 1, "id": RELAY, "name": "Relay", "version": "1.0.0",
        "abi": { "craft": 1 },
        "requires": { "ops": ["link.call", "link.apps"] },
        "permissions": { "required": required, "optional": optional },
        "limits": { "memory_mb": 4, "stack_kb": 128 },
        "contributions": [
            { "kind": "command", "id": "call", "entry": 0, "budget": "batch" },
            { "kind": "command", "id": "apps", "entry": 1, "budget": "batch" }
        ]
    })
}

/// A harness whose services send link traffic to the temporary runtime directory, as the
/// `filmcraft` app.
fn harness(dir: &RuntimeDir) -> Harness {
    let h = Harness::new();
    *h.services.link_client.lock().unwrap() = Some(LinkClient::for_app(dir.clone(), "filmcraft", "0.9.1").unwrap());
    h
}

fn install_relay(h: &Harness, required: &[&str], optional: &[&str], accept_optional: bool) {
    let bytes = package_from(&fixture_text("relay"), &relay_manifest(required, optional));
    match h.manager.install(&bytes, Source::File).unwrap() {
        InstallOutcome::NeedsGrant(req) => {
            let optional = if accept_optional { req.optional } else { vec![] };
            h.manager.grant(RELAY, Decision { accept_required: true, optional }).unwrap();
        }
        InstallOutcome::Loaded { .. } => {}
        other => panic!("{other:?}"),
    }
}

/// Run entry 0 (`link.call`) or 1 (`link.apps`) with `request` as the host call input.
fn relay(h: &Harness, entry: usize, request: &Value) -> Result<Value, CallError> {
    let generation = h.manager.router().current(RELAY).expect("the relay plugin is loaded");
    let c = generation.manifest.contributions[entry].clone();
    generation.call(&c, request, &[], 0)
}

/// `engine.execute` needs this next to `link.call`.
const EXEC: &str = "link.exec:effectcraft:effect.*";

fn effect_apply() -> Value {
    json!({"app": "effectcraft", "method": "engine.execute", "params": {"command": "effect.apply", "params": {"effect": "blur"}}})
}

#[test]
fn a_granted_call_reaches_the_real_peer_and_its_reply_comes_back() {
    let (_tmp, dir) = runtime();
    let ec = Sink::new(json!({"applied": true, "comp": "c1"}));
    let _ec_server = peer(&dir, "effectcraft", "0.8.3", &ec);
    let h = harness(&dir);
    install_relay(&h, &["link.call:effectcraft", EXEC], &[], true);
    let out = relay(&h, 0, &effect_apply()).unwrap();
    assert_eq!(out, json!({"host": {"result": {"applied": true, "comp": "c1"}}}));
    assert_eq!(ec.seen(), vec![("engine.execute".to_string(), json!({"command": "effect.apply", "params": {"effect": "blur"}}))]);
    // The call ran inside the guest's begin/end pair and completed.
    h.services.recorded(|x| assert!(matches!(&x.ended[..], [(_, craft_plugin_host::services::CallOutcome::Completed)])));
}

#[test]
fn another_app_is_refused_with_the_missing_permission_and_the_peer_never_hears_of_it() {
    let (_tmp, dir) = runtime();
    let ec = Sink::new(json!({"done": 1}));
    let fc = Sink::new(json!({"done": 2}));
    let _s1 = peer(&dir, "effectcraft", "0.8.3", &ec);
    let _s2 = peer(&dir, "photocraft", "2.0.0", &fc);
    let h = harness(&dir);
    install_relay(&h, &["link.call:effectcraft", EXEC], &[], true);
    let refused = relay(&h, 0, &json!({"app": "photocraft", "method": "engine.execute"})).unwrap();
    assert_eq!(refused, json!({"host": {"error": "missing permission link.call:photocraft"}}));
    assert!(fc.seen().is_empty(), "photocraft was never contacted");
    // Control: the granted app answers.
    let second = relay(&h, 0, &effect_apply()).unwrap();
    assert_eq!(second["host"]["result"], json!({"done": 1}));
    assert!(fc.seen().is_empty());
}

#[test]
fn a_plugin_with_no_link_grant_gets_the_generic_denial_and_no_traffic() {
    let (_tmp, dir) = runtime();
    let ec = Sink::new(json!({"done": 1}));
    let _s = peer(&dir, "effectcraft", "0.8.3", &ec);
    let h = harness(&dir);
    // A `batch` command must ask for a link permission. This one asks as optional and the user declines.
    install_relay(&h, &[], &["link.call:effectcraft"], false);
    assert_eq!(relay(&h, 0, &effect_apply()).unwrap(), json!({"status": -1}), "E_DENIED");
    assert_eq!(relay(&h, 1, &json!({})).unwrap(), json!({"status": -1}));
    assert!(ec.seen().is_empty());
}

#[test]
fn revoking_the_grant_turns_the_same_call_into_the_missing_permission_error() {
    let (_tmp, dir) = runtime();
    let ec = Sink::new(json!({"applied": true}));
    let _s = peer(&dir, "effectcraft", "0.8.3", &ec);
    let h = harness(&dir);
    install_relay(&h, &[], &["link.call:effectcraft", EXEC], true);
    assert_eq!(relay(&h, 0, &effect_apply()).unwrap(), json!({"host": {"result": {"applied": true}}}));
    h.manager.revoke(RELAY, &Permission::LinkCall("effectcraft".into())).unwrap();
    // The kind-level check fails first once no link grant is left at all.
    assert_eq!(relay(&h, 0, &effect_apply()).unwrap(), json!({"status": -1}));
    assert_eq!(ec.seen().len(), 1, "only the call before the revoke reached the peer");
}

#[test]
fn an_app_that_is_not_running_is_a_clear_error() {
    let (_tmp, dir) = runtime();
    let h = harness(&dir);
    install_relay(&h, &["link.call:effectcraft", EXEC], &[], true);
    assert_eq!(relay(&h, 0, &effect_apply()).unwrap(), json!({"host": {"error": "effectcraft is not running"}}));
}

#[test]
fn a_peer_error_reaches_the_guest_as_text() {
    let (_tmp, dir) = runtime();
    let ec = Sink::new(json!({"ok": false, "error": "unknown command effect.nope"}));
    let _s = peer(&dir, "effectcraft", "0.8.3", &ec);
    let h = harness(&dir);
    install_relay(&h, &["link.call:effectcraft", EXEC], &[], true);
    let out = relay(&h, 0, &effect_apply()).unwrap();
    let text = out["host"]["error"].as_str().unwrap_or_else(|| panic!("{out}"));
    assert!(text.contains("unknown command effect.nope"), "{text}");
}

#[test]
fn link_apps_lists_the_running_apps_the_plugin_may_call_and_not_itself() {
    let (_tmp, dir) = runtime();
    let (s1, s2, s3) = (Sink::new(json!({})), Sink::new(json!({})), Sink::new(json!({})));
    let _a = peer(&dir, "effectcraft", "0.8.3", &s1);
    let _b = peer(&dir, "photocraft", "2.0.0", &s2);
    // The caller's own app is running in this process too and must not list itself.
    let _c = peer(&dir, "filmcraft", "0.9.1", &s3);
    let h = harness(&dir);
    install_relay(&h, &["link.call:effectcraft", EXEC, "link.call:filmcraft"], &[], true);
    let out = relay(&h, 1, &json!({})).unwrap();
    assert_eq!(
        out,
        json!({"host": {"apps": [{"app": "effectcraft", "app_version": "0.8.3"}]}}),
        "photocraft is not granted and filmcraft is the caller"
    );
}

#[test]
fn cancelling_a_waiting_call_returns_at_once_with_e_cancelled() {
    let (_tmp, dir) = runtime();
    let ec = Sink::new(json!({}));
    ec.hang.store(true, Ordering::Relaxed);
    let _s = peer(&dir, "effectcraft", "0.8.3", &ec);
    let services = TestServices::new();
    *services.link_client.lock().unwrap() = Some(LinkClient::for_app(dir.clone(), "filmcraft", "0.9.1").unwrap());
    let cancel = Arc::new(AtomicBool::new(false));
    let ctx = PluginContext {
        owner: Owner { plugin_id: Arc::from(RELAY), generation: 1 },
        declared_ops: HashSet::from([Op::LinkCall]),
        grants: vec![Permission::LinkCall("effectcraft".into()), EXEC.parse().unwrap()],
        engine: EngineId::Wasmi,
        random_seed: 1,
    };
    let d =
        HostDispatcher::new(ctx, services, Arc::new(MemoryStore::new()), StepClock::new(Duration::ZERO), cancel.clone(), DispatchConfig::default());
    let canceller = {
        let (cancel, ec) = (cancel.clone(), ec.clone());
        std::thread::spawn(move || {
            // Wait until the peer holds the request, so the cancel lands on a call in flight.
            let start = Instant::now();
            while ec.seen().is_empty() && start.elapsed() < Duration::from_secs(10) {
                std::thread::sleep(Duration::from_millis(5));
            }
            cancel.store(true, Ordering::Relaxed);
        })
    };
    let started = Instant::now();
    let mut out = vec![0u8; 4096];
    let err = d.dispatch(Op::LinkCall, &serde_json::to_vec(&effect_apply()).unwrap(), &mut out).unwrap_err();
    canceller.join().unwrap();
    assert!(matches!(err, HostError::Status(Status::Cancelled)), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(5), "returned after {:?}, not at the 60 s reply deadline", started.elapsed());
    assert_eq!(ec.seen().len(), 1, "the peer did receive the request");
}

#[test]
fn the_client_reports_a_missing_app_and_a_cancel_before_the_call() {
    let (_tmp, dir) = runtime();
    let client = LinkClient::for_app(dir, "filmcraft", "0.9.1").unwrap();
    let flag = AtomicBool::new(false);
    assert_eq!(client.call("effectcraft", "link.resolve", &json!({}), &flag), Err(LinkFailure::NotRunning("effectcraft is not running".into())));
    flag.store(true, Ordering::Relaxed);
    assert_eq!(client.call("effectcraft", "link.resolve", &json!({}), &flag), Err(LinkFailure::Cancelled));
    assert_eq!(client.apps(), Ok(vec![]));
}
