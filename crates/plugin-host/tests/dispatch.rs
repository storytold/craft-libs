//! `HostDispatcher` tests. Each rule has a case that must fail when the rule is removed:
//! the denial tests check that the service was never reached, the retry test counts executions,
//! and the rate limit test shows the refill.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use common::services::TestServices;
use common::*;
use craft_command::Owner;
use craft_plugin_abi::ops::unpack_call_return;
use craft_plugin_abi::{Op, Permission, Status};
use craft_plugin_host::dispatch::{DispatchConfig, HostDispatcher, PluginContext};
use craft_plugin_host::engine::wasmi::WasmiEngine;
use craft_plugin_host::engine::{Clock, EngineId, Export, HostError, Instance, Trap};
use craft_plugin_host::services::{CommandFailure, LinkApp, LinkFailure, LogLevel, QueryFailure};
use craft_plugin_host::store::{MemoryStore, StateStore};
use serde_json::{Value, json};

struct ManualClock(AtomicU64);

impl ManualClock {
    fn advance(&self, d: Duration) {
        self.0.fetch_add(d.as_nanos() as u64, Ordering::Relaxed);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_nanos(self.0.load(Ordering::Relaxed))
    }
}

struct Rig {
    d: Arc<HostDispatcher>,
    services: Arc<TestServices>,
    store: Arc<MemoryStore>,
    clock: Arc<ManualClock>,
    cancel: Arc<AtomicBool>,
}

fn perms(list: &[&str]) -> Vec<Permission> {
    list.iter().map(|s| s.parse().unwrap()).collect()
}

fn rig_with(id: &str, ops: &[Op], grants: &[&str], store: Arc<MemoryStore>, cfg: DispatchConfig, seed: u64) -> Rig {
    let services = TestServices::new();
    let clock = Arc::new(ManualClock(AtomicU64::new(0)));
    let cancel = Arc::new(AtomicBool::new(false));
    let ctx = PluginContext {
        owner: Owner { plugin_id: Arc::from(id), generation: 7 },
        declared_ops: ops.iter().copied().collect::<HashSet<_>>(),
        grants: perms(grants),
        engine: EngineId::Wasmi,
        random_seed: seed,
    };
    let d = HostDispatcher::new(ctx, services.clone(), store.clone(), clock.clone(), cancel.clone(), cfg);
    Rig { d, services, store, clock, cancel }
}

fn rig(ops: &[Op], grants: &[&str]) -> Rig {
    rig_with("org.example.t", ops, grants, Arc::new(MemoryStore::new()), DispatchConfig::default(), 1)
}

/// Call with a roomy buffer and parse the JSON answer.
fn call(r: &Rig, op: Op, req: &Value) -> Result<Value, HostError> {
    let mut out = vec![0u8; 8192];
    let n = r.d.dispatch(op, &serde_json::to_vec(req).unwrap(), &mut out)?;
    Ok(serde_json::from_slice(&out[..n]).unwrap())
}

fn status_of(e: HostError) -> Status {
    match e {
        HostError::Status(s) => s,
        other => panic!("expected a status, got {other:?}"),
    }
}

#[test]
fn an_op_missing_from_requires_ops_is_denied_even_with_the_permission() {
    let r = rig(&[Op::HostLog], &["commands.run"]);
    let err = call(&r, Op::CmdExecute, &json!({"id": "app.set"})).unwrap_err();
    assert_eq!(status_of(err), Status::Denied);
    assert!(r.services.recorded(|x| x.executed.is_empty()), "the service must not be reached");
    // Control: declare it and the same call runs.
    let r = rig(&[Op::HostLog, Op::CmdExecute], &["commands.run"]);
    call(&r, Op::CmdExecute, &json!({"id": "app.set"})).unwrap();
    assert_eq!(r.services.recorded(|x| x.executed.len()), 1);
}

#[test]
fn a_declared_op_without_its_permission_is_denied() {
    let r = rig(&[Op::CmdList, Op::DocQuery, Op::StateGet, Op::StatePut, Op::NetFetch], &[]);
    for (op, req) in [
        (Op::CmdList, json!({})),
        (Op::DocQuery, json!({})),
        (Op::StateGet, json!({"key": "k"})),
        (Op::StatePut, json!({"key": "k", "value": 1})),
        (Op::NetFetch, json!({})),
    ] {
        assert_eq!(status_of(call(&r, op, &req).unwrap_err()), Status::Denied, "{op:?}");
    }
    assert!(r.services.recorded(|x| x.doc_queries.is_empty()));
    assert_eq!(r.store.get("org.example.t", "k").unwrap(), None);
}

