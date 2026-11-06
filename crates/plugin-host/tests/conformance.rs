//! Cross-engine conformance. The same plugins from `tests/fixtures/conformance/` run on wasmi and
//! on wasmtime. Each scenario runs on both engines. The test asserts that the two outcomes are
//! equal and that they equal the value the ABI demands, so two engines that agree on a wrong
//! answer still fail. Where the outcome is a trap, the trap variant is compared, not its text.
//!
//! Each scenario has a control (same plugin, a budget it fits in) so a pass shows the budget is the
//! cause. Kernel output is checked against a plain Rust reference and between engines.
//!
//! Not covered here: the browser engines, float edge cases (NaN payloads, `nearest` ties), simd128 kernels.

#![cfg(all(feature = "wasmi", feature = "wasmtime", not(target_arch = "wasm32")))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;
#[path = "fixtures/conformance/reference.rs"]
mod reference;

use std::fmt::Debug;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use common::{PAGE, STACK, block_on, gate, no_host};
use craft_plugin_abi::descriptor::{Alpha, Kind, ProcessDescriptor, Sample};
use craft_plugin_abi::ops::unpack_call_return;
use craft_plugin_abi::{HostPolicy, Op, Status, check_package};
use craft_plugin_host::engine::wasmi::WasmiEngine;
use craft_plugin_host::engine::wasmtime::WasmtimeEngine;
use craft_plugin_host::engine::{
    Budget, Caps, CompileOptions, Engine, EngineId, Export, HostDispatch, HostError, HostResult, Instance, LoadError, Trap, Trust,
    require_deadline_enforcement,
};
use reference::{Image, blur_reference, noise_image, posterize_reference};
use serde_json::{Value, json};

const MIB: u64 = 1024 * 1024;

// ---- harness ----------------------------------------------------------------------------

/// Both engines. wasmi runs small fuel slices so a debug build still checks its clock often.
fn engines() -> Vec<Box<dyn Engine>> {
    vec![Box::new(WasmiEngine::system().with_slice_fuel(20_000)), Box::new(WasmtimeEngine::system().unwrap())]
}

fn wat_path(name: &str) -> String {
    format!("{}/tests/fixtures/conformance/{name}.wat", env!("CARGO_MANIFEST_DIR"))
}

fn manifest(ops: &[&str], kind: &str) -> Value {
    let mut contribution = json!({ "kind": kind, "id": "c", "entry": 0 });
    if kind == "effect" {
        contribution["process"] = json!({ "sample": ["u8"], "alpha": "straight" });
    }
    json!({
        "schema": 1, "id": "org.craft.conformance", "name": "Conformance", "version": "1.0.0",
        "abi": { "craft": 1 },
        "requires": { "ops": ops },
        "contributions": [contribution]
    })
}

/// The package bytes for plugin `name`, with a control-plane or an effect contribution.
fn package(name: &str, ops: &[&str], kind: &str) -> Vec<u8> {
    let text = std::fs::read_to_string(wat_path(name)).unwrap_or_else(|e| panic!("{}: {e}", wat_path(name)));
    common::package_from(&text, &manifest(ops, kind))
}

fn opts() -> CompileOptions {
    CompileOptions { trust: Trust::Untrusted, stack_bytes: STACK }
}

fn budget() -> Budget {
    Budget::new(16 * PAGE, STACK)
}

fn norm<T>(r: Result<T, Trap>) -> Result<T, Trap> {
    r.map_err(|t| match t {
        Trap::GuestFault(_) => Trap::GuestFault(String::new()),
        other => other,
    })
}

fn instantiate(e: &dyn Engine, bytes: &[u8], host: HostDispatch, b: &Budget) -> Result<Box<dyn Instance>, LoadError> {
    let report = gate(bytes);
    assert!(report.is_accepted(), "gate rejected the plugin: {:#?}", report.violations);
    let module = block_on(e.compile(bytes, &report, opts()))?;
    block_on(module.instantiate(host, b))
}

fn loaded(e: &dyn Engine, name: &str, kind: &str, b: &Budget) -> Box<dyn Instance> {
    instantiate(e, &package(name, &["host.log"], kind), no_host(), b).unwrap_or_else(|err| panic!("{} on {:?}: {err}", name, e.id()))
}

