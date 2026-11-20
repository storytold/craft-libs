//! The pure helpers behind the browser engines run on every target, so a native build checks them.
//! `memory_patch` lowers a module's declared memory maximum. `js_fault` maps a browser exception to
//! a trap. The browser run of the same behavior is in `tests/web.rs`.

#![cfg(feature = "wasmi")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use craft_plugin_host::engine::js_fault::classify_js_error;
use craft_plugin_host::engine::memory_patch::{MemoryLimits, PatchError, memory_limits, pages_in, with_memory_max};
use craft_plugin_host::engine::wasmi::WasmiEngine;
use craft_plugin_host::engine::{Budget, Export, Trap};
use serde_json::json;

use common::{OPTS, PAGE, STACK, gate, no_host};

fn oom_package() -> Vec<u8> {
    let text = std::fs::read_to_string(format!("{}/tests/fixtures/conformance/oom.wat", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let manifest = json!({
        "schema": 1, "id": "org.craft.conformance", "name": "C", "version": "1.0.0",
        "abi": { "craft": 1 }, "requires": { "ops": [] },
        "contributions": [{ "kind": "command", "id": "c", "entry": 0 }]
    });
    common::package_from(&text, &manifest)
}

/// `craft_invoke(0, pages)` on `wasm` under a cap far above the module maximum, so only the module's
/// own declared maximum decides. Returns the result of each grow and the final memory size in pages.
fn grow_twice(wasm: &[u8], pages: i32) -> (Result<i64, Trap>, Result<i64, Trap>, usize) {
    let engine = WasmiEngine::system();
    let report = gate(wasm);
    assert!(report.is_accepted(), "{:?}", report.violations);
    let module = engine.compile_sync(wasm, &report, OPTS).unwrap();
    let b = Budget::new(1024 * 1024 * 1024, STACK);
    let mut inst = module.instantiate_sync(no_host(), &b).unwrap();
    let mut grow = || craft_plugin_host::engine::Instance::call(&mut inst, Export::Invoke, &[0, pages, 0, 0, 0], &b);
    let first = grow();
    let second = grow();
    (first, second, craft_plugin_host::engine::Instance::mem_len(&inst) / PAGE as usize)
}

#[test]
fn the_declared_limits_of_oom_are_one_to_1024_pages() {
    assert_eq!(memory_limits(&oom_package()), Ok(Some(MemoryLimits { min_pages: 1, max_pages: Some(1024) })));
}

#[test]
fn lowering_the_maximum_makes_a_later_grow_fail_with_minus_one_and_keeps_the_size() {
    let original = oom_package();
    // Control: unpatched, the cap is far away and both grows succeed.
    assert_eq!(grow_twice(&original, 2), (Ok(1), Ok(3), 5));
    let patched = with_memory_max(&original, 4).unwrap();
    assert_eq!(memory_limits(&patched), Ok(Some(MemoryLimits { min_pages: 1, max_pages: Some(4) })));
    // 1 -> 3 pages fits the new maximum of 4. 3 -> 5 does not: grow returns -1 and the size stays.
    assert_eq!(grow_twice(&patched, 2), (Ok(1), Ok(-1), 3));
}

#[test]
fn only_the_memory_section_changes() {
    let original = oom_package();
    let patched = with_memory_max(&original, 4).unwrap();
    // oom declares `1 1024`: the maximum 1024 takes two LEB bytes and 4 takes one.
    assert_eq!(patched.len() + 1, original.len());
    // The module before the memory section is identical, and so is the module after it.
    let common_prefix = original.iter().zip(&patched).take_while(|(a, b)| a == b).count();
    let common_suffix = original.iter().rev().zip(patched.iter().rev()).take_while(|(a, b)| a == b).count();
    // What differs is the section size byte and the 4 payload bytes `count flags min max`.
    let changed = patched.len() - common_prefix - common_suffix;
    assert!(changed <= 5, "prefix {common_prefix} suffix {common_suffix} of {}", patched.len());
    assert!(gate(&patched).is_accepted());
}

#[test]
fn a_maximum_that_is_already_lower_stays() {
    let original = oom_package();
    let patched = with_memory_max(&original, 4).unwrap();
    let again = with_memory_max(&patched, 100).unwrap();
    assert_eq!(memory_limits(&again), Ok(Some(MemoryLimits { min_pages: 1, max_pages: Some(4) })));
}

#[test]
fn a_maximum_below_the_minimum_is_an_error_not_a_silent_clamp() {
    let wasm = wat::parse_str("(module (memory (export \"memory\") 8 16))").unwrap();
    assert_eq!(with_memory_max(&wasm, 7), Err(PatchError::BelowMinimum { min_pages: 8, max_pages: 7 }));
    assert!(with_memory_max(&wasm, 8).is_ok());
}

#[test]
fn a_memory_without_a_maximum_gets_one() {
    let wasm = wat::parse_str("(module (memory (export \"memory\") 2))").unwrap();
    assert_eq!(memory_limits(&wasm), Ok(Some(MemoryLimits { min_pages: 2, max_pages: None })));
    let patched = with_memory_max(&wasm, 9).unwrap();
    assert_eq!(memory_limits(&patched), Ok(Some(MemoryLimits { min_pages: 2, max_pages: Some(9) })));
}

#[test]
fn a_module_with_no_memory_has_no_limits_and_cannot_be_patched() {
    let wasm = wat::parse_str("(module)").unwrap();
    assert_eq!(memory_limits(&wasm), Ok(None));
    assert!(matches!(with_memory_max(&wasm, 1), Err(PatchError::Unsupported(_))));
}

#[test]
fn shared_and_truncated_modules_are_refused() {
    let shared = wat::parse_str("(module (memory (export \"memory\") 1 2 shared))").unwrap();
    assert!(matches!(memory_limits(&shared), Err(PatchError::Unsupported(_))));
    let wasm = oom_package();
    for cut in [0, 4, 9, 12] {
        assert_eq!(memory_limits(&wasm[..cut]), Err(PatchError::Truncated), "cut at {cut}");
    }
}

#[test]
fn a_page_count_rounds_down_and_saturates() {
    assert_eq!(pages_in(0), 0);
    assert_eq!(pages_in(PAGE - 1), 0);
    assert_eq!(pages_in(PAGE), 1);
    assert_eq!(pages_in(3 * PAGE + 5), 3);
    assert_eq!(pages_in(u64::MAX), u32::MAX);
}

#[test]
fn browser_exceptions_map_to_traps() {
    assert_eq!(classify_js_error("RuntimeError", "unreachable"), Trap::Unreachable);
    assert_eq!(classify_js_error("RuntimeError", "unreachable executed"), Trap::Unreachable);
    assert_eq!(classify_js_error("RangeError", "Maximum call stack size exceeded"), Trap::StackOverflow);
    assert_eq!(classify_js_error("InternalError", "too much recursion"), Trap::StackOverflow);
    // A message alone is not enough: the name must fit.
    assert!(matches!(classify_js_error("TypeError", "Maximum call stack size exceeded"), Trap::GuestFault(_)));
    assert!(matches!(classify_js_error("Error", "unreachable"), Trap::GuestFault(_)));
    assert_eq!(classify_js_error("RuntimeError", "divide by zero"), Trap::GuestFault("RuntimeError: divide by zero".into()));
}
