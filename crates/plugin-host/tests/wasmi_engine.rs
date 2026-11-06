//! wasmi engine tests. Each test runs real WAT through the gate and the engine. No test passes
//! when the guarded behavior is removed: the deadline, memory, stack, cancel, and pointer tests
//! each have a control in the same file that shows the budget is the cause.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use common::*;
use craft_plugin_abi::ops::unpack_call_return;
use craft_plugin_abi::{HostPolicy, Op, Status, Violation};
use craft_plugin_host::engine::wasmi::{DEFAULT_SLICE_FUEL, WasmiEngine};
use craft_plugin_host::engine::{
    Budget, Caps, CompileOptions, Engine, EngineId, Export, HostDispatch, HostError, HostResult, Instance, LoadError, Module, Trap, Trust,
    load_package,
};

fn engine_with_step(step: Duration, slice: u64) -> (WasmiEngine, Arc<StepClock>) {
    let clock = StepClock::new(step);
    (WasmiEngine::new(clock.clone()).with_slice_fuel(slice), clock)
}

// ---- deadline, work, cancel -------------------------------------------------------------

#[test]
fn infinite_loop_stops_at_the_deadline_on_a_fake_clock() {
    // Every clock read moves time 1 ms. Deadline 5 ms. Slice 10k fuel.
    let (engine, clock) = engine_with_step(Duration::from_millis(1), 10_000);
    let mut b = budget();
    b.deadline = Some(Duration::from_millis(5));
    let mut inst = instance(&engine, "spin", no_host(), &b);
    let err = inst.call(Export::Invoke, &[0, 0, 0, 0, 0], &b).unwrap_err();
    assert_eq!(err, Trap::Deadline);
    let stats = inst.last_call_stats();
    // The start read, the pre-check read, and one read per slice boundary. Time hits 5 ms on read 6.
    assert_eq!(clock.reads(), 6, "clock reads");
    assert_eq!(stats.slices, 4, "slices before the deadline");
    assert_eq!(stats.fuel_used, 4 * 10_000, "fuel burned is exactly the slices handed out");
}

#[test]
fn without_a_deadline_the_same_loop_runs_until_the_work_cap() {
    // Control for the test above: the clock moves the same way, but no deadline is set.
    let (engine, _clock) = engine_with_step(Duration::from_millis(1), 10_000);
    let mut b = budget();
    b.work = Some(100_000);
    let mut inst = instance(&engine, "spin", no_host(), &b);
    let err = inst.call(Export::Invoke, &[0, 0, 0, 0, 0], &b).unwrap_err();
    assert_eq!(err, Trap::OutOfWork);
    assert_eq!(inst.last_call_stats().slices, 10);
    assert_eq!(inst.last_call_stats().fuel_used, 100_000);
}

#[test]
fn infinite_loop_stops_at_the_deadline_on_the_real_clock() {
    // Overshoot is at most one slice. A debug build of wasmi burns about 15M fuel per second, or
    // 130 ms per 1M fuel with indirect-dispatch, so the slice is small to keep the bound honest.
    let engine = WasmiEngine::system().with_slice_fuel(20_000);
    let mut b = budget();
    b.deadline = Some(Duration::from_millis(100));
    let mut inst = instance(&engine, "spin", no_host(), &b);
    let t0 = Instant::now();
    let err = inst.call(Export::Invoke, &[0, 0, 0, 0, 0], &b).unwrap_err();
    let took = t0.elapsed();
    assert_eq!(err, Trap::Deadline);
    assert!(took >= Duration::from_millis(100), "stopped early: {took:?}");
    // The late bound is loose on purpose: a loaded CI machine can stall a thread for a long time.
    // The tests above count slices on a step clock, so slicing itself does not depend on this one.
    assert!(took < Duration::from_secs(2), "stopped late: {took:?}");
}