/// Run `scenario` on every engine. Fail unless all outcomes are equal. Return the shared outcome.
fn on_all<T: Debug + PartialEq>(scenario: impl Fn(&dyn Engine) -> T) -> T {
    let mut results: Vec<(EngineId, T)> = engines().iter().map(|e| (e.id(), scenario(e.as_ref()))).collect();
    let (first_id, first) = results.remove(0);
    for (id, other) in &results {
        assert_eq!(&first, other, "engines disagree: {first_id:?} vs {id:?}");
    }
    first
}

/// The status and byte count of a packed `craft_host.call` answer.
fn unpack(packed: i64) -> (Status, u32) {
    let (code, count) = unpack_call_return(packed);
    (Status::from_code(code).unwrap_or_else(|| panic!("unknown status code {code}")), count)
}

fn invoke(inst: &mut dyn Instance, entry: i32, arg: i32, b: &Budget) -> Result<i64, Trap> {
    norm(inst.call(Export::Invoke, &[entry, arg, 0, 0, 0], b))
}

// ---- deadline, work, cancel ----------------------------------------------------

#[test]
fn loop_stops_at_the_deadline_within_two_seconds() {
    let out = on_all(|e| {
        let mut b = budget();
        b.deadline = Some(Duration::from_millis(100));
        let mut inst = loaded(e, "loop", "command", &b);
        let t0 = Instant::now();
        let r = invoke(inst.as_mut(), 0, 0, &b);
        let took = t0.elapsed();
        assert!(took >= Duration::from_millis(100), "{:?} stopped early: {took:?}", e.id());
        assert!(took < Duration::from_secs(2), "{:?} stopped late: {took:?}", e.id());
        r
    });
    assert_eq!(out, Err(Trap::Deadline));
}

#[test]
fn a_finite_loop_finishes_inside_the_same_deadline() {
    // Control for the test above: the deadline alone does not stop a call that ends in time.
    let out = on_all(|e| {
        let mut b = budget();
        b.deadline = Some(Duration::from_secs(10));
        let mut inst = loaded(e, "loop", "command", &b);
        invoke(inst.as_mut(), 1, 1000, &b)
    });
    assert_eq!(out, Ok(1000));
}

#[test]
fn loop_stops_at_the_work_cap() {
    let out = on_all(|e| {
        let mut b = budget();
        b.work = Some(1_000_000);
        let mut inst = loaded(e, "loop", "command", &b);
        invoke(inst.as_mut(), 0, 0, &b)
    });
    assert_eq!(out, Err(Trap::OutOfWork));
}

#[test]
fn a_finite_loop_finishes_inside_the_same_work_cap() {
    let out = on_all(|e| {
        let mut b = budget();
        b.work = Some(1_000_000);
        let mut inst = loaded(e, "loop", "command", &b);
        invoke(inst.as_mut(), 1, 100, &b)
    });
    assert_eq!(out, Ok(100));
}

#[test]
fn the_work_cap_counts_each_call_alone() {
    // Two calls that each fit the cap both finish. A cap shared across calls would stop the second.
    let out = on_all(|e| {
        let mut b = budget();
        b.work = Some(100_000);
        let mut inst = loaded(e, "loop", "command", &b);
        let a = invoke(inst.as_mut(), 1, 10_000, &b);
        let c = invoke(inst.as_mut(), 1, 10_000, &b);
        (a, c)
    });
    assert_eq!(out, (Ok(10_000), Ok(10_000)));
}

#[test]
fn a_cancel_set_before_the_call_stops_it_before_guest_code_runs() {
    let out = on_all(|e| {
        let b = budget();
        b.cancel.store(true, Ordering::Relaxed);
        let mut inst = loaded(e, "loop", "command", &b);
        invoke(inst.as_mut(), 0, 0, &b)
    });
    assert_eq!(out, Err(Trap::Cancelled));
}

#[test]
fn a_cancel_set_while_the_guest_runs_stops_it() {
    let out = on_all(|e| {
        let b = budget();
        let flag = Arc::clone(&b.cancel);
        let mut inst = loaded(e, "loop", "command", &b);
        let setter = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            flag.store(true, Ordering::Relaxed);
        });
        let t0 = Instant::now();
        let r = invoke(inst.as_mut(), 0, 0, &b);
        setter.join().unwrap();
        assert!(t0.elapsed() < Duration::from_secs(2), "{:?} ignored the cancel flag", e.id());
        r
    });
    assert_eq!(out, Err(Trap::Cancelled));
}

