//! Grant and namespace rules. A plugin may not use `link.call` to drive the peer app, a package
//! with the same id must not keep the old grants when its publisher or risk changes, and a plugin
//! may not take a host command id. Each test asserts the safe behavior and fails if the rule is removed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use common::services::*;
use common::*;
use craft_command::{DynCommand, Enablement, Owner, Visibility};
use craft_plugin_abi::{Op, Permission, Status};
use craft_plugin_host::budget::BudgetConfig;
use craft_plugin_host::dispatch::{DispatchConfig, HostDispatcher, PluginContext};
use craft_plugin_host::engine::wasmi::WasmiEngine;
use craft_plugin_host::engine::{EngineId, HostError, LoadError};
use craft_plugin_host::lifecycle::{Decision, InstallOutcome, LifecycleError, PluginState, UpdateRisk};
use craft_plugin_host::store::{DirStore, MemoryStore, PackageStore, Source, StoredGrant, StoredPlugin};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------------------------
// link.call stays inside its closed lists
// ---------------------------------------------------------------------------------------------

struct Rig {
    d: Arc<HostDispatcher>,
    services: Arc<TestServices>,
}

fn rig(grants: &[&str]) -> Rig {
    let services = TestServices::new();
    let ctx = PluginContext {
        owner: Owner { plugin_id: Arc::from("org.evil"), generation: 1 },
        declared_ops: [Op::LinkCall, Op::CmdExecute, Op::CmdList].into_iter().collect::<HashSet<_>>(),
        grants: grants.iter().map(|g| g.parse::<Permission>().unwrap()).collect(),
        engine: EngineId::Wasmi,
        random_seed: 1,
    };
    let d = HostDispatcher::new(
        ctx,
        services.clone(),
        Arc::new(MemoryStore::new()),
        StepClock::new(Duration::ZERO),
        Arc::new(AtomicBool::new(false)),
        DispatchConfig::default(),
    );
    Rig { d, services }
}

impl Rig {
    fn call(&self, op: Op, req: &Value) -> Result<Value, HostError> {
        let mut out = vec![0u8; 8192];
        let n = self.d.dispatch(op, &serde_json::to_vec(req).unwrap(), &mut out)?;
        Ok(serde_json::from_slice(&out[..n]).unwrap())
    }

    fn link(&self, req: &Value) -> Value {
        self.call(Op::LinkCall, req).expect("the dispatcher answers with an error object")
    }

    fn sent(&self) -> Vec<(String, Value)> {
        self.services.recorded(|r| r.link_calls.iter().map(|c| (c.2.clone(), c.3.clone())).collect())
    }
}

fn is_refused(reply: &Value) -> bool {
    reply.get("error").is_some() && reply.get("result").is_none()
}

#[test]
fn link_call_grant_does_not_reach_engine_execute_or_host_management_commands() {
    let r = rig(&["link.call:filmcraft"]); // and NOTHING else: no commands.run, no link.exec
    for req in [
        json!({"app":"filmcraft","method":"engine.execute","params":{"command":"plugin.app.filmcraft.install","params":{"path":"/tmp/x.wasm"}}}),
        json!({"app":"filmcraft","method":"engine.execute","params":{"command":"plugin.app.filmcraft.grant","params":{"id":"x","accept":true}}}),
        json!({"app":"filmcraft","method":"engine.execute","params":{"command":"effect.add"}}),
        json!({"app":"filmcraft","method":"ui.menu.invoke","params":{"command":"plugin.app.filmcraft.install"}}),
        json!({"app":"filmcraft","method":"ui.screenshot","params":{"path":"/tmp/victim-file"}}),
        json!({"app":"filmcraft","method":"render.frame","params":{"path":"/tmp/victim-file"}}),
        json!({"app":"filmcraft","method":"app.quit","params":{"force":true}}),
        json!({"app":"filmcraft","method":"link.open","params":{"result_path":"/home/u/.bashrc"}}),
        json!({"app":"filmcraft","method":"link.close","params":{}}),
        json!({"app":"filmcraft","method":"perf.stats"}),
        json!({"app":"filmcraft","method":"engine.commands"}),
    ] {
        let reply = r.link(&req);
        assert!(is_refused(&reply), "must be refused: {req} -> {reply}");
    }
    assert!(r.sent().is_empty(), "no refused call may reach the service: {:?}", r.sent());
}

