//! `hook` contributions. Every test drives a real `craft_command::Hooks` with real wasmi guests from
//! `tests/fixtures/{gates,observer}.wat`. Each rule has a case that fails when the rule is removed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::Duration;

use common::services::*;
use common::*;
use craft_command::{Call, Cause, Clock as AppClockTrait, Hooks, Interceptor, Interest, Origin, Outcome, Owner, Verdict};
use craft_plugin_abi::Permission;
use craft_plugin_abi::permissions::DocScope;
use craft_plugin_host::budget::BudgetConfig;
use craft_plugin_host::contrib::hook::{HookHost, SkippedHook};
use craft_plugin_host::lifecycle::{Decision, InstallOutcome};
use craft_plugin_host::services::*;
use craft_plugin_host::store::{Source, StateStore};
use serde_json::{Value, json};

type Snap = Arc<String>;

/// The app side clock `Hooks` reads. It never moves, so the command layer's own 2 ms rule stays out
/// of these tests. The guest deadline runs on the engine clock instead.
struct AppClock;

impl AppClockTrait for AppClock {
    fn now_ms(&self) -> u64 {
        0
    }
}

/// Wraps the recording services. It notes which thread each guest call began on, and it can run
/// `Hooks::before` from inside `cmd.execute`, as a real app does.
struct Spy {
    inner: Arc<TestServices>,
    hooks: Hooks<Sess>,
    reenter: bool,
    begun_on: Mutex<Vec<(String, ThreadId)>>,
    executes: AtomicU32,
    nested: Mutex<Vec<Verdict>>,
}

