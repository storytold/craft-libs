//! The EffectCraft v1 shim against the test matrix of EffectCraft's own plugin crate
//! (`crates/plugin/src/tests.rs`): register and render, sandbox, ABI violations, manifest schema,
//! render failure recovery, float determinism. The modules are that crate's WebAssembly text, built
//! with the `wat` crate. EffectCraft is `MIT OR Apache-2.0`, the license of this repository.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;

use common::gain::{gain_manifest, gain_module, module_with};
use craft_plugin_abi::manifest::{Instance, Kind, ParamType};
use craft_plugin_abi::{LegacyAbi, Permission};
use craft_plugin_host::engine::SystemClock;
use craft_plugin_host::shim::effectcraft::{EffectcraftPlugin, Limits};
use craft_plugin_host::shim::{LegacyPlugin, ShimError, detect, load_legacy};
use serde_json::json;

/// The v1 limits with a small fuel base, so a runaway loop ends in milliseconds under a debug build.
fn quick() -> Limits {
    Limits { fuel_base: 2_000_000, fuel_per_pixel: 20_000, ..Limits::default() }
}

fn load(wat: &str) -> Result<EffectcraftPlugin, ShimError> {
    let bytes = wat::parse_str(wat).unwrap_or_else(|e| panic!("bad WAT: {e}\n{wat}"));
    EffectcraftPlugin::load(&bytes, quick(), std::sync::Arc::new(SystemClock::new()))
}

fn load_err(wat: &str) -> String {
    match load(wat) {
        Err(e) => e.to_string(),
        Ok(p) => panic!("unexpectedly loaded {}", p.manifest().id),
    }
}

/// `base` with `from` replaced by `to`, which must occur.
fn edit(base: &str, from: &str, to: &str) -> String {
    assert!(base.contains(from), "{from}");
    base.replace(from, to)
}

const ORIG: [f32; 4] = [0.8, 0.6, 0.2, 1.0];

/// A 4 x 2 frame of `ORIG`, rendered with gain, the lift checkbox, and the tint [0,0,1,1].
fn render(p: &EffectcraftPlugin, gain: f64, on: bool, time: f64) -> (Result<(), ShimError>, Vec<[f32; 4]>) {
    let mut px = vec![ORIG; 8];
    let flat = [gain, if on { 1.0 } else { 0.0 }, 0.0, 0.0, 1.0, 1.0];
    let r = p.render(&mut px, 4, 2, &flat, time, 1.0);
    (r, px)
}

#[test]
fn the_v1_limits_are_the_numbers_effectcraft_ships() {
    let l = Limits::default();
    assert_eq!((l.fuel_base, l.fuel_per_pixel, l.max_memory_bytes), (50_000_000, 20_000, 1 << 30));
    // The shim adds a wall clock that v1 did not have.
    assert_eq!(l.wall_time_ms, 60_000);
}

/// A clock that stands still until `armed`, then jumps one minute per read, so the first wall check
/// after a fuel slice trips.
#[derive(Default)]
struct JumpClock {
    armed: std::sync::atomic::AtomicBool,
    reads: std::sync::atomic::AtomicU64,
}

impl craft_plugin_host::engine::Clock for JumpClock {
    fn now(&self) -> std::time::Duration {
        use std::sync::atomic::Ordering::Relaxed;
        if self.armed.load(Relaxed) { std::time::Duration::from_secs(60 * (1 + self.reads.fetch_add(1, Relaxed))) } else { std::time::Duration::ZERO }
    }
}

/// A render that has fuel left still stops on the wall clock.
#[test]
fn a_render_with_fuel_left_stops_on_the_wall_clock() {
    let bytes = wat::parse_str(gain_module("org.test.spin", "(loop $spin br $spin)")).unwrap();
    // Fuel for many 20M slices, so the instruction budget cannot be the cause.
    let limits = Limits { fuel_base: 100_000_000_000, ..Limits::default() };
    let clock = std::sync::Arc::new(JumpClock::default());
    let p = EffectcraftPlugin::load(&bytes, limits, clock.clone()).unwrap();
    clock.armed.store(true, std::sync::atomic::Ordering::Relaxed);
    let (r, px) = render(&p, 0.5, false, 0.0);
    assert_eq!(r, Err(ShimError::Limit("time budget".into())));
    assert_eq!(px, vec![ORIG; 8], "the frame comes back unchanged");
}