#[test]
fn the_refusal_names_the_method_and_what_link_call_does_allow() {
    let r = rig(&["link.call:filmcraft"]);
    let reply = r.link(&json!({"app":"filmcraft","method":"app.quit"}));
    let text = reply["error"].as_str().unwrap();
    assert!(text.contains("app.quit") && text.contains("link.resolve"), "{text}");
}

#[test]
fn link_call_forwards_the_read_only_methods() {
    let r = rig(&["link.call:filmcraft"]);
    for m in ["link.resolve", "asset.describe", "link.refresh", "link.focus"] {
        let reply = r.link(&json!({"app":"filmcraft","method":m,"params":{"link_id":"a"}}));
        assert!(!is_refused(&reply), "{m}: {reply}");
    }
    assert_eq!(r.sent().len(), 4);
}

#[test]
fn method_name_variants_do_not_slip_past_the_list() {
    let r = rig(&["link.call:filmcraft"]);
    for m in [
        "Engine.Execute",
        "ENGINE.EXECUTE",
        "engine.execute ",
        " engine.execute",
        "engine.execute\0",
        "engine.execute\n",
        "engine\u{2024}execute",
        "",
        "link.resolve.",
        ".link.resolve",
    ] {
        let outcome = r.call(Op::LinkCall, &json!({"app":"filmcraft","method":m,"params":{"command":"x"}}));
        match outcome {
            Err(HostError::Status(Status::BadArg)) => {}
            Ok(reply) => assert!(is_refused(&reply), "{m:?}: {reply}"),
            Err(other) => panic!("{m:?}: {other:?}"),
        }
    }
    assert!(r.sent().is_empty(), "{:?}", r.sent());
}

#[test]
fn engine_execute_needs_link_exec_for_that_command() {
    let r = rig(&["link.call:filmcraft", "link.exec:filmcraft:effect.*"]);
    let ok = r.link(&json!({"app":"filmcraft","method":"engine.execute","params":{"command":"effect.add","params":{"n":1}}}));
    assert!(!is_refused(&ok), "{ok}");
    assert_eq!(r.sent(), vec![("engine.execute".to_owned(), json!({"command":"effect.add","params":{"n":1}}))]);
    for command in ["project.save", "plugin.app.filmcraft.install", "Effect.add", "xeffect.add", "effect.add\n", "effect.add ", "effect.\u{ff41}dd"] {
        let reply = r.link(&json!({"app":"filmcraft","method":"engine.execute","params":{"command":command}}));
        let text = reply["error"].as_str().unwrap_or_else(|| panic!("{command:?} must be refused: {reply}"));
        assert!(text.contains("link.exec:filmcraft"), "{text}");
    }
    // Another app is not covered.
    let other = r.link(&json!({"app":"effectcraft","method":"engine.execute","params":{"command":"effect.add"}}));
    assert_eq!(other, json!({"error": "missing permission link.call:effectcraft"}));
    assert_eq!(r.sent().len(), 1);
}

#[test]
fn link_exec_rebuilds_the_params_so_a_second_command_key_cannot_ride_along() {
    let r = rig(&["link.call:filmcraft", "link.exec:filmcraft:effect.*"]);
    // The peer reads `command` or `id`. The host sends `command` only.
    let reply = r.link(&json!({"app":"filmcraft","method":"engine.execute","params":{
        "command":"effect.add","id":"plugin.app.filmcraft.install","path":"/etc/passwd","params":{"a":1}}}));
    assert!(!is_refused(&reply), "{reply}");
    assert_eq!(r.sent(), vec![("engine.execute".to_owned(), json!({"command":"effect.add","params":{"a":1}}))]);
    // A missing `command` or a non-text one is a bad argument, before any grant check.
    for params in [json!({}), json!({"id":"effect.add"}), json!({"command":7}), json!({"command":null}), json!({"command":["effect.add"]})] {
        let out = r.call(Op::LinkCall, &json!({"app":"filmcraft","method":"engine.execute","params":params}));
        assert!(matches!(out, Err(HostError::Status(Status::BadArg))), "{params}: {out:?}");
    }
    let out = r.call(Op::LinkCall, &json!({"app":"filmcraft","method":"engine.execute","params":{"command":"effect.add","params":[1]}}));
    assert!(matches!(out, Err(HostError::Status(Status::BadArg))), "{out:?}");
    assert_eq!(r.sent().len(), 1);
}

