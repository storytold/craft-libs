//! Lifecycle tests: install, grants, reload, disable, remove, restore. Every failure test
//! also proves generation N still works afterward, because that is the promise under test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use common::services::*;
use common::*;
use craft_plugin_abi::Permission;
use craft_plugin_host::budget::BudgetConfig;
use craft_plugin_host::engine::wasmi::WasmiEngine;
use craft_plugin_host::engine::{LoadError, Trap};
use craft_plugin_host::invoke::CallError;
use craft_plugin_host::lifecycle::*;
use craft_plugin_host::store::{MemoryStore, PackageStore, Source, StateStore, StoreError, StoredPlugin};
use serde_json::{Value, json};

const ID: &str = "org.example.t";

fn accept_required() -> Decision {
    Decision { accept_required: true, optional: vec![] }
}

fn install_v(h: &Harness, version: &str, digit: char) -> Result<InstallOutcome, LifecycleError> {
    h.manager.install(&echo_pkg(ID, version, &echo_wat_v(digit), std_contribs()), Source::File)
}

/// Install version 1 and accept the prompt.
fn active_v1(h: &Harness) {
    assert!(matches!(install_v(h, "1.0.0", '1').unwrap(), InstallOutcome::NeedsGrant(_)));
    assert_eq!(h.manager.grant(ID, accept_required()).unwrap(), InstallOutcome::Loaded { generation: 1, replaced: None });
}

fn cmd(name: &str) -> String {
    format!("plugin.{ID}.{name}")
}

fn ver(h: &Harness) -> Value {
    h.run(&cmd("ver"), &json!({})).unwrap()
}

fn dynamic_ids(h: &Harness) -> Vec<String> {
    let mut ids = Vec::new();
    h.registry.for_each(|v| {
        if v.dynamic {
            ids.push(v.id.to_owned());
        }
    });
    ids.sort();
    ids
}

#[test]
fn install_asks_then_loads_and_registers_the_commands() {
    let h = Harness::new();
    let InstallOutcome::NeedsGrant(req) = install_v(&h, "1.0.0", '1').unwrap() else { panic!("expected a prompt") };
    assert_eq!(req.required, vec!["commands.run:app.*".parse::<Permission>().unwrap()]);
    assert_eq!(req.new_required, req.required);
    assert_eq!(req.optional, vec![Permission::UiContribute]);
    assert_eq!(h.manager.state(ID), Some(PluginState::AwaitingGrant));
    assert!(dynamic_ids(&h).is_empty(), "nothing is registered before the user answers");
    assert!(h.store.load_all().unwrap().is_empty(), "nothing is stored before the user answers");

    assert_eq!(h.manager.grant(ID, accept_required()).unwrap(), InstallOutcome::Loaded { generation: 1, replaced: None });
    assert_eq!(h.manager.state(ID), Some(PluginState::Active { generation: 1 }));
    assert_eq!(dynamic_ids(&h).len(), 9);
    // The guest really ran: echo returns the request it was given.
    assert_eq!(h.run(&cmd("echo"), &json!({"a": 1})).unwrap(), json!({"params": {"a": 1}, "origin": "unknown"}));
    // The package is stored with its grants.
    let stored = h.store.load_all().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!((stored[0].version.as_str(), stored[0].enabled), ("1.0.0", true));
    assert_eq!(stored[0].grants.len(), 1);
    assert_eq!(stored[0].grants[0].permission, "commands.run:app.*");
    assert_eq!(stored[0].grants[0].plugin_id, ID);
    assert_eq!(stored[0].grants[0].granted_at_ms, 1_700_000_000_000);
}