#[test]
fn cancel_flag_stops_a_running_guest_between_slices() {
    // The guest polls host.cancelled in a loop. The dispatcher sets the flag on its fifth call.
    // Slices are tiny, so a slice boundary comes soon after.
    let (engine, _clock) = engine_with_step(Duration::ZERO, 200);
    let b = budget();
    let cancel = b.cancel.clone();
    let calls = Arc::new(AtomicU32::new(0));
    let seen = calls.clone();
    let host: HostDispatch = Arc::new(move |op: Op, _i: &[u8], _o: &mut [u8]| -> HostResult {
        assert_eq!(op, Op::HostCancelled);
        if seen.fetch_add(1, Ordering::Relaxed) == 4 {
            cancel.store(true, Ordering::Relaxed);
        }
        Ok(0)
    });
    let mut inst = instance(&engine, "spin_host", host, &b);
    let err = inst.call(Export::Invoke, &[0, 0, 0, 0, 0], &b).unwrap_err();
    assert_eq!(err, Trap::Cancelled);
    assert!(calls.load(Ordering::Relaxed) >= 5);
    // The guest ran a whole slice, and the flag stopped it at that slice's end.
    assert_eq!(inst.last_call_stats().slices, 1);
    assert_eq!(inst.last_call_stats().fuel_used, 200);
}

#[test]
fn a_cancel_set_before_the_call_stops_it_before_any_guest_code_runs() {
    let (engine, _clock) = engine_with_step(Duration::ZERO, 1000);
    let b = budget();
    b.cancel.store(true, Ordering::Relaxed);
    let mut inst = instance(&engine, "spin", no_host(), &b);
    assert_eq!(inst.call(Export::Invoke, &[0; 5], &b).unwrap_err(), Trap::Cancelled);
    assert_eq!(inst.last_call_stats().slices, 0);
}

// ---- memory, stack, unreachable --------------------------------------------------------

#[test]
fn memory_grow_past_the_cap_traps_out_of_memory() {
    let (engine, _) = engine_with_step(Duration::ZERO, DEFAULT_SLICE_FUEL);
    let mut b = budget();
    b.memory_bytes = 4 * PAGE; // initial 1 page, cap 4 pages
    let mut inst = instance(&engine, "grow", no_host(), &b);
    // Growing by 2 pages lands on 3 pages: inside the cap, returns the old size.
    assert_eq!(inst.call(Export::Invoke, &[2, 0, 0, 0, 0], &b).unwrap(), 1);
    assert_eq!(inst.mem_len(), 3 * PAGE as usize);
    // Growing by 2 more lands on 5 pages: past the cap.
    assert_eq!(inst.call(Export::Invoke, &[2, 0, 0, 0, 0], &b).unwrap_err(), Trap::OutOfMemory);
    assert_eq!(inst.mem_len(), 3 * PAGE as usize, "the failed grow changed nothing");
}

#[test]
fn growth_past_the_modules_own_maximum_returns_minus_one_like_the_spec() {
    // Declared max is 16 pages. The cap is far higher, so only the module's own max applies.
    let (engine, _) = engine_with_step(Duration::ZERO, DEFAULT_SLICE_FUEL);
    let mut b = budget();
    b.memory_bytes = 64 * PAGE;
    let mut inst = instance(&engine, "grow", no_host(), &b);
    assert_eq!(inst.call(Export::Invoke, &[20, 0, 0, 0, 0], &b).unwrap(), -1);
}

#[test]
fn initial_memory_over_the_cap_does_not_instantiate() {
    let (engine, _) = engine_with_step(Duration::ZERO, DEFAULT_SLICE_FUEL);
    let bytes = package("grow");
    let report = gate(&bytes);
    let module = engine.compile_sync(&bytes, &report, OPTS).unwrap();
    let mut b = budget();
    b.memory_bytes = PAGE / 2; // the module starts at one page
    assert_eq!(module.instantiate_sync(no_host(), &b).err(), Some(LoadError::MemoryOverBudget));
    b.memory_bytes = PAGE; // exactly the initial size: allowed
    assert!(module.instantiate_sync(no_host(), &b).is_ok());
}

#[test]
fn deep_recursion_traps_stack_overflow() {
    let (engine, _) = engine_with_step(Duration::ZERO, DEFAULT_SLICE_FUEL);
    let b = budget();
    let mut inst = instance(&engine, "recurse", no_host(), &b);
    // Control: a shallow recursion works and returns its depth.
    assert_eq!(inst.call(Export::Invoke, &[10, 0, 0, 0, 0], &b).unwrap(), 10);
    assert_eq!(inst.call(Export::Invoke, &[100_000_000, 0, 0, 0, 0], &b).unwrap_err(), Trap::StackOverflow);
    // The instance still works after the overflow.
    assert_eq!(inst.call(Export::Invoke, &[10, 0, 0, 0, 0], &b).unwrap(), 10);
}