#[test]
fn link_exec_never_reaches_a_host_command_even_with_a_match_all_glob() {
    let r = rig(&["link.call:filmcraft", "link.exec:filmcraft:*"]);
    for command in [
        "plugin.app.filmcraft.install",
        "plugin.app.filmcraft.grant",
        "plugin.app.filmcraft.enable",
        "plugin.app.filmcraft.remove",
        "plugin.app.filmcraft.revoke",
        "plugin.app.effectcraft.install",
        "PLUGIN.APP.FILMCRAFT.INSTALL",
        "Plugin.App.filmcraft.install",
    ] {
        let reply = r.link(&json!({"app":"filmcraft","method":"engine.execute","params":{"command":command}}));
        assert!(is_refused(&reply), "{command}: {reply}");
    }
    assert!(r.sent().is_empty());
    // A match-all glob still runs ordinary commands. That is what the user consented to.
    assert!(!is_refused(&r.link(&json!({"app":"filmcraft","method":"engine.execute","params":{"command":"layer.add"}}))));
}

#[test]
fn link_exec_alone_is_not_link_call() {
    let r = rig(&["link.exec:filmcraft:effect.*"]);
    let out = r.call(Op::LinkCall, &json!({"app":"filmcraft","method":"engine.execute","params":{"command":"effect.add"}}));
    assert!(matches!(out, Err(HostError::Status(Status::Denied))), "{out:?}");
    assert!(r.sent().is_empty());
}

#[test]
fn commands_run_cannot_name_a_host_command_on_cmd_execute() {
    let r = rig(&["commands.run"]);
    r.services.commands.lock().unwrap().extend(
        ["plugin.app.filmcraft.install", "effect.add"]
            .into_iter()
            .map(|id| craft_plugin_host::services::CommandInfo { id: id.to_owned(), label: id.to_owned() }),
    );
    for id in ["plugin.app.filmcraft.install", "PLUGIN.APP.FILMCRAFT.INSTALL", "plugin.app.x"] {
        let out = r.call(Op::CmdExecute, &json!({"id": id}));
        assert!(matches!(out, Err(HostError::Status(Status::Denied))), "{id}: {out:?}");
    }
    assert!(r.services.recorded(|x| x.executed.is_empty()), "the host command must not run");
    // The list leaves the host commands out.
    let listed = r.call(Op::CmdList, &json!({})).unwrap();
    let ids: Vec<&str> = listed["commands"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"effect.add"), "{ids:?}");
    assert!(ids.iter().all(|id| !id.starts_with("plugin.app.")), "{ids:?}");
    // An ordinary command still runs.
    r.call(Op::CmdExecute, &json!({"id": "effect.add"})).unwrap();
    assert_eq!(r.services.recorded(|x| x.executed.len()), 1);
}

// ---------------------------------------------------------------------------------------------
// A same-id update
// ---------------------------------------------------------------------------------------------

const TRUSTED: &str = "org.trusted";

fn signed(author: Option<&str>, version: &str, digit: char) -> Vec<u8> {
    let mut m = echo_manifest(TRUSTED, version, std_contribs());
    match author {
        Some(a) => m["author"] = json!(a),
        None => {
            m.as_object_mut().unwrap().remove("author");
        }
    }
    package_from(&echo_wat_v(digit), &m)
}

fn accept() -> Decision {
    Decision { accept_required: true, optional: vec![] }
}