#[test]
fn a_declined_install_is_stored_disabled_and_never_runs() {
    let h = Harness::new();
    install_v(&h, "1.0.0", '1').unwrap();
    let out = h.manager.grant(ID, Decision { accept_required: false, optional: vec![] }).unwrap();
    assert_eq!(out, InstallOutcome::StoredDisabled);
    assert_eq!(h.manager.state(ID), Some(PluginState::Disabled));
    assert!(dynamic_ids(&h).is_empty());
    let stored = h.store.load_all().unwrap();
    assert!(!stored[0].enabled && stored[0].grants.is_empty());
    // A grant with nothing pending is an error, not a silent success.
    assert!(matches!(h.manager.grant(ID, accept_required()), Err(LifecycleError::NothingPending(_))));
}

#[test]
fn a_grant_for_an_optional_permission_the_manifest_lacks_is_refused_and_the_prompt_stays() {
    let h = Harness::new();
    install_v(&h, "1.0.0", '1').unwrap();
    let bad = Decision { accept_required: true, optional: vec![Permission::DocWrite] };
    assert!(matches!(h.manager.grant(ID, bad), Err(LifecycleError::NotRequested(p)) if p == "doc.write"));
    assert_eq!(h.manager.state(ID), Some(PluginState::AwaitingGrant));
    h.manager.grant(ID, accept_required()).unwrap();
}

#[test]
fn a_package_the_gate_rejects_is_an_error_and_leaves_no_trace() {
    let h = Harness::new();
    let mut bad = echo_pkg(ID, "1.0.0", &echo_wat_v('1'), std_contribs());
    bad.truncate(bad.len() - 10);
    let err = h.manager.install(&bad, Source::File).unwrap_err();
    assert!(matches!(err, LifecycleError::Rejected(_)), "{err}");
    assert_eq!(h.manager.state(ID), None);
    assert!(h.store.load_all().unwrap().is_empty());
    // A valid wasm with no manifest: the gate names the violation.
    let no_manifest = wat::parse_str(fixture_text("echo")).unwrap();
    assert!(matches!(h.manager.install(&no_manifest, Source::File), Err(LifecycleError::Rejected(LoadError::Gate(_)))));
}

#[test]
fn reload_switches_new_calls_to_the_new_generation_and_drops_the_old_commands() {
    let h = Harness::new();
    active_v1(&h);
    assert_eq!(ver(&h), json!(1));
    let out = install_v(&h, "1.1.0", '2').unwrap();
    assert_eq!(out, InstallOutcome::Loaded { generation: 2, replaced: Some(1) });
    assert_eq!(ver(&h), json!(2), "the same registered command now reaches the new module");
    assert_eq!(dynamic_ids(&h).len(), 9, "the old generation's commands are gone, not doubled");
    assert_eq!(h.manager.state(ID), Some(PluginState::Active { generation: 2 }));
    assert_eq!(h.manager.info(ID).unwrap().version, "1.1.0");
    assert_eq!(h.store.load_all().unwrap()[0].version, "1.1.0");
    // The grant survived: the update asked for nothing new.
    assert_eq!(h.manager.info(ID).unwrap().grants, vec!["commands.run:app.*".to_string()]);
    let owner_gens: std::collections::HashSet<u64> = {
        let mut g = std::collections::HashSet::new();
        for id in dynamic_ids(&h) {
            if let Some(craft_command::CommandEntry::Dynamic(c)) = h.registry.resolve(&id) {
                g.insert(c.owner.generation);
            }
        }
        g
    };
    assert_eq!(owner_gens, std::collections::HashSet::from([2]));
}

#[test]
fn reload_carries_session_state_through_save_and_load() {
    let h = Harness::new();
    active_v1(&h);
    for want in 1..=3 {
        assert_eq!(h.run(&cmd("count"), &json!({})).unwrap(), json!(want));
    }
    install_v(&h, "1.1.0", '2').unwrap();
    assert_eq!(h.run(&cmd("count"), &json!({})).unwrap(), json!(4), "the counter continued from 3");
    // Control: a plugin that never received its state starts again at 1.
    let h2 = Harness::new();
    active_v1(&h2);
    h2.run(&cmd("count"), &json!({})).unwrap();
    h2.manager.disable(ID).unwrap();
    h2.manager.enable(ID).unwrap();
    assert_eq!(h2.run(&cmd("count"), &json!({})).unwrap(), json!(1), "disable and enable is a fresh instance, so there is no carried state");
}

