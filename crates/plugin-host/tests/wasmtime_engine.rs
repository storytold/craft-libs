//! wasmtime engine tests that the cross-engine file cannot hold: the epoch timer, the injected
//! clock, instance lifetime against the timer, and the pooling allocator limit.

#![cfg(all(feature = "wasmi", feature = "wasmtime", not(target_arch = "wasm32")))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use common::{PAGE, STACK, StepClock, block_on, gate, no_host, package_from};
use craft_plugin_host::engine::wasmtime::WasmtimeEngine;
use craft_plugin_host::engine::{Budget, CompileOptions, Engine, Export, Instance, LoadError, Trap, Trust};
use serde_json::json;

fn loop_package() -> Vec<u8> {
    let text = std::fs::read_to_string(format!("{}/tests/fixtures/conformance/loop.wat", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let manifest = json!({
        "schema": 1, "id": "org.craft.conformance", "name": "C", "version": "1.0.0", "abi": { "craft": 1 },
        "requires": { "ops": [] }, "contributions": [{ "kind": "command", "id": "c", "entry": 0 }]
    });
    package_from(&text, &manifest)
}

const OPTS: CompileOptions = CompileOptions { trust: Trust::Untrusted, stack_bytes: STACK };

fn budget() -> Budget {
    Budget::new(16 * PAGE, STACK)
}

fn spin(inst: &mut dyn Instance, b: &Budget) -> Result<i64, Trap> {
    inst.call(Export::Invoke, &[0, 0, 0, 0, 0], b)
}

#[test]
fn the_deadline_reads_the_injected_clock_not_the_wall_clock() {
    // The clock never moves, so a 10 ms deadline never passes. Real time runs 400 ms. Only the
    // cancel flag, set from another thread, ends the call.
    let engine = WasmtimeEngine::new(StepClock::new(Duration::ZERO)).unwrap();
    let bytes = loop_package();
    let module = block_on(engine.compile(&bytes, &gate(&bytes), OPTS)).unwrap();
    let mut b = budget();
    b.deadline = Some(Duration::from_millis(10));
    let mut inst = block_on(module.instantiate(no_host(), &b)).unwrap();
    let flag = Arc::clone(&b.cancel);
    let setter = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        flag.store(true, Ordering::Relaxed);
    });
    let t0 = Instant::now();
    assert_eq!(spin(inst.as_mut(), &b).unwrap_err(), Trap::Cancelled);
    assert!(t0.elapsed() >= Duration::from_millis(390), "the deadline fired on wall time");
    setter.join().unwrap();
}

#[test]
fn a_clock_that_moves_one_ms_per_read_ends_a_five_ms_deadline() {
    // Control for the test above: same plugin, a clock that moves, and the deadline fires.
    let engine = WasmtimeEngine::new(StepClock::new(Duration::from_millis(1))).unwrap();
    let bytes = loop_package();
    let module = block_on(engine.compile(&bytes, &gate(&bytes), OPTS)).unwrap();
    let mut b = budget();
    b.deadline = Some(Duration::from_millis(5));
    let mut inst = block_on(module.instantiate(no_host(), &b)).unwrap();
    let t0 = Instant::now();
    assert_eq!(spin(inst.as_mut(), &b).unwrap_err(), Trap::Deadline);
    assert!(t0.elapsed() < Duration::from_secs(2));
}

#[test]
fn an_instance_keeps_enforcing_deadlines_after_the_engine_and_module_are_dropped() {
    let b = {
        let mut b = budget();
        b.deadline = Some(Duration::from_millis(50));
        b
    };
    let mut inst = {
        let engine = WasmtimeEngine::system().unwrap();
        let bytes = loop_package();
        let module = block_on(engine.compile(&bytes, &gate(&bytes), OPTS)).unwrap();
        block_on(module.instantiate(no_host(), &b)).unwrap()
    };
    let t0 = Instant::now();
    assert_eq!(spin(inst.as_mut(), &b).unwrap_err(), Trap::Deadline);
    assert!(t0.elapsed() < Duration::from_secs(2));
}