/// Version 1.0.0 by `author`, installed with its grants accepted.
fn trusted(author: Option<&str>) -> Harness {
    let h = Harness::new();
    assert!(matches!(h.manager.install(&signed(author, "1.0.0", '1'), Source::File).unwrap(), InstallOutcome::NeedsGrant(_)));
    h.manager.grant(TRUSTED, accept()).unwrap();
    assert_eq!(ver(&h), json!(1));
    h
}

fn ver(h: &Harness) -> Value {
    h.run(&format!("plugin.{TRUSTED}.ver"), &json!({})).unwrap()
}

fn needs_grant(out: InstallOutcome) -> craft_plugin_host::lifecycle::GrantRequest {
    match out {
        InstallOutcome::NeedsGrant(r) => r,
        other => panic!("expected a prompt, got {other:?}"),
    }
}

#[test]
fn a_different_publisher_with_the_same_id_is_asked_about_and_inherits_nothing() {
    let h = trusted(Some("Alice"));
    // Mallory ships different code, a different author, a LOWER version, and the same permissions.
    let req = needs_grant(h.manager.install(&signed(Some("Mallory"), "0.0.1", '9'), Source::File).unwrap());
    assert_eq!(req.risk, UpdateRisk::Publisher { installed: Some("Alice".into()) });
    assert_eq!(req.new_required, req.required, "every required permission is asked again");
    assert_eq!(ver(&h), json!(1), "the old code keeps running while the prompt is open");
    assert!(h.manager.info(TRUSTED).unwrap().grants.iter().any(|g| g == "commands.run:app.*"));
    // Decline: the old plugin stays, with its grants.
    assert_eq!(h.manager.grant(TRUSTED, Decision { accept_required: false, optional: vec![] }).unwrap(), InstallOutcome::UpdateDeclined);
    assert_eq!(ver(&h), json!(1));
    assert_eq!(h.manager.info(TRUSTED).unwrap().grants, vec!["commands.run:app.*".to_string()]);
    // Accept: the new code runs with exactly the grants the user accepted now.
    needs_grant(h.manager.install(&signed(Some("Mallory"), "0.0.1", '9'), Source::File).unwrap());
    assert!(matches!(h.manager.grant(TRUSTED, accept()).unwrap(), InstallOutcome::Loaded { generation: 2, .. }));
    assert_eq!(ver(&h), json!(9));
}

#[test]
fn a_risky_update_drops_grants_the_new_package_no_longer_lists() {
    let h = Harness::new();
    let mut m = echo_manifest(TRUSTED, "1.0.0", std_contribs());
    m["author"] = json!("Alice");
    m["permissions"]["required"] = json!(["commands.run:app.*", "doc.read:metadata"]);
    h.manager.install(&package_from(&echo_wat_v('1'), &m), Source::File).unwrap();
    h.manager.grant(TRUSTED, accept()).unwrap();
    assert_eq!(h.manager.info(TRUSTED).unwrap().grants.len(), 2);
    // Another publisher asks for one of the two only. The other grant must not survive.
    let mut evil = echo_manifest(TRUSTED, "9.0.0", std_contribs());
    evil["author"] = json!("Mallory");
    needs_grant(h.manager.install(&package_from(&echo_wat_v('9'), &evil), Source::File).unwrap());
    h.manager.grant(TRUSTED, accept()).unwrap();
    assert_eq!(h.manager.info(TRUSTED).unwrap().grants, vec!["commands.run:app.*".to_string()]);
}

#[test]
fn the_same_publisher_needs_a_higher_version_to_keep_its_grants() {
    for (version, why) in
        [("0.9.0", "lower"), ("1.0.0", "equal"), ("latest", "not semver"), ("1.0.0-rc.1", "pre-release of the installed core"), ("", "empty")]
    {
        let h = trusted(Some("Alice"));
        // The manifest rules reject an empty version string only if the schema says so. Skip what the gate refuses.
        let Ok(out) = h.manager.install(&signed(Some("Alice"), version, '2'), Source::File) else { continue };
        let req = needs_grant(out);
        assert_eq!(req.risk, UpdateRisk::NotNewer { installed: "1.0.0".into() }, "{why}");
        assert_eq!(ver(&h), json!(1), "{why}");
    }
}