impl Services for Spy {
    fn host_info(&self) -> HostInfo {
        self.inner.host_info()
    }
    fn log(&self, owner: &Owner, level: LogLevel, message: &str) {
        self.inner.log(owner, level, message);
    }
    fn progress(&self, owner: &Owner, fraction: f64, message: Option<&str>) {
        self.inner.progress(owner, fraction, message);
    }
    fn list_commands(&self) -> Vec<CommandInfo> {
        self.inner.list_commands()
    }
    fn execute_command(&self, owner: &Owner, id: &str, params: &Value) -> Result<Value, CommandFailure> {
        self.executes.fetch_add(1, Ordering::Relaxed);
        if self.reenter {
            let mut call = Call { id: Arc::from(id), params: params.clone(), depth: 1, origin: Origin::Execute, seq: 99 };
            let verdict = self.hooks.before(&mut Sess, &mut call, &AppClock);
            self.hooks.after(&call.info(), &Outcome::Ok, &AppClock);
            self.nested.lock().unwrap().push(verdict);
        }
        self.inner.execute_command(owner, id, params)
    }
    fn doc_query(&self, owner: &Owner, scope: DocScope, query: &Value) -> Result<Value, QueryFailure> {
        self.inner.doc_query(owner, scope, query)
    }
    fn unix_millis(&self) -> u64 {
        self.inner.unix_millis()
    }
    fn fill_random(&self, buf: &mut [u8]) {
        self.inner.fill_random(buf);
    }
    fn notify(&self, notice: UserNotice) {
        self.inner.notify(notice);
    }
    fn link_apps(&self, owner: &Owner) -> Result<Vec<LinkApp>, LinkFailure> {
        self.inner.link_apps(owner)
    }
    fn link_call(
        &self,
        owner: &Owner,
        app: &str,
        method: &str,
        params: &Value,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Value, LinkFailure> {
        self.inner.link_call(owner, app, method, params, cancel)
    }
    fn lend_session(&self, session: &mut dyn std::any::Any, run: &mut dyn FnMut()) {
        self.inner.lend_session(session, run);
    }
    fn begin_call(&self, owner: &Owner, contribution: &str) -> CallScope {
        self.begun_on.lock().unwrap().push((contribution.to_owned(), std::thread::current().id()));
        self.inner.begin_call(owner, contribution)
    }
    fn end_call(&self, scope: CallScope, outcome: &CallOutcome) {
        self.inner.end_call(scope, outcome);
    }
}

struct Rig {
    h: Harness,
    hooks: Hooks<Sess>,
    host: Arc<HookHost<Sess, Snap>>,
    spy: Arc<Spy>,
}

fn rig_with(clock_step: Duration, reenter: bool) -> Rig {
    let hooks = Hooks::<Sess>::new();
    let slot: Arc<Mutex<Option<Arc<Spy>>>> = Arc::new(Mutex::new(None));
    let clock = StepClock::new(clock_step);
    let engine = Arc::new(craft_plugin_host::engine::wasmi::WasmiEngine::new(clock.clone()).with_slice_fuel(20_000));
    let store = Arc::new(craft_plugin_host::store::MemoryStore::new());
    let (hk, sl) = (hooks.clone(), slot.clone());
    let h = Harness::build_with(engine, clock, BudgetConfig::default(), store.clone(), store, move |inner| {
        let spy = Arc::new(Spy {
            inner,
            hooks: hk,
            reenter,
            begun_on: Mutex::new(Vec::new()),
            executes: AtomicU32::new(0),
            nested: Mutex::new(Vec::new()),
        });
        *sl.lock().unwrap() = Some(spy.clone());
        spy
    });
    let spy = slot.lock().unwrap().take().unwrap();
    let host = HookHost::<Sess, Snap>::new(hooks.clone(), h.manager.router().clone(), spy.clone(), h.clock.clone());
    h.manager.attach_hooks(host.clone());
    Rig { h, hooks, host, spy }
}

fn rig() -> Rig {
    rig_with(Duration::ZERO, false)
}

fn call(id: &str, params: Value) -> Call {
    Call { id: Arc::from(id), params, depth: 1, origin: Origin::Ui, seq: 1 }
}

/// One command attempt as an app runs it: `before`, then `after` with the matching outcome.
fn attempt(r: &Rig, id: &str, params: Value) -> (Verdict, Call) {
    let mut c = call(id, params);
    let verdict = r.hooks.before(&mut Sess, &mut c, &AppClock);
    let outcome = if matches!(verdict, Verdict::Veto { .. }) { Outcome::Vetoed } else { Outcome::Ok };
    r.hooks.after(&c.info(), &outcome, &AppClock);
    (verdict, c)
}

fn manifest(id: &str, name: &str, version: &str, ops: &[&str], required: &[&str], optional: &[&str], contributions: Value) -> Value {
    json!({
        "schema": 1, "id": id, "name": name, "version": version,
        "abi": { "craft": 1 },
        "requires": { "ops": ops },
        "permissions": { "required": required, "optional": optional },
        "limits": { "memory_mb": 4, "stack_kb": 128 },
        "contributions": contributions
    })
}

/// Install `bytes` and accept every permission it asks for.
fn install(r: &Rig, bytes: &[u8]) -> InstallOutcome {
    match r.h.manager.install(bytes, Source::File).unwrap() {
        InstallOutcome::NeedsGrant(req) => {
            let id = req.plugin_id.clone();
            r.h.manager.grant(&id, Decision { accept_required: true, optional: req.optional }).unwrap()
        }
        other => other,
    }
}

const GATES: &str = "org.example.gates";

fn gates_contribs() -> Value {
    json!([
        { "kind": "hook", "id": "nodel", "entry": 0, "phase": "before", "match": ["clip.delete*"], "instance": "session" },
        { "kind": "hook", "id": "rewrite", "entry": 1, "phase": "before", "match": ["clip.trim"], "instance": "session" },
        { "kind": "hook", "id": "pass", "entry": 2, "phase": "before", "match": ["clip.noop"], "instance": "session" },
        { "kind": "hook", "id": "trap", "entry": 3, "phase": "before", "match": ["clip.trap"], "instance": "session" },
        { "kind": "hook", "id": "spin", "entry": 4, "phase": "before", "match": ["clip.spin"], "instance": "session" },
        { "kind": "hook", "id": "badveto", "entry": 5, "phase": "before", "match": ["clip.badveto"], "instance": "session" },
        { "kind": "hook", "id": "notjson", "entry": 6, "phase": "before", "match": ["clip.notjson"], "instance": "session" },
        { "kind": "hook", "id": "reenter", "entry": 7, "phase": "before", "match": ["app.set"] },
        { "kind": "hook", "id": "both", "entry": 8, "phase": "before", "match": ["clip.both"], "instance": "session" },
        { "kind": "hook", "id": "blank", "entry": 9, "phase": "before", "match": ["clip.blank"], "instance": "session" }
    ])
}

fn gates_manifest(version: &str, required: &[&str], optional: &[&str]) -> Value {
    manifest(GATES, "No Delete", version, &["cmd.execute"], required, optional, gates_contribs())
}

fn gates_wat() -> String {
    fixture_text("gates")
}

fn install_gates(r: &Rig) {
    let bytes = package_from(&gates_wat(), &gates_manifest("1.0.0", &["commands.intercept", "commands.run:app.*"], &[]));
    assert!(matches!(install(r, &bytes), InstallOutcome::Loaded { .. }));
}

fn notices(r: &Rig) -> Vec<String> {
    r.h.services.recorded(|x| x.notices.iter().map(|n| n.message.clone()).collect())
}

fn begun(r: &Rig) -> usize {
    r.h.services.recorded(|x| x.begun.len())
}

// ---- before ---------------------------------------------------------------------------------

#[test]
fn a_before_hook_vetoes_and_the_user_hears_the_plugins_reason() {
    let r = rig();
    install_gates(&r);
    let (verdict, _) = attempt(&r, "clip.delete.all", json!({"ids": [1]}));
    assert_eq!(verdict, Verdict::Veto { reason: "clips cannot be deleted".into(), by: "No Delete".into() });
    assert_eq!(notices(&r), vec!["No Delete blocked clip.delete.all: clips cannot be deleted".to_string()]);
    r.h.services.recorded(|x| {
        assert_eq!(x.notices[0].plugin_id, GATES);
        assert_eq!(x.notices[0].contribution.as_deref(), Some("nodel"));
    });
    // `match` is honoured: an id outside the glob never reaches the guest.
    let before = begun(&r);
    let (verdict, _) = attempt(&r, "clip.add", json!({}));
    assert_eq!(verdict, Verdict::Pass);
    assert_eq!(begun(&r), before, "no guest call for an id outside the glob");
    assert_eq!(notices(&r).len(), 1);
}

#[test]
fn a_before_hook_rewrites_the_params_the_command_runs_with() {
    let r = rig();
    install_gates(&r);
    let (verdict, c) = attempt(&r, "clip.trim", json!({"gain": 1, "keep": true}));
    assert_eq!(verdict, Verdict::Rewrite(json!({"gain": 2})));
    assert_eq!(c.params, json!({"gain": 2}), "the call carries the rewritten params");
    let (verdict, c) = attempt(&r, "clip.noop", json!({"gain": 1}));
    assert_eq!(verdict, Verdict::Pass);
    assert_eq!(c.params, json!({"gain": 1}), "a pass leaves the params alone");
}

#[test]
fn the_request_carries_the_call_and_the_veto_wins_over_a_rewrite() {
    let r = rig();
    install_gates(&r);
    let (verdict, _) = attempt(&r, "clip.both", json!({}));
    assert_eq!(verdict, Verdict::Veto { reason: "no".into(), by: "No Delete".into() });
}

#[test]
fn a_trapping_before_hook_fails_open_is_reported_once_and_opens_the_circuit() {
    let r = rig();
    install_gates(&r);
    for i in 0..6 {
        let (verdict, _) = attempt(&r, "clip.trap", json!({}));
        assert_eq!(verdict, Verdict::Pass, "call {i}: the command still runs");
    }
    let n = notices(&r);
    assert_eq!(n.len(), 2, "one fault notice and one circuit notice, not one per call: {n:?}");
    assert!(n[0].contains("did not finish") && n[0].contains("unreachable"), "{}", n[0]);
    assert!(n[1].contains("stays off for this session"), "{}", n[1]);
    assert_eq!(begun(&r), 3, "three faults open the circuit, then the guest is not called again");
    r.h.services.recorded(|x| assert!(x.ended.iter().all(|(_, o)| matches!(o, CallOutcome::Faulted(_)))));
    assert!(r.h.manager.faults().is_open(GATES, "trap"));
    // The circuit is per contribution. A sibling hook still works.
    assert!(matches!(attempt(&r, "clip.delete", json!({})).0, Verdict::Veto { .. }));
}

#[test]
fn a_slow_before_hook_is_stopped_at_the_two_millisecond_deadline_and_fails_open() {
    // The engine clock moves 100 us per read, so 2 ms is about 20 reads. Without the 2 ms deadline
    // the interactive default (250 ms) would take 2500 reads.
    let r = rig_with(Duration::from_micros(100), false);
    install_gates(&r);
    let (verdict, _) = attempt(&r, "clip.spin", json!({}));
    assert_eq!(verdict, Verdict::Pass);
    let n = notices(&r);
    assert_eq!(n.len(), 1, "{n:?}");
    assert!(n[0].contains("deadline"), "{}", n[0]);
    r.h.services.recorded(|x| assert!(matches!(&x.ended[..], [(_, CallOutcome::Faulted(m))] if m.contains("deadline"))));
    let reads = r.h.clock.reads();
    assert!(reads < 100, "the spin stopped near 2 ms of clock time, not at the 250 ms default: {reads} reads");
}

#[test]
fn bad_answers_fail_open_and_are_reported_once() {
    let r = rig();
    install_gates(&r);
    for id in ["clip.badveto", "clip.notjson", "clip.blank"] {
        for _ in 0..2 {
            assert_eq!(attempt(&r, id, json!({})).0, Verdict::Pass, "{id}");
        }
    }
    let n = notices(&r);
    // The notice flag is per hook, so each of the three hooks reports once.
    assert_eq!(n.len(), 3, "{n:?}");
    assert!(n.iter().any(|m| m.contains("bad answer") && m.contains("`veto` must be a non-empty string")), "{n:?}");
    assert!(n.iter().any(|m| m.contains("badveto")) && n.iter().any(|m| m.contains("blank")), "{n:?}");
    assert!(n.iter().any(|m| m.contains("notjson") && m.contains("bad response")), "{n:?}");
}

#[test]
fn a_hook_that_runs_a_command_does_not_call_itself() {
    let r = rig_with(Duration::ZERO, true);
    install_gates(&r);
    let (verdict, _) = attempt(&r, "app.set", json!({}));
    assert_eq!(verdict, Verdict::Pass);
    assert_eq!(r.spy.executes.load(Ordering::Relaxed), 1, "the hook ran the command once");
    assert_eq!(*r.spy.nested.lock().unwrap(), vec![Verdict::Pass], "the nested before passed without a guest call");
    let reenter_calls = r.spy.begun_on.lock().unwrap().iter().filter(|(c, _)| c == "reenter").count();
    assert_eq!(reenter_calls, 1, "the nested before did not call the guest again");
    assert!(notices(&r).is_empty(), "{:?}", notices(&r));
}

#[test]
fn a_busy_session_instance_is_skipped_on_the_calling_thread_and_never_waited_for() {
    let r = rig();
    let contribs = json!([
        { "kind": "hook", "id": "gate", "entry": 4, "phase": "before", "match": ["clip.*"], "instance": "session" },
        { "kind": "hook", "id": "slow", "entry": 3, "phase": "after", "match": ["*"], "instance": "session" }
    ]);
    let m = manifest(
        "org.example.busy",
        "Busy",
        "1.0.0",
        &["host.log", "state.put"],
        &["commands.intercept", "commands.observe", "state.local"],
        &[],
        contribs,
    );
    install(&r, &package_from(&fixture_text("observer"), &m));
    // Hold the session instance inside an after hook: its host.log blocks in the services.
    let (entered_tx, entered_rx) = channel();
    let (release_tx, release_rx) = channel();
    *r.h.services.log_gate.lock().unwrap() = Some(LogGate { entered: entered_tx, release: release_rx });
    attempt(&r, "clip.first", json!({}));
    entered_rx.recv_timeout(Duration::from_secs(10)).expect("the after hook should reach host.log");
    // A before call on another thread must come back while the worker holds the instance.
    let (done_tx, done_rx) = channel();
    let hooks = r.hooks.clone();
    let t = std::thread::spawn(move || {
        let mut c = call("clip.second", json!({}));
        let v = hooks.before(&mut Sess, &mut c, &AppClock);
        hooks.after(&c.info(), &Outcome::Ok, &AppClock);
        done_tx.send(v).unwrap();
    });
    let verdict = done_rx.recv_timeout(Duration::from_secs(5));
    release_tx.send(()).unwrap();
    t.join().unwrap();
    assert_eq!(verdict.expect("before must not wait for the busy instance"), Verdict::Pass);
    assert!(r.host.flush(Duration::from_secs(10)));
    r.h.services.recorded(|x| assert!(x.ended.iter().any(|(_, o)| *o == CallOutcome::Cancelled), "the skipped call ends as cancelled, not faulted"));
    assert!(notices(&r).is_empty(), "a busy instance is not a fault: {:?}", notices(&r));
}

// ---- permissions, reload, disable -----------------------------------------------------------

#[test]
fn a_hook_without_its_permission_is_skipped_and_revoking_it_removes_the_hook() {
    let r = rig();
    // `commands.intercept` is optional here, and the user declines it.
    let m = gates_manifest("1.0.0", &["commands.run:app.*"], &["commands.intercept"]);
    let bytes = package_from(&gates_wat(), &m);
    let InstallOutcome::NeedsGrant(req) = r.h.manager.install(&bytes, Source::File).unwrap() else { panic!("expected a prompt") };
    r.h.manager.grant(GATES, Decision { accept_required: true, optional: vec![] }).unwrap();
    assert_eq!(req.optional, vec![Permission::CommandsIntercept]);
    let info = r.h.manager.info(GATES).unwrap();
    assert!(info.hooks.registered.is_empty());
    assert_eq!(info.hooks.skipped.len(), 10);
    assert_eq!(info.hooks.skipped[0], SkippedHook { contribution: "nodel".into(), reason: "missing permission commands.intercept".into() });
    assert_eq!(attempt(&r, "clip.delete", json!({})).0, Verdict::Pass, "no grant, no veto");
    assert_eq!(begun(&r), 0);
    assert!(r.hooks.is_idle(), "nothing is registered");

    // A second rig where the user accepts, then revokes.
    let r = rig();
    install(&r, &bytes);
    assert_eq!(r.h.manager.info(GATES).unwrap().hooks.registered.len(), 10);
    assert!(matches!(attempt(&r, "clip.delete", json!({})).0, Verdict::Veto { .. }));
    r.h.manager.revoke(GATES, &Permission::CommandsIntercept).unwrap();
    let info = r.h.manager.info(GATES).unwrap();
    assert!(info.hooks.registered.is_empty() && info.hooks.skipped.len() == 10);
    assert_eq!(attempt(&r, "clip.delete", json!({})).0, Verdict::Pass, "revoking the grant took the gate out");
    assert!(r.hooks.is_idle());
}

#[test]
fn a_reload_moves_the_gate_to_the_new_generation_and_leaves_no_stale_gate() {
    let r = rig();
    install_gates(&r);
    assert_eq!(attempt(&r, "clip.noop", json!({})).0, Verdict::Pass);
    assert_eq!(begun(&r), 1);
    let v2 = swap(&gates_wat(), "clips cannot be deleted", "clips cannot be erased!");
    let bytes = package_from(&v2, &gates_manifest("2.0.0", &["commands.intercept", "commands.run:app.*"], &[]));
    assert!(matches!(install(&r, &bytes), InstallOutcome::Loaded { generation: 2, replaced: Some(1) }));
    let (verdict, _) = attempt(&r, "clip.delete", json!({}));
    assert_eq!(verdict, Verdict::Veto { reason: "clips cannot be erased!".into(), by: "No Delete".into() });
    let before = begun(&r);
    assert_eq!(attempt(&r, "clip.noop", json!({})).0, Verdict::Pass);
    assert_eq!(begun(&r) - before, 1, "exactly one gate answers: generation 1 was removed");
}

#[test]
fn disabling_a_plugin_removes_its_gates() {
    let r = rig();
    install_gates(&r);
    assert!(!r.hooks.is_idle());
    r.h.manager.disable(GATES).unwrap();
    assert!(r.hooks.is_idle());
    assert_eq!(attempt(&r, "clip.delete", json!({})).0, Verdict::Pass);
    assert!(r.h.manager.info(GATES).unwrap().hooks.registered.is_empty());
}

// ---- after and change -----------------------------------------------------------------------

const LOG: &str = "org.example.log";

fn log_edits_contribs() -> Value {
    json!([
        { "kind": "hook", "id": "after", "entry": 0, "phase": "after", "match": ["*"], "instance": "session" },
        { "kind": "hook", "id": "change", "entry": 1, "phase": "change", "instance": "session" }
    ])
}

fn install_log_edits(r: &Rig, required: &[&str]) {
    let m = manifest(LOG, "Log Edits", "1.0.0", &["state.put"], required, &[], log_edits_contribs());
    install(r, &package_from(&fixture_text("observer"), &m));
}

fn state(r: &Rig, key: &str) -> Option<Value> {
    r.h.store.get(LOG, key).unwrap()
}

#[test]
fn after_and_change_hooks_log_every_action_in_order_off_the_calling_thread() {
    let r = rig();
    install_log_edits(&r, &["commands.observe", "state.local"]);
    let me = std::thread::current().id();
    let ids = ["clip.add", "clip.trim", "clip.move"];
    for (i, id) in ids.iter().enumerate() {
        let mut c = Call { id: Arc::from(*id), params: json!({}), depth: 1, origin: [Origin::Ui, Origin::Shortcut, Origin::Mcp][i], seq: i as u64 };
        r.hooks.before(&mut Sess, &mut c, &AppClock);
        r.hooks.after(&c.info(), if i == 1 { &Outcome::Err } else { &Outcome::Ok }, &AppClock);
    }
    let cause = Cause::Command { id: Arc::from("clip.add"), depth: 1, origin: Origin::Ui };
    r.hooks.changed::<Snap>(7, cause, true, Arc::new("snapshot".into()));
    r.hooks.changed::<Snap>(8, Cause::Undo, false, Arc::new("snapshot".into()));
    assert!(r.host.flush(Duration::from_secs(10)));
    let expected = [("clip.add", "ui", "ok"), ("clip.trim", "shortcut", "err"), ("clip.move", "mcp", "ok")];
    for (i, (id, origin, outcome)) in expected.iter().enumerate() {
        let v = state(&r, &format!("e{i}")).unwrap_or_else(|| panic!("e{i} missing"));
        assert_eq!(v["call"]["id"], *id, "{v}");
        assert_eq!(v["call"]["origin"], *origin);
        assert_eq!(v["call"]["seq"], i as u64);
        assert_eq!(v["call"]["depth"], 1);
        assert_eq!(v["outcome"], *outcome);
        assert!(v["call"].get("params").is_none(), "an after request carries no params");
    }
    assert!(state(&r, "e3").is_none(), "exactly three actions were logged");
    let c0 = state(&r, "c0").unwrap();
    assert_eq!(c0, json!({"change": {"revision": 7, "cause": {"kind": "command", "id": "clip.add", "depth": 1, "origin": "ui"}, "merged": true}}));
    let c1 = state(&r, "c1").unwrap();
    assert_eq!(c1, json!({"change": {"revision": 8, "cause": {"kind": "undo"}, "merged": false}}));
    assert!(state(&r, "c2").is_none());
    // The guest calls ran on delivery threads, never on the test thread that ran the command.
    let threads = r.spy.begun_on.lock().unwrap().clone();
    assert_eq!(threads.len(), 5, "three after calls and two change calls: {threads:?}");
    assert!(threads.iter().all(|(_, t)| *t != me), "a wasm call ran on the command thread");
}

#[test]
fn without_commands_observe_nothing_is_delivered() {
    let r = rig();
    let m = manifest(LOG, "Log Edits", "1.0.0", &["state.put"], &["state.local"], &["commands.observe"], log_edits_contribs());
    let bytes = package_from(&fixture_text("observer"), &m);
    let InstallOutcome::NeedsGrant(_) = r.h.manager.install(&bytes, Source::File).unwrap() else { panic!("expected a prompt") };
    r.h.manager.grant(LOG, Decision { accept_required: true, optional: vec![] }).unwrap();
    attempt(&r, "clip.add", json!({}));
    r.hooks.changed::<Snap>(1, Cause::Load, false, Arc::new("s".into()));
    assert!(r.host.flush(Duration::from_secs(5)));
    assert!(state(&r, "e0").is_none() && state(&r, "c0").is_none());
    assert_eq!(begun(&r), 0);
    let info = r.h.manager.info(LOG).unwrap();
    assert_eq!(info.hooks.skipped.iter().map(|s| s.reason.as_str()).collect::<Vec<_>>(), vec!["missing permission commands.observe"; 2]);
    assert!(r.hooks.is_idle(), "no subscriber or gate exists, so the app keeps its fast path");
}

#[test]
fn removing_the_plugin_stops_delivery_and_frees_the_subscription() {
    let r = rig();
    install_log_edits(&r, &["commands.observe", "state.local"]);
    attempt(&r, "clip.add", json!({}));
    assert!(r.host.flush(Duration::from_secs(10)));
    assert!(state(&r, "e0").is_some());
    let delivered = begun(&r);
    r.h.manager.disable(LOG).unwrap();
    assert!(r.hooks.is_idle(), "gate and change subscriber are gone");
    attempt(&r, "clip.move", json!({}));
    r.hooks.changed::<Snap>(9, Cause::Load, false, Arc::new("s".into()));
    assert!(r.host.flush(Duration::from_secs(5)));
    assert_eq!(begun(&r), delivered, "nothing reached the guest after the disable");
}

#[test]
fn a_flood_drops_the_oldest_and_delivers_one_events_lost_marker() {
    let r = rig();
    let m = manifest(
        "org.example.flood",
        "Flood",
        "1.0.0",
        &["host.log", "state.put"],
        &["commands.observe", "state.local"],
        &[],
        json!([{ "kind": "hook", "id": "slow", "entry": 3, "phase": "after", "match": ["*"], "instance": "session" }]),
    );
    install(&r, &package_from(&fixture_text("observer"), &m));
    let (entered_tx, entered_rx) = channel();
    let (release_tx, release_rx) = channel();
    *r.h.services.log_gate.lock().unwrap() = Some(LogGate { entered: entered_tx, release: release_rx });
    // The first event is taken by the delivery thread, which then blocks in host.log.
    attempt(&r, "clip.0", json!({}));
    entered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let extra = 1024 + 6;
    for i in 1..=extra {
        attempt(&r, &format!("clip.{i}"), json!({}));
    }
    release_tx.send(()).unwrap();
    assert!(r.host.flush(Duration::from_secs(60)));
    let calls = r.h.services.recorded(|x| x.begun.len());
    assert_eq!(calls, 1 + 1 + 1024, "the blocked event, one lost marker, then the newest 1024");
    let lost = r.h.store.get("org.example.flood", "lost").unwrap().unwrap();
    assert_eq!(lost, json!({"events.lost": 6}));
    let last = r.h.store.get("org.example.flood", "last").unwrap().unwrap();
    assert_eq!(last["call"]["id"], format!("clip.{extra}"), "the newest event survived");
}

#[test]
fn a_trapping_after_hook_is_reported_once_and_never_touches_the_command() {
    let r = rig();
    let m = manifest(
        "org.example.badafter",
        "Bad After",
        "1.0.0",
        &[],
        &["commands.observe"],
        &[],
        json!([{ "kind": "hook", "id": "boom", "entry": 2, "phase": "after", "match": ["*"], "instance": "session" }]),
    );
    install(&r, &package_from(&fixture_text("observer"), &m));
    for i in 0..6 {
        assert_eq!(attempt(&r, &format!("clip.{i}"), json!({})).0, Verdict::Pass);
    }
    assert!(r.host.flush(Duration::from_secs(10)));
    let n = notices(&r);
    assert_eq!(n.len(), 2, "{n:?}");
    assert!(n[0].contains("did not finish") && n[1].contains("stays off"), "{n:?}");
    assert_eq!(begun(&r), 3);
}

// ---- a gate reaches the session; a shut-out gate tells the user ---------------------

#[test]
fn a_before_hook_reaches_the_session_for_its_host_calls() {
    // The app can serve `cmd.execute` only while the session is lent. A gate used to get the busy
    // error here. `reenter` runs `app.set` from a `before` hook on `app.set`.
    let r = rig();
    r.h.services.require_lend.store(true, Ordering::Relaxed);
    install_gates(&r);
    let (verdict, _) = attempt(&r, "app.set", json!({}));
    assert_eq!(verdict, Verdict::Pass);
    r.h.services.recorded(|x| {
        assert_eq!(x.executed.len(), 1, "the hook's cmd.execute reached the app: {:?}", x.executed);
        assert_eq!(x.executed[0].2, "app.set");
    });
    assert_eq!(r.h.services.lends.load(Ordering::Relaxed), 1, "one lend for the one guest call");
    assert!(notices(&r).is_empty(), "{:?}", notices(&r));
}

struct Hog {
    interest: Interest,
    clock: Arc<ManualCommandClock>,
}

impl Interceptor<Sess> for Hog {
    fn interest(&self) -> &Interest {
        &self.interest
    }

