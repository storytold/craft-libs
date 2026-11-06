//! The PhotoCraft v1 shim against a generated stand-in plugin (an invert filter written as WAT in
//! `common/v1.rs`) and a matrix of hostile modules. PhotoCraft's `Surface`, tiles and selection blend stay in the app, so
//! these tests run on plain interleaved `f32` buffers.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use common::v1::{build, invert_module, pc_module as module};
use craft_plugin_abi::LegacyAbi;
use craft_plugin_abi::manifest::{AlphaMode, Instance, Kind as CraftKind, LegacyTag, ParamType};
use craft_plugin_host::engine::{Clock, SystemClock};
use craft_plugin_host::shim::photocraft::{self, ImageInfo, Limits, PhotocraftPlugin, format_code, mode};
use craft_plugin_host::shim::{LegacyPlugin, ShimError, detect, load_legacy};
use serde_json::{Value, json};

static INVERT: LazyLock<Vec<u8>> = LazyLock::new(invert_module);
const MANIFEST: &str = r#"{"id":"test.plugin","name":"Test","version":"1","kind":"filter"}"#;

fn ok_module(filter: &str) -> Vec<u8> {
    module(MANIFEST, 1, filter, "")
}

fn clock() -> Arc<SystemClock> {
    Arc::new(SystemClock::new())
}

fn load(bytes: &[u8], limits: Limits) -> Result<PhotocraftPlugin, ShimError> {
    PhotocraftPlugin::load(bytes, limits, clock())
}

/// 8-bit RGBA, straight alpha, like a PhotoCraft RGBA8 layer.
fn rgba8(w: u32, h: u32) -> ImageInfo {
    ImageInfo { width: w, height: h, channels: 4, depth: 8, alpha: true, color_mode: mode::RGB }
}

fn image(w: u32, h: u32) -> Vec<f32> {
    let mut v = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let a = ((x * 7 + y * 13) % 31) as f32 / 30.0;
            v.extend([a, 1.0 - a, (x % 5) as f32 / 4.0, 0.25 + 0.75 * ((x % 4) as f32 / 3.0)]);
        }
    }
    v
}

fn run_err(bytes: &[u8], limits: Limits) -> (ShimError, Duration) {
    let t = Instant::now();
    let r = load(bytes, limits).and_then(|p| p.apply(&mut image(32, 24), &rgba8(32, 24), &json!({})));
    (r.expect_err("hostile module must fail"), t.elapsed())
}

#[test]
fn the_v1_limits_are_the_numbers_photocraft_ships() {
    let l = Limits::default();
    assert_eq!((l.max_module_bytes, l.max_memory_bytes, l.band_bytes), (32 << 20, 512 << 20, 4 << 20));
    assert_eq!((l.fuel_base, l.fuel_per_sample, l.max_recursion_depth, l.wall_time_ms), (50_000_000, 4_000, 1024, 60_000));
    assert_eq!((photocraft::MAX_MANIFEST_BYTES, photocraft::MAX_PARAMS, photocraft::MAX_OVERLAP), (65_536, 64, 256));
}

#[test]
fn the_invert_plugin_loads_and_reports_its_manifest() {
    let p = load(&INVERT, Limits::default()).unwrap();
    let m = p.manifest();
    assert_eq!((m.id.as_str(), m.name.as_str(), m.overlap), ("test.photocraft.invert", "Invert", 0));
    assert!(m.params.is_empty());
    assert_eq!(p.size(), INVERT.len());
}

#[test]
fn the_invert_plugin_inverts_colour_and_keeps_alpha_at_every_depth() {
    let p = load(&INVERT, Limits::default()).unwrap();
    for depth in [8, 16, 32] {
        let info = ImageInfo { depth, ..rgba8(6, 4) };
        let src = image(6, 4);
        let mut data = src.clone();
        p.apply(&mut data, &info, &json!({})).unwrap();
        for (o, n) in src.chunks(4).zip(data.chunks(4)) {
            for ch in 0..3 {
                assert!((n[ch] - (1.0 - o[ch])).abs() < 1e-6, "depth {depth}: {o:?} -> {n:?}");
            }
            assert_eq!(n[3], o[3], "depth {depth}: alpha is not inverted");
        }
    }
    // Opaque RGB without alpha: all three channels invert.
    let info = ImageInfo { channels: 3, alpha: false, ..rgba8(2, 1) };
    let mut data = vec![0.1, 0.5, 0.9, 0.0, 1.0, 0.25];
    p.apply(&mut data, &info, &json!({})).unwrap();
    for (n, want) in data.iter().zip([0.9, 0.5, 0.1, 1.0, 0.0, 0.75]) {
        assert!((n - want).abs() < 1e-6, "{data:?}");
    }
}