#[test]
fn the_same_publisher_with_a_higher_version_updates_without_a_prompt_and_keeps_its_grants() {
    for next in ["1.0.1", "1.1.0", "2.0.0", "10.0.0"] {
        let h = trusted(Some("Alice"));
        let out = h.manager.install(&signed(Some("Alice"), next, '2'), Source::File).unwrap();
        assert!(matches!(out, InstallOutcome::Loaded { generation: 2, .. }), "{next}: {out:?}");
        assert_eq!(h.manager.info(TRUSTED).unwrap().grants, vec!["commands.run:app.*".to_string()]);
        assert_eq!(ver(&h), json!(2));
    }
}

#[test]
fn a_release_replaces_its_own_pre_release_and_a_pre_release_does_not_replace_a_release() {
    let h = Harness::new();
    let mut m = echo_manifest(TRUSTED, "1.0.0-rc.1", std_contribs());
    m["author"] = json!("Alice");
    h.manager.install(&package_from(&echo_wat_v('1'), &m), Source::File).unwrap();
    h.manager.grant(TRUSTED, accept()).unwrap();
    assert!(matches!(h.manager.install(&signed(Some("Alice"), "1.0.0", '2'), Source::File).unwrap(), InstallOutcome::Loaded { .. }));
    let req = needs_grant(h.manager.install(&signed(Some("Alice"), "1.0.0-rc.2", '3'), Source::File).unwrap());
    assert!(matches!(req.risk, UpdateRisk::NotNewer { .. }));
}

#[test]
fn publisher_names_are_compared_exactly() {
    for lookalike in ["alice", "ALICE", "Alice ", " Alice", "\u{410}lice", "Al\u{131}ce", "Alic\u{435}", ""] {
        let h = trusted(Some("Alice"));
        let req = needs_grant(h.manager.install(&signed(Some(lookalike), "2.0.0", '2'), Source::File).unwrap());
        assert_eq!(req.risk, UpdateRisk::Publisher { installed: Some("Alice".into()) }, "{lookalike:?}");
    }
}

#[test]
fn a_publisher_name_with_invisible_characters_is_refused_at_install() {
    // The gate rejects zero-width, bidi and control characters in `author`, so these lookalikes never reach the publisher compare.
    for lookalike in ["Alice\u{200b}", "Alice\u{202e}", "Ali\u{0}ce"] {
        let h = trusted(Some("Alice"));
        let err = h.manager.install(&signed(Some(lookalike), "2.0.0", '2'), Source::File).unwrap_err();
        assert!(err.to_string().contains("author"), "{lookalike:?}: {err}");
    }
}

#[test]
fn adding_or_removing_the_publisher_name_is_a_publisher_change() {
    let h = trusted(Some("Alice"));
    let req = needs_grant(h.manager.install(&signed(None, "2.0.0", '2'), Source::File).unwrap());
    assert_eq!(req.risk, UpdateRisk::Publisher { installed: Some("Alice".into()) });
    let h = trusted(None);
    let req = needs_grant(h.manager.install(&signed(Some("Alice"), "2.0.0", '2'), Source::File).unwrap());
    assert_eq!(req.risk, UpdateRisk::Publisher { installed: None });
    // Two anonymous packages count as one publisher, and the version rule still applies.
    let h = trusted(None);
    assert!(matches!(h.manager.install(&signed(None, "1.0.1", '2'), Source::File).unwrap(), InstallOutcome::Loaded { generation: 2, .. }));
    let h = trusted(None);
    assert!(matches!(needs_grant(h.manager.install(&signed(None, "0.1.0", '2'), Source::File).unwrap()).risk, UpdateRisk::NotNewer { .. }));
}

#[test]
fn installing_the_same_bytes_again_is_not_an_update() {
    let h = trusted(Some("Alice"));
    let out = h.manager.install(&signed(Some("Alice"), "1.0.0", '1'), Source::File).unwrap();
    assert!(matches!(out, InstallOutcome::Loaded { .. }), "{out:?}");
    assert_eq!(h.manager.info(TRUSTED).unwrap().grants, vec!["commands.run:app.*".to_string()]);
}