#[test]
fn a_start_function_that_never_returns_does_not_instantiate() {
    // The gate rejects start functions (`Violation::StartFunction`). This test hands the engines a
    // report that accepts the package anyway, because the start fuel is the second line of defense.
    let bytes = package("start-loop", &["host.log"], "command");
    assert_eq!(gate(&bytes).violations, vec![craft_plugin_abi::Violation::StartFunction], "the gate must reject it first");
    let out = on_all(|e| {
        let b = budget();
        let report = craft_plugin_abi::PolicyReport { violations: vec![], legacy: None, manifest: None };
        let module = block_on(e.compile(&bytes, &report, opts())).unwrap();
        let t0 = Instant::now();
        let r = block_on(module.instantiate(no_host(), &b));
        assert!(t0.elapsed() < Duration::from_secs(5), "{:?} hung in the start function", e.id());
        r.map(|_| ()).map_err(|err| matches!(err, LoadError::Instantiate(_)))
    });
    assert_eq!(out, Err(true));
}

// ---- memory -----------------------------------------------------------------------------

#[test]
fn grow_past_the_cap_traps_and_changes_nothing() {
    let out = on_all(|e| {
        let mut b = budget();
        b.memory_bytes = 4 * PAGE;
        let mut inst = loaded(e, "oom", "command", &b);
        let inside = invoke(inst.as_mut(), 0, 2, &b);
        let len_after_ok = inst.mem_len();
        let past = invoke(inst.as_mut(), 0, 2, &b);
        (inside, len_after_ok, past, inst.mem_len())
    });
    assert_eq!(out, (Ok(1), 3 * PAGE as usize, Err(Trap::OutOfMemory), 3 * PAGE as usize));
}

#[test]
fn the_same_grow_succeeds_under_a_bigger_cap() {
    let out = on_all(|e| {
        let mut b = budget();
        b.memory_bytes = 64 * PAGE;
        let mut inst = loaded(e, "oom", "command", &b);
        (invoke(inst.as_mut(), 0, 20, &b), inst.mem_len())
    });
    assert_eq!(out, (Ok(1), 21 * PAGE as usize));
}

#[test]
fn a_grow_storm_stops_at_the_cap_with_out_of_memory() {
    let out = on_all(|e| {
        let mut b = budget();
        b.memory_bytes = 8 * PAGE;
        let mut inst = loaded(e, "alloc-bomb", "command", &b);
        (invoke(inst.as_mut(), 0, 0, &b), inst.mem_len())
    });
    assert_eq!(out, (Err(Trap::OutOfMemory), 8 * PAGE as usize));
}

#[test]
fn a_grow_storm_under_a_loose_cap_ends_at_the_modules_own_maximum() {
    // Control: the cap is above the module's maximum of 64 pages, so grow returns -1, like the spec.
    let out = on_all(|e| {
        let mut b = budget();
        b.memory_bytes = 1024 * MIB;
        let mut inst = loaded(e, "alloc-bomb", "command", &b);
        (invoke(inst.as_mut(), 0, 0, &b), inst.mem_len())
    });
    assert_eq!(out, (Ok(63), 64 * PAGE as usize));
}

#[test]
fn initial_memory_over_the_cap_does_not_instantiate() {
    let out = on_all(|e| {
        let mut b = budget();
        b.memory_bytes = PAGE; // the kernels start with 8 pages
        instantiate(e, &package("kernel-posterize", &[], "effect"), no_host(), &b).map(|_| ())
    });
    assert_eq!(out, Err(LoadError::MemoryOverBudget));
}

// ---- traps and stack --------------------------------------------------------------------

#[test]
fn guest_faults_map_to_the_same_trap_on_both_engines() {
    let out = on_all(|e| {
        let b = budget();
        let mut inst = loaded(e, "trap", "command", &b);
        (0..=6).map(|entry| invoke(inst.as_mut(), entry, 0, &b)).collect::<Vec<_>>()
    });
    let fault = || Err(Trap::GuestFault(String::new()));
    assert_eq!(out, vec![Err(Trap::Unreachable), fault(), fault(), fault(), fault(), fault(), Ok(7)]);
}

#[test]
fn division_by_a_non_zero_value_does_not_trap() {
    // Control for entry 1 of the trap plugin.
    let out = on_all(|e| {
        let b = budget();
        let mut inst = loaded(e, "trap", "command", &b);
        invoke(inst.as_mut(), 1, 5, &b)
    });
    assert_eq!(out, Ok(2));
}