#[test]
fn a_bigger_stack_budget_allows_deeper_recursion() {
    // The stack budget is the cause: the same depth passes at 1 MiB and fails at 64 KiB.
    let depth = 5000;
    let (engine, _) = engine_with_step(Duration::ZERO, DEFAULT_SLICE_FUEL);
    let bytes = package("recurse");
    let report = gate(&bytes);
    let run = |stack: u32| {
        let opts = CompileOptions { trust: Trust::Untrusted, stack_bytes: stack };
        let module = engine.compile_sync(&bytes, &report, opts).unwrap();
        let b = Budget::new(16 * PAGE, stack);
        let mut inst = module.instantiate_sync(no_host(), &b).unwrap();
        inst.call(Export::Invoke, &[depth, 0, 0, 0, 0], &b)
    };
    assert_eq!(run(STACK).unwrap_err(), Trap::StackOverflow);
    assert_eq!(run(1024 * 1024).unwrap(), i64::from(depth));
}

#[test]
fn unreachable_traps_unreachable() {
    let (engine, _) = engine_with_step(Duration::ZERO, DEFAULT_SLICE_FUEL);
    let b = budget();
    let mut inst = instance(&engine, "unreachable", no_host(), &b);
    assert_eq!(inst.call(Export::Invoke, &[0; 5], &b).unwrap_err(), Trap::Unreachable);
}

// ---- call plumbing ---------------------------------------------------------------------

#[test]
fn bad_calls_are_typed_errors() {
    let (engine, _) = engine_with_step(Duration::ZERO, DEFAULT_SLICE_FUEL);
    let b = budget();
    let mut inst = instance(&engine, "spin", no_host(), &b);
    assert_eq!(inst.call(Export::Abi, &[], &b).unwrap(), 1);
    assert_eq!(inst.call(Export::Abi, &[1], &b).unwrap_err(), Trap::BadArity { export: Export::Abi, expected: 0, got: 1 });
    assert_eq!(inst.call(Export::Process, &[0, 0], &b).unwrap_err(), Trap::NoSuchExport(Export::Process));
    let mut other = b.clone();
    other.stack_bytes += 1;
    assert_eq!(inst.call(Export::Abi, &[], &other).unwrap_err(), Trap::BudgetMismatch);
}

#[test]
fn alloc_and_free_round_trip() {
    let (engine, _) = engine_with_step(Duration::ZERO, DEFAULT_SLICE_FUEL);
    let b = budget();
    let mut inst = instance(&engine, "alloc", no_host(), &b);
    let p1 = inst.call(Export::Alloc, &[100, 16], &b).unwrap();
    let p2 = inst.call(Export::Alloc, &[100, 16], &b).unwrap();
    assert!(p1 >= 1024 && p1 % 16 == 0, "p1 = {p1}");
    assert!(p2 >= p1 + 100 && p2 % 16 == 0, "p2 = {p2}");
    // The host writes into the block and reads it back from guest memory.
    inst.write(p1 as u32, b"payload").unwrap();
    let mut back = [0u8; 7];
    inst.read(p1 as u32, &mut back).unwrap();
    assert_eq!(&back, b"payload");
    // free(p1, 100, 16) reaches the guest with exactly those arguments.
    assert_eq!(inst.call(Export::Free, &[p1 as i32, 100, 16], &b).unwrap(), 0);
    let freed = inst.call(Export::Invoke, &[0; 5], &b).unwrap();
    assert_eq!(freed, (100i64 << 32) | p1);
    // A block larger than memory comes back as 0.
    assert_eq!(inst.call(Export::Alloc, &[i32::MAX, 8], &b).unwrap(), 0);
}

#[test]
fn with_mem_edits_guest_memory_in_place() {
    let (engine, _) = engine_with_step(Duration::ZERO, DEFAULT_SLICE_FUEL);
    let b = budget();
    let mut inst = instance(&engine, "alloc", no_host(), &b);
    let mut len = 0;
    inst.with_mem(&mut |mem| {
        len = mem.len();
        mem[512..516].copy_from_slice(b"abcd");
    })
    .unwrap();
    assert_eq!(len, inst.mem_len());
    let mut back = [0u8; 4];
    inst.read(512, &mut back).unwrap();
    assert_eq!(&back, b"abcd");
}

