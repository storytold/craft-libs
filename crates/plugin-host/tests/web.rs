//! Browser conformance. Runs in a headless browser under `wasm-bindgen-test`.
//!
//! Three engines meet the conformance plugins of `tests/fixtures/conformance/` here:
//!
//! - wasmi inside the page, on a `WebClock` (the default engine on web),
//! - the browser engine on the page thread (`BrowserEngine`),
//! - the Worker engine (`BrowserWorkerEngine`), which the page can terminate.
//!
//! Where two engines can express the same scenario, the test runs both and asserts the outcomes are
//! equal, and equal to the value the ABI demands. Where a browser cannot express it, the test
//! says so and asserts what the browser does instead. The full difference table is the module
//! comment of `src/engine/browser.rs`. In short:
//!
//! - an infinite loop cannot run on the page thread engine (it would freeze the tab), so `loop`,
//!   fuel and slow kernel deadlines run on wasmi and on the Worker engine only,
//! - a grow past the memory cap returns -1 in a browser and traps with `OutOfMemory` in wasmi, and the
//!   memory size afterwards is the same,
//! - the Worker engine has no host: every op answers `E_UNSUPPORTED`.
//!
//! Not covered: Firefox, float edge cases and NaN payloads, simd128
//! kernels, an infinite loop on the page thread engine, a deadline on wasmi inside a Worker.

#![cfg(target_arch = "wasm32")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "common/pkg.rs"]
mod pkg;
#[path = "fixtures/conformance/reference.rs"]
mod reference;

use std::collections::HashSet;
use std::fmt::Debug;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use craft_command::Owner;
use craft_plugin_abi::descriptor::{Alpha, Kind, ProcessDescriptor, Sample};
use craft_plugin_abi::ops::unpack_call_return;
use craft_plugin_abi::permissions::DocScope;
use craft_plugin_abi::{HostPolicy, Op, Permission, PolicyReport, Status, check_package};
use craft_plugin_host::dispatch::{DispatchConfig, HostDispatcher, PluginContext};
use craft_plugin_host::engine::browser::worker::BrowserWorkerEngine;
use craft_plugin_host::engine::browser::{BrowserEngine, WebClock};
use craft_plugin_host::engine::wasmi::WasmiEngine;
use craft_plugin_host::engine::{
    Budget, Caps, CompileOptions, Engine, EngineId, Export, HostDispatch, HostError, HostResult, Instance, LoadError, Trap, Trust,
    require_deadline_enforcement,
};
use craft_plugin_host::services::*;
use craft_plugin_host::store::MemoryStore;
use reference::{Image, blur_reference, noise_image, posterize_reference};
use serde_json::{Value, json};
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
use web_time::Instant;

wasm_bindgen_test_configure!(run_in_browser);

const PAGE: u64 = 65_536;
const MIB: u64 = 1024 * 1024;
const STACK: u32 = 64 * 1024;

// ---- harness ----------------------------------------------------------------------------

fn wat_text(name: &str) -> &'static str {
    match name {
        "loop" => include_str!("fixtures/conformance/loop.wat"),
        "oom" => include_str!("fixtures/conformance/oom.wat"),
        "alloc-bomb" => include_str!("fixtures/conformance/alloc-bomb.wat"),
        "trap" => include_str!("fixtures/conformance/trap.wat"),
        "stack" => include_str!("fixtures/conformance/stack.wat"),
        "bad-pointer" => include_str!("fixtures/conformance/bad-pointer.wat"),
        "probe" => include_str!("fixtures/conformance/probe.wat"),
        "kernel-posterize" => include_str!("fixtures/conformance/kernel-posterize.wat"),
        "kernel-blur" => include_str!("fixtures/conformance/kernel-blur.wat"),
        "slow-posterize" => include_str!("fixtures/conformance/slow-posterize.wat"),
        "start-loop" => include_str!("fixtures/conformance/start-loop.wat"),
        other => panic!("no conformance plugin {other}"),
    }
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

fn package(name: &str, ops: &[&str], kind: &str) -> Vec<u8> {
    pkg::package_from(wat_text(name), &manifest(ops, kind))
}

fn gate(bytes: &[u8]) -> PolicyReport {
    check_package(bytes, &HostPolicy::web())
}

/// Trusted, because the page thread engine refuses untrusted plugins by design (tested below).
fn opts() -> CompileOptions {
    CompileOptions { trust: Trust::Trusted, stack_bytes: STACK }
}

fn budget() -> Budget {
    Budget::new(16 * PAGE, STACK)
}

fn no_host() -> HostDispatch {
    Arc::new(|_op, _inp: &[u8], _out: &mut [u8]| -> HostResult { Ok(0) })
}

fn wasmi() -> WasmiEngine {
    WasmiEngine::new(Arc::new(WebClock::new())).with_slice_fuel(20_000)
}

fn norm<T>(r: Result<T, Trap>) -> Result<T, Trap> {
    r.map_err(|t| match t {
        Trap::GuestFault(_) => Trap::GuestFault(String::new()),
        other => other,
    })
}

async fn instantiate(e: &dyn Engine, bytes: &[u8], host: HostDispatch, b: &Budget) -> Result<Box<dyn Instance>, LoadError> {
    let report = gate(bytes);
    assert!(report.is_accepted(), "gate rejected the plugin: {:#?}", report.violations);
    let module = e.compile(bytes, &report, opts()).await?;
    module.instantiate(host, b).await
}

