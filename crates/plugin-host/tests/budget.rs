//! Budget classes, limits, fault handling, and the circuit breaker.
//!
//! The call lifecycle promise has two halves. The plugin host guarantees the app hears
//! about every fault through `end_call(Faulted)` with the checkpoint it opened in `begin_call`.
//! Restoring the document is the app's job and is not tested here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use common::services::*;
use common::*;
use craft_plugin_abi::manifest::{Contribution, Kind, Manifest};
use craft_plugin_host::budget::*;
use craft_plugin_host::engine::Trap;
use craft_plugin_host::invoke::CallError;
use craft_plugin_host::lifecycle::Decision;
use craft_plugin_host::services::CallOutcome;
use craft_plugin_host::store::Source;
use serde_json::{Value, json};

const ID: &str = "org.example.t";

fn cmd(name: &str) -> String {
    format!("plugin.{ID}.{name}")
}

fn manifest(limits: Value) -> Manifest {
    let mut v = echo_manifest(ID, "1.0.0", json!([{ "kind": "command", "id": "c", "entry": 0 }]));
    match limits {
        Value::Null => {
            v.as_object_mut().unwrap().remove("limits");
        }
        l => v["limits"] = l,
    }
    Manifest::parse(&serde_json::to_vec(&v).unwrap()).unwrap()
}

fn contribution(v: Value) -> Contribution {
    let mut m = echo_manifest(ID, "1.0.0", json!([v]));
    m["requires"] = json!({});
    Manifest::parse(&serde_json::to_vec(&m).unwrap()).unwrap().contributions.remove(0)
}

/// A contribution as `serde` reads it, with no manifest validation. For the guard behind `validate`.
fn raw_contribution(v: Value) -> Contribution {
    serde_json::from_value(v).unwrap()
}

#[test]
fn classes_follow_the_contribution_budget_field_then_its_kind() {
    let c = |v: Value| BudgetClass::of(&contribution(v));
    assert_eq!(c(json!({"kind": "command", "id": "a", "entry": 0})), BudgetClass::Interactive);
    assert_eq!(c(json!({"kind": "panel", "id": "a", "entry": 0})), BudgetClass::Interactive);
    assert_eq!(c(json!({"kind": "effect", "id": "a", "entry": 0})), BudgetClass::Render);
    assert_eq!(c(json!({"kind": "audio-effect", "id": "a", "entry": 0})), BudgetClass::Render);
    assert_eq!(BudgetClass::of(&raw_contribution(json!({"kind": "command", "id": "a", "entry": 0, "budget": "batch"}))), BudgetClass::Batch);
    assert_eq!(c(json!({"kind": "filter", "id": "a", "entry": 0, "budget": "batch"})), BudgetClass::Batch);
    assert_eq!(c(json!({"kind": "effect", "id": "a", "entry": 0, "budget": "interactive"})), BudgetClass::Interactive);
    // A hook, panel or menu never gets the batch class: it runs on the UI thread.
    for kind in ["hook", "panel", "menu"] {
        let raw = raw_contribution(json!({"kind": kind, "id": "a", "entry": 0, "budget": "batch"}));
        assert_eq!(BudgetClass::of(&raw), BudgetClass::Interactive, "{kind}");
    }
    assert_eq!(c(json!({"kind": "teleport", "id": "a", "entry": 0})), BudgetClass::Interactive);
    assert!(matches!(contribution(json!({"kind": "teleport", "id": "a", "entry": 0})).kind, Kind::Unknown(_)));
}

#[test]
fn each_class_builds_the_budget_the_abi_lists() {
    let cfg = BudgetConfig::default();
    let limits = cfg.limits_for(&manifest(json!({"memory_mb": 4, "stack_kb": 128})));
    let cancel = Arc::new(AtomicBool::new(false));
    let i = cfg.budget(BudgetClass::Interactive, limits, 0, cancel.clone());
    assert_eq!((i.deadline, i.work), (Some(Duration::from_millis(250)), None));
    assert_eq!((i.memory_bytes, i.stack_bytes), (4 * 1024 * 1024, 128 * 1024));
    // Render: 2 s per band, fuel scales with the pixel count, and an empty band still gets fuel.
    let r = cfg.budget(BudgetClass::Render, limits, 1000, cancel.clone());
    assert_eq!((r.deadline, r.work), (Some(Duration::from_secs(2)), Some(1000 * cfg.render_fuel_per_unit)));
    let r2 = cfg.budget(BudgetClass::Render, limits, 4000, cancel.clone());
    assert_eq!(r2.work.unwrap(), 4 * r.work.unwrap(), "fuel is linear in pixels");
    assert_eq!(cfg.budget(BudgetClass::Render, limits, 0, cancel.clone()).work, Some(cfg.render_fuel_per_unit));
    // Batch: cancelable, and finite. The cancel flag is the caller's.
    let b = cfg.budget(BudgetClass::Batch, limits, 0, cancel.clone());
    assert_eq!((b.deadline, b.work), (Some(cfg.batch_cap), Some(cfg.batch_fuel_cap)));
    assert!(b.deadline.is_some() && b.work.is_some(), "batch must never run without a deadline and a fuel cap");
    assert!(Arc::ptr_eq(&b.cancel, &cancel));
    // The interactive default can never pass the interactive cap, even if configured to.
    let loose = BudgetConfig { interactive_default: Duration::from_secs(30), ..BudgetConfig::default() };
    assert_eq!(loose.budget(BudgetClass::Interactive, limits, 0, cancel).deadline, Some(Duration::from_secs(2)));
}

