//! The v1 shims behind `PluginManager`: a PhotoCraft (`pc_`), VectorCraft (`vc_`) or EffectCraft
//! (`ec_`) module installs through the same `install`, shows up as legacy with its limits, and its
//! contributions run through the data plane entry (`Generation::process_image`, `run_objects`). The
//! modules are generated: the stand-ins in `common/v1.rs` and the gain plugin in `common/gain.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use common::gain::{gain_manifest, gain_module};
use common::services::*;
use common::surfaces::RecordingSink;
use common::v1::{desaturate_module, invert_module};
use craft_plugin_abi::LegacyAbi;
use craft_plugin_abi::descriptor::{Alpha, Sample};
use craft_plugin_abi::manifest::Kind;
use craft_plugin_host::budget::BudgetConfig;
use craft_plugin_host::engine::wasmi::WasmiEngine;
use craft_plugin_host::invoke::CallError;
use craft_plugin_host::lifecycle::{InstallOutcome, LifecycleError, PluginState};
use craft_plugin_host::process::{ImageFormat, ImageJob, ProcessOpts};
use craft_plugin_host::services::CallOutcome;
use craft_plugin_host::store::{MemoryStore, PackageStore, Source};
use serde_json::{Value, json};

static INVERT: LazyLock<Vec<u8>> = LazyLock::new(invert_module);
static DESATURATE: LazyLock<Vec<u8>> = LazyLock::new(desaturate_module);
const GAIN_ID: &str = "org.test.gain";

fn setup() -> (Harness, Arc<RecordingSink>) {
    let h = Harness::new();
    let sink = Arc::new(RecordingSink::default());
    h.manager.attach_surfaces(sink.clone());
    (h, sink)
}

fn gain(id: &str, body: &str) -> Vec<u8> {
    wat::parse_str(gain_module(id, body)).unwrap()
}

fn rgba_f32(w: u32, h: u32, alpha: Alpha) -> ImageFormat {
    ImageFormat { width: w, height: h, channels: 4, sample: Sample::F32, alpha, color_mode: 3, depth: 32 }
}

fn bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn floats(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn process(sink: &RecordingSink, id: &str, format: ImageFormat, data: &mut [u8], params: &Value) -> Result<usize, CallError> {
    let mut job = ImageJob::new(format, data);
    sink.surface(id).process_image(&mut job, params, &ProcessOpts::default()).map(|r| r.bands)
}

fn near(got: &[f32], want: &[f32]) {
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() < 1e-6, "{got:?} vs {want:?}");
    }
}

// ---- PhotoCraft ----------------------------------------------------------------------------

#[test]
fn a_photocraft_module_installs_without_a_prompt_and_is_shown_as_legacy_with_its_limits() {
    let (h, sink) = setup();
    // No permissions to ask for: the plugin gets no host ops, so install loads it at once.
    assert_eq!(h.manager.install(&INVERT, Source::File).unwrap(), InstallOutcome::Loaded { generation: 1, replaced: None });
    let id = "test.photocraft.invert";
    let info = h.manager.info(id).unwrap();
    assert_eq!(info.state, PluginState::Active { generation: 1 });
    assert!(info.grants.is_empty() && info.commands.is_empty() && info.unsupported_kinds.is_empty());
    let legacy = info.legacy.expect("shown as legacy");
    assert_eq!(legacy.abi, LegacyAbi::Photocraft);
    let limit = |name: &str| {
        legacy.limits.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str()).unwrap_or_else(|| panic!("no limit {name}: {:?}", legacy.limits))
    };
    assert_eq!(limit("module size"), "32 MiB");
    assert_eq!(limit("memory"), "512 MiB");
    assert_eq!(limit("band size"), "4 MiB");
    assert_eq!(limit("instructions per call"), "50,000,000 plus 4,000 per sample");
    assert_eq!(limit("call depth"), "1,024");
    assert_eq!(limit("wall time per run"), "60 s");
    assert_eq!(info.surfaces.registered, vec!["filter".to_string()]);
    let surface = sink.surface("filter");
    assert!(surface.legacy);
    assert_eq!(surface.contribution.kind, Kind::Filter);
    // A craft-abi 1 plugin shows no legacy block.
    h.manager.install(&common::surfaces::pixels_package("org.example.pixels", "1.0.0"), Source::File).unwrap();
    assert!(h.manager.info("org.example.pixels").unwrap().legacy.is_none());
}