#[test]
fn deep_recursion_overflows_the_stack_and_shallow_recursion_does_not() {
    let out = on_all(|e| {
        let b = budget();
        let mut inst = loaded(e, "stack", "command", &b);
        let shallow = invoke(inst.as_mut(), 0, 8, &b);
        let deep = invoke(inst.as_mut(), 0, 1_000_000, &b);
        let after = invoke(inst.as_mut(), 0, 8, &b);
        (shallow, deep, after)
    });
    assert_eq!(out, (Ok(8), Err(Trap::StackOverflow), Ok(8)));
}

// ---- host call pointers and dispatcher results ------------------------------------------

#[test]
fn a_host_call_with_pointers_outside_memory_gets_bad_arg_and_the_guest_keeps_running() {
    let out = on_all(|e| {
        let b = budget();
        let mut inst = loaded(e, "bad-pointer", "command", &b);
        (0..=3).map(|entry| invoke(inst.as_mut(), entry, 0, &b).map(|r| unpack(r).0)).collect::<Vec<_>>()
    });
    assert_eq!(out, vec![Ok(Status::BadArg); 4]);
}

#[test]
fn a_host_call_with_valid_pointers_reaches_the_dispatcher_and_writes_guest_memory() {
    let out = on_all(|e| {
        let b = budget();
        let host: HostDispatch = Arc::new(|op: Op, input: &[u8], out: &mut [u8]| -> HostResult {
            assert_eq!(op, Op::HostLog);
            assert_eq!(input, b"{}");
            out[..2].copy_from_slice(b"ok");
            Ok(2)
        });
        let mut inst = instantiate(e, &package("bad-pointer", &["host.log"], "command"), host, &b).unwrap();
        let r = invoke(inst.as_mut(), 4, 0, &b).map(unpack);
        let mut bytes = [0u8; 2];
        inst.read(1024, &mut bytes).unwrap();
        (r, bytes)
    });
    assert_eq!(out, (Ok((Status::Ok, 2)), *b"ok"));
}

#[test]
fn a_dispatcher_that_kills_the_guest_returns_its_trap() {
    let out = on_all(|e| {
        let b = budget();
        let host: HostDispatch = Arc::new(|_op: Op, _i: &[u8], _o: &mut [u8]| -> HostResult { Err(HostError::Trap(Trap::Denied(Op::HostLog))) });
        let mut inst = instantiate(e, &package("bad-pointer", &["host.log"], "command"), host, &b).unwrap();
        invoke(inst.as_mut(), 4, 0, &b)
    });
    assert_eq!(out, Err(Trap::Denied(Op::HostLog)));
}

#[test]
fn a_too_small_answer_reports_the_required_size() {
    let out = on_all(|e| {
        let b = budget();
        let host: HostDispatch = Arc::new(|_op: Op, _i: &[u8], _o: &mut [u8]| -> HostResult { Err(HostError::TooSmall { required: 500 }) });
        let mut inst = instantiate(e, &package("bad-pointer", &["host.log"], "command"), host, &b).unwrap();
        invoke(inst.as_mut(), 4, 0, &b).map(unpack)
    });
    assert_eq!(out, Ok((Status::TooSmall, 500)));
}

#[test]
fn host_side_reads_and_writes_outside_memory_are_bad_pointer() {
    let out = on_all(|e| {
        let b = budget();
        let mut inst = loaded(e, "loop", "command", &b);
        let len = inst.mem_len() as u32;
        let mut one = [0u8; 1];
        let mut two = [0u8; 2];
        (
            inst.read(len - 1, &mut one),
            inst.read(len, &mut one),
            inst.read(u32::MAX, &mut two),
            inst.write(len - 1, &[1]),
            inst.write(len - 1, &[1, 2]),
            inst.write(u32::MAX, &[1, 2]),
        )
    });
    assert_eq!(out, (Ok(()), Err(Trap::BadPointer), Err(Trap::BadPointer), Ok(()), Err(Trap::BadPointer), Err(Trap::BadPointer)));
}

// ---- call plumbing ----------------------------------------------------------------------

#[test]
fn call_argument_and_budget_errors_match() {
    let out = on_all(|e| {
        let b = budget();
        let mut inst = loaded(e, "loop", "command", &b);
        let arity = inst.call(Export::Invoke, &[0, 0], &b);
        let missing = inst.call(Export::StateSave, &[0], &b);
        let mut other_stack = budget();
        other_stack.stack_bytes = STACK * 2;
        let mismatch = inst.call(Export::Abi, &[], &other_stack);
        let abi = inst.call(Export::Abi, &[], &b);
        let load = instantiate(e, &package("loop", &["host.log"], "command"), no_host(), &other_stack).map(|_| ());
        (arity, missing, mismatch, abi, load)
    });
    assert_eq!(
        out,
        (
            Err(Trap::BadArity { export: Export::Invoke, expected: 5, got: 2 }),
            Err(Trap::NoSuchExport(Export::StateSave)),
            Err(Trap::BudgetMismatch),
            Ok(1),
            Err(LoadError::StackBudgetMismatch { compiled: STACK, requested: STACK * 2 }),
        )
    );
}