#[test]
fn an_id_that_differs_only_by_letter_case_is_refused() {
    let h = trusted(Some("Alice"));
    let mut m = echo_manifest("Org.Trusted", "9.0.0", std_contribs());
    m["author"] = json!("Alice");
    let err = h.manager.install(&package_from(&echo_wat_v('9'), &m), Source::File).unwrap_err();
    assert!(matches!(&err, LifecycleError::IdCollision { id, existing } if id == "Org.Trusted" && existing == TRUSTED), "{err:?}");
    assert!(err.to_string().contains("letter case"));
    assert_eq!(ver(&h), json!(1));
}

// ---------------------------------------------------------------------------------------------
// Host command namespace and a start that survives one bad plugin
// ---------------------------------------------------------------------------------------------

fn no_permission_pkg(id: &str, contribution: &str) -> Vec<u8> {
    let mut m = echo_manifest(id, "1.0.0", json!([{ "kind": "command", "id": contribution, "entry": 0 }]));
    m.as_object_mut().unwrap().remove("permissions");
    package_from(&fixture_text("echo"), &m)
}

/// The app registers its management commands under the pseudo plugin id `app.<app>`, so the id is
/// `plugin.app.<app>.<verb>`. That is why a real plugin with the id `app.<app>` collided with it.
fn host_command(id: &str) -> DynCommand<Sess, AppError> {
    DynCommand {
        id: Arc::from(id),
        label: Arc::from("Install Plug-in"),
        params: Arc::from("{}"),
        shortcut: None,
        menu: Arc::from(Vec::new()),
        owner: Owner { plugin_id: Arc::from("app.filmcraft"), generation: 0 },
        enablement: Enablement::Always,
        visibility: Visibility { menu: false, palette: true, shortcut: false },
        journal: false,
        run: Arc::new(|_s: &mut Sess, _p: &Value| Ok(Value::Null)),
    }
}

#[test]
fn a_plugin_cannot_take_an_id_in_the_host_namespace() {
    let h = Harness::new();
    for id in ["app.filmcraft", "App.filmcraft", "APP.FILMCRAFT", "app.", "app", "App", "plugin.x", "Plugin.app.filmcraft", "plugin"] {
        let err = h.manager.install(&no_permission_pkg(id, "install"), Source::File).unwrap_err();
        assert!(matches!(err, LifecycleError::Rejected(LoadError::Gate(_))), "{id}: {err:?}");
        assert!(err.to_string().contains("reserved"), "{id}: {err}");
    }
    assert!(h.manager.list().is_empty());
    // The host registers its own commands at any time and finds the ids free.
    h.registry.register(host_command("plugin.app.filmcraft.install")).unwrap();
}

#[test]
fn near_miss_ids_still_install() {
    let h = Harness::new();
    for id in ["apps.filmcraft", "my.app.x", "application", "plugins.x"] {
        let out = h.manager.install(&no_permission_pkg(id, "install"), Source::File).unwrap();
        assert!(matches!(out, InstallOutcome::Loaded { generation: 1, .. }), "{id}: {out:?}");
    }
}