#[test]
fn photocraft_invert_runs_through_the_data_plane_entry() {
    let (h, sink) = setup();
    h.manager.install(&INVERT, Source::File).unwrap();
    let src = [0.1f32, 0.5, 0.9, 0.25, 0.0, 1.0, 0.2, 1.0];
    let mut data = bytes(&src);
    let bands = process(&sink, "filter", rgba_f32(2, 1, Alpha::Straight), &mut data, &Value::Null).unwrap();
    assert_eq!(bands, 1);
    near(&floats(&data), &[0.9, 0.5, 0.1, 0.25, 1.0, 0.0, 0.8, 1.0]);
    h.services.recorded(|rec| {
        assert_eq!((rec.begun.len(), rec.ended.len()), (1, 1));
        assert_eq!(rec.ended[0].1, CallOutcome::Completed);
    });
}

#[test]
fn a_job_the_photocraft_manifest_does_not_take_is_refused() {
    let (h, sink) = setup();
    h.manager.install(&INVERT, Source::File).unwrap();
    let mut data = vec![9u8; 8];
    let u8_format = ImageFormat { sample: Sample::U8, depth: 8, ..rgba_f32(2, 1, Alpha::Straight) };
    let run = |format: ImageFormat, bytes: &mut [u8], params: Value| process(&sink, "filter", format, bytes, &params);
    assert!(matches!(run(u8_format, &mut data, json!({})), Err(CallError::BadJob(m)) if m.contains("takes [F32]")));
    let mut f = vec![0u8; 32];
    assert!(matches!(run(rgba_f32(2, 1, Alpha::Premultiplied), &mut f, json!({})), Err(CallError::BadJob(m)) if m.contains("takes Straight alpha")));
    // v1 rules for parameters: this plugin declares none, and a name it did not declare is dropped.
    assert_eq!(f, vec![0u8; 32]);
    assert!(run(rgba_f32(2, 1, Alpha::Straight), &mut f, json!({ "amount": 1 })).is_ok());
    h.services.recorded(|rec| assert_eq!(rec.begun.len(), 1, "only the job that ran opened an undo scope"));
}

// ---- EffectCraft ---------------------------------------------------------------------------

const ORIG: [f32; 4] = [0.8, 0.6, 0.2, 1.0];

fn frame() -> Vec<u8> {
    bytes(&ORIG.repeat(8))
}

fn premul_4x2() -> ImageFormat {
    rgba_f32(4, 2, Alpha::Premultiplied)
}

#[test]
fn an_effectcraft_module_installs_as_legacy_and_renders_with_named_params() {
    let (h, sink) = setup();
    assert_eq!(h.manager.install(&gain(GAIN_ID, ""), Source::File).unwrap(), InstallOutcome::Loaded { generation: 1, replaced: None });
    let info = h.manager.info(GAIN_ID).unwrap();
    let legacy = info.legacy.expect("legacy");
    assert_eq!(legacy.abi, LegacyAbi::Effectcraft);
    assert!(legacy.limits.contains(&("memory".to_string(), "1024 MiB".to_string())), "{:?}", legacy.limits);
    assert!(legacy.limits.contains(&("instructions per call".to_string(), "50,000,000 plus 20,000 per pixel".to_string())));
    assert_eq!(info.surfaces.registered, vec!["effect".to_string()]);
    assert_eq!(sink.surface("effect").contribution.kind, Kind::Effect);

    // All defaults: gain 0.5, lift off.
    let mut data = frame();
    process(&sink, "effect", premul_4x2(), &mut data, &Value::Null).unwrap();
    near(&floats(&data)[28..32], &[0.4, 0.3, 0.1, 1.0]);
    // The checkbox and the color param arrive flattened in manifest order: tint blue 1 adds 1 to blue.
    let mut data = frame();
    process(&sink, "effect", premul_4x2(), &mut data, &json!({ "gain": 1.0, "on": true, "tint": [0, 0, 1, 1] })).unwrap();
    near(&floats(&data)[28..32], &[0.8, 0.6, 1.2, 1.0]);
}