#[test]
fn a_package_the_gate_rejects_on_update_keeps_the_old_generation() {
    let h = Harness::new();
    active_v1(&h);
    let mut bad = echo_pkg(ID, "1.1.0", &echo_wat_v('2'), std_contribs());
    bad.truncate(bad.len() - 10);
    assert!(matches!(h.manager.install(&bad, Source::File), Err(LifecycleError::Rejected(_))));
    assert_eq!(ver(&h), json!(1));
    assert_eq!(h.manager.state(ID), Some(PluginState::Active { generation: 1 }));
    assert_eq!(h.store.load_all().unwrap()[0].version, "1.0.0");
}

#[test]
fn a_failing_state_save_on_the_old_generation_keeps_it_running() {
    let h = Harness::new();
    let broken_save = swap(&echo_wat_v('1'), ";;SAVE_HOOK", "unreachable");
    h.manager.install(&echo_pkg(ID, "1.0.0", &broken_save, std_contribs()), Source::File).unwrap();
    h.manager.grant(ID, accept_required()).unwrap();
    h.run(&cmd("count"), &json!({})).unwrap(); // makes a session instance, so there is state to save
    let err = install_v(&h, "1.1.0", '2').unwrap_err();
    assert!(matches!(err, LifecycleError::StateSave(CallError::Trap(Trap::Unreachable))), "{err}");
    assert_eq!(ver(&h), json!(1));
    assert_eq!(h.run(&cmd("count"), &json!({})).unwrap(), json!(2), "the session instance of N is intact");
    assert_eq!(h.store.load_all().unwrap()[0].version, "1.0.0");
}

#[test]
fn a_failing_state_load_on_the_new_generation_keeps_the_old_one() {
    let h = Harness::new();
    active_v1(&h);
    h.run(&cmd("count"), &json!({})).unwrap();
    let broken_load = swap(&echo_wat_v('2'), ";;LOAD_HOOK", "(return (i32.const -1))");
    let err = h.manager.install(&echo_pkg(ID, "1.1.0", &broken_load, std_contribs()), Source::File).unwrap_err();
    assert!(matches!(err, LifecycleError::StateLoad(CallError::GuestStatus(-1))), "{err}");
    assert_eq!(ver(&h), json!(1));
    assert_eq!(h.manager.state(ID), Some(PluginState::Active { generation: 1 }));
    assert_eq!(h.store.load_all().unwrap()[0].version, "1.0.0");
}

#[test]
fn a_registration_failure_on_update_restores_the_old_commands_and_record() {
    let h = Harness::new();
    active_v1(&h);
    // Two contributions with one id: the second register is a duplicate.
    let mut dup = std_contribs();
    let first = dup[0].clone();
    dup.as_array_mut().unwrap().push(first);
    let err = h.manager.install(&echo_pkg(ID, "1.1.0", &echo_wat_v('2'), dup), Source::File).unwrap_err();
    assert!(matches!(err, LifecycleError::Register(_)), "{err}");
    assert_eq!(dynamic_ids(&h).len(), 9, "the old commands are back");
    assert_eq!(ver(&h), json!(1));
    assert_eq!(h.manager.state(ID), Some(PluginState::Active { generation: 1 }));
    assert_eq!(h.store.load_all().unwrap()[0].version, "1.0.0", "the stored record was put back");
    // The restored commands belong to generation 1 and work.
    assert_eq!(h.run(&cmd("count"), &json!({})).unwrap(), json!(1));
}

struct FailingPackages {
    inner: MemoryStore,
    fail: AtomicBool,
}

impl PackageStore for FailingPackages {
    fn save(&self, p: &StoredPlugin) -> Result<(), StoreError> {
        if self.fail.load(Ordering::Relaxed) {
            return Err(StoreError::Io("disk full".into()));
        }
        self.inner.save(p)
    }
    fn load_all(&self) -> Result<Vec<StoredPlugin>, StoreError> {
        self.inner.load_all()
    }
    fn remove(&self, id: &str) -> Result<(), StoreError> {
        self.inner.remove(id)
    }
}