    fn before(&self, _session: &mut Sess, _call: &mut Call) -> Verdict {
        self.clock.0.fetch_add(2, Ordering::SeqCst);
        Verdict::Pass
    }
}

struct ManualCommandClock(AtomicU64);

impl AppClockTrait for ManualCommandClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Gate time is charged per owner. Four app gates that burn the 6 ms window cost the app its
/// own gates. They do not shut out the plugin's gate, and its veto holds.
#[test]
fn app_gates_that_burn_the_window_cannot_shut_out_a_plugin_gate() {
    let r = rig();
    install_gates(&r);
    let clock = Arc::new(ManualCommandClock(AtomicU64::new(100)));
    for priority in 0..4 {
        let hog = Hog { interest: Interest { globs: vec!["*".into()], priority }, clock: Arc::clone(&clock) };
        r.hooks.register(&Owner { plugin_id: Arc::from("app"), generation: 1 }, Arc::new(hog));
    }
    let mut c = call("clip.delete.all", json!({"ids": [1]}));
    let verdict = r.hooks.before(&mut Sess, &mut c, clock.as_ref());
    assert!(matches!(verdict, Verdict::Veto { .. }), "the plugin's veto must hold: {verdict:?}");
    r.hooks.after(&c.info(), &Outcome::Ok, clock.as_ref());
    assert_eq!(begun(&r), 1, "the plugin gate ran");
    assert_eq!(notices(&r).len(), 1, "only the veto notice: {:?}", notices(&r));
    // A later call in the same window still vetoes.
    let mut again = call("clip.delete.all", json!({"ids": [1]}));
    assert!(matches!(r.hooks.before(&mut Sess, &mut again, clock.as_ref()), Verdict::Veto { .. }));
}