#[test]
fn a_plugin_loads_and_renders_like_effectcraft_renders_it() {
    let p = load(&gain_module("org.test.gain", "")).unwrap();
    let m = p.manifest();
    assert_eq!((m.id.as_str(), m.category.as_str(), m.params.len(), m.flat_len()), ("org.test.gain", "Test Plug-ins", 3, 6));
    let (r, px) = render(&p, 0.5, false, 0.0);
    r.unwrap();
    assert_eq!(px[3 + 4], [0.4, 0.3, 0.1, 1.0]);
    // The checkbox and color parameters arrive flattened: tint blue is 1, so lift adds 1 to blue.
    let (r, px) = render(&p, 1.0, true, 0.0);
    r.unwrap();
    assert_eq!(px[3], [0.8, 0.6, 1.2, 1.0]);
    // A wrong parameter count is refused before any guest code runs.
    let mut px = vec![ORIG; 8];
    assert!(matches!(p.render(&mut px, 4, 2, &[1.0], 0.0, 1.0), Err(ShimError::Params(_))));
    assert!(matches!(p.render(&mut px, 3, 2, &[0.5, 0.0, 0.0, 0.0, 1.0, 1.0], 0.0, 1.0), Err(ShimError::Params(_))));
    assert_eq!(px, vec![ORIG; 8]);
}

#[test]
fn renders_on_many_threads_share_the_pool_and_agree() {
    let p = load(&gain_module("org.test.gain", "")).unwrap();
    let first = render(&p, 0.25, false, 0.0).1;
    let runs: Vec<Vec<[f32; 4]>> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..4).map(|_| sc.spawn(|| render(&p, 0.25, false, 0.0).1)).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(runs.iter().all(|r| *r == first));
    assert!(p.pooled() >= 1 && p.pooled() <= 5, "instances return to the pool after a success: {}", p.pooled());
}

#[test]
fn a_runaway_render_runs_out_of_fuel_and_leaves_the_frame() {
    let p = load(&gain_module("org.test.spin", "(loop $spin br $spin)")).unwrap();
    for _ in 0..3 {
        let (r, px) = render(&p, 0.5, false, 0.0);
        assert_eq!(r, Err(ShimError::Limit("instruction budget".into())));
        assert_eq!(px, vec![ORIG; 8], "the frame comes back unchanged");
    }
}

#[test]
fn imports_garbage_wrong_versions_and_reserved_ids_are_refused() {
    let imports = r#"(module (import "env" "now" (func $now (result f64))) (memory (export "memory") 1))"#;
    assert!(load_err(imports).contains("import"));
    assert!(EffectcraftPlugin::load(b"\0asm nope", quick(), std::sync::Arc::new(SystemClock::new())).is_err());
    let v2 = edit(
        &gain_module("org.test.v2", ""),
        "(func (export \"ec_api_version\") (result i32) i32.const 1)",
        "(func (export \"ec_api_version\") (result i32) i32.const 2)",
    );
    assert!(load_err(&v2).contains("API 2"));
    assert!(load_err(&gain_module("ec.blur.mine", "")).contains("reserved"));
}