#[test]
fn cmd_list_shows_only_commands_the_glob_allows() {
    let r = rig(&[Op::CmdList], &["commands.run:app.*"]);
    let v = call(&r, Op::CmdList, &json!({})).unwrap();
    let ids: Vec<&str> = v["commands"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["app.set", "app.delete"]);
    let all = rig(&[Op::CmdList], &["commands.run"]);
    assert_eq!(call(&all, Op::CmdList, &json!({})).unwrap()["commands"].as_array().unwrap().len(), 3);
}

#[test]
fn cmd_execute_checks_the_glob_per_call_and_runs_as_the_plugin_owner() {
    let r = rig(&[Op::CmdExecute], &["commands.run:app.*"]);
    let v = call(&r, Op::CmdExecute, &json!({"id": "app.set", "params": {"x": 1}})).unwrap();
    assert_eq!(v, json!({"result": {"done": true}}));
    r.services.recorded(|x| assert_eq!(x.executed, vec![("org.example.t".to_string(), 7, "app.set".to_string(), json!({"x": 1}))]));
    // A command outside the glob is denied and never executed.
    assert_eq!(status_of(call(&r, Op::CmdExecute, &json!({"id": "other.thing"})).unwrap_err()), Status::Denied);
    assert_eq!(r.services.recorded(|x| x.executed.len()), 1);
    // Malformed requests.
    assert_eq!(status_of(call(&r, Op::CmdExecute, &json!({"nope": 1})).unwrap_err()), Status::BadArg);
    let mut out = [0u8; 16];
    assert_eq!(status_of(r.d.dispatch(Op::CmdExecute, b"{not json", &mut out).unwrap_err()), Status::BadArg);
}

#[test]
fn cmd_execute_maps_service_failures() {
    let r = rig(&[Op::CmdExecute], &["commands.run"]);
    *r.services.exec_result.lock().unwrap() = Err(CommandFailure::Unknown);
    assert_eq!(status_of(call(&r, Op::CmdExecute, &json!({"id": "app.nope"})).unwrap_err()), Status::BadArg);
    *r.services.exec_result.lock().unwrap() = Err(CommandFailure::Failed("layer is locked".into()));
    assert_eq!(call(&r, Op::CmdExecute, &json!({"id": "app.set"})).unwrap(), json!({"error": "layer is locked"}));
}

#[test]
fn too_small_reports_the_exact_size_and_the_retry_fits() {
    let r = rig(&[Op::HostInfo], &[]);
    let mut tiny = [0u8; 4];
    let required = match r.d.dispatch(Op::HostInfo, b"", &mut tiny) {
        Err(HostError::TooSmall { required }) => required,
        other => panic!("{other:?}"),
    };
    assert!(required > 4);
    let mut exact = vec![0u8; required];
    assert_eq!(r.d.dispatch(Op::HostInfo, b"", &mut exact).unwrap(), required, "an exact buffer fits");
    let v: Value = serde_json::from_slice(&exact).unwrap();
    assert_eq!(v["app"], "testapp");
    assert_eq!(v["plugin"], json!({"id": "org.example.t", "generation": 7}));
    assert_eq!(v["ops"], json!(["host.info"]));
    let mut short = vec![0u8; required - 1];
    assert!(matches!(r.d.dispatch(Op::HostInfo, b"", &mut short), Err(HostError::TooSmall { required: n }) if n == required));
}