#[test]
fn effectcraft_params_follow_the_v1_rules_and_are_settled_before_the_guest_runs() {
    let (h, sink) = setup();
    h.manager.install(&gain(GAIN_ID, ""), Source::File).unwrap();
    let blue = |params: Value| {
        let mut data = frame();
        process(&sink, "effect", premul_4x2(), &mut data, &params).unwrap();
        floats(&data)[28..32].to_vec()
    };
    // A slider value past its range is clamped, as EffectCraft's app did before it called the plug-in.
    near(&blue(json!({ "gain": 9 })), &[3.2, 2.4, 0.8, 1.0]);
    near(&blue(json!({ "gain": -3 })), &[0.0, 0.0, 0.0, 1.0]);
    // A name the plug-in did not declare is dropped.
    near(&blue(json!({ "volume": 11 })), &[0.4, 0.3, 0.1, 1.0]);
    // A value of the wrong type is an error, and no guest code runs for it.
    for (params, needle) in [
        (json!({ "gain": "loud" }), "finite number"),
        (json!({ "on": 1 }), "true or false"),
        (json!({ "tint": [0, 0, 1] }), "list of 4 numbers"),
        (json!(5), "must be an object"),
    ] {
        let mut data = frame();
        match process(&sink, "effect", premul_4x2(), &mut data, &params) {
            Err(CallError::BadJob(m)) => assert!(m.contains(needle), "{params}: {m}"),
            other => panic!("{params}: expected BadJob({needle}), got {other:?}"),
        }
        assert_eq!(data, frame());
    }
    let mut data = bytes(&ORIG.repeat(6));
    let rgb = ImageFormat { channels: 3, ..premul_4x2() };
    let rgb = ImageFormat { width: 4, height: 2, ..rgb };
    data.truncate(4 * 2 * 3 * 4);
    assert!(matches!(process(&sink, "effect", rgb, &mut data, &Value::Null), Err(CallError::BadJob(m)) if m.contains("RGBA")));
    h.services.recorded(|rec| assert_eq!(rec.begun.len(), 3, "only the three runs that reached the guest opened a scope"));
}

#[test]
fn a_v1_plugin_that_traps_leaves_the_frame_tells_the_app_to_undo_and_opens_the_circuit() {
    let (h, sink) = setup();
    // Traps when the layer time is past half a second.
    h.manager.install(&gain("org.test.trap", "local.get $t f64.const 0.5 f64.gt if unreachable end"), Source::File).unwrap();
    let run_at = |time: f64| {
        let mut data = frame();
        let mut job = ImageJob::new(premul_4x2(), &mut data);
        job.time = time;
        let r = sink.surface("effect").process_image(&mut job, &Value::Null, &ProcessOpts::default());
        (r, data)
    };
    // The layer time reaches the plugin: at 0.25 s it renders, at 1 s it traps.
    assert!(run_at(0.25).0.is_ok());
    for n in 1..=3 {
        let (r, data) = run_at(1.0);
        match r {
            Err(CallError::Legacy(m)) => assert!(m.contains("trapped"), "{m}"),
            other => panic!("fault {n}: {other:?}"),
        }
        assert_eq!(data, frame(), "the frame comes back unchanged");
    }
    h.services.recorded(|rec| {
        assert_eq!(rec.ended.iter().filter(|(_, o)| matches!(o, CallOutcome::Faulted(_))).count(), 3);
        assert_eq!(rec.notices.len(), 3);
        assert!(rec.notices[2].message.contains("stays off for this session after 3 faults"));
    });
    assert!(matches!(run_at(0.25).0, Err(CallError::CircuitOpen)), "even a good frame is refused while the circuit is open");
}

