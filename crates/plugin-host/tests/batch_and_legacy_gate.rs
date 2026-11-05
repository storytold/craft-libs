//! Two rules. `"budget":"batch"` must not remove the deadline and the fuel cap, and an `ec_` or
//! `pc_` export prefix must not skip the resource caps of the policy gate. Each test asserts the
//! safe behavior and fails if the rule is removed. Grant and namespace rules are in `grants_and_namespace.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::mpsc;
use std::time::Duration;

use common::services::*;
use common::*;
use craft_plugin_host::budget::BudgetConfig;
use craft_plugin_host::lifecycle::{Decision, InstallOutcome};
use craft_plugin_host::store::Source;
use serde_json::json;

// ---------------------------------------------------------------------------------------------
// `"budget": "batch"` is refused on the UI thread, and finite where it is allowed
// ---------------------------------------------------------------------------------------------

fn spin_pkg(id: &str, budget: &str) -> Vec<u8> {
    // entry 2 of the echo fixture spins forever. The commands.run permission is replaced by none.
    let mut m = echo_manifest(id, "1.0.0", json!([{ "kind": "command", "id": "spin", "entry": 2, "budget": budget }]));
    m.as_object_mut().unwrap().remove("permissions");
    package_from(&fixture_text("echo"), &m)
}

#[test]
fn a_zero_permission_batch_command_is_refused_at_install() {
    let h = Harness::new();
    let err = h
        .manager
        .install(&spin_pkg("org.spin.batch", "batch"), Source::File)
        .expect_err("batch on a UI command with no link permission must not install");
    assert!(err.to_string().contains("batch"), "{err}");
}

#[test]
fn a_batch_command_with_a_link_permission_still_hits_the_hard_cap() {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        // 50 us per clock tick; a 100 ms batch cap is about 2000 ticks.
        let cfg = BudgetConfig { batch_cap: Duration::from_millis(100), ..BudgetConfig::default() };
        let h = Harness::with_clock(Duration::from_micros(50), cfg);
        let id = "org.spin.link";
        let mut m = echo_manifest(id, "1.0.0", json!([{ "kind": "command", "id": "spin", "entry": 2, "budget": "batch" }]));
        m["permissions"] = json!({ "required": ["link.call:filmcraft"] });
        let out = h.manager.install(&package_from(&fixture_text("echo"), &m), Source::File).unwrap();
        assert!(matches!(out, InstallOutcome::NeedsGrant(_)), "{out:?}");
        h.manager.grant(id, Decision { accept_required: true, optional: vec![] }).unwrap();
        let r = h.run(&format!("plugin.{id}.spin"), &json!({}));
        let _ = tx.send(r.is_err());
    });
    assert_eq!(rx.recv_timeout(Duration::from_secs(20)), Ok(true), "the batch spin must trap on the batch cap");
}

// ---------------------------------------------------------------------------------------------
// An `ec_` export prefix skips the whole policy gate; the EffectCraft shim sets no table cap
// ---------------------------------------------------------------------------------------------

#[cfg(feature = "legacy-v1")]
fn vm_size_kb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with("VmSize:")).unwrap();
    line.split_whitespace().nth(1).unwrap().parse().unwrap()
}

#[cfg(feature = "legacy-v1")]
#[test]
fn an_ec_prefixed_module_no_longer_skips_the_gate_or_gets_an_800_mb_table() {
    use craft_plugin_abi::{HostPolicy, check_package};
    let text = common::gain::gain_module("org.evil.gain", "");
    // 200 million funcref slots = about 800 MB. The gate allows 100,000.
    let with_table = text.replacen("(memory (export \"memory\") 2)", "(memory (export \"memory\") 2) (table 200000000 200000000 funcref)", 1);
    assert_ne!(with_table, text);
    let wasm = wat::parse_str(&with_table).unwrap();
    // The gate still says "legacy", and now lists the table violation, as it does for the renamed control.
    let report = check_package(&wasm, &HostPolicy::native());
    assert!(report.legacy.is_some());
    assert!(report.violations.iter().any(|v| matches!(v, craft_plugin_abi::Violation::TableTooLarge { .. })), "{:?}", report.violations);
    // The shim refuses it before wasmi allocates the table.
    let before = vm_size_kb();
    let clock = StepClock::new(Duration::ZERO);
    let plugin = craft_plugin_host::shim::load_legacy(&wasm, clock);
    let grew_mb = vm_size_kb().saturating_sub(before) / 1024;
    assert!(plugin.is_err(), "the legacy shim accepted the oversized table");
    assert!(grew_mb < 100, "the table was allocated anyway, VmSize grew {grew_mb} MB");
}