#[test]
fn a_retry_after_too_small_does_not_run_the_side_effect_twice() {
    let r = rig(&[Op::CmdExecute], &["commands.run"]);
    let req = serde_json::to_vec(&json!({"id": "app.set", "params": {}})).unwrap();
    let mut tiny = [0u8; 1];
    let required = match r.d.dispatch(Op::CmdExecute, &req, &mut tiny) {
        Err(HostError::TooSmall { required }) => required,
        other => panic!("{other:?}"),
    };
    assert_eq!(r.services.recorded(|x| x.executed.len()), 1, "the first call ran the command");
    let mut big = vec![0u8; required];
    assert_eq!(r.d.dispatch(Op::CmdExecute, &req, &mut big).unwrap(), required);
    assert_eq!(r.services.recorded(|x| x.executed.len()), 1, "the retry replayed the answer");
    // A later identical call is a new call and runs again.
    r.d.dispatch(Op::CmdExecute, &req, &mut big).unwrap();
    assert_eq!(r.services.recorded(|x| x.executed.len()), 2);
    // A different request after a too-small call does not get the stored answer.
    let other = serde_json::to_vec(&json!({"id": "app.delete"})).unwrap();
    assert!(r.d.dispatch(Op::CmdExecute, &req, &mut tiny).is_err());
    r.d.dispatch(Op::CmdExecute, &other, &mut big).unwrap();
    r.services.recorded(|x| assert_eq!(x.executed.last().unwrap().2, "app.delete"));
}

#[test]
fn state_put_retry_after_too_small_writes_once() {
    let r = rig(&[Op::StatePut, Op::StateGet], &["state.local"]);
    let put = serde_json::to_vec(&json!({"key": "n", "value": 1})).unwrap();
    let mut tiny = [0u8; 1];
    assert!(matches!(r.d.dispatch(Op::StatePut, &put, &mut tiny), Err(HostError::TooSmall { .. })));
    // Another writer changes the key between the two calls. A replay must not clobber it.
    r.store.put("org.example.t", "n", &json!(99), 1000).unwrap();
    let mut big = [0u8; 64];
    r.d.dispatch(Op::StatePut, &put, &mut big).unwrap();
    assert_eq!(r.store.get("org.example.t", "n").unwrap(), Some(json!(99)), "the replay did not write again");
}

#[test]
fn host_log_is_rate_limited_refills_and_reports_drops() {
    let cfg = DispatchConfig { log_burst: 3, log_per_second: 2, ..DispatchConfig::default() };
    let r = rig_with("p", &[Op::HostLog], &[], Arc::new(MemoryStore::new()), cfg, 1);
    let log = |m: &str| call(&r, Op::HostLog, &json!({"msg": m}));
    for i in 0..3 {
        log(&format!("line {i}")).unwrap();
    }
    assert_eq!(status_of(log("over").unwrap_err()), Status::Limit);
    assert_eq!(status_of(log("over again").unwrap_err()), Status::Limit);
    assert_eq!(r.services.recorded(|x| x.logs.len()), 3, "dropped lines never reach the app");
    r.clock.advance(Duration::from_secs(1)); // two tokens
    log("after refill").unwrap();
    r.services.recorded(|x| {
        let texts: Vec<&str> = x.logs.iter().map(|l| l.2.as_str()).collect();
        assert_eq!(texts, vec!["line 0", "line 1", "line 2", "2 log lines were dropped by the rate limit", "after refill"]);
        assert_eq!(x.logs[3].1, LogLevel::Warn);
    });
}

#[test]
fn host_log_levels_bad_args_and_truncation() {
    let cfg = DispatchConfig { max_log_bytes: 5, ..DispatchConfig::default() };
    let r = rig_with("p", &[Op::HostLog], &[], Arc::new(MemoryStore::new()), cfg, 1);
    call(&r, Op::HostLog, &json!({"level": "error", "msg": "a\u{e9}\u{e9}\u{e9}"})).unwrap(); // 7 bytes, cut inside a character
    r.services.recorded(|x| assert_eq!(x.logs[0], ("p".to_string(), LogLevel::Error, "a\u{e9}\u{e9}".to_string())));
    for bad in [json!({"level": "loud", "msg": "x"}), json!({"level": 3, "msg": "x"}), json!({"level": "info"}), json!({"msg": 4})] {
        assert_eq!(status_of(call(&r, Op::HostLog, &bad).unwrap_err()), Status::BadArg, "{bad}");
    }
    assert_eq!(r.services.recorded(|x| x.logs.len()), 1);
}

