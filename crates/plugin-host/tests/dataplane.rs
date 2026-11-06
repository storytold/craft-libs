//! The data plane: pixels through `craft_process` on a
//! craft-abi 1 module. The module is `tests/fixtures/pixels.wat`, a real plugin; every test runs it
//! on wasmi through a real `PluginManager`. Each rule has a case that fails when the rule is removed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::services::*;
use common::surfaces::*;
use craft_plugin_abi::ProcessDescriptor;
use craft_plugin_abi::descriptor::{Alpha, Kind, Sample};
use craft_plugin_host::budget::BudgetConfig;
use craft_plugin_host::engine::Trap;
use craft_plugin_host::invoke::CallError;
use craft_plugin_host::lifecycle::{InstallOutcome, PluginState};
use craft_plugin_host::process::{ImageFormat, ImageJob, ProcessOpts};
use craft_plugin_host::services::CallOutcome;
use craft_plugin_host::store::Source;
use serde_json::{Value, json};

const ID: &str = "org.example.pixels";

struct Rig {
    h: Harness,
    sink: Arc<RecordingSink>,
}

fn rig_with(h: Harness) -> Rig {
    let sink = Arc::new(RecordingSink::default());
    h.manager.attach_surfaces(sink.clone());
    let out = h.manager.install(&common::surfaces::pixels_package(ID, "1.0.0"), Source::File).unwrap();
    assert_eq!(out, InstallOutcome::Loaded { generation: 1, replaced: None }, "the fixture asks for no permission");
    Rig { h, sink }
}

fn rig() -> Rig {
    rig_with(Harness::new())
}

fn fmt(sample: Sample, alpha: Alpha, channels: u32, depth: u32, w: u32, h: u32) -> ImageFormat {
    ImageFormat { width: w, height: h, channels, sample, alpha, color_mode: 3, depth }
}

fn u8_fmt(w: u32, h: u32, channels: u32) -> ImageFormat {
    fmt(Sample::U8, Alpha::None, channels, 8, w, h)
}

fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| ((i * 37 + i / 7 * 11 + 5) % 251) as u8).collect()
}