#[test]
fn a_failing_store_write_on_update_keeps_the_old_generation() {
    let clock = StepClock::new(Duration::ZERO);
    let engine = Arc::new(WasmiEngine::new(clock.clone()).with_slice_fuel(20_000));
    let packages = Arc::new(FailingPackages { inner: MemoryStore::new(), fail: AtomicBool::new(false) });
    let h = Harness::build(engine, clock, BudgetConfig::default(), packages.clone(), Arc::new(MemoryStore::new()));
    active_v1(&h);
    packages.fail.store(true, Ordering::Relaxed);
    let err = install_v(&h, "1.1.0", '2').unwrap_err();
    assert!(matches!(err, LifecycleError::Store(StoreError::Io(_))), "{err}");
    assert_eq!(ver(&h), json!(1));
    assert_eq!(dynamic_ids(&h).len(), 9);
}

#[test]
fn an_update_that_adds_a_required_permission_asks_again_and_declining_keeps_the_old_version() {
    let h = Harness::new();
    active_v1(&h);
    let mut m = echo_manifest(ID, "2.0.0", std_contribs());
    m["permissions"]["required"] = json!(["commands.run:app.*", "doc.read:metadata"]);
    let v2 = package_from(&echo_wat_v('2'), &m);
    let InstallOutcome::NeedsGrant(req) = h.manager.install(&v2, Source::File).unwrap() else { panic!("expected a prompt") };
    assert_eq!(req.new_required, vec![Permission::DocRead(Some(craft_plugin_abi::permissions::DocScope::Metadata))], "only the growth is new");
    assert_eq!(req.required.len(), 2);
    assert!(h.manager.info(ID).unwrap().update_pending);
    assert_eq!(ver(&h), json!(1), "the old version keeps running while the prompt is open");
    assert_eq!(h.manager.grant(ID, Decision { accept_required: false, optional: vec![] }).unwrap(), InstallOutcome::UpdateDeclined);
    assert_eq!(ver(&h), json!(1));
    assert!(!h.manager.info(ID).unwrap().update_pending);
    // Ask again and accept.
    h.manager.install(&v2, Source::File).unwrap();
    assert_eq!(h.manager.grant(ID, accept_required()).unwrap(), InstallOutcome::Loaded { generation: 2, replaced: Some(1) });
    assert_eq!(ver(&h), json!(2));
    let mut grants = h.manager.info(ID).unwrap().grants;
    grants.sort();
    assert_eq!(grants, vec!["commands.run:app.*".to_string(), "doc.read:metadata".to_string()]);
}

#[test]
fn a_call_running_on_the_old_generation_finishes_there_while_new_calls_use_the_new_one() {
    let h = Arc::new(Harness::new());
    active_v1(&h);
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    *h.services.log_gate.lock().unwrap() = Some(LogGate { entered: entered_tx, release: release_rx });
    let worker = {
        let h = h.clone();
        std::thread::spawn(move || h.run(&cmd("logs"), &json!({})))
    };
    entered_rx.recv_timeout(Duration::from_secs(10)).expect("the guest reached host.log");
    // The call is now parked inside generation 1. Reload under it.
    assert_eq!(install_v(&h, "1.1.0", '2').unwrap(), InstallOutcome::Loaded { generation: 2, replaced: Some(1) });
    assert_eq!(h.manager.draining(ID), 1, "generation 1 still runs one call");
    assert_eq!(ver(&h), json!(2), "a new call does not wait for the old one");
    release_tx.send(()).unwrap();
    assert_eq!(worker.join().unwrap().unwrap(), json!(1), "the parked call finished with its own answer");
    assert_eq!(h.manager.draining(ID), 0, "generation 1 dropped after its last call");
}