#[test]
fn cancelled_and_progress_follow_the_cancel_flag() {
    let r = rig(&[Op::HostCancelled, Op::HostProgress], &[]);
    assert_eq!(call(&r, Op::HostCancelled, &json!({})).unwrap(), json!({"cancelled": false}));
    call(&r, Op::HostProgress, &json!({"fraction": 0.25, "message": "tile 1"})).unwrap();
    for bad in [json!({"fraction": 1.5}), json!({"fraction": -0.1}), json!({}), json!({"fraction": 0.5, "message": 3})] {
        assert_eq!(status_of(call(&r, Op::HostProgress, &bad).unwrap_err()), Status::BadArg, "{bad}");
    }
    r.cancel.store(true, Ordering::Relaxed);
    assert_eq!(call(&r, Op::HostCancelled, &json!({})).unwrap(), json!({"cancelled": true}));
    assert_eq!(status_of(call(&r, Op::HostProgress, &json!({"fraction": 0.5})).unwrap_err()), Status::Cancelled);
    r.services.recorded(|x| assert_eq!(x.progress, vec![(0.25, Some("tile 1".to_string())), (0.5, None)]));
}

#[test]
fn doc_query_checks_the_scope_it_is_asked_for() {
    let metadata_only = rig(&[Op::DocQuery], &["doc.read:metadata"]);
    assert_eq!(call(&metadata_only, Op::DocQuery, &json!({})).unwrap(), json!({"layers": 3}));
    assert_eq!(call(&metadata_only, Op::DocQuery, &json!({"scope": "metadata"})).unwrap(), json!({"layers": 3}));
    assert_eq!(status_of(call(&metadata_only, Op::DocQuery, &json!({"scope": "pixels"})).unwrap_err()), Status::Denied);
    assert_eq!(metadata_only.services.recorded(|x| x.doc_queries.len()), 2, "the denied query never reached the app");
    let pixels_only = rig(&[Op::DocQuery], &["doc.read:pixels"]);
    assert_eq!(status_of(call(&pixels_only, Op::DocQuery, &json!({})).unwrap_err()), Status::Denied);
    call(&pixels_only, Op::DocQuery, &json!({"scope": "pixels"})).unwrap();
    let both = rig(&[Op::DocQuery], &["doc.read"]);
    call(&both, Op::DocQuery, &json!({"scope": "pixels"})).unwrap();
    assert_eq!(status_of(call(&both, Op::DocQuery, &json!({"scope": "all"})).unwrap_err()), Status::BadArg);
    *both.services.doc_result.lock().unwrap() = Err(QueryFailure::Unsupported);
    assert_eq!(status_of(call(&both, Op::DocQuery, &json!({})).unwrap_err()), Status::Unsupported);
    *both.services.doc_result.lock().unwrap() = Err(QueryFailure::BadQuery("x".into()));
    assert_eq!(status_of(call(&both, Op::DocQuery, &json!({})).unwrap_err()), Status::BadArg);
}

#[test]
fn state_round_trips_and_the_quota_is_per_plugin_and_counts_overwrites_once() {
    let store = Arc::new(MemoryStore::new());
    let cfg = DispatchConfig { state_quota_bytes: 100, ..DispatchConfig::default() };
    let a = rig_with("a", &[Op::StateGet, Op::StatePut], &["state.local"], store.clone(), cfg.clone(), 1);
    let b = rig_with("b", &[Op::StateGet, Op::StatePut], &["state.local"], store.clone(), cfg, 1);
    let blob = "x".repeat(60);
    call(&a, Op::StatePut, &json!({"key": "k", "value": blob})).unwrap();
    assert_eq!(call(&a, Op::StateGet, &json!({"key": "k"})).unwrap(), json!({"found": true, "value": blob}));
    assert_eq!(call(&a, Op::StateGet, &json!({"key": "nope"})).unwrap(), json!({"found": false}));
    // Overwriting the same key with a same-size value stays inside the quota.
    call(&a, Op::StatePut, &json!({"key": "k", "value": "y".repeat(60)})).unwrap();
    // A second 60 byte key does not.
    assert_eq!(status_of(call(&a, Op::StatePut, &json!({"key": "k2", "value": blob})).unwrap_err()), Status::Limit);
    assert_eq!(call(&a, Op::StateGet, &json!({"key": "k2"})).unwrap(), json!({"found": false}), "a refused write stores nothing");
    // Plugin b has its own quota and cannot see a's keys.
    call(&b, Op::StatePut, &json!({"key": "k2", "value": blob})).unwrap();
    assert_eq!(call(&b, Op::StateGet, &json!({"key": "k"})).unwrap(), json!({"found": false}));
    // Key rules.
    for bad in [json!({"key": ""}), json!({"key": "k".repeat(257)}), json!({"value": 1})] {
        assert_eq!(status_of(call(&a, Op::StatePut, &bad).unwrap_err()), Status::BadArg, "{bad}");
    }
}