#[test]
fn an_effectcraft_reload_replaces_the_plugin_in_place_and_the_app_handle_follows() {
    let (h, sink) = setup();
    h.manager.install(&gain(GAIN_ID, ""), Source::File).unwrap();
    let handle = sink.surface("effect");
    // Version 2: the gain slider's default is 2.
    let v2 = common::swap(&gain_manifest(GAIN_ID), "\"default\":0.5", "\"default\":2");
    let v2 = common::swap(&v2, "\"version\":\"1.0\"", "\"version\":\"2.0\"");
    let v2 = wat::parse_str(common::gain::module_with(&v2, "")).unwrap();
    assert_eq!(h.manager.install(&v2, Source::File).unwrap(), InstallOutcome::Loaded { generation: 2, replaced: Some(1) });
    assert_eq!(h.manager.info(GAIN_ID).unwrap().version, "2.0");
    assert_eq!(sink.log().iter().filter(|l| l.starts_with("remove")).count(), 1);
    let mut data = frame();
    let mut job = ImageJob::new(premul_4x2(), &mut data);
    handle.process_image(&mut job, &Value::Null, &ProcessOpts::default()).unwrap();
    near(&floats(&data)[28..32], &[1.6, 1.2, 0.4, 1.0]);
}

// ---- VectorCraft ---------------------------------------------------------------------------

#[test]
fn a_vectorcraft_module_installs_as_legacy_and_runs_objects_through_the_manager() {
    let (h, sink) = setup();
    assert_eq!(h.manager.install(&DESATURATE, Source::File).unwrap(), InstallOutcome::Loaded { generation: 1, replaced: None });
    let id = "test.vectorcraft.desaturate";
    let info = h.manager.info(id).unwrap();
    let legacy = info.legacy.unwrap();
    assert_eq!(legacy.abi, LegacyAbi::Vectorcraft);
    assert!(legacy.limits.contains(&("wall time per run".to_string(), "60 s (5 s for a live effect)".to_string())), "{:?}", legacy.limits);
    assert!(legacy.limits.contains(&("output size".to_string(), "64 MiB".to_string())));
    let surface = sink.surface("filter");
    assert_eq!(surface.contribution.kind, Kind::ObjectFilter);

    // The stand-in answers with the params it got, so the answer shows the v1 rules at work.
    let input = br#"{"objects":[]}"#;
    let params = |given: &Value| -> Value { serde_json::from_slice(&surface.run_objects(input, given).unwrap()).unwrap() };
    assert_eq!(params(&json!({ "amount": 40 })), json!({ "amount": 40.0 }));
    // The default amount is 100.
    assert_eq!(params(&Value::Null), json!({ "amount": 100.0 }));
    // v1 rules: a number past the range is clamped, so 500 means 100. A wrong type is an error.
    assert_eq!(params(&json!({ "amount": 500 })), json!({ "amount": 100.0 }));
    assert!(matches!(surface.run_objects(input, &json!({ "amount": "all" })), Err(CallError::BadJob(_))));
    // Not pixels.
    let mut data = vec![0u8; 16];
    let mut job = ImageJob::new(rgba_f32(2, 2, Alpha::Straight), &mut data);
    assert!(
        matches!(surface.process_image(&mut job, &Value::Null, &ProcessOpts::default()), Err(CallError::NotCallable(m)) if m.contains("run_objects"))
    );
    h.services.recorded(|rec| {
        assert_eq!(rec.begun.len(), 3, "the refused jobs opened no scope");
        assert!(rec.ended.iter().all(|(_, o)| *o == CallOutcome::Completed));
    });
}