/// Reads the sample above: `out[y] = in[max(y - 1, 0)]`, bottom-up, so a band only sees the right
/// rows if the host gave it one row of context above.
const SHIFT_DOWN: &str = r#"(local $y i32) (local $rb i32)
  (local.set $rb (i32.mul (i32.mul (local.get $w) (local.get $ch)) (i32.const 4)))
  (local.set $y (i32.sub (local.get $h) (i32.const 1)))
  (block $done (loop $l
    (br_if $done (i32.le_s (local.get $y) (i32.const 0)))
    (memory.copy
      (i32.add (local.get $buf) (i32.mul (local.get $y) (local.get $rb)))
      (i32.add (local.get $buf) (i32.mul (i32.sub (local.get $y) (i32.const 1)) (local.get $rb)))
      (local.get $rb))
    (local.set $y (i32.sub (local.get $y) (i32.const 1)))
    (br $l)))
  (i32.const 0)"#;

fn shifted(src: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = Vec::new();
    for y in 0..h {
        let sy = y.saturating_sub(1);
        out.extend_from_slice(&src[sy * w * 4..(sy + 1) * w * 4]);
    }
    out
}

#[test]
fn bands_are_seamless_when_the_plugin_asks_for_overlap_and_not_otherwise() {
    let src = image(5, 9);
    let want = shifted(&src, 5, 9);
    let row = 5 * 4 * 4;
    let tiny_bands = |overlap: u32| {
        let man = format!(r#"{{"id":"t.shift","name":"Shift","kind":"filter","overlap":{overlap}}}"#);
        load(&module(&man, 1, SHIFT_DOWN, ""), Limits { band_bytes: row, ..Limits::default() }).unwrap()
    };
    // One row per band, one row of context: identical to a single pass.
    let mut a = src.clone();
    tiny_bands(1).apply(&mut a, &rgba8(5, 9), &json!({})).unwrap();
    assert_eq!(a, want);
    // The same bands with no declared overlap show the seams: the plug-in has nothing above row y.
    let mut b = src.clone();
    tiny_bands(0).apply(&mut b, &rgba8(5, 9), &json!({})).unwrap();
    assert_eq!(b, src, "each band is one row, so nothing moves");
    // A single band is the same as one pass, with or without overlap.
    let one = load(&module(r#"{"id":"t.shift","name":"Shift","kind":"filter","overlap":1}"#, 1, SHIFT_DOWN, ""), Limits::default()).unwrap();
    let mut c = src.clone();
    one.apply(&mut c, &rgba8(5, 9), &json!({})).unwrap();
    assert_eq!(c, want);
}

/// Copies the params JSON over the start of the band, so the test can read what the plug-in received.
const ECHO_PARAMS: &str = "(memory.copy (local.get $buf) (local.get $pp) (local.get $pl)) (i32.const 0)";

/// The same, written at the band's inner origin for `overlap` 2: `(2 * w + 2)` pixels in.
const ECHO_PARAMS_INNER: &str = "(memory.copy (i32.add (local.get $buf) (i32.mul (i32.add (i32.mul (local.get $w) (i32.const 2)) (i32.const 2)) (i32.mul (local.get $ch) (i32.const 4)))) (local.get $pp) (local.get $pl)) (i32.const 0)";

fn received(p: &PhotocraftPlugin, info: &ImageInfo, user: &Value) -> Value {
    let mut data = vec![0.0f32; (info.width * info.height * info.channels) as usize];
    p.apply(&mut data, info, user).unwrap();
    let bytes: Vec<u8> = data.iter().flat_map(|f| f.to_le_bytes()).collect();
    serde_json::Deserializer::from_slice(&bytes).into_iter::<Value>().next().unwrap().unwrap()
}

#[test]
fn params_and_image_context_reach_the_plugin_as_json() {
    let man = r#"{"id":"t.params","name":"P","kind":"filter","overlap":2,"params":{"amount":{"type":"number","min":0,"max":10,"default":5},"mode":{"type":"choice","options":["a","b"]}}}"#;
    let p = load(&module(man, 1, ECHO_PARAMS_INNER, ""), Limits::default()).unwrap();
    assert_eq!(p.manifest().params_notation(), r#"{"amount":0..10=5,"mode":"a|b"}"#);
    let info = ImageInfo { depth: 32, alpha: false, channels: 4, ..rgba8(64, 4) };
    let got = received(&p, &info, &json!({"amount": 99, "mode": "b", "nope": 1}));
    assert_eq!((&got["amount"], &got["mode"]), (&json!(10.0), &json!("b")), "clamped, defaults and unknown keys handled: {got}");
    assert!(got.get("nope").is_none());
    // One band covers the whole image; overlap 2 adds 2 samples around it.
    assert_eq!(
        got["_image"],
        json!({"x": -2, "y": -2, "width": 68, "height": 8, "overlap": 2, "canvasWidth": 64, "canvasHeight": 4, "mode": "rgb", "depth": 32, "alpha": false, "channels": 4})
    );
    // Defaults fill the gaps.
    let got = received(&p, &info, &Value::Null);
    assert_eq!((&got["amount"], &got["mode"]), (&json!(5.0), &json!("a")));
    // Wrong-typed parameters are refused before the plug-in runs.
    let mut data = vec![0.0; 64 * 4 * 4];
    assert!(matches!(p.apply(&mut data, &info, &json!({"amount": "x"})), Err(ShimError::Params(_))));
    assert!(matches!(p.apply(&mut data, &info, &json!({"mode": "c"})), Err(ShimError::Params(_))));
    assert!(matches!(p.apply(&mut data, &info, &json!([1])), Err(ShimError::Params(_))));
    assert!(matches!(p.apply(&mut data[..10], &info, &json!({})), Err(ShimError::Params(_))));
}

#[test]
fn a_banded_run_tells_each_band_where_it_sits() {
    // No overlap, one row per band: band y writes its own params over the start of its row.
    let man = r#"{"id":"t.band","name":"B","kind":"filter"}"#;
    let row = 64 * 4 * 4;
    let p = load(&module(man, 1, ECHO_PARAMS, ""), Limits { band_bytes: row, ..Limits::default() }).unwrap();
    let info = ImageInfo { depth: 32, alpha: false, ..rgba8(64, 3) };
    let mut data = vec![0.0f32; 64 * 3 * 4];
    p.apply(&mut data, &info, &json!({})).unwrap();
    for y in 0..3 {
        let bytes: Vec<u8> = data[y * 64 * 4..(y + 1) * 64 * 4].iter().flat_map(|f| f.to_le_bytes()).collect();
        let got = serde_json::Deserializer::from_slice(&bytes).into_iter::<Value>().next().unwrap().unwrap();
        assert_eq!(
            (&got["_image"]["y"], &got["_image"]["height"], &got["_image"]["canvasHeight"]),
            (&json!(y), &json!(1), &json!(3)),
            "band {y}: {got}"
        );
    }
}

#[test]
fn nan_infinity_and_out_of_range_output_is_sanitised() {
    let fill = |value: &str| {
        format!(
            r#"(local $i i32)
      (block $done (loop $l
        (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
        (f32.store (i32.add (local.get $buf) (local.get $i)) (f32.const {value}))
        (local.set $i (i32.add (local.get $i) (i32.const 4)))
        (br $l)))
      (i32.const 0)"#
        )
    };
    let info = ImageInfo { depth: 32, ..rgba8(4, 4) };
    for bad in ["nan", "inf", "-inf"] {
        let p = load(&ok_module(&fill(bad)), Limits::default()).unwrap();
        let src = image(4, 4);
        let mut data = src.clone();
        p.apply(&mut data, &info, &json!({})).unwrap();
        assert_eq!(data, src, "{bad}: every sample keeps its value");
    }
    let p = load(&ok_module(&fill("5")), Limits::default()).unwrap();
    let mut data = image(4, 4);
    p.apply(&mut data, &info, &json!({})).unwrap();
    assert!(data.chunks(4).all(|px| px[..3] == [5.0; 3] && px[3] == 1.0), "float depth keeps colour 5.0 and clamps alpha");
    let mut data = image(4, 4);
    p.apply(&mut data, &ImageInfo { depth: 8, ..info }, &json!({})).unwrap();
    assert!(data.iter().all(|v| *v == 1.0), "integer depth clamps everything to 1");
    let mut data = image(4, 4);
    let neg = load(&ok_module(&fill("-3")), Limits::default()).unwrap();
    neg.apply(&mut data, &ImageInfo { depth: 16, ..info }, &json!({})).unwrap();
    assert!(data.iter().all(|v| *v == 0.0));
}

#[test]
fn the_format_word_and_the_color_modes_follow_the_abi() {
    assert_eq!(format_code(8, true, mode::RGB), 8 | 1 << 8 | 3 << 16);
    assert_eq!(format_code(32, false, mode::CMYK), 32 | 4 << 16);
    assert_eq!(format_code(16, true, mode::LAB), 16 | 1 << 8 | 9 << 16);
    assert_eq!((mode::name(mode::GRAYSCALE), mode::name(mode::MULTICHANNEL), mode::name(99)), ("grayscale", "multichannel", "unknown"));
    // The plug-in sees the word: write it into the first sample, read it back from the output.
    let body = "(i32.store (local.get $buf) (local.get $fmt)) (i32.store offset=4 (local.get $buf) (local.get $ch)) (i32.const 0)";
    let p = load(&ok_module(body), Limits::default()).unwrap();
    let info = ImageInfo { width: 1, height: 1, channels: 5, depth: 8, alpha: true, color_mode: mode::CMYK };
    let mut data = vec![0.0f32; 5];
    // Integer depth clamps to 0..1 and these bit patterns are tiny or huge floats, so use depth 32.
    p.apply(&mut data, &ImageInfo { depth: 32, ..info }, &json!({})).unwrap();
    assert_eq!(data[0].to_bits(), format_code(32, true, mode::CMYK));
    assert_eq!(data[1].to_bits(), 5);
}

#[test]
fn hostile_modules_fail_quickly_with_the_right_class_of_error() {
    let quick = Duration::from_secs(5);
    type Check = fn(&ShimError) -> bool;
    let cases: Vec<(&str, Vec<u8>, Check)> = vec![
        ("infinite loop", ok_module("(loop $l (br $l)) (i32.const 0)"), |e| matches!(e, ShimError::Limit(m) if m == "instruction budget")),
        ("memory.grow bomb, then trap", ok_module("(loop $l (br_if $l (i32.ne (memory.grow (i32.const 512)) (i32.const -1)))) unreachable"), |e| {
            matches!(e, ShimError::Trap(_) | ShimError::Limit(_))
        }),
        (
            "memory.grow forever",
            ok_module("(loop $l (drop (memory.grow (i32.const 1))) (br $l)) (i32.const 0)"),
            |e| matches!(e, ShimError::Limit(m) if m == "instruction budget"),
        ),
        ("out-of-bounds write", ok_module("(i32.store (i32.const -4) (i32.const 0)) (i32.const 0)"), |e| matches!(e, ShimError::Trap(_))),
        ("trap", ok_module("unreachable"), |e| matches!(e, ShimError::Trap(_))),
        ("division by zero", ok_module("(i32.div_u (i32.const 1) (local.get $ch))  (drop) (i32.div_u (i32.const 1) (i32.const 0))"), |e| {
            matches!(e, ShimError::Trap(_))
        }),
        ("deep recursion", module(MANIFEST, 1, "(call $r) (i32.const 0)", "(func $r (call $r))"), |e| matches!(e, ShimError::Trap(_))),
        ("error code", ok_module("(i32.const 7)"), |e| matches!(e, ShimError::Failed(m) if m.contains("error code 7"))),
        (
            "wrong ABI version",
            module(MANIFEST, 2, "(i32.const 0)", ""),
            |e| matches!(e, ShimError::Abi(m) if m.contains("pc_abi_version returned 2")),
        ),
        ("bad manifest", module("not json", 1, "(i32.const 0)", ""), |e| matches!(e, ShimError::Manifest(_))),
        ("manifest with a bad id", module(r#"{"id":"a b","name":"x","kind":"filter"}"#, 1, "(i32.const 0)", ""), |e| {
            matches!(e, ShimError::Manifest(_))
        }),
        ("manifest of the wrong kind", module(r#"{"id":"a","name":"x","kind":"panel"}"#, 1, "(i32.const 0)", ""), |e| {
            matches!(e, ShimError::Manifest(_))
        }),
        (
            "start function loops",
            module(MANIFEST, 1, "(i32.const 0)", "(func $spin (loop $l (br $l))) (start $spin)"),
            |e| matches!(e, ShimError::Abi(m) if m.contains("instantiation")),
        ),
        (
            "WASI import",
            module(MANIFEST, 1, "(i32.const 0)", r#"(import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))"#),
            |e| matches!(e, ShimError::Abi(m) if m.contains("wasi_snapshot_preview1.fd_write")),
        ),
    ];
    let limits = Limits { fuel_base: 5_000_000, ..Limits::default() };
    for (name, bytes, check) in cases {
        let (e, t) = run_err(&bytes, limits.clone());
        assert!(t < quick, "{name} took {t:?}");
        assert!(check(&e), "{name}: wrong error class: {e}");
    }
}

#[test]
fn malformed_and_oversized_modules_are_refused() {
    let half = &INVERT[..INVERT.len() / 2];
    for (name, bytes) in [("empty", &b""[..]), ("garbage", &b"\0asm\x01\0\0\0\xff\xff\xff"[..]), ("not wasm", &b"MZ\x90\0"[..]), ("truncated", half)]
    {
        assert!(matches!(load(bytes, Limits::default()), Err(ShimError::Module(_))), "{name}");
    }
    assert!(matches!(load(&INVERT, Limits { max_module_bytes: 100, ..Limits::default() }), Err(ShimError::Module(_))));
    assert!(load(&build(r#"(module (memory (export "memory") 60000))"#), Limits::default()).is_err());
    let no_filter = build(
        r#"(module (memory (export "memory") 1) (func (export "pc_abi_version") (result i32) (i32.const 1))
           (func (export "pc_manifest") (result i64) (i64.const 0)))"#,
    );
    assert!(load(&no_filter, Limits::default()).is_err());
    let outside = r#"(module (memory (export "memory") 1) (func (export "pc_abi_version") (result i32) (i32.const 1))
           (func (export "pc_manifest") (result i64) (i64.const 0x00000064ffffff00))
           (func (export "pc_alloc") (param i32) (result i32) (i32.const 16))
           (func (export "pc_filter") (param i32 i32 i32 i32 i32 i32 i32 i32) (result i32) (i32.const 0)))"#;
    assert!(matches!(load(&build(outside), Limits::default()), Err(ShimError::Abi(_))));
    let bogus_alloc = build(&format!(
        r#"(module (memory (export "memory") 1) (data (i32.const 16) "{}")
           (func (export "pc_abi_version") (result i32) (i32.const 1))
           (func (export "pc_manifest") (result i64) (i64.const {}))
           (func (export "pc_alloc") (param i32) (result i32) (i32.const -16))
           (func (export "pc_filter") (param i32 i32 i32 i32 i32 i32 i32 i32) (result i32) (i32.const 0)))"#,
        MANIFEST.replace('"', "\\\""),
        ((MANIFEST.len() as i64) << 32) | 16
    ));
    let (e, _) = run_err(&bogus_alloc, Limits::default());
    assert!(matches!(e, ShimError::Abi(_)), "{e}");
    let big = format!(r#"{{"id":"t.big","name":"B","kind":"filter","description":"{}"}}"#, "x".repeat(70_000));
    assert!(matches!(
        load(&module(&big, 1, "(i32.const 0)", ""), Limits::default()),
        Err(ShimError::Manifest(_) | ShimError::Abi(_) | ShimError::Module(_))
    ));
}

#[test]
fn the_instruction_budget_and_the_wall_clock_stop_runaway_plugins() {
    let spin = ok_module("(loop $l (br $l)) (i32.const 0)");
    let (e, _) = run_err(&spin, Limits { fuel_base: 1_000_000, fuel_per_sample: 10, ..Limits::default() });
    assert_eq!(e, ShimError::Limit("instruction budget".into()));
    let (e, t) = run_err(&spin, Limits { fuel_base: u64::MAX / 4, wall_time_ms: 300, ..Limits::default() });
    assert_eq!(e, ShimError::Limit("time budget".into()));
    assert!(t < Duration::from_secs(3), "{t:?}");
}

/// A clock that stands still until armed, then jumps 10 s on every read.
struct JumpClock {
    armed: AtomicBool,
    now_ms: AtomicU64,
}

impl Clock for JumpClock {
    fn now(&self) -> Duration {
        let step = if self.armed.load(Ordering::SeqCst) { 10_000 } else { 0 };
        Duration::from_millis(self.now_ms.fetch_add(step, Ordering::SeqCst))
    }
}

#[test]
fn the_wall_clock_is_the_injected_clock_and_the_first_failing_band_is_the_reported_cause() {
    let jump = Arc::new(JumpClock { armed: false.into(), now_ms: 0.into() });
    let spin = ok_module("(loop $l (br $l)) (i32.const 0)");
    let p = PhotocraftPlugin::load(&spin, Limits { fuel_base: u64::MAX / 4, wall_time_ms: 5_000, ..Limits::default() }, jump.clone()).unwrap();
    let t = Instant::now();
    jump.armed.store(true, Ordering::SeqCst);
    assert_eq!(p.apply(&mut image(8, 8), &rgba8(8, 8), &json!({})), Err(ShimError::Limit("time budget".into())));
    assert!(t.elapsed() < Duration::from_secs(5));
    // Band 1 passes (reads a red sample), band 2 traps. The error is the trap, not a sibling's abort.
    let body = "(if (result i32) (f32.gt (f32.load (local.get $buf)) (f32.const 0.5)) (then unreachable) (else (i32.const 0)))";
    let p = load(&ok_module(body), Limits { band_bytes: 1, ..Limits::default() }).unwrap();
    let mut data = vec![0.0f32; 8 * 4 * 4];
    for px in data.chunks_mut(4).skip(8 * 2) {
        px[0] = 1.0;
    }
    let before = data.clone();
    assert!(matches!(p.apply(&mut data, &rgba8(8, 4), &json!({})), Err(ShimError::Trap(_))));
    assert_eq!(data, before, "a failed run leaves the image as it was");
}

#[test]
fn the_memory_cap_limits_bands() {
    let info = rgba8(64, 8);
    let p = load(&INVERT, Limits { max_memory_bytes: 4 << 20, band_bytes: 64 << 20, ..Limits::default() }).unwrap();
    assert!(p.apply(&mut image(64, 8), &info, &json!({})).is_ok(), "small bands still fit");
    let p = load(&INVERT, Limits { max_memory_bytes: 1024, ..Limits::default() });
    assert!(p.is_err() || p.unwrap().apply(&mut image(64, 8), &info, &json!({})).is_err());
    // An overlap that cannot fit in half the memory is refused before any band runs.
    let man = r#"{"id":"t.ov","name":"O","kind":"filter","overlap":256}"#;
    let p = load(&module(man, 1, "(i32.const 0)", ""), Limits { max_memory_bytes: 1 << 20, ..Limits::default() }).unwrap();
    assert!(matches!(p.apply(&mut image(64, 8), &info, &json!({})), Err(ShimError::Limit(_))));
}

#[test]
fn manifests_are_validated_like_photocraft_validates_them() {
    let man = |extra: &str| format!(r#"{{"id":"t.m","name":"M","kind":"filter"{extra}}}"#);
    let bad = |m: String| matches!(load(&module(&m, 1, "(i32.const 0)", ""), Limits::default()), Err(ShimError::Manifest(_)));
    assert!(bad(man(r#","overlap":257"#)));
    assert!(bad(man(r#","params":{"_x":{"type":"bool"}}"#)));
    assert!(bad(man(r#","params":{"id":{"type":"bool"}}"#)));
    assert!(bad(man(r#","params":{"layer":{"type":"bool"}}"#)));
    assert!(bad(man(r#","params":{"a":{"type":"number","min":5,"max":1,"default":2}}"#)));
    assert!(bad(man(r#","params":{"c":{"type":"choice","options":["a|b"]}}"#)));
    assert!(bad(man(r#","params":{"c":{"type":"choice","options":["a"],"default":"z"}}"#)));
    // A name of only hidden characters, and one that reorders text, are refused. Joiners are fine.
    assert!(bad(r#"{"id":"t.m","name":"\u200b\u200b","kind":"filter"}"#.into()));
    assert!(bad(r#"{"id":"t.m","name":"abc\u202edef","kind":"filter"}"#.into()));
    assert!(load(&module(r#"{"id":"t.m","name":"\u0645\u200c\u0646","kind":"filter"}"#, 1, "(i32.const 0)", ""), Limits::default()).is_ok());
    // 64 parameters pass, 65 do not.
    let params = |n: usize| {
        let items: Vec<String> = (0..n).map(|i| format!(r#""p{i}":{{"type":"bool"}}"#)).collect();
        man(&format!(r#","params":{{{}}}"#, items.join(",")))
    };
    assert!(load(&module(&params(64), 1, "(i32.const 0)", ""), Limits::default()).is_ok());
    assert!(bad(params(65)));
    // PhotoCraft clamps a default that is outside its range (issue 1012), and keeps a duplicate key.
    let p = load(
        &module(&man(r#","params":{"a":{"type":"number","min":0,"max":10,"default":50},"a":{"type":"bool"}}"#), 1, "(i32.const 0)", ""),
        Limits::default(),
    )
    .unwrap();
    assert_eq!(p.manifest().params.len(), 2);
    assert_eq!(p.manifest().resolve_params(&json!({})).unwrap()["a"], json!(false), "the later duplicate (a bool, default false) wins");
    let q =
        load(&module(&man(r#","params":{"a":{"type":"number","min":0,"max":10,"default":50}}"#), 1, "(i32.const 0)", ""), Limits::default()).unwrap();
    assert_eq!(q.manifest().resolve_params(&json!({})).unwrap()["a"], json!(10.0));
    // Whole numbers are rounded through f64.
    let r = load(&module(&man(r#","params":{"n":{"type":"int","min":-5,"max":5,"default":0}}"#), 1, "(i32.const 0)", ""), Limits::default()).unwrap();
    assert_eq!(r.manifest().resolve_params(&json!({"n": 2.5})).unwrap()["n"], json!(3));
    assert_eq!(r.manifest().resolve_params(&json!({"n": 99})).unwrap()["n"], json!(5));
}

#[test]
fn the_plugin_becomes_a_craft_abi_1_filter() {
    let man = r#"{"id":"t.craft","name":"Craft","kind":"filter","overlap":3,"area":"canvas","params":{"amount":{"type":"number","min":0,"max":10,"default":5},"mode":{"type":"choice","options":["a","b"],"default":"b"}}}"#;
    let p = load(&module(man, 1, "(i32.const 0)", ""), Limits::default()).unwrap();
    let m = p.craft_manifest().unwrap();
    assert_eq!((m.id.as_str(), m.legacy), ("t.craft", Some(LegacyTag::PhotocraftV1)));
    assert!(m.permissions.is_none() && m.requires.is_none());
    let c = &m.contributions[0];
    assert_eq!((&c.kind, c.entry, c.instance()), (&CraftKind::Filter, 0, Instance::Fresh));
    let process = c.process.as_ref().unwrap();
    assert_eq!((process.overlap, process.alpha), (Some(3), Some(AlphaMode::Straight)));
    let params = c.params.as_ref().unwrap();
    assert_eq!((params["amount"].ty, params["amount"].max), (ParamType::Number, Some(10.0)));
    assert_eq!((params["mode"].ty, params["mode"].default.clone()), (ParamType::Choice, Some(json!("b"))));
}

#[test]
fn the_gate_names_the_abi_and_load_legacy_picks_the_adapter() {
    assert_eq!(detect(&INVERT), Some(LegacyAbi::Photocraft));
    let plugin = load_legacy(&INVERT, clock()).unwrap();
    assert_eq!(plugin.abi(), LegacyAbi::Photocraft);
    assert!(matches!(&plugin, LegacyPlugin::Photocraft(p) if p.manifest().id == "test.photocraft.invert"));
    assert_eq!(plugin.craft_manifest().unwrap().legacy, Some(LegacyTag::PhotocraftV1));
}