#[test]
fn random_bytes_fail_when_the_app_has_no_entropy() {
    let r = rig(&[Op::RandomBytes], &["random"]);
    r.services.entropy_fails.store(true, Ordering::Relaxed);
    // The guest must see an error. It must not see a buffer of zeros that looks random.
    assert_eq!(status_of(call(&r, Op::RandomBytes, &json!({"len": 4})).unwrap_err()), Status::Unsupported);
    // Control: without the failure the same call answers.
    r.services.entropy_fails.store(false, Ordering::Relaxed);
    assert_eq!(call(&r, Op::RandomBytes, &json!({"len": 4})).unwrap(), json!({"hex": "abababab"}));
}

#[test]
fn time_and_random_degrade_without_their_permission() {
    let none = rig(&[Op::TimeNow, Op::RandomBytes], &[]);
    assert_eq!(call(&none, Op::TimeNow, &json!({})).unwrap(), json!({"unix_ms": 0}));
    let some = rig(&[Op::TimeNow, Op::RandomBytes], &["clock", "random"]);
    assert_eq!(call(&some, Op::TimeNow, &json!({})).unwrap(), json!({"unix_ms": 1_700_000_000_000u64}));
    // With the grant the app supplies entropy (the test service fills 0xAB).
    assert_eq!(call(&some, Op::RandomBytes, &json!({"len": 4})).unwrap(), json!({"hex": "abababab"}));
    // Without it: a stream from the host seed. Same seed, same bytes. Other seed, other bytes.
    let a1 = call(&none, Op::RandomBytes, &json!({"len": 12})).unwrap();
    let a2 = call(
        &rig_with("x", &[Op::RandomBytes], &[], Arc::new(MemoryStore::new()), DispatchConfig::default(), 1),
        Op::RandomBytes,
        &json!({"len": 12}),
    )
    .unwrap();
    let b = call(
        &rig_with("x", &[Op::RandomBytes], &[], Arc::new(MemoryStore::new()), DispatchConfig::default(), 2),
        Op::RandomBytes,
        &json!({"len": 12}),
    )
    .unwrap();
    assert_eq!(a1, a2);
    assert_ne!(a1, b);
    assert_ne!(a1["hex"], json!("abababababababababababab"));
    // The stream advances: a second call gives new bytes.
    assert_ne!(call(&none, Op::RandomBytes, &json!({"len": 12})).unwrap(), a1);
    assert_eq!(status_of(call(&none, Op::RandomBytes, &json!({"len": 4097})).unwrap_err()), Status::Limit);
    assert_eq!(status_of(call(&none, Op::RandomBytes, &json!({})).unwrap_err()), Status::BadArg);
}

#[test]
fn listed_but_unserved_ops_say_unsupported_after_the_denial_checks() {
    let r = rig(&[Op::NetFetch, Op::EventPoll, Op::DocApply], &["net.fetch:https://example.org", "commands.observe", "doc.write"]);
    for op in [Op::NetFetch, Op::EventPoll, Op::DocApply] {
        assert_eq!(status_of(call(&r, op, &json!({})).unwrap_err()), Status::Unsupported, "{op:?}");
    }
    let no_grant = rig(&[Op::NetFetch], &[]);
    assert_eq!(status_of(call(&no_grant, Op::NetFetch, &json!({})).unwrap_err()), Status::Denied);
}

// ---- link.call and link.apps ---------------------------------------------------------

fn link_call_req(app: &str) -> Value {
    json!({"app": app, "method": "engine.execute", "params": {"command": "effect.apply"}})
}