async fn loaded(e: &dyn Engine, name: &str, kind: &str, b: &Budget) -> Box<dyn Instance> {
    match instantiate(e, &package(name, &["host.log"], kind), no_host(), b).await {
        Ok(i) => i,
        Err(err) => panic!("{name} on {:?}: {err}", e.id()),
    }
}

/// Run `scenario` on wasmi and on the page thread browser engine. Return both outcomes.
async fn pair<T>(scenario: impl AsyncFn(&dyn Engine) -> T) -> (T, T) {
    let w = wasmi();
    let b = BrowserEngine::new();
    let from_wasmi = scenario(&w).await;
    let from_browser = scenario(&b).await;
    (from_wasmi, from_browser)
}

/// Like `pair`, and fail unless both outcomes are equal. Return the shared outcome.
async fn both<T: Debug + PartialEq>(scenario: impl AsyncFn(&dyn Engine) -> T) -> T {
    let (a, b) = pair(scenario).await;
    assert_eq!(a, b, "wasmi and the browser engine disagree");
    a
}

fn invoke(inst: &mut dyn Instance, entry: i32, arg: i32, b: &Budget) -> Result<i64, Trap> {
    norm(inst.call(Export::Invoke, &[entry, arg, 0, 0, 0], b))
}

fn unpack(packed: i64) -> (Status, u32) {
    let (code, count) = unpack_call_return(packed);
    (Status::from_code(code).unwrap_or_else(|| panic!("unknown status code {code}")), count)
}

/// Resolve after `ms` milliseconds.
async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        let global = js_sys::global();
        let set: js_sys::Function = js_sys::Reflect::get(&global, &"setTimeout".into()).unwrap().into();
        set.call2(&global, &resolve, &ms.into()).unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

// ---- caps are truthful ------------------------------------------------------------------

#[wasm_bindgen_test]
fn the_page_thread_engine_reports_only_what_it_can_enforce() {
    let e = BrowserEngine::new();
    let caps = e.caps();
    assert_eq!(e.id(), EngineId::BrowserJit);
    assert!(caps.contains(Caps::SYNC_CALL | Caps::MEMORY_CAP), "{caps:?}");
    for missing in [Caps::FUEL, Caps::DEADLINE_PREEMPT, Caps::DEADLINE_KILL, Caps::STACK_CAP, Caps::CANON_NAN] {
        assert!(!caps.contains(missing), "the page thread engine cannot enforce {missing:?}");
    }
    assert!(!caps.enforces_deadline());
    assert!(require_deadline_enforcement(e.id(), caps, Trust::Untrusted).is_err());
    assert!(require_deadline_enforcement(e.id(), caps, Trust::Trusted).is_ok());
}

#[wasm_bindgen_test]
fn the_worker_engine_reports_a_kill_deadline_and_no_sync_call() {
    let e = BrowserWorkerEngine::new();
    let caps = e.caps();
    assert_eq!(e.id(), EngineId::BrowserJitWorker);
    assert!(caps.contains(Caps::DEADLINE_KILL | Caps::MEMORY_CAP), "{caps:?}");
    for missing in [Caps::FUEL, Caps::DEADLINE_PREEMPT, Caps::STACK_CAP, Caps::CANON_NAN, Caps::SYNC_CALL] {
        assert!(!caps.contains(missing), "the Worker engine cannot offer {missing:?}");
    }
    assert!(caps.enforces_deadline());
    assert!(require_deadline_enforcement(e.id(), caps, Trust::Untrusted).is_ok());
}

#[wasm_bindgen_test]
fn simd128_is_reported_when_this_browser_validates_it() {
    // The probe module is the same bytes `simd128_supported` validates. A headless Chromium has simd128.
    let supported = craft_plugin_host::engine::browser::simd128_supported();
    assert_eq!(BrowserEngine::new().caps().contains(Caps::SIMD128), supported);
    assert_eq!(BrowserWorkerEngine::new().caps().contains(Caps::SIMD128), supported);
    assert!(supported, "Chromium 146 validates simd128");
}

#[wasm_bindgen_test]
async fn the_page_thread_engine_refuses_an_untrusted_plugin_and_wasmi_does_not() {
    let bytes = package("loop", &["host.log"], "command");
    let report = gate(&bytes);
    let untrusted = CompileOptions { trust: Trust::Untrusted, stack_bytes: STACK };
    let browser = BrowserEngine::new().compile(&bytes, &report, untrusted).await.map(|_| ());
    assert_eq!(browser.unwrap_err(), LoadError::EngineCannotEnforceDeadline { engine: EngineId::BrowserJit });
    wasmi().compile(&bytes, &report, untrusted).await.unwrap();
    BrowserWorkerEngine::new().compile(&bytes, &report, untrusted).await.unwrap();
}

#[wasm_bindgen_test]
async fn compile_refuses_a_package_the_gate_rejected() {
    let out = both(async |e| {
        let bytes = package("kernel-posterize", &[], "effect");
        let report = check_package(&bytes, &HostPolicy { memory_cap_mb: 1, simd128: false });
        assert!(!report.is_accepted());
        e.compile(&bytes, &report, opts()).await.map(|_| ()).map_err(|err| matches!(err, LoadError::Gate(_)))
    })
    .await;
    assert_eq!(out, Err(true));
}

// ---- control plane: wasmi and the page thread engine -----------------------------------

