//! The VectorCraft v1 shim against a generated stand-in plugin (`common/v1.rs`) and a matrix of
//! hostile modules.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use common::v1::{ECHO_INPUT, build, desaturate_module, vc_module, vc_wat as wat_module};
use craft_plugin_abi::LegacyAbi;
use craft_plugin_abi::manifest::{Instance, Kind as CraftKind, LegacyTag, ParamType};
use craft_plugin_host::engine::SystemClock;
use craft_plugin_host::shim::vectorcraft::{self, Kind, Limits, VectorcraftPlugin};
use craft_plugin_host::shim::{LegacyPlugin, ShimError, detect, load_legacy};
use serde_json::{Value, json};

static DESATURATE: LazyLock<Vec<u8>> = LazyLock::new(desaturate_module);
const MANIFEST: &str = r#"{"id":"test.plugin","name":"Test","version":"1","kind":"filter"}"#;
const INPUT: &str = r#"{"objects":[]}"#;

fn module(manifest: &str, run: &str, extra: &str) -> Vec<u8> {
    vc_module(manifest, run, extra)
}

fn ok_module(run: &str) -> Vec<u8> {
    module(MANIFEST, run, "")
}

const ECHO: &str = ECHO_INPUT;

fn clock() -> Arc<SystemClock> {
    Arc::new(SystemClock::new())
}

fn load(bytes: &[u8], limits: Limits) -> Result<VectorcraftPlugin, ShimError> {
    VectorcraftPlugin::load(bytes, limits, clock())
}

fn run_err(bytes: &[u8], limits: Limits) -> (ShimError, Duration) {
    let t = Instant::now();
    let r = load(bytes, limits).and_then(|p| p.run(INPUT.as_bytes(), b"{}"));
    (r.expect_err("hostile module must fail"), t.elapsed())
}

#[test]
fn the_v1_limits_are_the_numbers_vectorcraft_ships() {
    let l = Limits::default();
    assert_eq!((l.max_module_bytes, l.max_memory_bytes, l.max_output_bytes), (32 << 20, 512 << 20, 64 << 20));
    assert_eq!((l.fuel_base, l.fuel_per_byte, l.max_recursion_depth, l.wall_time_ms), (50_000_000, 4_000, 1024, 60_000));
    assert_eq!(vectorcraft::LIVE_EFFECT_WALL_MS, 5_000);
}

#[test]
fn the_desaturate_plugin_loads_and_reports_its_manifest() {
    let p = load(&DESATURATE, Limits::default()).unwrap();
    let m = p.manifest();
    assert_eq!((m.id.as_str(), m.name.as_str(), m.kind), ("test.vectorcraft.desaturate", "Desaturate", Kind::Filter));
    assert_eq!(m.params_doc(), "{amount: 0..100 (100)}");
    assert_eq!(m.defaults()["amount"], json!(100.0));
    assert_eq!(p.size(), DESATURATE.len());
}