#[test]
fn disable_unregisters_and_enable_reloads_from_the_stored_package() {
    let h = Harness::new();
    active_v1(&h);
    h.manager.disable(ID).unwrap();
    assert_eq!(h.manager.state(ID), Some(PluginState::Disabled));
    assert!(dynamic_ids(&h).is_empty());
    assert!(h.registry.resolve(&cmd("ver")).is_none());
    assert!(!h.store.load_all().unwrap()[0].enabled);
    assert_eq!(h.manager.enable(ID).unwrap(), InstallOutcome::Loaded { generation: 2, replaced: None });
    assert_eq!(ver(&h), json!(1));
    assert!(h.store.load_all().unwrap()[0].enabled);
    // Installing a new version of a disabled plugin stores it and leaves it off.
    h.manager.disable(ID).unwrap();
    assert_eq!(install_v(&h, "1.2.0", '3').unwrap(), InstallOutcome::StoredDisabled);
    assert_eq!(h.manager.state(ID), Some(PluginState::Disabled));
    assert_eq!(h.store.load_all().unwrap()[0].version, "1.2.0");
    h.manager.enable(ID).unwrap();
    assert_eq!(ver(&h), json!(3));
}

#[test]
fn remove_deletes_package_grants_state_and_commands() {
    let h = Harness::new();
    active_v1(&h);
    h.store.put(ID, "k", &json!(1), 1000).unwrap();
    h.manager.remove(ID).unwrap();
    assert_eq!(h.manager.state(ID), None);
    assert!(dynamic_ids(&h).is_empty());
    assert!(h.store.load_all().unwrap().is_empty());
    assert_eq!(h.store.get(ID, "k").unwrap(), None, "state.* data goes with the plugin");
    assert!(matches!(h.manager.remove(ID), Err(LifecycleError::NoSuchPlugin(_))));
}

#[test]
fn restore_loads_enabled_plugins_and_reports_the_ones_that_fail() {
    let h = Harness::new();
    active_v1(&h);
    // A second, disabled plugin and a third whose stored bytes are damaged.
    let other = echo_pkg("org.example.off", "1.0.0", &echo_wat_v('1'), std_contribs());
    h.manager.install(&other, Source::File).unwrap();
    h.manager.grant("org.example.off", Decision { accept_required: false, optional: vec![] }).unwrap();
    let mut damaged = h.store.load_all().unwrap().into_iter().find(|p| p.id == ID).unwrap();
    damaged.id = "org.example.bad".into();
    damaged.bytes.truncate(50);
    h.store.save(&damaged).unwrap();

    // A new app run: new manager and registry over the same store.
    let clock = StepClock::new(Duration::ZERO);
    let engine = Arc::new(WasmiEngine::new(clock.clone()).with_slice_fuel(20_000));
    let h2 = Harness::build(engine, clock, BudgetConfig::default(), h.store.clone(), h.store.clone());
    let mut report = h2.manager.restore().unwrap();
    report.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(report.len(), 3);
    assert_eq!(report[0].0, "org.example.bad");
    assert!(matches!(report[0].1, PluginState::Faulted { .. }));
    assert_eq!(report[1], ("org.example.off".to_string(), PluginState::Disabled));
    assert_eq!(report[2], (ID.to_string(), PluginState::Active { generation: 1 }));
    assert_eq!(h2.run(&cmd("echo"), &json!({"z": 1})).unwrap()["params"], json!({"z": 1}));
    assert!(matches!(h2.manager.state("org.example.bad"), Some(PluginState::Faulted { .. })));
    // The damaged package can still be removed.
    h2.manager.remove("org.example.bad").unwrap();
    assert!(h2.store.load_all().unwrap().iter().all(|p| p.id != "org.example.bad"));
}