#[wasm_bindgen_test]
async fn a_finite_loop_returns_its_count_on_both_engines() {
    let out = both(async |e| {
        let b = budget();
        let mut inst = loaded(e, "loop", "command", &b).await;
        invoke(inst.as_mut(), 1, 1000, &b)
    })
    .await;
    assert_eq!(out, Ok(1000));
}

#[wasm_bindgen_test]
async fn wasmi_on_web_stops_an_infinite_loop_at_the_deadline() {
    // The page thread engine cannot run this scenario: nothing could stop it, and the tab would freeze.
    let e = wasmi();
    let mut b = budget();
    b.deadline = Some(Duration::from_millis(100));
    let mut inst = loaded(&e, "loop", "command", &b).await;
    let t0 = Instant::now();
    let r = invoke(inst.as_mut(), 0, 0, &b);
    let took = t0.elapsed();
    assert_eq!(r, Err(Trap::Deadline));
    assert!(took >= Duration::from_millis(100) && took < Duration::from_secs(2), "stopped after {took:?}");
}

#[wasm_bindgen_test]
async fn a_cancel_set_before_the_call_stops_it_on_both_engines() {
    let out = both(async |e| {
        let b = budget();
        b.cancel.store(true, Ordering::Relaxed);
        let mut inst = loaded(e, "loop", "command", &b).await;
        invoke(inst.as_mut(), 0, 0, &b)
    })
    .await;
    assert_eq!(out, Err(Trap::Cancelled));
}

#[wasm_bindgen_test]
async fn grow_past_the_cap_changes_nothing_but_the_signal_differs() {
    let (w, br) = pair(async |e| {
        let mut b = budget();
        b.memory_bytes = 4 * PAGE;
        let mut inst = loaded(e, "oom", "command", &b).await;
        let inside = invoke(inst.as_mut(), 0, 2, &b);
        let len_after_ok = inst.mem_len();
        let past = invoke(inst.as_mut(), 0, 2, &b);
        (inside, len_after_ok, past, inst.mem_len())
    })
    .await;
    let three = 3 * PAGE as usize;
    assert_eq!(w, (Ok(1), three, Err(Trap::OutOfMemory), three), "wasmi traps");
    assert_eq!(br, (Ok(1), three, Ok(-1), three), "a browser returns -1 from memory.grow and keeps the old size");
}

#[wasm_bindgen_test]
async fn the_same_grow_succeeds_under_a_bigger_cap() {
    let out = both(async |e| {
        let mut b = budget();
        b.memory_bytes = 64 * PAGE;
        let mut inst = loaded(e, "oom", "command", &b).await;
        (invoke(inst.as_mut(), 0, 20, &b), inst.mem_len())
    })
    .await;
    assert_eq!(out, (Ok(1), 21 * PAGE as usize));
}

#[wasm_bindgen_test]
async fn a_grow_storm_stops_at_the_cap() {
    let (w, br) = pair(async |e| {
        let mut b = budget();
        b.memory_bytes = 8 * PAGE;
        let mut inst = loaded(e, "alloc-bomb", "command", &b).await;
        (invoke(inst.as_mut(), 0, 0, &b), inst.mem_len())
    })
    .await;
    assert_eq!(w, (Err(Trap::OutOfMemory), 8 * PAGE as usize));
    assert_eq!(br, (Ok(7), 8 * PAGE as usize), "the browser gains 7 pages, then grow returns -1");
}

#[wasm_bindgen_test]
async fn a_grow_storm_under_a_loose_cap_ends_at_the_modules_own_maximum() {
    let out = both(async |e| {
        let mut b = budget();
        b.memory_bytes = 1024 * MIB;
        let mut inst = loaded(e, "alloc-bomb", "command", &b).await;
        (invoke(inst.as_mut(), 0, 0, &b), inst.mem_len())
    })
    .await;
    assert_eq!(out, (Ok(63), 64 * PAGE as usize));
}

#[wasm_bindgen_test]
async fn initial_memory_over_the_cap_does_not_instantiate() {
    let out = both(async |e| {
        let mut b = budget();
        b.memory_bytes = PAGE; // the kernels start with 8 pages
        instantiate(e, &package("kernel-posterize", &[], "effect"), no_host(), &b).await.map(|_| ())
    })
    .await;
    assert_eq!(out, Err(LoadError::MemoryOverBudget));
}

#[wasm_bindgen_test]
async fn guest_faults_map_to_the_same_trap_on_both_engines() {
    let out = both(async |e| {
        let b = budget();
        let mut inst = loaded(e, "trap", "command", &b).await;
        (0..=6).map(|entry| invoke(inst.as_mut(), entry, 0, &b)).collect::<Vec<_>>()
    })
    .await;
    let fault = || Err(Trap::GuestFault(String::new()));
    assert_eq!(out, vec![Err(Trap::Unreachable), fault(), fault(), fault(), fault(), fault(), Ok(7)]);
}

#[wasm_bindgen_test]
async fn division_by_a_non_zero_value_does_not_trap() {
    let out = both(async |e| {
        let b = budget();
        let mut inst = loaded(e, "trap", "command", &b).await;
        invoke(inst.as_mut(), 1, 5, &b)
    })
    .await;
    assert_eq!(out, Ok(2));
}