#[test]
fn an_instance_runs_again_after_a_deadline_trap() {
    let engine = WasmtimeEngine::system().unwrap();
    let bytes = loop_package();
    let module = block_on(engine.compile(&bytes, &gate(&bytes), OPTS)).unwrap();
    let mut b = budget();
    b.deadline = Some(Duration::from_millis(30));
    let mut inst = block_on(module.instantiate(no_host(), &b)).unwrap();
    assert_eq!(spin(inst.as_mut(), &b).unwrap_err(), Trap::Deadline);
    // A finite call on the same instance, long deadline, finishes. A stale epoch deadline or a
    // stuck fuel counter would stop it.
    b.deadline = Some(Duration::from_secs(10));
    assert_eq!(inst.call(Export::Invoke, &[1, 50_000, 0, 0, 0], &b), Ok(50_000));
    // And a second runaway is stopped again.
    b.deadline = Some(Duration::from_millis(30));
    assert_eq!(spin(inst.as_mut(), &b).unwrap_err(), Trap::Deadline);
}

#[test]
fn many_threads_with_their_own_instances_all_hit_their_deadlines() {
    let engine = Arc::new(WasmtimeEngine::system().unwrap());
    let bytes = Arc::new(loop_package());
    let module = block_on(engine.compile(&bytes, &gate(&bytes), OPTS)).unwrap();
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let module = Arc::clone(&module);
            std::thread::spawn(move || {
                let mut b = budget();
                b.deadline = Some(Duration::from_millis(40 + 10 * i));
                let mut inst = block_on(module.instantiate(no_host(), &b)).unwrap();
                let t0 = Instant::now();
                let r = spin(inst.as_mut(), &b);
                (r, t0.elapsed())
            })
        })
        .collect();
    for (i, h) in handles.into_iter().enumerate() {
        let (r, took) = h.join().unwrap();
        assert_eq!(r.unwrap_err(), Trap::Deadline, "thread {i}");
        assert!(took >= Duration::from_millis(40 + 10 * i as u64), "thread {i} stopped early: {took:?}");
        assert!(took < Duration::from_secs(2), "thread {i} stopped late: {took:?}");
    }
}

#[test]
fn the_pool_refuses_instances_past_its_size_and_frees_a_slot_on_drop() {
    let engine = WasmtimeEngine::system().unwrap().with_pool_instances(2);
    let bytes = loop_package();
    let module = block_on(engine.compile(&bytes, &gate(&bytes), OPTS)).unwrap();
    let b = budget();
    let first = block_on(module.instantiate(no_host(), &b)).unwrap();
    let second = block_on(module.instantiate(no_host(), &b)).unwrap();
    match block_on(module.instantiate(no_host(), &b)) {
        Err(LoadError::Instantiate(_)) => {}
        other => panic!("a third instance in a pool of 2 must fail, got {:?}", other.map(|_| ())),
    }
    drop(first);
    let third = block_on(module.instantiate(no_host(), &b));
    assert!(third.is_ok(), "a freed slot must be reusable");
    drop(second);
}

#[test]
fn work_is_fuel_per_call_and_reports_what_it_used() {
    let engine = WasmtimeEngine::system().unwrap();
    let bytes = loop_package();
    let module = engine.compile_sync(&bytes, &gate(&bytes), OPTS).unwrap();
    let mut b = budget();
    b.work = Some(10_000_000);
    let mut inst = module.instantiate_sync(no_host(), &b).unwrap();
    inst.call(Export::Invoke, &[1, 1000, 0, 0, 0], &b).unwrap();
    let small = inst.last_call_stats().fuel_used;
    inst.call(Export::Invoke, &[1, 100_000, 0, 0, 0], &b).unwrap();
    let big = inst.last_call_stats().fuel_used;
    assert!(small > 0, "a call burns fuel");
    assert!(big > small * 50, "fuel grows with the loop: {small} then {big}");
}
