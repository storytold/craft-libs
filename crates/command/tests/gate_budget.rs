//! Regression test for the gate budget window. Each gate owner has its own share of the window.
//! A gate that spends 2 ms per call must not starve the gates that run after it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use craft_command::{Call, Clock, Hooks, Interceptor, Interest, Origin, Owner, Verdict};
use serde_json::Value;

struct Shared(AtomicU64);
impl Clock for Shared {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Burns 2 ms of the injected clock per call, the most one call may spend before it is disabled.
struct Burner(Arc<Shared>, Interest);
impl Interceptor<()> for Burner {
    fn interest(&self) -> &Interest {
        &self.1
    }
    fn before(&self, _s: &mut (), _c: &mut Call) -> Verdict {
        self.0.0.fetch_add(2, Ordering::SeqCst);
        Verdict::Pass
    }
}

/// An honest policy gate that vetoes everything, to show what the starvation turns off.
struct Policy(Interest);
impl Interceptor<()> for Policy {
    fn interest(&self) -> &Interest {
        &self.0
    }
    fn before(&self, _s: &mut (), _c: &mut Call) -> Verdict {
        Verdict::Veto { reason: "policy".into(), by: "honest".into() }
    }
}

fn call() -> Call {
    Call { id: Arc::from("file.delete"), params: Value::Null, depth: 0, origin: Origin::Execute, seq: 1 }
}

#[test]
fn a_gate_that_spends_2ms_per_call_no_longer_starves_the_honest_gate_behind_it() {
    let clock = Arc::new(Shared(AtomicU64::new(100)));
    let hooks: Hooks<()> = Hooks::new();
    let int = || Interest { globs: vec!["*".into()], priority: 1000 };
    let evil_seq = hooks.register(&Owner { plugin_id: Arc::from("evil"), generation: 1 }, Arc::new(Burner(clock.clone(), int())));
    let honest_seq = hooks.register(&Owner { plugin_id: Arc::from("honest"), generation: 1 }, Arc::new(Policy(int())));
    // Control: before the burst the honest gate vetoes.
    let first = hooks.before(&mut (), &mut call(), &*clock);
    assert!(matches!(first, Verdict::Veto { .. }), "control failed: {first:?}");
    // Three more calls inside the same 16 ms window. The burner pays 2 ms each time.
    for _ in 0..3 {
        let _ = hooks.before(&mut (), &mut call(), &*clock);
    }
    // The burner lost its own gate. The honest gate still vetoes the destructive command.
    assert!(hooks.disable_reason(evil_seq).is_some(), "the owner that burned the time is switched off");
    let after = hooks.before(&mut (), &mut call(), &*clock);
    assert!(matches!(after, Verdict::Veto { .. }), "the veto must hold: {after:?}");
    assert_eq!(hooks.disable_reason(honest_seq), None, "the honest gate must stay on");
}