#[wasm_bindgen_test]
async fn deep_recursion_overflows_the_stack_and_shallow_recursion_does_not() {
    // The depth that overflows is engine defined. 1 million frames overflows both.
    let out = both(async |e| {
        let b = budget();
        let mut inst = loaded(e, "stack", "command", &b).await;
        let shallow = invoke(inst.as_mut(), 0, 8, &b);
        let deep = invoke(inst.as_mut(), 0, 1_000_000, &b);
        let after = invoke(inst.as_mut(), 0, 8, &b);
        (shallow, deep, after)
    })
    .await;
    assert_eq!(out, (Ok(8), Err(Trap::StackOverflow), Ok(8)));
}

// ---- host calls --------------------------------------------------------------------------

#[wasm_bindgen_test]
async fn a_host_call_with_pointers_outside_memory_gets_bad_arg_and_the_guest_keeps_running() {
    let out = both(async |e| {
        let b = budget();
        let mut inst = loaded(e, "bad-pointer", "command", &b).await;
        (0..=3).map(|entry| invoke(inst.as_mut(), entry, 0, &b).map(|r| unpack(r).0)).collect::<Vec<_>>()
    })
    .await;
    assert_eq!(out, vec![Ok(Status::BadArg); 4]);
}

#[wasm_bindgen_test]
async fn a_host_call_with_valid_pointers_reaches_the_dispatcher_and_writes_guest_memory() {
    let out = both(async |e| {
        let b = budget();
        let host: HostDispatch = Arc::new(|op: Op, input: &[u8], out: &mut [u8]| -> HostResult {
            assert_eq!(op, Op::HostLog);
            assert_eq!(input, b"{}");
            out[..2].copy_from_slice(b"ok");
            Ok(2)
        });
        let mut inst = instantiate(e, &package("bad-pointer", &["host.log"], "command"), host, &b).await.unwrap();
        let r = invoke(inst.as_mut(), 4, 0, &b).map(unpack);
        let mut bytes = [0u8; 2];
        inst.read(1024, &mut bytes).unwrap();
        (r, bytes)
    })
    .await;
    assert_eq!(out, (Ok((Status::Ok, 2)), *b"ok"));
}

#[wasm_bindgen_test]
async fn a_dispatcher_that_kills_the_guest_returns_its_trap() {
    let out = both(async |e| {
        let b = budget();
        let host: HostDispatch = Arc::new(|_op: Op, _i: &[u8], _o: &mut [u8]| -> HostResult { Err(HostError::Trap(Trap::Denied(Op::HostLog))) });
        let mut inst = instantiate(e, &package("bad-pointer", &["host.log"], "command"), host, &b).await.unwrap();
        invoke(inst.as_mut(), 4, 0, &b)
    })
    .await;
    assert_eq!(out, Err(Trap::Denied(Op::HostLog)));
}

#[wasm_bindgen_test]
async fn a_too_small_answer_reports_the_required_size() {
    let out = both(async |e| {
        let b = budget();
        let host: HostDispatch = Arc::new(|_op: Op, _i: &[u8], _o: &mut [u8]| -> HostResult { Err(HostError::TooSmall { required: 500 }) });
        let mut inst = instantiate(e, &package("bad-pointer", &["host.log"], "command"), host, &b).await.unwrap();
        invoke(inst.as_mut(), 4, 0, &b).map(unpack)
    })
    .await;
    assert_eq!(out, Ok((Status::TooSmall, 500)));
}

#[wasm_bindgen_test]
async fn host_side_reads_and_writes_outside_memory_are_bad_pointer() {
    let out = both(async |e| {
        let b = budget();
        let mut inst = loaded(e, "loop", "command", &b).await;
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
    })
    .await;
    assert_eq!(out, (Ok(()), Err(Trap::BadPointer), Err(Trap::BadPointer), Ok(()), Err(Trap::BadPointer), Err(Trap::BadPointer)));
}

#[wasm_bindgen_test]
async fn with_mem_edits_guest_memory_on_both_engines() {
    let out = both(async |e| {
        let b = budget();
        let mut inst = loaded(e, "loop", "command", &b).await;
        inst.with_mem(&mut |mem| mem[100..104].copy_from_slice(b"abcd")).unwrap();
        let mut back = [0u8; 4];
        inst.read(100, &mut back).unwrap();
        back
    })
    .await;
    assert_eq!(&out, b"abcd");
}