#[test]
fn both_engines_can_stop_an_untrusted_plugin() {
    for e in engines() {
        let caps = e.caps();
        assert!(caps.enforces_deadline(), "{:?}", e.id());
        assert!(caps.contains(Caps::FUEL | Caps::MEMORY_CAP | Caps::STACK_CAP | Caps::SYNC_CALL), "{:?}", e.id());
        assert!(caps.contains(Caps::CANON_NAN), "{:?} must canonicalize NaN to match the other engine", e.id());
        require_deadline_enforcement(e.id(), caps, Trust::Untrusted).unwrap();
    }
}

#[test]
fn compile_refuses_a_package_the_gate_rejected() {
    let out = on_all(|e| {
        let bytes = package("kernel-posterize", &[], "effect");
        // Gate with a memory policy below the plugin's declared maximum (1024 pages = 64 MiB).
        let report = check_package(&bytes, &HostPolicy { memory_cap_mb: 1, simd128: false });
        assert!(!report.is_accepted());
        block_on(e.compile(&bytes, &report, opts())).map(|_| ()).map_err(|err| matches!(err, LoadError::Gate(_)))
    });
    assert_eq!(out, Err(true));
}

// ---- kernels ----------------------------------------------------------------------------

/// Run `craft_process` the way the host does: allocate in the guest, copy in, call, copy out.
fn process(e: &dyn Engine, plugin: &str, img: &Image, params: &[u8], b: &Budget) -> Result<(i64, Vec<u8>), Trap> {
    let mut inst = instantiate(e, &package(plugin, &[], "effect"), no_host(), b).unwrap_or_else(|err| panic!("{plugin}: {err}"));
    let alloc = |inst: &mut dyn Instance, len: usize| -> Result<u32, Trap> {
        let p = inst.call(Export::Alloc, &[len as i32, 4], b)?;
        if p == 0 { Err(Trap::OutOfMemory) } else { Ok(p as u32) }
    };
    let len = img.pixels.len();
    let desc_ptr = alloc(inst.as_mut(), 96)?;
    let in_ptr = alloc(inst.as_mut(), len)?;
    let out_ptr = alloc(inst.as_mut(), len)?;
    let params_ptr = alloc(inst.as_mut(), params.len().max(1))?;
    inst.write(in_ptr, &img.pixels)?;
    inst.write(params_ptr, params)?;
    let desc = ProcessDescriptor {
        kind: Kind::Image,
        entry: 0,
        w: img.w,
        h: img.h,
        stride_bytes: img.stride,
        channels: 4,
        sample: Sample::U8,
        alpha: Alpha::Straight,
        color_mode: 0,
        depth: 8,
        origin_x: 0,
        origin_y: 0,
        overlap: 0,
        scale: 1.0,
        time: 0.0,
        sample_rate: 0,
        frames: 0,
        in_ptr,
        out_ptr,
        params_ptr,
        params_len: params.len() as u32,
    };
    inst.write(desc_ptr, &desc.encode())?;
    let code = inst.call(Export::Process, &[desc_ptr as i32, 96], b)?;
    let mut out = vec![0u8; len];
    inst.read(out_ptr, &mut out)?;
    Ok((code, out))
}

fn kernel_budget() -> Budget {
    Budget::new(96 * MIB, STACK)
}

#[test]
fn posterize_matches_the_reference_on_both_engines() {
    for (w, h, pad, levels) in [(67u32, 31u32, 12u32, 4u32), (1, 1, 0, 2), (16, 9, 0, 256), (33, 5, 4, 7)] {
        let img = noise_image(w, h, pad, 0x9E37_79B9 + u64::from(w));
        let out = on_all(|e| process(e, "kernel-posterize", &img, &levels.to_le_bytes(), &kernel_budget()));
        assert_eq!(out, Ok((0, posterize_reference(&img, levels))), "{w}x{h} pad {pad} levels {levels}");
    }
}