#[test]
fn input_and_params_reach_the_plugin_and_output_comes_back() {
    let p = load(&module(MANIFEST, ECHO, ""), Limits::default()).unwrap();
    let input = br#"{"objects":[{"id":1}]}"#;
    assert_eq!(p.run(input, b"{}").unwrap(), input);
    let params = module(MANIFEST, "(i64.or (i64.shl (i64.extend_i32_u (local.get $pl)) (i64.const 32)) (i64.extend_i32_u (local.get $pp)))", "");
    assert_eq!(load(&params, Limits::default()).unwrap().run(input, br#"{"amount":5}"#).unwrap(), br#"{"amount":5}"#);
}

#[test]
fn hostile_modules_fail_quickly_with_the_right_class_of_error() {
    let quick = Duration::from_secs(5);
    type Check = fn(&ShimError) -> bool;
    let cases: Vec<(&str, Vec<u8>, Check)> = vec![
        ("infinite loop", ok_module("(loop $l (br $l)) (i64.const 0)"), |e| matches!(e, ShimError::Limit(m) if m == "instruction budget")),
        ("memory.grow bomb, then trap", ok_module("(loop $l (br_if $l (i32.ne (memory.grow (i32.const 512)) (i32.const -1)))) unreachable"), |e| {
            matches!(e, ShimError::Trap(_) | ShimError::Limit(_))
        }),
        (
            "memory.grow forever",
            ok_module("(loop $l (drop (memory.grow (i32.const 1))) (br $l)) (i64.const 0)"),
            |e| matches!(e, ShimError::Limit(m) if m == "instruction budget"),
        ),
        ("out-of-bounds write", ok_module("(i32.store (i32.const -4) (i32.const 0)) (i64.const 0)"), |e| matches!(e, ShimError::Trap(_))),
        ("trap", ok_module("unreachable"), |e| matches!(e, ShimError::Trap(_))),
        ("division by zero", ok_module("(drop (i32.div_u (i32.const 1) (i32.const 0))) (i64.const 0)"), |e| matches!(e, ShimError::Trap(_))),
        ("deep recursion", module(MANIFEST, "(call $r) (i64.const 0)", "(func $r (call $r))"), |e| matches!(e, ShimError::Trap(_))),
        ("error code", ok_module("(i64.const -7)"), |e| matches!(e, ShimError::Failed(m) if m.contains("-7"))),
        ("output outside memory", ok_module("(i64.const 0x00000100fffffff0)"), |e| matches!(e, ShimError::Abi(_))),
        ("huge output", ok_module("(i64.const 0x7fffffff00000000)"), |e| matches!(e, ShimError::Limit(m) if m.contains("output size"))),
        (
            "wrong ABI version",
            build(&wat_module(MANIFEST, "(i64.const 0)", "", 1).replace("(result i32) (i32.const 1))", "(result i32) (i32.const 2))")),
            |e| matches!(e, ShimError::Abi(m) if m.contains("vc_abi_version returned 2")),
        ),
        ("bad manifest", module("not json", "(i64.const 0)", ""), |e| matches!(e, ShimError::Manifest(_))),
        ("manifest with a bad id", module(r#"{"id":"a b","name":"x","kind":"filter"}"#, "(i64.const 0)", ""), |e| {
            matches!(e, ShimError::Manifest(_))
        }),
        ("manifest of the wrong kind", module(r#"{"id":"a","name":"x","kind":"panel"}"#, "(i64.const 0)", ""), |e| {
            matches!(e, ShimError::Manifest(_))
        }),
        (
            "start function loops",
            module(MANIFEST, "(i64.const 0)", "(func $spin (loop $l (br $l))) (start $spin)"),
            |e| matches!(e, ShimError::Abi(m) if m.contains("instantiation")),
        ),
        (
            "WASI import",
            module(MANIFEST, "(i64.const 0)", r#"(import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))"#),
            |e| matches!(e, ShimError::Abi(m) if m.contains("wasi_snapshot_preview1.fd_write")),
        ),
    ];
    // A small instruction budget keeps debug builds fast. The classes do not depend on its size.
    let limits = Limits { fuel_base: 5_000_000, ..Limits::default() };
    for (name, bytes, check) in cases {
        let (e, t) = run_err(&bytes, limits.clone());
        assert!(t < quick, "{name} took {t:?}");
        assert!(check(&e), "{name}: wrong error class: {e}");
    }
}

#[test]
fn malformed_and_oversized_modules_are_refused() {
    let half = &DESATURATE[..DESATURATE.len() / 2];
    for (name, bytes) in [("empty", &b""[..]), ("garbage", &b"\0asm\x01\0\0\0\xff\xff\xff"[..]), ("not wasm", &b"MZ\x90\0"[..]), ("truncated", half)]
    {
        assert!(matches!(load(bytes, Limits::default()), Err(ShimError::Module(_))), "{name}");
    }
    let tiny = Limits { max_module_bytes: 100, ..Limits::default() };
    assert!(matches!(load(&DESATURATE, tiny), Err(ShimError::Module(_))));
    assert!(load(&build(r#"(module (memory (export "memory") 60000))"#), Limits::default()).is_err());
    let no_run = build(
        r#"(module (memory (export "memory") 1) (func (export "vc_abi_version") (result i32) (i32.const 1))
           (func (export "vc_manifest") (result i64) (i64.const 0)))"#,
    );
    assert!(load(&no_run, Limits::default()).is_err());
    let src = r#"(module (memory (export "memory") 1) (func (export "vc_abi_version") (result i32) (i32.const 1))
           (func (export "vc_manifest") (result i64) (i64.const 0x00000064ffffff00))
           (func (export "vc_alloc") (param i32) (result i32) (i32.const 16))
           (func (export "vc_run") (param i32 i32 i32 i32) (result i64) (i64.const 0)))"#;
    assert!(matches!(load(&build(src), Limits::default()), Err(ShimError::Abi(_))));
    let bogus_alloc = build(&wat_module(MANIFEST, "(i64.const 0)", "", 1).replace(
        "(func (export \"vc_alloc\") (param $n i32) (result i32) (local $old i32)",
        "(func (export \"vc_alloc\") (param $n i32) (result i32) (local $old i32) (return (i32.const -16))",
    ));
    let (e, _) = run_err(&bogus_alloc, Limits::default());
    assert!(matches!(e, ShimError::Abi(_)), "{e}");
}

#[test]
fn the_instruction_budget_and_the_wall_clock_stop_runaway_plugins() {
    let spin = ok_module("(loop $l (br $l)) (i64.const 0)");
    let (e, _) = run_err(&spin, Limits { fuel_base: 1_000_000, fuel_per_byte: 10, ..Limits::default() });
    assert_eq!(e, ShimError::Limit("instruction budget".into()));
    let (e, t) = run_err(&spin, Limits { fuel_base: u64::MAX / 4, wall_time_ms: 300, ..Limits::default() });
    assert_eq!(e, ShimError::Limit("time budget".into()));
    assert!(t < Duration::from_secs(3), "{t:?}");
}

/// A clock that stands still until armed, then jumps 10 s on every read.
struct JumpClock {
    armed: std::sync::atomic::AtomicBool,
    now_ms: std::sync::atomic::AtomicU64,
}

impl craft_plugin_host::engine::Clock for JumpClock {
    fn now(&self) -> Duration {
        let step = if self.armed.load(std::sync::atomic::Ordering::SeqCst) { 10_000 } else { 0 };
        Duration::from_millis(self.now_ms.fetch_add(step, std::sync::atomic::Ordering::SeqCst))
    }
}

#[test]
fn the_wall_clock_is_the_injected_clock() {
    // The module spins, with an instruction budget it could never use up. Real time barely moves.
    // The run ends with a time limit only because the injected clock says the wall budget is gone.
    let jump = Arc::new(JumpClock { armed: false.into(), now_ms: 0.into() });
    let spin = ok_module("(loop $l (br $l)) (i64.const 0)");
    let p = VectorcraftPlugin::load(&spin, Limits { fuel_base: u64::MAX / 4, wall_time_ms: 5_000, ..Limits::default() }, jump.clone()).unwrap();
    let t = Instant::now();
    jump.armed.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(p.run(INPUT.as_bytes(), b"{}"), Err(ShimError::Limit("time budget".into())));
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
}

#[test]
fn a_live_effect_gets_the_shorter_wall_time() {
    let p = load(&ok_module("(i64.const 0)"), Limits::default()).unwrap();
    assert_eq!(p.limits().wall_time_ms, 60_000);
    assert_eq!(p.live_effect_limits().wall_time_ms, 5_000);
    let spin = ok_module("(loop $l (br $l)) (i64.const 0)");
    let p = load(&spin, Limits { fuel_base: u64::MAX / 4, ..Limits::default() }).unwrap();
    let live = Limits { wall_time_ms: 200, ..p.live_effect_limits() };
    let t = Instant::now();
    assert_eq!(p.run_limited(INPUT.as_bytes(), b"{}", &live), Err(ShimError::Limit("time budget".into())));
    assert!(t.elapsed() < Duration::from_secs(3));
}

#[test]
fn the_memory_cap_limits_input_and_growth() {
    let echo = module(MANIFEST, ECHO, "");
    let p = load(&echo, Limits { max_memory_bytes: 1 << 20, ..Limits::default() }).unwrap();
    assert!(p.run(&vec![b' '; 300 << 10], b"{}").is_ok(), "fits in half the memory");
    assert!(matches!(p.run(&vec![b' '; 600 << 10], b"{}"), Err(ShimError::Limit(_))), "does not");
    let p = load(&echo, Limits { max_memory_bytes: 64 << 10, ..Limits::default() }).unwrap();
    assert!(matches!(p.run(b"{}", b"{}"), Err(ShimError::Failed(m)) if m.contains("vc_alloc")));
}

#[test]
fn params_resolve_like_vectorcraft_resolves_them() {
    let m = r#"{"id":"t.p","name":"P","kind":"filter","params":{
        "amount":{"type":"number","min":0,"max":10,"default":5},
        "steps":{"type":"int","min":-5,"max":5,"default":2},
        "mono":{"type":"bool","default":true},
        "mode":{"type":"choice","options":["soft","hard"],"default":"hard"}}}"#
        .replace('\n', " ");
    let p = load(&module(&m, "(i64.const 0)", ""), Limits::default()).unwrap();
    let man = p.manifest();
    assert_eq!(man.params.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["amount", "steps", "mono", "mode"], "dialog order is kept");
    assert_eq!(man.params_doc(), r#"{amount: 0..10 (5), steps: int -5..5 (2), mono: bool (true), mode: "soft"|"hard" (hard)}"#);
    assert_eq!(Value::Object(man.resolve_params(&json!({})).unwrap()), json!({"amount": 5.0, "steps": 2, "mono": true, "mode": "hard"}));
    let r = man.resolve_params(&json!({"amount": 99, "steps": 3.6, "mono": false, "mode": "soft", "extra": 1})).unwrap();
    assert_eq!(Value::Object(r), json!({"amount": 10.0, "steps": 4, "mono": false, "mode": "soft"}), "clamped, rounded, unknown dropped");
    // Whole numbers keep full i64 precision.
    let r = man.resolve_params(&json!({"steps": i64::MAX})).unwrap();
    assert_eq!(r["steps"], json!(5));
    let r = man.resolve_params(&json!({"steps": u64::MAX})).unwrap();
    assert_eq!(r["steps"], json!(5));
    for bad in [json!({"amount": "x"}), json!({"mono": 1}), json!({"mode": "medium"}), json!({"steps": "1"}), json!([1])] {
        assert!(matches!(man.resolve_params(&bad), Err(ShimError::Params(_))), "{bad}");
    }
}

#[test]
fn manifests_with_bad_parameters_are_refused() {
    let with = |params: &str| format!(r#"{{"id":"t.m","name":"M","kind":"filter","params":{params}}}"#);
    for (params, why) in [
        (r#"{"a":{"type":"number","min":5,"max":1,"default":2}}"#, "range"),
        (r#"{"id":{"type":"bool"}}"#, "reserved key"),
        (r#"{"ids":{"type":"bool"}}"#, "reserved key"),
        (r#"{"params":{"type":"bool"}}"#, "reserved key"),
        (r#"{"preview":{"type":"bool"}}"#, "reserved key"),
        (r#"{"_x":{"type":"bool"}}"#, "underscore"),
        (r#"{"a b":{"type":"bool"}}"#, "space"),
        (r#"{"a":{"type":"bool"},"a":{"type":"bool"}}"#, "duplicate"),
        (r#"{"c":{"type":"choice","options":[],"default":null}}"#, "empty options"),
        (r#"{"c":{"type":"choice","options":["a"],"default":"b"}}"#, "default not in options"),
        (r#"{"c":{"type":"wobble"}}"#, "type"),
    ] {
        let r = load(&module(&with(params), "(i64.const 0)", ""), Limits::default());
        assert!(matches!(r, Err(ShimError::Manifest(_))), "{why}: {:?}", r.err());
    }
    let many: Vec<String> = (0..65).map(|i| format!(r#""p{i}":{{"type":"bool"}}"#)).collect();
    let r = load(&module(&with(&format!("{{{}}}", many.join(","))), "(i64.const 0)", ""), Limits::default());
    assert!(matches!(r, Err(ShimError::Manifest(_))), "65 parameters");
}

#[test]
fn effect_ids_follow_the_plugin_prefix() {
    assert_eq!(vectorcraft::effect_id("org.x.blur"), "plugin.org.x.blur");
    assert_eq!(vectorcraft::plugin_id("plugin.org.x.blur"), Some("org.x.blur"));
    assert_eq!(vectorcraft::plugin_id("blur"), None);
}

#[test]
fn a_filter_and_an_effect_become_craft_abi_1_contributions() {
    let p = load(&DESATURATE, Limits::default()).unwrap();
    let m = p.craft_manifest().unwrap();
    assert_eq!((m.id.as_str(), m.legacy), ("test.vectorcraft.desaturate", Some(LegacyTag::VectorcraftV1)));
    assert!(m.permissions.is_none() && m.requires.is_none());
    let c = &m.contributions[0];
    assert_eq!((&c.kind, c.entry, c.instance()), (&CraftKind::ObjectFilter, 0, Instance::Fresh));
    let amount = &c.params.as_ref().unwrap()["amount"];
    assert_eq!((amount.ty, amount.min, amount.max, amount.default.clone()), (ParamType::Number, Some(0.0), Some(100.0), Some(json!(100.0))));
    let effect = r#"{"id":"t.fx","name":"Fx","kind":"effect","params":{"r":{"type":"number","min":0,"max":1,"default":0.5}}}"#;
    let m = load(&module(effect, "(i64.const 0)", ""), Limits::default()).unwrap().craft_manifest().unwrap();
    assert_eq!(m.contributions[0].kind, CraftKind::LiveEffect);
}

#[test]
fn the_gate_names_the_abi_and_load_legacy_picks_the_adapter() {
    assert_eq!(detect(&DESATURATE), Some(LegacyAbi::Vectorcraft));
    let plugin = load_legacy(&DESATURATE, clock()).unwrap();
    assert_eq!(plugin.abi(), LegacyAbi::Vectorcraft);
    assert!(matches!(&plugin, LegacyPlugin::Vectorcraft(p) if p.manifest().name == "Desaturate"));
    assert_eq!(plugin.craft_manifest().unwrap().legacy, Some(LegacyTag::VectorcraftV1));
}