#[test]
fn limits_use_the_manifest_then_the_defaults_and_never_pass_the_host_caps() {
    let cfg = BudgetConfig::default();
    assert_eq!(cfg.limits_for(&manifest(json!({"memory_mb": 4, "stack_kb": 128}))), Limits { memory_bytes: 4 << 20, stack_bytes: 128 << 10 });
    assert_eq!(cfg.limits_for(&manifest(Value::Null)), Limits { memory_bytes: 256 << 20, stack_bytes: 1 << 20 });
    assert_eq!(cfg.limits_for(&manifest(json!({"memory_mb": 4096, "stack_kb": 8192}))), Limits { memory_bytes: 1024 << 20, stack_bytes: 8192 << 10 });
    let web = BudgetConfig { host_memory_cap_mb: 256, host_stack_cap_kb: 512, ..BudgetConfig::default() };
    assert_eq!(web.limits_for(&manifest(json!({"memory_mb": 4096, "stack_kb": 8192}))), Limits { memory_bytes: 256 << 20, stack_bytes: 512 << 10 });
}

#[test]
fn the_tracker_trips_on_consecutive_faults_and_a_success_in_between_resets_the_count() {
    let t = FaultTracker::new(3);
    assert_eq!(t.record_fault("p", "c"), FaultVerdict { consecutive: 1, tripped_now: false });
    assert_eq!(t.record_fault("p", "c").consecutive, 2);
    t.record_success("p", "c");
    assert_eq!(t.record_fault("p", "c"), FaultVerdict { consecutive: 1, tripped_now: false }, "the success cleared the count");
    t.record_fault("p", "c");
    let v = t.record_fault("p", "c");
    assert_eq!(v, FaultVerdict { consecutive: 3, tripped_now: true });
    assert!(t.is_open("p", "c"));
    assert!(!t.record_fault("p", "c").tripped_now, "the notice fires once");
    t.record_success("p", "c");
    assert!(t.is_open("p", "c"), "a success cannot close an open circuit");
    // Other contributions and plugins are independent.
    assert!(!t.is_open("p", "other") && !t.is_open("q", "c"));
    t.reset_plugin("p");
    assert!(!t.is_open("p", "c"));
    assert!(!is_fault(&Trap::Cancelled) && is_fault(&Trap::Deadline) && is_fault(&Trap::OutOfWork) && is_fault(&Trap::Unreachable));
}

fn loaded(h: &Harness) {
    h.manager.install(&echo_pkg(ID, "1.0.0", &echo_wat_v('1'), std_contribs()), Source::File).unwrap();
    h.manager.grant(ID, Decision { accept_required: true, optional: vec![] }).unwrap();
}

#[test]
fn a_guest_that_never_returns_stops_at_the_interactive_deadline_and_the_app_is_told() {
    // Every clock read moves 1 ms, so 250 reads reach the 250 ms deadline.
    let h = Harness::with_clock(Duration::from_millis(1), BudgetConfig::default());
    loaded(&h);
    let err = h.run(&cmd("spin"), &json!({})).unwrap_err();
    assert_eq!(err.0, CallError::Trap(Trap::Deadline));
    h.services.recorded(|r| {
        assert_eq!(r.begun.len(), 1);
        assert_eq!(r.ended.len(), 1);
        assert_eq!(r.ended[0].0, r.begun[0].0, "end_call closes the scope begin_call opened");
        assert!(matches!(&r.ended[0].1, CallOutcome::Faulted(m) if m.contains("deadline")));
        assert_eq!(r.notices.len(), 1);
        assert_eq!(r.notices[0].contribution.as_deref(), Some("spin"));
        assert!(r.notices[0].message.contains("undone") && r.notices[0].message.contains("deadline"), "{}", r.notices[0].message);
    });
    // Control: with a clock that does not move, the same module's other commands are not stopped by time.
    let calm = Harness::new();
    loaded(&calm);
    calm.run(&cmd("echo"), &json!({})).unwrap();
    calm.services.recorded(|r| assert!(matches!(r.ended[0].1, CallOutcome::Completed) && r.notices.is_empty()));
}