#[wasm_bindgen_test]
async fn call_argument_and_budget_errors_match() {
    let out = both(async |e| {
        let b = budget();
        let mut inst = loaded(e, "loop", "command", &b).await;
        let arity = inst.call(Export::Invoke, &[0, 0], &b);
        let missing = inst.call(Export::StateSave, &[0], &b);
        let mut other_stack = budget();
        other_stack.stack_bytes = STACK * 2;
        let mismatch = inst.call(Export::Abi, &[], &other_stack);
        let abi = inst.call(Export::Abi, &[], &b);
        let load = instantiate(e, &package("loop", &["host.log"], "command"), no_host(), &other_stack).await.map(|_| ());
        (arity, missing, mismatch, abi, load)
    })
    .await;
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

// ---- permissions ----------------------------------------------------------------------

/// A `Services` that answers "nothing here". The probe plugin asks for ops, and the dispatcher
/// decides before it reaches these methods.
struct NullServices;

impl Services for NullServices {
    fn host_info(&self) -> HostInfo {
        HostInfo { app: "web-test".into(), version: "0".into() }
    }
    fn log(&self, _owner: &Owner, _level: LogLevel, _message: &str) {}
    fn progress(&self, _owner: &Owner, _fraction: f64, _message: Option<&str>) {}
    fn list_commands(&self) -> Vec<CommandInfo> {
        Vec::new()
    }
    fn execute_command(&self, _owner: &Owner, _id: &str, _params: &Value) -> Result<Value, CommandFailure> {
        Err(CommandFailure::Unknown)
    }
    fn doc_query(&self, _owner: &Owner, _scope: DocScope, _query: &Value) -> Result<Value, QueryFailure> {
        Err(QueryFailure::Unsupported)
    }
    fn unix_millis(&self) -> u64 {
        0
    }
    fn fill_random(&self, buf: &mut [u8]) -> Result<(), EntropyUnavailable> {
        buf.fill(0xAB);
        Ok(())
    }
    fn notify(&self, _notice: UserNotice) {}
    fn link_apps(&self, _owner: &Owner) -> Result<Vec<LinkApp>, LinkFailure> {
        Err(LinkFailure::Failed("no link in the web test".into()))
    }
    fn link_call(&self, _owner: &Owner, _app: &str, _method: &str, _params: &Value, _cancel: &AtomicBool) -> Result<Value, LinkFailure> {
        Err(LinkFailure::Failed("no link in the web test".into()))
    }
    fn lend_session(&self, _session: &mut dyn std::any::Any, run: &mut dyn FnMut()) {
        run();
    }
    fn begin_call(&self, _owner: &Owner, _contribution: &str) -> CallScope {
        CallScope(0)
    }
    fn end_call(&self, _scope: CallScope, _outcome: &CallOutcome) {}
}

async fn probe_with(e: &dyn Engine, declared: &[Op], grants: &[&str]) -> Vec<Result<Status, Trap>> {
    let names: Vec<&str> = declared.iter().map(|o| o.name()).collect();
    let ctx = PluginContext {
        owner: Owner { plugin_id: Arc::from("org.craft.conformance"), generation: 1 },
        declared_ops: declared.iter().copied().collect::<HashSet<_>>(),
        grants: grants.iter().map(|g| g.parse::<Permission>().unwrap()).collect(),
        engine: e.id(),
        random_seed: 1,
    };
    let dispatcher = HostDispatcher::new(
        ctx,
        Arc::new(NullServices),
        Arc::new(MemoryStore::new()),
        Arc::new(WebClock::new()),
        Arc::new(AtomicBool::new(false)),
        DispatchConfig::default(),
    );
    let b = budget();
    let mut inst = instantiate(e, &package("probe", &names, "command"), dispatcher.handle(), &b).await.unwrap();
    (0..20).map(|slot| invoke(inst.as_mut(), slot, 0, &b).map(|r| unpack(r).0)).collect()
}

#[wasm_bindgen_test]
async fn a_plugin_that_lists_no_ops_is_denied_every_op() {
    let out = both(async |e| probe_with(e, &[], &[]).await).await;
    let mut want = vec![Ok(Status::Denied); 19];
    want.push(Ok(Status::Unsupported)); // slot 19 is the reserved 0x00A0
    assert_eq!(out, want);
}

#[wasm_bindgen_test]
async fn a_plugin_that_lists_every_op_but_holds_no_grant_is_denied_every_privileged_op() {
    let out = both(async |e| probe_with(e, &Op::ALL, &[]).await).await;
    let status = |op: Op| out[Op::ALL.iter().position(|o| *o == op).unwrap()].clone();
    let privileged = [
        Op::CmdList,
        Op::CmdExecute,
        Op::DocQuery,
        Op::DocApply,
        Op::EventPoll,
        Op::StateGet,
        Op::StatePut,
        Op::FsOpenPicked,
        Op::FsRead,
        Op::FsWrite,
        Op::NetFetch,
    ];
    for op in privileged {
        assert_eq!(status(op), Ok(Status::Denied), "{} must be denied without a grant", op.name());
    }
    for op in [Op::HostInfo, Op::HostCancelled, Op::TimeNow] {
        assert_eq!(status(op), Ok(Status::Ok), "{} needs no grant", op.name());
    }
    assert_eq!(status(Op::RandomBytes), Ok(Status::BadArg));
    assert_eq!(out[19], Ok(Status::Unsupported));
}

#[wasm_bindgen_test]
async fn a_grant_turns_the_same_denial_into_an_answer() {
    let out = both(async |e| probe_with(e, &Op::ALL, &["state.local"]).await).await;
    let at = |op: Op| out[Op::ALL.iter().position(|o| *o == op).unwrap()].clone();
    assert_eq!(at(Op::StateGet), Ok(Status::BadArg));
    assert_eq!(at(Op::StatePut), Ok(Status::BadArg));
    assert_eq!(at(Op::CmdList), Ok(Status::Denied));
}

// ---- kernels: wasmi and the page thread engine ------------------------------------------

fn desc_for(img: &Image) -> ProcessDescriptor {
    ProcessDescriptor {
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
        in_ptr: 0,
        out_ptr: 0,
        params_ptr: 0,
        params_len: 0,
    }
}

/// `craft_process` the way the host does it: allocate in the guest, copy in, call, copy out.
async fn process(e: &dyn Engine, plugin: &str, img: &Image, params: &[u8], b: &Budget) -> Result<(i64, Vec<u8>), Trap> {
    let mut inst = instantiate(e, &package(plugin, &[], "effect"), no_host(), b).await.unwrap_or_else(|err| panic!("{plugin}: {err}"));
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
    let desc = ProcessDescriptor { in_ptr, out_ptr, params_ptr, params_len: params.len() as u32, ..desc_for(img) };
    inst.write(desc_ptr, &desc.encode())?;
    let code = inst.call(Export::Process, &[desc_ptr as i32, 96], b)?;
    let mut out = vec![0u8; len];
    inst.read(out_ptr, &mut out)?;
    Ok((code, out))
}

fn kernel_budget() -> Budget {
    Budget::new(96 * MIB, STACK)
}

#[wasm_bindgen_test]
async fn posterize_matches_the_reference_on_both_engines() {
    for (w, h, pad, levels) in [(67u32, 31u32, 12u32, 4u32), (1, 1, 0, 2), (16, 9, 0, 256), (33, 5, 4, 7)] {
        let img = noise_image(w, h, pad, 0x9E37_79B9 + u64::from(w));
        let out = both(async |e| process(e, "kernel-posterize", &img, &levels.to_le_bytes(), &kernel_budget()).await).await;
        assert_eq!(out, Ok((0, posterize_reference(&img, levels))), "{w}x{h} pad {pad} levels {levels}");
    }
}

#[wasm_bindgen_test]
async fn blur_matches_the_reference_on_both_engines() {
    for (w, h, pad) in [(67u32, 31u32, 12u32), (1, 1, 0), (3, 2, 0), (5, 40, 8), (13, 13, 0)] {
        let img = noise_image(w, h, pad, 0xC0FF_EE00 + u64::from(h));
        let out = both(async |e| process(e, "kernel-blur", &img, &[], &kernel_budget()).await).await;
        assert_eq!(out, Ok((0, blur_reference(&img))), "{w}x{h} pad {pad}");
    }
}

#[wasm_bindgen_test]
async fn a_kernel_whose_buffers_do_not_fit_the_memory_cap_fails_with_out_of_memory() {
    let img = noise_image(512, 512, 0, 3);
    let out = both(async |e| process(e, "kernel-blur", &img, &[], &Budget::new(2 * MIB, STACK)).await).await;
    assert_eq!(out, Err(Trap::OutOfMemory));
}

#[wasm_bindgen_test]
async fn a_kernel_given_an_unsupported_layout_returns_its_error_code_on_both_engines() {
    let out = both(async |e| {
        let b = kernel_budget();
        let mut inst = instantiate(e, &package("kernel-posterize", &[], "effect"), no_host(), &b).await.unwrap();
        let desc = ProcessDescriptor {
            stride_bytes: 3,
            channels: 3,
            alpha: Alpha::None,
            w: 1,
            h: 1,
            in_ptr: 8192,
            out_ptr: 8200,
            params_ptr: 8208,
            params_len: 4,
            ..desc_for(&noise_image(1, 1, 0, 1))
        };
        let p = inst.call(Export::Alloc, &[96, 4], &b).unwrap() as u32;
        inst.write(p, &desc.encode()).unwrap();
        inst.call(Export::Process, &[p as i32, 96], &b)
    })
    .await;
    assert_eq!(out, Ok(3));
}

#[wasm_bindgen_test]
async fn wasmi_on_web_stops_a_slow_kernel_mid_image_and_a_fast_one_finishes() {
    let img = noise_image(256, 256, 0, 99);
    let e = wasmi();
    let slow = [4u32.to_le_bytes(), 4000u32.to_le_bytes()].concat();
    let mut b = kernel_budget();
    b.deadline = Some(Duration::from_millis(150));
    let t0 = Instant::now();
    let r = process(&e, "slow-posterize", &img, &slow, &b).await;
    assert!(t0.elapsed() < Duration::from_secs(2), "did not stop in time: {:?}", t0.elapsed());
    assert_eq!(r, Err(Trap::Deadline));
    let fast = [4u32.to_le_bytes(), 1u32.to_le_bytes()].concat();
    b.deadline = Some(Duration::from_secs(20));
    assert_eq!(process(&e, "slow-posterize", &img, &fast, &b).await, Ok((0, posterize_reference(&img, 4))));
}

// ---- the Worker engine ------------------------------------------------------------------

async fn spawn(plugin: &str, kind: &str, b: &Budget) -> craft_plugin_host::engine::browser::worker::WorkerInstance {
    let bytes = package(plugin, &["host.log"], kind);
    let report = gate(&bytes);
    assert!(report.is_accepted(), "{:#?}", report.violations);
    let untrusted = CompileOptions { trust: Trust::Untrusted, stack_bytes: STACK };
    let module = BrowserWorkerEngine::new().compile_worker(&bytes, &report, untrusted).await.unwrap();
    module.spawn(b).await.unwrap_or_else(|e| panic!("{plugin}: {e}"))
}

fn invoke_args(entry: i32, arg: i32) -> [i32; 5] {
    [entry, arg, 0, 0, 0]
}

#[wasm_bindgen_test]
async fn the_worker_engine_has_no_synchronous_instance() {
    let bytes = package("loop", &["host.log"], "command");
    let report = gate(&bytes);
    let untrusted = CompileOptions { trust: Trust::Untrusted, stack_bytes: STACK };
    let module = BrowserWorkerEngine::new().compile(&bytes, &report, untrusted).await.unwrap();
    let b = budget();
    let err = module.instantiate(no_host(), &b).await.map(|_| ()).unwrap_err();
    assert!(matches!(err, LoadError::Instantiate(ref m) if m.contains("spawn")), "{err:?}");
}

#[wasm_bindgen_test]
async fn a_worker_runs_a_finite_loop_and_kills_an_infinite_one_at_the_deadline() {
    let b = budget();
    let mut w = spawn("loop", "command", &b).await;
    // Control: the same worker finishes a finite loop under the same deadline.
    let mut timed = budget();
    timed.deadline = Some(Duration::from_millis(100));
    assert_eq!(w.call(Export::Invoke, &invoke_args(1, 1000), &timed).await, Ok(1000));
    assert!(!w.is_terminated());
    let t0 = Instant::now();
    let r = w.call(Export::Invoke, &invoke_args(0, 0), &timed).await;
    let took = t0.elapsed();
    assert_eq!(r, Err(Trap::Deadline));
    assert!(took >= Duration::from_millis(100) && took < Duration::from_secs(2), "stopped after {took:?}");
    assert!(w.is_terminated(), "the deadline must kill the worker");
    assert!(matches!(w.call(Export::Abi, &[], &timed).await, Err(Trap::Other(_))), "a terminated worker answers nothing");
}

#[wasm_bindgen_test]
async fn a_cancel_set_while_a_worker_runs_ends_the_call() {
    let b = budget();
    let mut w = spawn("loop", "command", &b).await;
    let flag = Arc::clone(&b.cancel);
    wasm_bindgen_futures::spawn_local(async move {
        sleep(50).await;
        flag.store(true, Ordering::Relaxed);
    });
    let t0 = Instant::now();
    let r = w.call(Export::Invoke, &invoke_args(0, 0), &b).await;
    assert_eq!(r, Err(Trap::Cancelled));
    assert!(t0.elapsed() < Duration::from_secs(2), "ignored the cancel flag for {:?}", t0.elapsed());
    assert!(w.is_terminated());
}

#[wasm_bindgen_test]
async fn a_start_function_that_never_returns_is_killed_at_spawn() {
    // The gate rejects start functions. This test hands the engine a report that accepts the package
    // anyway, because the spawn deadline is the second line of defense.
    let bytes = package("start-loop", &["host.log"], "command");
    assert_eq!(gate(&bytes).violations, vec![craft_plugin_abi::Violation::StartFunction], "the gate must reject it first");
    let report = PolicyReport { violations: vec![], legacy: None, manifest: None };
    let untrusted = CompileOptions { trust: Trust::Untrusted, stack_bytes: STACK };
    let module = BrowserWorkerEngine::new().compile_worker(&bytes, &report, untrusted).await.unwrap();
    let mut b = budget();
    b.deadline = Some(Duration::from_millis(300));
    let t0 = Instant::now();
    let r = module.spawn(&b).await.map(|_| ());
    assert!(t0.elapsed() < Duration::from_secs(3), "spawn hung for {:?}", t0.elapsed());
    assert!(matches!(r, Err(LoadError::Instantiate(_))), "{r:?}");
}

#[wasm_bindgen_test]
async fn a_worker_maps_traps_like_wasmi() {
    let b = budget();
    let mut w = spawn("trap", "command", &b).await;
    let mut got = Vec::new();
    for entry in 0..=6 {
        got.push(norm(w.call(Export::Invoke, &invoke_args(entry, 0), &b).await));
    }
    let fault = || Err(Trap::GuestFault(String::new()));
    assert_eq!(got, vec![Err(Trap::Unreachable), fault(), fault(), fault(), fault(), fault(), Ok(7)]);
    assert!(!w.is_terminated(), "a guest trap leaves the worker usable");
    assert_eq!(w.call(Export::Invoke, &invoke_args(6, 0), &b).await, Ok(7));
}

#[wasm_bindgen_test]
async fn a_worker_overflows_the_stack_like_wasmi() {
    let b = budget();
    let mut w = spawn("stack", "command", &b).await;
    let shallow = w.call(Export::Invoke, &invoke_args(0, 8), &b).await;
    let deep = w.call(Export::Invoke, &invoke_args(0, 1_000_000), &b).await;
    let after = w.call(Export::Invoke, &invoke_args(0, 8), &b).await;
    assert_eq!((shallow, deep, after), (Ok(8), Err(Trap::StackOverflow), Ok(8)));
}

#[wasm_bindgen_test]
async fn a_worker_caps_memory_like_the_page_thread_engine() {
    let mut b = budget();
    b.memory_bytes = 4 * PAGE;
    let mut w = spawn("oom", "command", &b).await;
    assert_eq!(w.call(Export::Invoke, &invoke_args(0, 2), &b).await, Ok(1));
    assert_eq!(w.mem_len(), 3 * PAGE as usize);
    assert_eq!(w.call(Export::Invoke, &invoke_args(0, 2), &b).await, Ok(-1));
    assert_eq!(w.mem_len(), 3 * PAGE as usize);
    // A module whose initial memory is over the cap does not spawn.
    let bytes = package("kernel-posterize", &[], "effect");
    let report = gate(&bytes);
    let untrusted = CompileOptions { trust: Trust::Untrusted, stack_bytes: STACK };
    let module = BrowserWorkerEngine::new().compile_worker(&bytes, &report, untrusted).await.unwrap();
    let mut small = budget();
    small.memory_bytes = PAGE;
    assert!(matches!(module.spawn(&small).await, Err(LoadError::MemoryOverBudget)));
}

#[wasm_bindgen_test]
async fn a_worker_has_no_host_so_every_op_is_unsupported() {
    // The probe asks for ops 0 to 18. A Worker cannot reach the document, so each answer is E_UNSUPPORTED,
    // not E_DENIED: the plugin never reaches a permission check. Control plane plugins stay on wasmi.
    let b = budget();
    let mut w = spawn("probe", "command", &b).await;
    let mut got = Vec::new();
    for slot in 0..20 {
        got.push(w.call(Export::Invoke, &invoke_args(slot, 0), &b).await.map(|r| unpack(r).0));
    }
    assert_eq!(got, vec![Ok(Status::Unsupported); 20]);
}

#[wasm_bindgen_test]
async fn a_worker_runs_the_kernels_to_the_same_bytes_as_wasmi() {
    for (w, h, pad, levels) in [(67u32, 31u32, 12u32, 4u32), (1, 1, 0, 2), (33, 5, 4, 7)] {
        let img = noise_image(w, h, pad, 0x9E37_79B9 + u64::from(w));
        let mut worker = spawn("kernel-posterize", "effect", &kernel_budget()).await;
        let got = worker.process(&desc_for(&img), &img.pixels, &levels.to_le_bytes(), img.pixels.len(), &kernel_budget()).await;
        let via_wasmi = process(&wasmi(), "kernel-posterize", &img, &levels.to_le_bytes(), &kernel_budget()).await;
        assert_eq!(got, via_wasmi, "{w}x{h} levels {levels}");
        assert_eq!(got, Ok((0, posterize_reference(&img, levels))));
    }
    for (w, h, pad) in [(67u32, 31u32, 12u32), (3, 2, 0), (13, 13, 0)] {
        let img = noise_image(w, h, pad, 0xC0FF_EE00 + u64::from(h));
        let mut worker = spawn("kernel-blur", "effect", &kernel_budget()).await;
        let got = worker.process(&desc_for(&img), &img.pixels, &[], img.pixels.len(), &kernel_budget()).await;
        assert_eq!(got, Ok((0, blur_reference(&img))), "{w}x{h} pad {pad}");
    }
}

#[wasm_bindgen_test]
async fn a_worker_stops_a_slow_kernel_mid_image_and_a_fast_one_finishes() {
    let img = noise_image(256, 256, 0, 99);
    let slow = [4u32.to_le_bytes(), 4000u32.to_le_bytes()].concat();
    let mut worker = spawn("slow-posterize", "effect", &kernel_budget()).await;
    let mut b = kernel_budget();
    b.deadline = Some(Duration::from_millis(150));
    let t0 = Instant::now();
    let r = worker.process(&desc_for(&img), &img.pixels, &slow, img.pixels.len(), &b).await;
    assert!(t0.elapsed() < Duration::from_secs(2), "did not stop in time: {:?}", t0.elapsed());
    assert_eq!(r, Err(Trap::Deadline));
    assert!(worker.is_terminated());
    // Control: one spin per pixel finishes inside a long deadline and matches posterize.
    let fast = [4u32.to_le_bytes(), 1u32.to_le_bytes()].concat();
    let mut worker = spawn("slow-posterize", "effect", &kernel_budget()).await;
    b.deadline = Some(Duration::from_secs(20));
    let r = worker.process(&desc_for(&img), &img.pixels, &fast, img.pixels.len(), &b).await;
    assert_eq!(r, Ok((0, posterize_reference(&img, 4))));
}

#[wasm_bindgen_test]
async fn a_worker_returns_the_kernel_error_code_for_an_unsupported_layout() {
    let img = noise_image(1, 1, 0, 1);
    let desc = ProcessDescriptor { stride_bytes: 3, channels: 3, alpha: Alpha::None, ..desc_for(&img) };
    let mut worker = spawn("kernel-posterize", "effect", &kernel_budget()).await;
    let r = worker.process(&desc, &[0; 3], &4u32.to_le_bytes(), 3, &kernel_budget()).await;
    assert_eq!(r.map(|(code, _)| code), Ok(3));
}

#[wasm_bindgen_test]
async fn worker_reads_and_writes_outside_memory_are_bad_pointer() {
    let b = budget();
    let mut w = spawn("loop", "command", &b).await;
    let len = w.mem_len() as u32;
    assert_eq!(w.write(len - 1, &[1], &b).await, Ok(()));
    assert_eq!(w.write(len - 1, &[1, 2], &b).await, Err(Trap::BadPointer));
    assert_eq!(w.read(len, 1, &b).await, Err(Trap::BadPointer));
    assert_eq!(w.read(len - 1, 1, &b).await, Ok(vec![1]));
}

#[wasm_bindgen_test]
async fn worker_call_argument_errors_match() {
    let b = budget();
    let mut w = spawn("loop", "command", &b).await;
    let mut other_stack = budget();
    other_stack.stack_bytes = STACK * 2;
    assert_eq!(w.call(Export::Invoke, &[0, 0], &b).await, Err(Trap::BadArity { export: Export::Invoke, expected: 5, got: 2 }));
    assert_eq!(w.call(Export::StateSave, &[0], &b).await, Err(Trap::NoSuchExport(Export::StateSave)));
    assert_eq!(w.call(Export::Abi, &[], &other_stack).await, Err(Trap::BudgetMismatch));
    assert_eq!(w.call(Export::Abi, &[], &b).await, Ok(1));
}