#[test]
fn link_call_needs_the_per_app_grant_and_names_the_missing_permission() {
    let r = rig(&[Op::LinkCall], &["link.call:effectcraft", "link.exec:effectcraft:effect.*"]);
    assert_eq!(call(&r, Op::LinkCall, &link_call_req("effectcraft")).unwrap(), json!({"result": {"linked": true}}));
    r.services.recorded(|x| {
        assert_eq!(
            x.link_calls,
            vec![("org.example.t".into(), "effectcraft".into(), "engine.execute".into(), json!({"command": "effect.apply", "params": {}}))]
        );
    });
    // A granted app is not a granted suite: filmcraft is refused before the service runs.
    assert_eq!(call(&r, Op::LinkCall, &link_call_req("filmcraft")).unwrap(), json!({"error": "missing permission link.call:filmcraft"}));
    assert_eq!(r.services.recorded(|x| x.link_calls.len()), 1, "the refused call never reached the service");
    // No link grant at all is the generic denial, before any parsing.
    let none = rig(&[Op::LinkCall], &[]);
    assert_eq!(status_of(call(&none, Op::LinkCall, &link_call_req("effectcraft")).unwrap_err()), Status::Denied);
    assert_eq!(none.services.recorded(|x| x.link_calls.len()), 0);
    // And an op the manifest never listed.
    let unlisted = rig(&[Op::HostLog], &["link.call:effectcraft"]);
    assert_eq!(status_of(call(&unlisted, Op::LinkCall, &link_call_req("effectcraft")).unwrap_err()), Status::Denied);
}

#[test]
fn link_call_rejects_malformed_requests_before_the_service() {
    let r = rig(&[Op::LinkCall], &["link.call:effectcraft", "link.exec:effectcraft:effect.*"]);
    for bad in [
        json!({"method": "engine.execute"}),
        json!({"app": "effectcraft"}),
        json!({"app": "EffectCraft", "method": "engine.execute"}),
        json!({"app": "effectcraft", "method": "Engine.Execute"}),
        json!({"app": "effectcraft", "method": "engine execute"}),
        json!({"app": "effectcraft", "method": "engine.execute", "params": [1]}),
        json!({"app": "effectcraft", "method": "engine.execute", "params": "x"}),
        json!({"app": "effectcraft", "method": "e".repeat(65)}),
        json!({"app": 3, "method": "engine.execute"}),
    ] {
        assert_eq!(status_of(call(&r, Op::LinkCall, &bad).unwrap_err()), Status::BadArg, "{bad}");
    }
    assert_eq!(r.services.recorded(|x| x.link_calls.len()), 0);
    // Missing and null params both reach the peer as an empty object.
    call(&r, Op::LinkCall, &json!({"app": "effectcraft", "method": "link.resolve"})).unwrap();
    call(&r, Op::LinkCall, &json!({"app": "effectcraft", "method": "link.resolve", "params": null})).unwrap();
    r.services.recorded(|x| assert!(x.link_calls.iter().all(|c| c.3 == json!({}))));
}

#[test]
fn link_call_maps_service_failures() {
    let r = rig(&[Op::LinkCall], &["link.call:effectcraft", "link.exec:effectcraft:effect.*"]);
    *r.services.link_result.lock().unwrap() = Err(LinkFailure::NotRunning("effectcraft is not running".into()));
    assert_eq!(call(&r, Op::LinkCall, &link_call_req("effectcraft")).unwrap(), json!({"error": "effectcraft is not running"}));
    *r.services.link_result.lock().unwrap() = Err(LinkFailure::Failed("busy (busy)".into()));
    assert_eq!(call(&r, Op::LinkCall, &link_call_req("effectcraft")).unwrap(), json!({"error": "busy (busy)"}));
    *r.services.link_result.lock().unwrap() = Err(LinkFailure::Cancelled);
    assert_eq!(status_of(call(&r, Op::LinkCall, &link_call_req("effectcraft")).unwrap_err()), Status::Cancelled);
}

#[test]
fn link_apps_lists_only_apps_the_plugin_may_call() {
    let r = rig(&[Op::LinkApps], &["link.call:effectcraft", "link.call:photocraft"]);
    *r.services.link_apps_result.lock().unwrap() = Ok(vec![
        LinkApp { app: "effectcraft".into(), app_version: "0.8.3".into() },
        LinkApp { app: "filmcraft".into(), app_version: "0.9.1".into() },
        LinkApp { app: "photocraft".into(), app_version: "2.0.0".into() },
    ]);
    assert_eq!(
        call(&r, Op::LinkApps, &json!({})).unwrap(),
        json!({"apps": [{"app": "effectcraft", "app_version": "0.8.3"}, {"app": "photocraft", "app_version": "2.0.0"}]})
    );
    *r.services.link_apps_result.lock().unwrap() = Err(LinkFailure::Failed("no runtime directory".into()));
    assert_eq!(call(&r, Op::LinkApps, &json!({})).unwrap(), json!({"error": "no runtime directory"}));
    let none = rig(&[Op::LinkApps], &[]);
    assert_eq!(status_of(call(&none, Op::LinkApps, &json!({})).unwrap_err()), Status::Denied);
}