#[test]
fn host_helpers_reject_out_of_bounds_pointers() {
    let (engine, _) = engine_with_step(Duration::ZERO, DEFAULT_SLICE_FUEL);
    let b = budget();
    let mut inst = instance(&engine, "alloc", no_host(), &b);
    let end = inst.mem_len() as u32;
    // Exactly at the end is fine for an empty range and a full-length range.
    inst.write(end, &[]).unwrap();
    inst.write(end - 4, &[1, 2, 3, 4]).unwrap();
    // One byte over is not.
    assert_eq!(inst.write(end - 3, &[1, 2, 3, 4]).unwrap_err(), Trap::BadPointer);
    assert_eq!(inst.write(end + 1, &[]).unwrap_err(), Trap::BadPointer);
    assert_eq!(inst.write(u32::MAX, &[1, 2]).unwrap_err(), Trap::BadPointer);
    let mut dst = [0u8; 8];
    assert_eq!(inst.read(end - 7, &mut dst).unwrap_err(), Trap::BadPointer);
    assert_eq!(inst.read(u32::MAX, &mut dst).unwrap_err(), Trap::BadPointer);
    // A failed write must not have written a prefix.
    let mut tail = [9u8; 4];
    inst.read(end - 4, &mut tail).unwrap();
    assert_eq!(tail, [1, 2, 3, 4]);
    inst.write(end - 4, &[0; 4]).unwrap();
    assert_eq!(inst.write(end - 2, &[7, 7, 7, 7]).unwrap_err(), Trap::BadPointer);
    inst.read(end - 4, &mut tail).unwrap();
    assert_eq!(tail, [0; 4], "the failed write left memory alone");
}

// ---- host call -------------------------------------------------------------------------

/// Uppercases the request into the output buffer. Reports the size needed when it does not fit.
fn upper_host() -> HostDispatch {
    Arc::new(|op: Op, inp: &[u8], out: &mut [u8]| -> HostResult {
        match op {
            Op::HostLog => {
                if out.len() < inp.len() {
                    return Err(HostError::TooSmall { required: inp.len() });
                }
                for (o, i) in out.iter_mut().zip(inp) {
                    *o = i.to_ascii_uppercase();
                }
                Ok(inp.len())
            }
            Op::NetFetch => Err(HostError::Trap(Trap::Denied(Op::NetFetch))),
            Op::CmdExecute => Err(HostError::Status(Status::Denied)),
            Op::HostInfo => Ok(out.len() + 1), // a lie: more than the buffer
            _ => Err(HostError::Status(Status::Unsupported)),
        }
    })
}

fn host_call(inst: &mut impl Instance, b: &Budget, op: u32, ip: u32, il: u32, op_: u32, oc: u32) -> Result<(i32, u32), Trap> {
    let r = inst.call(Export::Invoke, &[op as i32, ip as i32, il as i32, op_ as i32, oc as i32], b)?;
    Ok(unpack_call_return(r))
}

#[test]
fn host_call_round_trip_writes_into_the_guest_supplied_buffer() {
    let (engine, _) = engine_with_step(Duration::ZERO, DEFAULT_SLICE_FUEL);
    let b = budget();
    let mut inst = instance(&engine, "host", upper_host(), &b);
    inst.write(2048, b"hello").unwrap();
    let (status, n) = host_call(&mut inst, &b, Op::HostLog.number(), 2048, 5, 4096, 64).unwrap();
    assert_eq!((status, n), (0, 5));
    let mut out = [0u8; 5];
    inst.read(4096, &mut out).unwrap();
    assert_eq!(&out, b"HELLO");
}

#[test]
fn host_call_too_small_reports_the_required_size_and_writes_nothing() {
    let (engine, _) = engine_with_step(Duration::ZERO, DEFAULT_SLICE_FUEL);
    let b = budget();
    let mut inst = instance(&engine, "host", upper_host(), &b);
    inst.write(2048, b"hello").unwrap();
    inst.write(4096, b"zzzzz").unwrap();
    let (status, n) = host_call(&mut inst, &b, Op::HostLog.number(), 2048, 5, 4096, 3).unwrap();
    assert_eq!(status, Status::TooSmall.code());
    assert_eq!(n, 5, "the count is the size the guest must supply");
    let mut out = [0u8; 5];
    inst.read(4096, &mut out).unwrap();
    assert_eq!(&out, b"zzzzz");
    // Retrying with the reported size succeeds.
    let (status, n) = host_call(&mut inst, &b, Op::HostLog.number(), 2048, 5, 4096, n).unwrap();
    assert_eq!((status, n), (0, 5));
}