#[test]
fn abi_violations_fail_to_load_with_reasons() {
    let base = gain_module("org.test.abi", "");
    for export in ["ec_render", "ec_alloc", "ec_manifest_ptr", "ec_manifest_len", "ec_api_version"] {
        let e = load_err(&edit(&base, &format!("(export \"{export}\")"), ""));
        assert!(e.contains(export), "{export}: {e}");
    }
    assert!(load_err(&edit(&base, "(memory (export \"memory\") 2)", "(memory 2)")).contains("memory"));
    let m = edit(&base, "(func (export \"ec_api_version\") (result i32) i32.const 1)", "(func (export \"ec_api_version\") (result f64) f64.const 1)");
    assert!(load_err(&m).contains("ec_api_version"));
    assert!(load_err(&edit(&base, "(param $scale f64) (result i32)", "(param $scale f32) (result i32)")).contains("ec_render"));
    for v in ["0", "-1", "2147483647"] {
        let m = edit(
            &base,
            "(func (export \"ec_api_version\") (result i32) i32.const 1)",
            &format!("(func (export \"ec_api_version\") (result i32) i32.const {v})"),
        );
        assert!(load_err(&m).contains(&format!("API {v}")), "{v}");
    }
    // Manifest out of bounds, negative or absurd lengths, not JSON.
    let ptr = "(func (export \"ec_manifest_ptr\") (result i32) i32.const 16)";
    assert!(load_err(&edit(&base, ptr, "(func (export \"ec_manifest_ptr\") (result i32) i32.const 200000)")).contains("out of bounds"));
    let len = gain_manifest("org.test.abi").len();
    for bad in ["-5", "2000000000"] {
        let m = edit(&base, &format!("(result i32) i32.const {len})"), &format!("(result i32) i32.const {bad})"));
        assert!(load_err(&m).contains("manifest length"), "{bad}");
    }
    assert!(load_err(&edit(&base, ptr, "(func (export \"ec_manifest_ptr\") (result i32) i32.const 4000)")).contains("manifest JSON"));
    // Traps and runaway code while loading.
    let version = "(func (export \"ec_api_version\") (result i32) i32.const 1)";
    assert!(load(&edit(&base, version, "(func (export \"ec_api_version\") (result i32) unreachable)")).is_err());
    assert!(
        load(&edit(&base, version, "(func (export \"ec_api_version\") (result i32) (loop $l br $l) i32.const 1)")).is_err(),
        "fuel runs out in ec_api_version"
    );
    let m = edit(&base, "(memory (export \"memory\") 2)", "(memory (export \"memory\") 2) (func $s (loop $l br $l)) (start $s)");
    assert!(load_err(&m).contains("instantiation"), "fuel runs out in the start function");
    // Memory beyond the sandbox's limit.
    assert!(load_err(&edit(&base, "(memory (export \"memory\") 2)", "(memory (export \"memory\") 20000)")).contains("instantiation"));
    for junk in [&b""[..], b"\0asm", b"\0asm\x01\0\0\0\xff\xff", b"(module", b"(module (func (export \"x\") unknown_op))"] {
        assert!(EffectcraftPlugin::load(junk, quick(), std::sync::Arc::new(SystemClock::new())).is_err());
    }
}