static NEXT: AtomicU32 = AtomicU32::new(0);

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Tmp {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("craft-plugin-host-sec-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        Tmp(p)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn dir_harness(path: &std::path::Path) -> Harness {
    let clock = StepClock::new(Duration::ZERO);
    let engine = Arc::new(WasmiEngine::new(clock.clone()).with_slice_fuel(20_000));
    Harness::build(engine, clock, BudgetConfig::default(), Arc::new(DirStore::new(path)), Arc::new(MemoryStore::new()))
}

fn stored(id: &str, bytes: Vec<u8>, grant: Option<&str>) -> StoredPlugin {
    StoredPlugin {
        id: id.to_owned(),
        version: "1.0.0".into(),
        bytes,
        enabled: true,
        installed_at_ms: 1,
        source: Source::File,
        grants: grant
            .map(|g| StoredGrant {
                plugin_id: id.to_owned(),
                publisher: None,
                permission: g.to_owned(),
                granted_at_ms: 1,
                decided_by: craft_plugin_host::store::Decider::User,
            })
            .into_iter()
            .collect(),
    }
}

#[test]
fn start_survives_a_squatter_already_on_disk_and_other_bad_entries() {
    let tmp = Tmp::new();
    let store = DirStore::new(&tmp.0);
    // 1. a package an older build accepted: it squats the host namespace
    store.save(&stored("app.filmcraft", no_permission_pkg("app.filmcraft", "install"), None)).unwrap();
    // 2. a good plugin
    store.save(&stored("org.good", echo_package("org.good", "1.0.0", std_contribs()), Some("commands.run:app.*"))).unwrap();
    // 3. a directory with broken metadata, 4. a directory that holds another id, 5. a package that is not wasm
    let plugins = tmp.0.join("plugins");
    std::fs::create_dir_all(plugins.join("org.garbage")).unwrap();
    std::fs::write(plugins.join("org.garbage").join("meta.json"), b"{ not json").unwrap();
    std::fs::write(plugins.join("org.garbage").join("package.wasm"), b"x").unwrap();
    store.save(&stored("org.moved", echo_package("org.moved", "1.0.0", std_contribs()), None)).unwrap();
    std::fs::rename(plugins.join("org.moved"), plugins.join("org.elsewhere")).unwrap();
    store.save(&stored("org.junk", b"not wasm".to_vec(), None)).unwrap();

    let h = dir_harness(&tmp.0);
    // PluginHost::start order: restore first, then the host registers its own commands.
    let report = h.manager.restore().expect("one bad entry must not fail the start");
    let state = |id: &str| report.iter().find(|(i, _)| i == id).map(|(_, s)| s.clone());
    assert!(matches!(state("org.good"), Some(PluginState::Active { .. })), "{report:?}");
    for bad in ["app.filmcraft", "org.garbage", "org.elsewhere", "org.junk"] {
        assert!(matches!(state(bad), Some(PluginState::Faulted { .. })), "{bad}: {report:?}");
        assert!(matches!(h.manager.state(bad), Some(PluginState::Faulted { .. })), "{bad}");
    }
    h.registry.register(host_command("plugin.app.filmcraft.install")).expect("the squatter must not own the host command id");
    assert_eq!(h.run("plugin.org.good.ver", &json!({})).unwrap(), json!(1));
    // The user can remove a bad entry.
    h.manager.remove("app.filmcraft").unwrap();
    assert_eq!(h.manager.state("app.filmcraft"), None);
}

#[test]
fn ids_that_differ_only_by_case_on_disk_load_one_and_fault_the_other() {
    let tmp = Tmp::new();
    let store = DirStore::new(&tmp.0);
    store.save(&stored("Org.Dup", echo_package("Org.Dup", "1.0.0", std_contribs()), Some("commands.run:app.*"))).unwrap();
    store.save(&stored("org.dup", echo_package("org.dup", "1.0.0", std_contribs()), Some("commands.run:app.*"))).unwrap();
    let h = dir_harness(&tmp.0);
    let report = h.manager.restore().unwrap();
    let active = report.iter().filter(|(_, s)| matches!(s, PluginState::Active { .. })).count();
    let faulted = report.iter().filter(|(_, s)| matches!(s, PluginState::Faulted { .. })).count();
    assert_eq!((active, faulted), (1, 1), "{report:?}");
}

#[test]
fn the_strict_store_call_still_fails_on_a_bad_entry_and_the_lenient_one_does_not() {
    let tmp = Tmp::new();
    let store = DirStore::new(&tmp.0);
    store.save(&stored("org.good", b"w".to_vec(), None)).unwrap();
    std::fs::create_dir_all(tmp.0.join("plugins").join("org.bad")).unwrap();
    assert!(store.load_all().is_err());
    let lenient = store.load_all_lenient().unwrap();
    assert_eq!(lenient.plugins.len(), 1);
    assert_eq!(lenient.skipped.len(), 1);
    assert_eq!(lenient.skipped[0].name, "org.bad");
}