#[test]
fn host_call_handles_overlapping_request_and_output() {
    let (engine, _) = engine_with_step(Duration::ZERO, DEFAULT_SLICE_FUEL);
    let b = budget();
    let mut inst = instance(&engine, "host", upper_host(), &b);
    inst.write(2048, b"hello").unwrap();
    // Output on top of the request: the engine copies the request first.
    let (status, n) = host_call(&mut inst, &b, Op::HostLog.number(), 2048, 5, 2048, 5).unwrap();
    assert_eq!((status, n), (0, 5));
    let mut out = [0u8; 5];
    inst.read(2048, &mut out).unwrap();
    assert_eq!(&out, b"HELLO");
    // Output below the request.
    inst.write(3000, b"abc").unwrap();
    let (status, n) = host_call(&mut inst, &b, Op::HostLog.number(), 3000, 3, 2000, 8).unwrap();
    assert_eq!((status, n), (0, 3));
    let mut out = [0u8; 3];
    inst.read(2000, &mut out).unwrap();
    assert_eq!(&out, b"ABC");
}

#[test]
fn host_call_status_codes() {
    let (engine, _) = engine_with_step(Duration::ZERO, DEFAULT_SLICE_FUEL);
    let b = budget();
    let mut inst = instance(&engine, "host", upper_host(), &b);
    let mem = inst.mem_len() as u32;
    // An op number this host does not know.
    assert_eq!(host_call(&mut inst, &b, 0x7777, 0, 0, 0, 0).unwrap(), (Status::Unsupported.code(), 0));
    // A known op the dispatcher refuses with a status.
    assert_eq!(host_call(&mut inst, &b, Op::CmdExecute.number(), 0, 0, 0, 0).unwrap(), (Status::Denied.code(), 0));
    // A request range outside memory, then an output range outside memory.
    assert_eq!(host_call(&mut inst, &b, Op::HostLog.number(), mem - 2, 4, 0, 8).unwrap(), (Status::BadArg.code(), 0));
    assert_eq!(host_call(&mut inst, &b, Op::HostLog.number(), 0, 0, mem - 2, 8).unwrap(), (Status::BadArg.code(), 0));
    // Negative i32 values read as huge unsigned lengths, not as small ones.
    assert_eq!(host_call(&mut inst, &b, Op::HostLog.number(), 0, u32::MAX, 0, 0).unwrap(), (Status::BadArg.code(), 0));
}

#[test]
fn host_dispatcher_can_trap_the_guest_and_cannot_overrun_the_buffer() {
    let (engine, _) = engine_with_step(Duration::ZERO, DEFAULT_SLICE_FUEL);
    let b = budget();
    let mut inst = instance(&engine, "host", upper_host(), &b);
    let err = host_call(&mut inst, &b, Op::NetFetch.number(), 0, 0, 0, 0).unwrap_err();
    assert_eq!(err, Trap::Denied(Op::NetFetch));
    let err = host_call(&mut inst, &b, Op::HostInfo.number(), 0, 0, 4096, 8).unwrap_err();
    assert!(matches!(err, Trap::Other(ref m) if m.contains("9 bytes into a 8 byte buffer")), "{err:?}");
    // The instance is usable after either trap.
    inst.write(2048, b"ok").unwrap();
    assert_eq!(host_call(&mut inst, &b, Op::HostLog.number(), 2048, 2, 4096, 8).unwrap(), (0, 2));
}

// ---- loader and gate -------------------------------------------------------------------

#[test]
fn a_package_that_passes_the_gate_loads_and_runs() {
    let engine = WasmiEngine::system();
    let bytes = package("alloc");
    let (module, report) = block_on(load_package(&engine, &bytes, &HostPolicy::native(), OPTS)).unwrap();
    assert!(report.is_accepted());
    assert_eq!(report.manifest.as_ref().map(|m| m.id.as_str()), Some("org.example.t"));
    let b = budget();
    let mut inst = block_on(module.instantiate(no_host(), &b)).unwrap();
    assert_eq!(inst.call(Export::Abi, &[], &b).unwrap(), 1);
}