#[test]
fn a_link_call_retry_after_too_small_reaches_the_peer_once() {
    let r = rig(&[Op::LinkCall], &["link.call:effectcraft", "link.exec:effectcraft:effect.*"]);
    let req = serde_json::to_vec(&link_call_req("effectcraft")).unwrap();
    let mut tiny = [0u8; 1];
    let required = match r.d.dispatch(Op::LinkCall, &req, &mut tiny) {
        Err(HostError::TooSmall { required }) => required,
        other => panic!("{other:?}"),
    };
    let mut big = vec![0u8; required];
    assert_eq!(r.d.dispatch(Op::LinkCall, &req, &mut big).unwrap(), required);
    assert_eq!(r.services.recorded(|x| x.link_calls.len()), 1, "the peer got the command once");
}

// ---- through a real wasmi guest -------------------------------------------------------------

fn guest(r: &Rig) -> craft_plugin_host::engine::wasmi::WasmiInstance {
    let engine = WasmiEngine::system();
    let manifest = echo_manifest("org.example.t", "1.0.0", json!([{ "kind": "command", "id": "c", "entry": 0 }]));
    let bytes = package_from(&fixture_text("host"), &manifest);
    let report = gate(&bytes);
    assert!(report.is_accepted(), "{:#?}", report.violations);
    let module = engine.compile_sync(&bytes, &report, OPTS).unwrap();
    module.instantiate_sync(r.d.handle(), &budget()).unwrap()
}

#[test]
fn a_guest_gets_status_and_count_from_the_dispatcher_through_the_engine() {
    let r = rig(&[Op::HostLog, Op::HostInfo], &[]);
    let mut g = guest(&r);
    let b = budget();
    let msg = br#"{"msg":"from guest"}"#;
    g.write(512, msg).unwrap();
    let ret = g.call(Export::Invoke, &[Op::HostLog.number() as i32, 512, msg.len() as i32, 1024, 64], &b).unwrap();
    assert_eq!(unpack_call_return(ret), (Status::Ok.code(), 2));
    let mut buf = [0u8; 2];
    g.read(1024, &mut buf).unwrap();
    assert_eq!(&buf, b"{}");
    r.services.recorded(|x| assert_eq!(x.logs[0].2, "from guest"));
    // host.info into a 4 byte buffer: E_TOO_SMALL with the size the guest must allocate.
    let ret = g.call(Export::Invoke, &[Op::HostInfo.number() as i32, 0, 0, 1024, 4], &b).unwrap();
    let (code, required) = unpack_call_return(ret);
    assert_eq!(code, Status::TooSmall.code());
    let ret = g.call(Export::Invoke, &[Op::HostInfo.number() as i32, 0, 0, 1024, required as i32], &b).unwrap();
    assert_eq!(unpack_call_return(ret), (Status::Ok.code(), required));
    let mut info = vec![0u8; required as usize];
    g.read(1024, &mut info).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&info).unwrap()["engine"], "wasmi");
    // An op the manifest did not list is denied inside the guest too.
    let ret = g.call(Export::Invoke, &[Op::CmdList.number() as i32, 0, 0, 1024, 64], &b).unwrap();
    assert_eq!(unpack_call_return(ret).0, Status::Denied.code());
}

#[test]
fn a_dispatcher_trap_stops_the_guest() {
    // Control for the denial tests: a Trap from the dispatcher still reaches the caller.
    let engine = WasmiEngine::system();
    let manifest = echo_manifest("org.example.t", "1.0.0", json!([{ "kind": "command", "id": "c", "entry": 0 }]));
    let bytes = package_from(&fixture_text("host"), &manifest);
    let report = gate(&bytes);
    let module = engine.compile_sync(&bytes, &report, OPTS).unwrap();
    let trapping: craft_plugin_host::engine::HostDispatch = Arc::new(|op: Op, _i: &[u8], _o: &mut [u8]| Err(HostError::Trap(Trap::Denied(op))));
    let mut g = module.instantiate_sync(trapping, &budget()).unwrap();
    assert_eq!(g.call(Export::Invoke, &[1, 0, 0, 0, 0], &budget()).unwrap_err(), Trap::Denied(Op::HostLog));
}