#[test]
fn posterize_reference_differs_from_the_input_so_the_comparison_can_fail() {
    // Control: a kernel that copied its input would not match the reference, so equality above means work was done.
    let img = noise_image(67, 31, 12, 7);
    assert_ne!(posterize_reference(&img, 4), img.pixels);
}

#[test]
fn blur_matches_the_reference_on_both_engines() {
    for (w, h, pad) in [(67u32, 31u32, 12u32), (1, 1, 0), (3, 2, 0), (5, 40, 8), (13, 13, 0)] {
        let img = noise_image(w, h, pad, 0xC0FF_EE00 + u64::from(h));
        let out = on_all(|e| process(e, "kernel-blur", &img, &[], &kernel_budget()));
        assert_eq!(out, Ok((0, blur_reference(&img))), "{w}x{h} pad {pad}");
    }
}

#[test]
fn a_kernel_given_an_unsupported_layout_returns_its_error_code_on_both_engines() {
    // The kernel checks channels. A 3 channel descriptor is code 3, and the output stays untouched.
    let out = on_all(|e| {
        let b = kernel_budget();
        let mut inst = instantiate(e, &package("kernel-posterize", &[], "effect"), no_host(), &b).unwrap();
        let desc = ProcessDescriptor {
            kind: Kind::Image,
            entry: 0,
            w: 1,
            h: 1,
            stride_bytes: 3,
            channels: 3,
            sample: Sample::U8,
            alpha: Alpha::None,
            color_mode: 0,
            depth: 8,
            origin_x: 0,
            origin_y: 0,
            overlap: 0,
            scale: 1.0,
            time: 0.0,
            sample_rate: 0,
            frames: 0,
            in_ptr: 8192,
            out_ptr: 8200,
            params_ptr: 8208,
            params_len: 4,
        };
        let p = inst.call(Export::Alloc, &[96, 4], &b).unwrap() as u32;
        inst.write(p, &desc.encode()).unwrap();
        inst.call(Export::Process, &[p as i32, 96], &b)
    });
    assert_eq!(out, Ok(3));
}

#[test]
fn a_slow_kernel_is_stopped_by_the_deadline_mid_image_and_a_fast_one_is_not() {
    let img = noise_image(256, 256, 0, 99);
    // spin 4000 per pixel on 65536 pixels is about 260M loop iterations: far past 150 ms on either engine.
    let slow_params = [4u32.to_le_bytes(), 4000u32.to_le_bytes()].concat();
    let stopped = on_all(|e| {
        let mut b = kernel_budget();
        b.deadline = Some(Duration::from_millis(150));
        let t0 = Instant::now();
        let r = process(e, "slow-posterize", &img, &slow_params, &b);
        assert!(t0.elapsed() < Duration::from_secs(2), "{:?} did not stop the kernel in time: {:?}", e.id(), t0.elapsed());
        r
    });
    assert_eq!(stopped, Err(Trap::Deadline));
    // Control: the same plugin with one spin per pixel finishes inside the same deadline and matches posterize.
    let fast_params = [4u32.to_le_bytes(), 1u32.to_le_bytes()].concat();
    let done = on_all(|e| {
        let mut b = kernel_budget();
        b.deadline = Some(Duration::from_secs(20));
        process(e, "slow-posterize", &img, &fast_params, &b)
    });
    assert_eq!(done, Ok((0, posterize_reference(&img, 4))));
}

#[test]
fn a_kernel_over_the_work_cap_stops_with_out_of_work() {
    let img = noise_image(256, 256, 0, 5);
    let out = on_all(|e| {
        let mut b = kernel_budget();
        b.work = Some(200_000); // 65536 pixels cost far more than this on any engine
        process(e, "kernel-posterize", &img, &4u32.to_le_bytes(), &b)
    });
    assert_eq!(out, Err(Trap::OutOfWork));
    let ok = on_all(|e| {
        let mut b = kernel_budget();
        b.work = Some(500_000_000);
        process(e, "kernel-posterize", &img, &4u32.to_le_bytes(), &b).map(|(c, _)| c)
    });
    assert_eq!(ok, Ok(0));
}

#[test]
fn a_kernel_whose_buffers_do_not_fit_the_memory_cap_fails_with_out_of_memory() {
    let img = noise_image(512, 512, 0, 3); // three buffers of 1 MiB in the blur
    let out = on_all(|e| {
        let b = Budget::new(2 * MIB, STACK);
        process(e, "kernel-blur", &img, &[], &b)
    });
    assert_eq!(out, Err(Trap::OutOfMemory));
}