#[test]
fn run_objects_is_for_vectorcraft_only() {
    let (h, sink) = setup();
    h.manager.install(&INVERT, Source::File).unwrap();
    assert!(matches!(sink.surface("filter").run_objects(b"{}", &Value::Null), Err(CallError::NotCallable(m)) if m.contains("VectorCraft")));
    h.services.recorded(|rec| assert!(rec.begun.is_empty()));
}

// ---- lifecycle -----------------------------------------------------------------------------

#[test]
fn a_legacy_plugin_survives_a_restart_and_leaves_with_remove() {
    let clock = common::StepClock::new(Duration::ZERO);
    let store = Arc::new(MemoryStore::new());
    let engine = || Arc::new(WasmiEngine::new(clock.clone()).with_slice_fuel(20_000));
    let first = Harness::build(engine(), clock.clone(), BudgetConfig::default(), store.clone(), store.clone());
    first.manager.install(&INVERT, Source::File).unwrap();
    first.manager.install(&DESATURATE, Source::File).unwrap();
    assert_eq!(store.load_all().unwrap().len(), 2, "the original bytes are stored");

    // A new process: same store, new manager.
    let second = Harness::build(engine(), clock.clone(), BudgetConfig::default(), store.clone(), store.clone());
    let sink = Arc::new(RecordingSink::default());
    second.manager.attach_surfaces(sink.clone());
    let mut restored = second.manager.restore().unwrap();
    restored.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        restored,
        vec![
            ("test.photocraft.invert".to_string(), PluginState::Active { generation: 1 }),
            ("test.vectorcraft.desaturate".to_string(), PluginState::Active { generation: 1 }),
        ]
    );
    assert_eq!(sink.ids(), vec!["filter".to_string(), "filter".to_string()]);
    assert_eq!(second.manager.info("test.photocraft.invert").unwrap().legacy.unwrap().abi, LegacyAbi::Photocraft);

    // Disable drops the surface; enable puts the module through its shim again.
    second.manager.disable("test.photocraft.invert").unwrap();
    assert_eq!(sink.ids().len(), 1);
    assert_eq!(second.manager.state("test.photocraft.invert"), Some(PluginState::Disabled));
    assert!(second.manager.info("test.photocraft.invert").unwrap().legacy.is_some(), "a disabled plugin is still known as legacy");
    second.manager.enable("test.photocraft.invert").unwrap();
    assert_eq!(sink.ids().len(), 2);
    second.manager.remove("test.photocraft.invert").unwrap();
    second.manager.remove("test.vectorcraft.desaturate").unwrap();
    assert!(sink.ids().is_empty());
    assert!(store.load_all().unwrap().is_empty());
}

#[test]
fn a_v1_module_that_breaks_its_abi_is_refused_and_leaves_no_trace() {
    let (h, sink) = setup();
    // The manifest says API 3, and this build implements API 1.
    let api3 = common::swap(&gain_manifest("org.test.api3"), "\"api\":1", "\"api\":3");
    let bytes = wat::parse_str(common::gain::module_with(&api3, "")).unwrap();
    match h.manager.install(&bytes, Source::File) {
        Err(LifecycleError::Legacy(m)) => assert!(m.contains("API 3"), "{m}"),
        other => panic!("expected a legacy load error, got {other:?}"),
    }
    assert!(h.store.load_all().unwrap().is_empty());
    assert!(h.manager.list().is_empty() && sink.ids().is_empty());
    // Not a module at all, but with a v1 looking export name: still an error, never a panic.
    let junk = [b"\0asm\x01\0\0\0".as_slice(), b"garbage"].concat();
    assert!(h.manager.install(&junk, Source::File).is_err());
}