#[test]
fn restore_refuses_to_run_a_plugin_whose_required_grant_is_missing() {
    let h = Harness::new();
    active_v1(&h);
    let mut s = h.store.load_all().unwrap().remove(0);
    s.grants.clear();
    h.store.save(&s).unwrap();
    let clock = StepClock::new(Duration::ZERO);
    let engine = Arc::new(WasmiEngine::new(clock.clone()).with_slice_fuel(20_000));
    let h2 = Harness::build(engine, clock, BudgetConfig::default(), h.store.clone(), h.store.clone());
    let report = h2.manager.restore().unwrap();
    assert!(matches!(&report[0].1, PluginState::Faulted { reason } if reason.contains("commands.run:app.*")), "{report:?}");
    assert!(dynamic_ids(&h2).is_empty());
}

#[test]
fn revoking_an_optional_permission_takes_effect_through_a_reload() {
    let h = Harness::new();
    h.manager.install(&echo_pkg(ID, "1.0.0", &echo_wat_v('1'), std_contribs()), Source::File).unwrap();
    h.manager.grant(ID, Decision { accept_required: true, optional: vec![Permission::UiContribute] }).unwrap();
    let view = |h: &Harness| {
        let mut out = None;
        h.registry.for_each(|v| {
            if v.id == cmd("echo") {
                out = Some((v.shortcut.map(str::to_owned), matches!(&v.menu, craft_command::Menu::Shared(m) if !m.is_empty())));
            }
        });
        out.unwrap()
    };
    assert_eq!(view(&h), (Some("Ctrl+E".to_string()), true));
    h.manager.revoke(ID, &Permission::UiContribute).unwrap();
    assert_eq!(view(&h), (None, false), "no menu path and no shortcut without ui.contribute");
    assert_eq!(h.manager.state(ID), Some(PluginState::Active { generation: 2 }));
    assert_eq!(h.manager.info(ID).unwrap().grants, vec!["commands.run:app.*".to_string()]);
    // Revoking a required permission switches the plugin off.
    h.manager.revoke(ID, &"commands.run:app.*".parse().unwrap()).unwrap();
    assert_eq!(h.manager.state(ID), Some(PluginState::Disabled));
}

#[test]
fn a_newer_manifest_with_an_unknown_kind_loads_partly_supported() {
    let h = Harness::new();
    let mut c = std_contribs();
    c.as_array_mut().unwrap().push(json!({ "kind": "teleport", "id": "zap", "entry": 99 }));
    c.as_array_mut().unwrap().push(json!({ "kind": "gizmo-x", "id": "zip", "entry": 98 }));
    h.manager.install(&echo_pkg(ID, "1.0.0", &echo_wat_v('1'), c), Source::File).unwrap();
    h.manager.grant(ID, accept_required()).unwrap();
    let info = h.manager.info(ID).unwrap();
    assert_eq!(info.state, PluginState::Active { generation: 1 });
    assert_eq!(info.unsupported_kinds, vec!["gizmo-x".to_string(), "teleport".to_string()]);
    assert_eq!(ver(&h), json!(1), "the known contributions run");
    assert_eq!(dynamic_ids(&h).len(), 9, "the unknown ones register nothing");
    // The stored bytes keep the manifest whole, so a newer host can run them.
    let stored = h.store.load_all().unwrap();
    assert!(stored[0].bytes.windows(8).any(|w| w == b"teleport"));
}

/// Without feature `legacy-v1` the shims are not built, so a v1 package is refused with a clear error.
#[cfg(not(feature = "legacy-v1"))]
#[test]
fn a_v1_package_is_refused_when_the_shims_are_not_built() {
    use craft_plugin_abi::LegacyAbi;
    let h = Harness::new();
    let bytes = wat::parse_str(common::gain::gain_module("org.test.gain", "")).unwrap();
    let err = h.manager.install(&bytes, Source::File).unwrap_err();
    assert!(matches!(err, LifecycleError::Rejected(LoadError::LegacyNeedsShim(LegacyAbi::Effectcraft))), "{err:?}");
    assert!(err.to_string().contains("shim"), "{err}");
    assert!(h.manager.info("org.test.gain").is_none(), "a refused package leaves no entry");
}