fn f32_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn f32s(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// `out[y] = in[y - 1] + in[y + 1]` per byte, wrapping, rows clamped at the image edge.
fn vsum_reference(src: &[u8], w: usize, h: usize, ch: usize) -> Vec<u8> {
    let rb = w * ch;
    let mut out = vec![0u8; src.len()];
    for y in 0..h {
        let (up, dn) = (y.saturating_sub(1), (y + 1).min(h - 1));
        for i in 0..rb {
            out[y * rb + i] = src[up * rb + i].wrapping_add(src[dn * rb + i]);
        }
    }
    out
}

fn run(rig: &Rig, id: &str, format: ImageFormat, data: &mut [u8], params: &Value, opts: &ProcessOpts) -> Result<usize, CallError> {
    let mut job = ImageJob::new(format, data);
    rig.sink.surface(id).process_image(&mut job, params, opts).map(|r| r.bands)
}

fn small_bands() -> ProcessOpts {
    // Padded rows are 126 bytes at 3 channels and overlap 1, so 700 bytes is 5 padded rows: 3 inner rows.
    ProcessOpts { band_bytes: Some(700), ..ProcessOpts::default() }
}

#[test]
fn the_fixture_registers_what_the_host_can_run_and_says_why_it_left_out_the_rest() {
    let r = rig();
    let info = r.h.manager.info(ID).unwrap();
    assert_eq!(info.state, PluginState::Active { generation: 1 });
    assert!(info.commands.is_empty(), "image kinds are not commands");
    let skipped: Vec<(&str, &str)> = info.surfaces.skipped.iter().map(|s| (s.contribution.as_str(), s.reason.as_str())).collect();
    assert_eq!(
        skipped,
        vec![("audio", "this host has no audio block runner yet"), ("objects", "object-filter runs through the VectorCraft v1 shim only")]
    );
    assert_eq!(r.sink.ids().len(), 13, "{:?}", r.sink.ids());
    assert!(r.sink.ids().contains(&"kind-filter".to_string()), "a filter is a surface too");
}

#[test]
fn a_banded_job_has_no_seams_because_each_band_gets_real_context() {
    let r = rig();
    let (w, h, ch) = (40usize, 37usize, 3usize);
    let src = pattern(w * h * ch);
    let want = vsum_reference(&src, w, h, ch);

    // One band: the whole image.
    let mut whole = src.clone();
    let bands = run(&r, "vsum", u8_fmt(w as u32, h as u32, ch as u32), &mut whole, &Value::Null, &ProcessOpts::default()).unwrap();
    assert_eq!(bands, 1);
    assert_eq!(whole, want);

    // Many bands: 37 rows in strips of 3 inner rows.
    let mut banded = src.clone();
    let bands = run(&r, "vsum", u8_fmt(w as u32, h as u32, ch as u32), &mut banded, &Value::Null, &small_bands()).unwrap();
    assert_eq!(bands, 13, "ceil(37 / 3)");
    assert_eq!(banded, want, "the seams between bands are invisible");

    // More context than the plugin needs gives the same pixels.
    let mut wide = src.clone();
    let bands = run(
        &r,
        "vsum3",
        u8_fmt(w as u32, h as u32, ch as u32),
        &mut wide,
        &Value::Null,
        &ProcessOpts { band_bytes: Some(1400), ..Default::default() },
    )
    .unwrap();
    assert!(bands > 1);
    assert_eq!(wide, want);
}

#[test]
fn a_job_is_one_logical_call_however_many_bands_it_runs() {
    let r = rig();
    let mut data = pattern(40 * 37 * 3);
    run(&r, "vsum", u8_fmt(40, 37, 3), &mut data, &Value::Null, &small_bands()).unwrap();
    r.h.services.recorded(|rec| {
        assert_eq!(rec.begun.len(), 1, "one begin_call for 13 bands");
        assert_eq!(rec.begun[0].1, "vsum");
        assert_eq!(rec.ended.len(), 1);
        assert_eq!(rec.ended[0].1, CallOutcome::Completed);
        assert_eq!(rec.ended[0].0, rec.begun[0].0, "the scope pairs");
    });
}

#[test]
fn parallel_bands_equal_serial_bands() {
    let r = rig();
    let src = pattern(40 * 37 * 3);
    let mut results = Vec::new();
    for threads in [1usize, 2, 8] {
        let mut data = src.clone();
        let opts = ProcessOpts { threads: Some(threads), ..small_bands() };
        run(&r, "vsum", u8_fmt(40, 37, 3), &mut data, &Value::Null, &opts).unwrap();
        results.push(data);
    }
    assert!(results.windows(2).all(|w| w[0] == w[1]));
    assert_eq!(results[0], vsum_reference(&src, 40, 37, 3));
}

#[test]
fn every_band_gets_the_descriptor_the_abi_describes() {
    let r = rig();
    let (w, h, ch) = (40usize, 37usize, 3usize);
    let format = ImageFormat { color_mode: 7, depth: 8, ..u8_fmt(w as u32, h as u32, ch as u32) };
    let mut data = vec![0u8; w * h * ch];
    let mut job = ImageJob::new(format, &mut data);
    (job.scale, job.time, job.origin) = (0.5, 12.25, (100, -20));
    let opts = ProcessOpts { band_bytes: Some(1200), ..Default::default() };
    let report = r.sink.surface("desc").process_image(&mut job, &Value::Null, &opts).unwrap();
    assert!(report.bands >= 3, "{report:?}");

    // The plugin wrote each band's descriptor at the band's first inner pixel. Walk the bands down the image.
    let rb = w * ch;
    let (mut y, mut seen) = (0usize, 0usize);
    while y < h {
        let d = ProcessDescriptor::decode(&data[y * rb..y * rb + 96]).unwrap();
        assert_eq!((d.kind, d.entry, d.channels, d.sample, d.alpha, d.color_mode, d.depth), (Kind::Image, 8, 3, Sample::U8, Alpha::None, 7, 8));
        assert_eq!(d.overlap, 2);
        assert_eq!((d.w, d.stride_bytes), (w as u32 + 4, (w as u32 + 4) * 3), "padded by the overlap on both sides");
        assert_eq!((d.scale, d.time), (0.5, 12.25));
        assert_eq!((d.sample_rate, d.frames), (0, 0));
        assert_eq!(d.origin_x, 100 - 2, "the padded band starts `overlap` pixels left of the image");
        assert_eq!(d.origin_y, -20 + y as i32 - 2, "and `overlap` rows above this band");
        assert!(d.in_ptr != 0 && d.out_ptr != 0 && d.in_ptr != d.out_ptr);
        y += d.h as usize - 4;
        seen += 1;
    }
    assert_eq!(y, h, "the bands cover the image exactly");
    assert_eq!(seen, report.bands);
}

#[test]
fn params_are_checked_against_the_schema_and_the_guest_gets_the_full_object() {
    let r = rig();
    let guest_params = |params: Value| -> Result<Value, CallError> {
        let mut data = vec![0u8; 40 * 4 * 3];
        run(&r, "params", u8_fmt(40, 4, 3), &mut data, &params, &ProcessOpts::default())?;
        let first: Value = serde_json::Deserializer::from_slice(&data).into_iter::<Value>().next().unwrap().unwrap();
        Ok(first)
    };
    // A default fills what the caller left out.
    assert_eq!(guest_params(json!({ "steps": 2 })).unwrap(), json!({ "amount": 0.5, "mode": "a", "steps": 2 }));
    assert_eq!(guest_params(json!({ "steps": 9, "amount": 1, "mode": "b" })).unwrap(), json!({ "amount": 1, "mode": "b", "steps": 9 }));
    let bad = |params: Value, needle: &str| {
        let before = r.h.services.recorded(|rec| rec.begun.len());
        match guest_params(params) {
            Err(CallError::BadJob(m)) => assert!(m.contains(needle), "{m}"),
            other => panic!("expected BadJob({needle}), got {other:?}"),
        }
        assert_eq!(r.h.services.recorded(|rec| rec.begun.len()), before, "a bad job opens no undo scope");
    };
    bad(json!({}), "`steps` is required and has no default");
    bad(json!({ "steps": 2, "volume": 1 }), "unknown parameter `volume`");
    bad(json!({ "steps": 2, "amount": 1.5 }), "outside 0..1");
    bad(json!({ "steps": 2, "mode": "c" }), "not one of");
    bad(json!({ "steps": 2.5 }), "whole number");
    bad(json!({ "steps": 10 }), "outside 1..9");
    bad(json!([1]), "must be an object");
}

#[test]
fn a_job_the_contribution_does_not_take_is_refused_before_any_guest_code_runs() {
    let r = rig();
    let refused = |id: &str, format: ImageFormat, len: usize, needle: &str| {
        let mut data = vec![7u8; len];
        match run(&r, id, format, &mut data, &Value::Null, &ProcessOpts::default()) {
            Err(CallError::BadJob(m)) => assert!(m.contains(needle), "{m}"),
            other => panic!("expected BadJob({needle}), got {other:?}"),
        }
        assert!(data.iter().all(|b| *b == 7), "the buffer is untouched");
    };
    refused("vsum", fmt(Sample::F32, Alpha::None, 1, 32, 2, 2), 16, "takes [U8]");
    refused("x4", fmt(Sample::F32, Alpha::None, 1, 32, 2, 2), 16, "takes Straight alpha");
    refused("copy", u8_fmt(4, 4, 3), 47, "is 48 bytes, the buffer has 47");
    refused("copy", u8_fmt(0, 4, 3), 0, "0 x 4");
    refused("copy", u8_fmt(4, 4, 0), 0, "0 channels");
    assert_eq!(r.h.services.recorded(|rec| rec.begun.len()), 0);
}

#[test]
fn kinds_that_are_not_pixels_say_so() {
    let r = rig();
    let generation = r.h.manager.router().current(ID).unwrap();
    let contribution = |id: &str| generation.manifest.contributions.iter().find(|c| c.id == id).unwrap().clone();
    let mut data = vec![0u8; 12];
    for (id, needle) in [("audio", "audio blocks are not wired"), ("objects", "takes vector objects")] {
        let mut job = ImageJob::new(u8_fmt(4, 1, 3), &mut data);
        match generation.process_image(&contribution(id), &mut job, &Value::Null, &ProcessOpts::default()) {
            Err(CallError::NotCallable(m)) => assert!(m.contains(needle), "{m}"),
            other => panic!("{id}: expected NotCallable({needle}), got {other:?}"),
        }
    }
    assert_eq!(r.h.services.recorded(|rec| rec.begun.len()), 0);
}

#[test]
fn output_that_cannot_be_a_pixel_is_fixed_by_the_host() {
    let r = rig();
    // NaN and infinity take the input value at the same place.
    let input = [0.25f32, 0.5, 0.75, 1.0, 0.1, 0.2, 0.3, 0.4];
    let mut data = f32_bytes(&input);
    run(&r, "nan", fmt(Sample::F32, Alpha::None, 4, 32, 2, 1), &mut data, &Value::Null, &ProcessOpts::default()).unwrap();
    assert_eq!(f32s(&data), input.to_vec());

    // x4 on a 32 bit image: colour may leave 0..1 (HDR), alpha may not.
    let mut data = f32_bytes(&[0.5, 0.5, 0.5, 0.5]);
    run(&r, "x4", fmt(Sample::F32, Alpha::Straight, 4, 32, 1, 1), &mut data, &Value::Null, &ProcessOpts::default()).unwrap();
    assert_eq!(f32s(&data), vec![2.0, 2.0, 2.0, 1.0]);
    // The same plugin on an image that came from 8 bit data: everything is clamped.
    let mut data = f32_bytes(&[0.5, 0.5, 0.5, 0.5]);
    run(&r, "x4", fmt(Sample::F32, Alpha::Straight, 4, 8, 1, 1), &mut data, &Value::Null, &ProcessOpts::default()).unwrap();
    assert_eq!(f32s(&data), vec![1.0, 1.0, 1.0, 1.0]);

    // 255 in every byte reads as 65535 in a u16 sample. A 10 bit image tops out at 1023.
    let mut data = vec![0u8; 8];
    run(&r, "big", fmt(Sample::U16, Alpha::None, 1, 10, 4, 1), &mut data, &Value::Null, &ProcessOpts::default()).unwrap();
    assert_eq!(data.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>(), vec![1023; 4]);
    let mut data = vec![0u8; 8];
    run(&r, "big", fmt(Sample::U16, Alpha::None, 1, 16, 4, 1), &mut data, &Value::Null, &ProcessOpts::default()).unwrap();
    assert_eq!(data.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>(), vec![65535; 4]);
}

#[test]
fn a_trap_in_a_late_band_leaves_the_whole_image_as_it_was_and_the_app_is_told_to_undo() {
    let r = rig();
    let src = pattern(40 * 37 * 3);
    let mut data = src.clone();
    // Bands near the top run fine; bands below row 10 trap. Nothing may be written.
    let err = run(&r, "trap-low", u8_fmt(40, 37, 3), &mut data, &Value::Null, &ProcessOpts { threads: Some(1), ..small_bands() }).unwrap_err();
    assert_eq!(err, CallError::Trap(Trap::Unreachable));
    assert_eq!(data, src, "early bands that succeeded were not committed");
    r.h.services.recorded(|rec| {
        assert_eq!(rec.begun.len(), 1);
        assert!(matches!(&rec.ended[0].1, CallOutcome::Faulted(m) if m.contains("unreachable")), "{:?}", rec.ended);
        assert_eq!(rec.notices.len(), 1);
        assert!(rec.notices[0].message.contains("failed and its changes were undone"), "{}", rec.notices[0].message);
    });
    // The image the plugin handles fine still works afterwards: the fault did not poison the plugin.
    let mut ok = src.clone();
    run(&r, "copy", u8_fmt(40, 37, 3), &mut ok, &Value::Null, &small_bands()).unwrap();
    assert_eq!(ok, src);
}

#[test]
fn three_faults_in_a_row_open_the_circuit_and_a_reload_closes_it() {
    let r = rig();
    let mut data = pattern(12);
    for n in 1..=3 {
        let err = run(&r, "trap", u8_fmt(4, 1, 3), &mut data, &Value::Null, &ProcessOpts::default()).unwrap_err();
        assert_eq!(err, CallError::Trap(Trap::Unreachable), "fault {n}");
    }
    let begun = r.h.services.recorded(|rec| rec.begun.len());
    assert_eq!(begun, 3);
    assert_eq!(run(&r, "trap", u8_fmt(4, 1, 3), &mut data, &Value::Null, &ProcessOpts::default()), Err(CallError::CircuitOpen));
    assert_eq!(r.h.services.recorded(|rec| rec.begun.len()), 3, "an open circuit opens no undo scope");
    assert!(r.h.services.recorded(|rec| rec.notices.last().unwrap().message.contains("stays off for this session after 3 faults")));
    // Other contributions of the plugin keep running.
    run(&r, "copy", u8_fmt(4, 1, 3), &mut data, &Value::Null, &ProcessOpts::default()).unwrap();
    // A reload resets the circuit.
    r.h.manager.install(&pixels_package(ID, "1.0.1"), Source::File).unwrap();
    assert_eq!(run(&r, "trap", u8_fmt(4, 1, 3), &mut data, &Value::Null, &ProcessOpts::default()), Err(CallError::Trap(Trap::Unreachable)));
}

#[test]
fn a_plugin_error_code_discards_the_output_without_counting_toward_the_circuit() {
    let r = rig();
    let src = pattern(12);
    for _ in 0..5 {
        let mut data = src.clone();
        let err = run(&r, "err", u8_fmt(4, 1, 3), &mut data, &Value::Null, &ProcessOpts::default()).unwrap_err();
        assert_eq!(err, CallError::Plugin("craft_process returned 7".into()));
        assert_eq!(data, src);
    }
    r.h.services.recorded(|rec| {
        assert_eq!(rec.ended.len(), 5);
        assert!(rec.ended.iter().all(|(_, o)| matches!(o, CallOutcome::Faulted(_))), "the app undoes a failed call");
        assert!(rec.notices.is_empty(), "the plugin said no on purpose; that is not a fault the user must hear about");
    });
}

#[test]
fn a_cancelled_job_ends_as_cancelled_and_touches_nothing() {
    let r = rig();
    let src = pattern(12);
    let mut data = src.clone();
    let cancel = Arc::new(AtomicBool::new(true));
    let opts = ProcessOpts { cancel: Some(cancel.clone()), ..Default::default() };
    assert_eq!(run(&r, "copy", u8_fmt(4, 1, 3), &mut data, &Value::Null, &opts).unwrap_err(), CallError::Trap(Trap::Cancelled));
    assert_eq!(data, src);
    r.h.services.recorded(|rec| {
        assert_eq!(rec.ended[0].1, CallOutcome::Cancelled);
        assert!(rec.notices.is_empty(), "a cancel the user asked for is not a failure");
    });
    cancel.store(false, Ordering::Relaxed);
    run(&r, "copy", u8_fmt(4, 1, 3), &mut data, &Value::Null, &opts).unwrap();
}

#[test]
fn a_band_that_runs_out_of_fuel_or_time_is_stopped() {
    // Fuel scales with the samples of the band: 16 samples at 100 per sample is 1,600 units.
    let tight = BudgetConfig { render_fuel_per_unit: 100, ..BudgetConfig::default() };
    let r = rig_with(Harness::with_clock(Duration::ZERO, tight));
    let mut data = vec![1u8; 16];
    let err = run(&r, "spin", u8_fmt(4, 4, 1), &mut data, &Value::Null, &ProcessOpts::default()).unwrap_err();
    assert_eq!(err, CallError::Trap(Trap::OutOfWork));
    assert_eq!(data, vec![1u8; 16]);

    // The wall clock: every clock read moves 1 ms and a band may take 20 ms.
    let slow = BudgetConfig { render_cap: Duration::from_millis(20), ..BudgetConfig::default() };
    let r = rig_with(Harness::with_clock(Duration::from_millis(1), slow));
    let err = run(&r, "spin", u8_fmt(4, 4, 1), &mut data, &Value::Null, &ProcessOpts::default()).unwrap_err();
    assert_eq!(err, CallError::Trap(Trap::Deadline));
}

#[test]
fn the_surface_follows_a_reload_and_a_removal() {
    let r = rig();
    let log = r.sink.log();
    assert!(log.iter().all(|l| l.starts_with("add org.example.pixels@1:")), "{log:?}");
    let surface = r.sink.surface("copy");
    let mut data = pattern(12);
    let mut job = ImageJob::new(u8_fmt(4, 1, 3), &mut data);
    surface.process_image(&mut job, &Value::Null, &ProcessOpts::default()).unwrap();

    r.h.manager.install(&pixels_package(ID, "2.0.0"), Source::File).unwrap();
    let log = r.sink.log();
    let tail: Vec<&String> = log.iter().skip_while(|l| !l.starts_with("remove")).collect();
    assert_eq!(tail[0], "remove org.example.pixels@1", "the old generation leaves first");
    assert!(tail[1..].iter().all(|l| l.starts_with("add org.example.pixels@2:")), "{tail:?}");
    // The handle the app kept before the reload runs generation 2 without being replaced.
    let mut job = ImageJob::new(u8_fmt(4, 1, 3), &mut data);
    surface.process_image(&mut job, &Value::Null, &ProcessOpts::default()).unwrap();

    r.h.manager.remove(ID).unwrap();
    assert!(r.sink.ids().is_empty(), "removal drops every surface");
    let mut job = ImageJob::new(u8_fmt(4, 1, 3), &mut data);
    assert_eq!(surface.process_image(&mut job, &Value::Null, &ProcessOpts::default()).unwrap_err(), CallError::NotLoaded);
}