#[test]
fn a_trap_after_a_command_ran_still_tells_the_app_to_restore_the_checkpoint() {
    let h = Harness::new();
    loaded(&h);
    let err = h.run(&cmd("exec_boom"), &json!({})).unwrap_err();
    assert_eq!(err.0, CallError::Trap(Trap::Unreachable));
    h.services.recorded(|r| {
        assert_eq!(r.executed.len(), 1, "the guest's cmd.execute really ran before the trap");
        assert_eq!(r.begun.len(), 1);
        assert!(matches!(r.ended[..], [(s, CallOutcome::Faulted(_))] if s == r.begun[0].0), "{:?}", r.ended);
    });
}

#[test]
fn three_faults_in_a_row_switch_the_contribution_off_for_the_session_with_one_notice() {
    let h = Harness::new();
    loaded(&h);
    for n in 1..=3 {
        let err = h.run(&cmd("boom"), &json!({})).unwrap_err();
        assert_eq!(err.0, CallError::Trap(Trap::Unreachable), "call {n}");
    }
    h.services.recorded(|r| {
        assert_eq!(r.notices.len(), 3);
        assert!(!r.notices[0].message.contains("stays off") && !r.notices[1].message.contains("stays off"));
        assert!(r.notices[2].message.contains("stays off for this session after 3 faults"), "{}", r.notices[2].message);
    });
    let (begun, ended) = h.services.recorded(|r| (r.begun.len(), r.ended.len()));
    // The fourth call is refused before the guest or the app is touched.
    let err = h.run(&cmd("boom"), &json!({})).unwrap_err();
    assert_eq!(err.0, CallError::CircuitOpen);
    h.services.recorded(|r| assert_eq!((r.begun.len(), r.ended.len(), r.notices.len()), (begun, ended, 3)));
    // Other contributions of the same plugin still run.
    h.run(&cmd("echo"), &json!({})).unwrap();
    // A reload gives the new version a clean slate.
    h.manager.install(&echo_pkg(ID, "1.0.1", &echo_wat_v('2'), std_contribs()), Source::File).unwrap();
    assert_eq!(h.run(&cmd("boom"), &json!({})).unwrap_err().0, CallError::Trap(Trap::Unreachable));
}

#[test]
fn a_plugin_reported_error_rolls_back_but_never_opens_the_circuit() {
    let h = Harness::new();
    loaded(&h);
    for _ in 0..6 {
        let err = h.run(&cmd("err"), &json!({})).unwrap_err();
        assert_eq!(err.0, CallError::Plugin("nope".into()));
    }
    h.services.recorded(|r| {
        assert_eq!(r.ended.len(), 6);
        assert!(r.ended.iter().all(|(_, o)| matches!(o, CallOutcome::Faulted(m) if m == "nope")), "the app is told to restore");
        assert!(r.notices.is_empty(), "the plugin chose to fail, so there is nothing to warn about");
    });
    assert!(!h.manager.faults().is_open(ID, "err"));
}

#[test]
fn a_malformed_answer_is_a_fault_and_a_faulted_session_instance_is_replaced() {
    let h = Harness::new();
    loaded(&h);
    let err = h.run(&cmd("badjson"), &json!({})).unwrap_err();
    assert!(matches!(&err.0, CallError::BadResponse(_)), "{err:?}");
    // The `count` command is `instance: session`. Nine calls work, the tenth writes ':' and breaks JSON.
    for want in 1..=9 {
        assert_eq!(h.run(&cmd("count"), &json!({})).unwrap(), json!(want));
    }
    let err = h.run(&cmd("count"), &json!({})).unwrap_err();
    assert!(matches!(&err.0, CallError::BadResponse(_)), "{err:?}");
    assert_eq!(h.run(&cmd("count"), &json!({})).unwrap(), json!(1), "the broken instance was dropped, so the counter restarted");
}

#[test]
fn limits_from_the_manifest_reach_the_running_generation() {
    let h = Harness::new();
    loaded(&h);
    let g = h.manager.router().current(ID).unwrap();
    assert_eq!((g.limits().memory_bytes, g.limits().stack_bytes), (4 << 20, 128 << 10));
}