#[test]
fn manifest_parameter_schemas_are_validated() {
    let p =
        |params: &str| format!(r#"{{"api":1,"id":"org.test.schema","name":"Schema","category":"Test Plug-ins","version":"1","params":[{params}]}}"#);
    let bad = [
        (r#"{"id":"a","name":"A","type":"checkbox"},{"id":"a","name":"B","type":"checkbox"}"#, "duplicate"),
        (r#"{"id":"a b","name":"A","type":"checkbox"}"#, "parameter id"),
        (r#"{"id":"","name":"A","type":"checkbox"}"#, "parameter id"),
        (r#"{"id":"p","name":"P","type":"popup","options":[],"default":0}"#, "popup"),
        (r#"{"id":"p","name":"P","type":"popup","options":["x"],"default":3}"#, "popup"),
        (r#"{"id":"s","name":"S","type":"slider","default":1,"min":5,"max":0}"#, "slider"),
        (r#"{"id":"s","name":"S","type":"slider","default":9,"min":0,"max":4}"#, "slider"),
        (r#"{"id":"s","name":"S","type":"slider","default":1,"min":0,"max":4,"sliderMin":3,"sliderMax":2}"#, "slider"),
        (r#"{"id":"s","name":"S","type":"wobble"}"#, "manifest JSON"),
        (r#"{"id":"s","name":"S","type":"slider"}"#, "manifest JSON"),
    ];
    for (params, want) in bad {
        let e = load_err(&module_with(&p(params), ""));
        assert!(e.contains(want), "{params}: {e}");
    }
    let many: Vec<String> = (0..300).map(|i| format!(r#"{{"id":"c{i}","name":"C","type":"checkbox"}}"#)).collect();
    assert!(load_err(&module_with(&p(&many.join(",")), "")).contains("at most"));
    let noname = r#"{"api":1,"id":"org.test.noname","name":" ","category":"X","version":"1","params":[]}"#;
    assert!(load_err(&module_with(noname, "")).contains("name"));
    let api3 = r#"{"api":3,"id":"org.test.api3","name":"N","category":"X","version":"1","params":[]}"#;
    assert!(load_err(&module_with(api3, "")).contains("API 3"));
    let ok = p(
        r#"{"id":"s","name":"S","type":"slider","default":2,"min":0,"max":4,"sliderMin":1,"sliderMax":3},{"id":"p","name":"P","type":"popup","options":["x","y"],"default":1}"#,
    );
    assert_eq!(load(&module_with(&ok, "")).unwrap().manifest().params.len(), 2);
}

#[test]
fn a_failed_render_leaves_the_frame_drops_the_instance_and_the_plugin_recovers() {
    // ec_render traps when time > 0.5, returns an error code, or ec_alloc hands back memory outside the module.
    let trap = load(&gain_module("org.test.trap", "local.get $t f64.const 0.5 f64.gt if unreachable end")).unwrap();
    assert_eq!(trap.pooled(), 1);
    let (r, px) = render(&trap, 0.5, false, 1.0);
    assert!(matches!(&r, Err(ShimError::Trap(m)) if m.contains("ec_render trapped")), "{r:?}");
    assert_eq!(px, vec![ORIG; 8], "trapped");
    assert_eq!(trap.pooled(), 0, "the instance that trapped is not returned to the pool");
    let (r, px) = render(&trap, 0.5, false, 0.0);
    r.unwrap();
    assert_eq!(px[0], [0.4, 0.3, 0.1, 1.0], "a fresh instance after the trap");
    assert_eq!(trap.pooled(), 1);
    let code = load(&gain_module("org.test.code", "i32.const 7 return")).unwrap();
    let (r, px) = render(&code, 0.5, false, 0.0);
    assert_eq!(r, Err(ShimError::Failed("ec_render returned error 7".into())));
    assert_eq!(px, vec![ORIG; 8]);
    let oob = load(&edit(&gain_module("org.test.oob", ""), "    i32.const 65536)", "    i32.const -16)")).unwrap();
    let (r, px) = render(&oob, 0.5, false, 0.0);
    assert!(matches!(r, Err(ShimError::Abi(m)) if m.contains("out of bounds")));
    assert_eq!(px, vec![ORIG; 8]);
}

#[test]
fn float_edge_cases_are_bit_identical_across_instances_threads_and_twins() {
    // sqrt of a negative gives NaN and a divide by zero gives infinity. With the deterministic
    // profile the NaN is the canonical quiet NaN on every machine.
    let body = "local.get $px local.get $px f32.load f32.const -1 f32.mul f32.sqrt f32.store \
                local.get $px local.get $px f32.load offset=4 f32.const 0 f32.div f32.store offset=4";
    let p = load(&gain_module("org.test.det", body)).unwrap();
    let bits = |px: &[[f32; 4]]| -> Vec<u32> { px.iter().flatten().map(|f| f.to_bits()).collect() };
    let first = bits(&render(&p, 0.37, true, 0.0).1);
    assert_eq!(first[0], 0x7fc0_0000, "NaN is canonical");
    assert_eq!(first[1], f32::INFINITY.to_bits());
    let runs: Vec<Vec<u32>> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..6).map(|_| sc.spawn(|| bits(&render(&p, 0.37, true, 0.0).1))).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(runs.iter().all(|r| *r == first));
    let twin = load(&gain_module("org.test.det2", body)).unwrap();
    assert_eq!(bits(&render(&twin, 0.37, true, 0.0).1), first);
}

#[test]
fn the_plugin_is_described_as_a_craft_abi_1_effect_with_no_permissions() {
    let manifest = r#"{"api":1,"id":"org.test.all","name":"All Kinds","category":"Test Plug-ins","version":"2.1","author":"A. Person","params":[
        {"id":"amount","name":"Amount","type":"slider","default":2,"min":0,"max":4},
        {"id":"turn","name":"Turn","type":"angle","default":90},
        {"id":"on","name":"On","type":"checkbox","default":true},
        {"id":"mode","name":"Mode","type":"popup","options":["soft","hard"],"default":1},
        {"id":"at","name":"At","type":"point","default":[0.25,0.75]},
        {"id":"tint","name":"Tint","type":"color","default":[0,0,1,1]}]}"#;
    let p = load(&module_with(manifest, "")).unwrap();
    let m = p.craft_manifest().unwrap();
    assert_eq!((m.id.as_str(), m.name.as_str(), m.version.as_str(), m.author.as_deref()), ("org.test.all", "All Kinds", "2.1", Some("A. Person")));
    assert_eq!(m.legacy, Some(craft_plugin_abi::manifest::LegacyTag::EffectcraftV1));
    assert!(m.permissions.is_none() && m.requires.is_none(), "a v1 plugin asks for nothing and imports nothing");
    assert_eq!(m.parsed_permissions().unwrap(), (vec![], vec![]));
    let c = &m.contributions[0];
    assert_eq!((&c.kind, c.entry, c.instance(), c.category.as_deref()), (&Kind::Effect, 0, Instance::Session, Some("Test Plug-ins")));
    let params = c.params.as_ref().unwrap();
    assert_eq!(params.len(), 6);
    assert_eq!(
        (params["amount"].ty, params["amount"].min, params["amount"].max, params["amount"].default.clone()),
        (ParamType::Number, Some(0.0), Some(4.0), Some(json!(2.0)))
    );
    assert_eq!(params["turn"].ty, ParamType::Angle);
    assert_eq!((params["on"].ty, params["on"].default.clone()), (ParamType::Bool, Some(json!(true))));
    assert_eq!(
        (params["mode"].ty, params["mode"].options.clone(), params["mode"].default.clone()),
        (ParamType::Choice, Some(vec!["soft".into(), "hard".into()]), Some(json!("hard")))
    );
    assert_eq!((params["at"].ty, params["at"].default.clone()), (ParamType::Point2, Some(json!([0.25, 0.75]))));
    assert_eq!((params["tint"].ty, params["tint"].default.clone()), (ParamType::Color, Some(json!([0.0, 0.0, 1.0, 1.0]))));
    let process = c.process.as_ref().unwrap();
    assert_eq!(process.alpha, Some(craft_plugin_abi::manifest::AlphaMode::Premultiplied));
    assert_eq!(process.sample.as_deref(), Some(&[craft_plugin_abi::manifest::SampleType::F32][..]));
    assert_eq!(m.limits.as_ref().unwrap().memory_mb, Some(1024));
}

#[test]
fn the_gate_names_the_abi_and_load_legacy_picks_the_adapter() {
    let bytes = wat::parse_str(gain_module("org.test.gain", "")).unwrap();
    assert_eq!(detect(&bytes), Some(LegacyAbi::Effectcraft));
    let clock = Arc::new(SystemClock::new());
    let plugin = load_legacy(&bytes, clock.clone()).unwrap();
    assert_eq!(plugin.abi(), LegacyAbi::Effectcraft);
    assert!(matches!(&plugin, LegacyPlugin::Effectcraft(p) if p.manifest().id == "org.test.gain"));
    assert_eq!(plugin.craft_manifest().unwrap().legacy, Some(craft_plugin_abi::manifest::LegacyTag::EffectcraftV1));
    // A craft-abi 1 package is not a legacy module.
    let native = common::package("echo");
    assert_eq!(detect(&native), None);
    assert!(matches!(load_legacy(&native, clock), Err(ShimError::Abi(_))));
    let _ = Permission::Clock;
}
