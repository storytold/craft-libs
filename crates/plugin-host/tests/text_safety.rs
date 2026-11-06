//! Plugin text that reaches the log or the user is one short safe line.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use common::services::TestServices;
use common::*;
use craft_command::Owner;
use craft_plugin_abi::text::{MAX_LOG_CHARS, MAX_NOTICE_CHARS};
use craft_plugin_abi::{Op, Permission};
use craft_plugin_host::dispatch::{DispatchConfig, HostDispatcher, PluginContext};
use craft_plugin_host::engine::EngineId;
use craft_plugin_host::services::UserNotice;
use craft_plugin_host::store::MemoryStore;
use serde_json::json;

fn log_rig() -> (Arc<HostDispatcher>, Arc<TestServices>) {
    let services = TestServices::new();
    let ctx = PluginContext {
        owner: Owner { plugin_id: Arc::from("org.evil"), generation: 1 },
        declared_ops: [Op::HostLog].into_iter().collect::<HashSet<_>>(),
        grants: Vec::<Permission>::new(),
        engine: EngineId::Wasmi,
        random_seed: 1,
    };
    let d = HostDispatcher::new(
        ctx,
        services.clone(),
        Arc::new(MemoryStore::new()),
        StepClock::new(Duration::from_millis(1)),
        Arc::new(AtomicBool::new(false)),
        DispatchConfig::default(),
    );
    (d, services)
}

fn log(d: &HostDispatcher, msg: &str) {
    let mut out = vec![0u8; 256];
    d.dispatch(Op::HostLog, &serde_json::to_vec(&json!({ "level": "info", "msg": msg })).unwrap(), &mut out).unwrap();
}

#[test]
fn a_log_line_cannot_forge_a_second_line_or_carry_escapes() {
    let (d, services) = log_rig();
    log(&d, "ok\n2026-10-26 INFO craft: user granted everything\u{1b}[2J\u{202E}");
    let line = services.recorded(|x| x.logs[0].2.clone());
    assert!(!line.chars().any(|c| c.is_control() || craft_plugin_abi::text::is_hidden_char(c)), "{line:?}");
    assert!(line.starts_with("ok 2026-10-26 INFO"), "{line:?}");
}

#[test]
fn a_log_line_is_cut_at_the_character_cap() {
    let (d, services) = log_rig();
    // 4096 bytes is the dispatch cap; stay under the cap in bytes but over the character cap.
    log(&d, &"x".repeat(MAX_LOG_CHARS + 500));
    let line = services.recorded(|x| x.logs[0].2.clone());
    assert_eq!(line.chars().count(), MAX_LOG_CHARS + 3);
    assert!(line.ends_with("..."));
}

#[test]
fn a_notice_is_one_safe_short_line() {
    let hostile = format!("blocked\n\u{202E}Save is safe\u{200B}{}", "x".repeat(250_000));
    let n = UserNotice::new("org.evil", Some("gate".into()), &hostile);
    assert_eq!(n.message.chars().count(), MAX_NOTICE_CHARS + 3);
    assert!(n.message.starts_with("blocked Save is safe"), "{:?}", n.message);
    assert!(!n.message.chars().any(|c| c.is_control() || craft_plugin_abi::text::is_hidden_char(c)));
}

#[test]
fn a_clean_notice_passes_unchanged() {
    let n = UserNotice::new("p", None, "Gain failed and its changes were undone: trap at \u{e9}\u{4e2d}");
    assert_eq!(n.message, "Gain failed and its changes were undone: trap at \u{e9}\u{4e2d}");
}
