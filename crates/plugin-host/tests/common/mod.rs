//! Shared helpers: WAT fixtures, packages with a manifest section, clocks, budgets.

#![allow(dead_code, unused_imports, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod pkg;
pub use pkg::{package_from, with_section};

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use craft_plugin_abi::{HostPolicy, PolicyReport, check_package};
use craft_plugin_host::engine::wasmi::{WasmiEngine, WasmiInstance};
use craft_plugin_host::engine::{Budget, Clock, CompileOptions, HostDispatch, HostResult, Trust};
use serde_json::{Value, json};

pub const PAGE: u64 = 65_536;
pub const STACK: u32 = 64 * 1024;

pub fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}.wat", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    wat::parse_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn manifest() -> Value {
    json!({
        "schema": 1, "id": "org.example.t", "name": "T", "version": "1.0.0",
        "abi": { "craft": 1 },
        "requires": { "ops": ["host.log"] },
        "contributions": [{ "kind": "command", "id": "c", "entry": 0 }]
    })
}

/// A fixture with a valid `craft.manifest` section appended.
pub fn package(name: &str) -> Vec<u8> {
    with_section(fixture(name), "craft.manifest", &serde_json::to_vec(&manifest()).unwrap())
}

pub fn gate(bytes: &[u8]) -> PolicyReport {
    check_package(bytes, &HostPolicy::native())
}

/// A clock that moves `step` on every read, so a test controls time by counting reads.
pub struct StepClock {
    ns: AtomicU64,
    step_ns: u64,
}

impl StepClock {
    pub fn new(step: Duration) -> Arc<StepClock> {
        Arc::new(StepClock { ns: AtomicU64::new(0), step_ns: step.as_nanos() as u64 })
    }

    pub fn reads(&self) -> u64 {
        self.ns.load(Ordering::Relaxed) / self.step_ns.max(1)
    }
}

impl Clock for StepClock {
    fn now(&self) -> Duration {
        Duration::from_nanos(self.ns.fetch_add(self.step_ns, Ordering::Relaxed))
    }
}

pub fn no_host() -> HostDispatch {
    Arc::new(|_op, _inp: &[u8], _out: &mut [u8]| -> HostResult { Ok(0) })
}

pub fn budget() -> Budget {
    Budget::new(16 * PAGE, STACK)
}

pub const OPTS: CompileOptions = CompileOptions { trust: Trust::Untrusted, stack_bytes: STACK };

/// Compile `fixture_name` through the gate and instantiate it on `engine`.
pub fn instance(engine: &WasmiEngine, fixture_name: &str, host: HostDispatch, b: &Budget) -> WasmiInstance {
    let bytes = package(fixture_name);
    let report = gate(&bytes);
    assert!(report.is_accepted(), "{fixture_name}: {:#?}", report.violations);
    let module = engine.compile_sync(&bytes, &report, OPTS).unwrap();
    module.instantiate_sync(host, b).unwrap()
}

/// Poll a future that never waits. Every engine in this crate completes on the first poll.
pub fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = std::pin::pin!(fut);
    let mut cx = Context::from_waker(Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("future was pending"),
    }
}