#[test]
fn a_package_that_fails_the_gate_never_instantiates() {
    let engine = WasmiEngine::system();
    let bytes = package("reject_no_memory_max");
    // The module is valid Wasm and wasmi would run it. Only the gate stops it.
    let direct = wasmi_accepts(&engine, &bytes);
    assert!(direct, "control: the bytes are a valid module for wasmi");
    let err = block_on(load_package(&engine, &bytes, &HostPolicy::native(), OPTS)).err().unwrap();
    match err {
        LoadError::Gate(v) => assert_eq!(v, vec![Violation::MemoryMaxMissing]),
        other => panic!("expected a gate error, got {other:?}"),
    }
}

/// Compile with a forged accepted report to prove wasmi itself can take the bytes.
fn wasmi_accepts(engine: &WasmiEngine, bytes: &[u8]) -> bool {
    let forged = craft_plugin_abi::PolicyReport { violations: Vec::new(), legacy: None, manifest: None };
    engine.compile_sync(bytes, &forged, OPTS).is_ok()
}

#[test]
fn the_engine_refuses_a_rejected_report_even_when_called_directly() {
    let engine = WasmiEngine::system();
    let bytes = package("reject_no_memory_max");
    let report = gate(&bytes);
    assert!(!report.is_accepted());
    let err = engine.compile_sync(&bytes, &report, OPTS).err().unwrap();
    assert_eq!(err, LoadError::Gate(vec![Violation::MemoryMaxMissing]));
}

#[test]
fn a_legacy_package_is_sent_to_its_shim() {
    let engine = WasmiEngine::system();
    let bytes = fixture("legacy_pc");
    let err = block_on(load_package(&engine, &bytes, &HostPolicy::native(), OPTS)).err().unwrap();
    assert_eq!(err, LoadError::LegacyNeedsShim(craft_plugin_abi::LegacyAbi::Photocraft));
}

#[test]
fn wasmi_reports_the_caps_it_enforces() {
    let engine = WasmiEngine::system();
    assert_eq!(engine.id(), EngineId::Wasmi);
    let caps = engine.caps();
    assert!(caps.contains(Caps::FUEL | Caps::DEADLINE_PREEMPT | Caps::MEMORY_CAP | Caps::STACK_CAP | Caps::SYNC_CALL));
    assert!(!caps.contains(Caps::SIMD128));
    assert_eq!(caps.contains(Caps::CANON_NAN), cfg!(feature = "deterministic"));
    assert!(caps.enforces_deadline());
}

// ---- the untrusted rule, on an engine that lacks a deadline -----------------------------

/// A test double: an engine that reports no deadline enforcement. It records whether compile ran.
struct WeakEngine {
    compiled: std::sync::atomic::AtomicBool,
}

impl Engine for WeakEngine {
    fn id(&self) -> EngineId {
        EngineId::BrowserJit
    }

    fn caps(&self) -> Caps {
        Caps::MEMORY_CAP | Caps::STACK_CAP | Caps::SYNC_CALL
    }

    fn compile<'a>(
        &'a self,
        _wasm: &'a [u8],
        _report: &'a craft_plugin_abi::PolicyReport,
        _opts: CompileOptions,
    ) -> craft_plugin_host::engine::BoxFuture<'a, Result<Arc<dyn Module>, LoadError>> {
        self.compiled.store(true, Ordering::Relaxed);
        Box::pin(std::future::ready(Err(LoadError::Compile("test double compiles nothing".into()))))
    }
}

#[test]
fn an_untrusted_plugin_cannot_load_on_an_engine_without_deadline_enforcement() {
    let weak = WeakEngine { compiled: false.into() };
    let bytes = package("alloc");
    let untrusted = CompileOptions { trust: Trust::Untrusted, stack_bytes: STACK };
    let err = block_on(load_package(&weak, &bytes, &HostPolicy::native(), untrusted)).err().unwrap();
    assert_eq!(err, LoadError::EngineCannotEnforceDeadline { engine: EngineId::BrowserJit });
    assert!(!weak.compiled.load(Ordering::Relaxed), "the engine saw no bytes");
    // Control: a trusted plugin gets past the rule and reaches the engine.
    let trusted = CompileOptions { trust: Trust::Trusted, stack_bytes: STACK };
    let err = block_on(load_package(&weak, &bytes, &HostPolicy::native(), trusted)).err().unwrap();
    assert_eq!(err, LoadError::Compile("test double compiles nothing".into()));
    assert!(weak.compiled.load(Ordering::Relaxed));
}
