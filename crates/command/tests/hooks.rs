//! Hook order, veto, unregister snapshot, circuit breaker, and the observer queue.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use craft_command::{
    Call, CallInfo, Cause, Change, Clock, DisableCause, GateDisabled, Hooks, Interceptor, Interest, ObserverEvent, ObserverQueue, Origin, Outcome,
    Owner, Verdict,
};

fn owner(plugin_id: &str) -> Owner {
    Owner { plugin_id: Arc::from(plugin_id), generation: 1 }
}

fn interest(glob: &str, priority: i32) -> Interest {
    Interest { globs: vec![glob.to_string()], priority }
}

fn call(id: &str) -> Call {
    Call { id: Arc::from(id), params: Value::Null, depth: 0, origin: Origin::Execute, seq: 7 }
}

fn info(call: &Call) -> CallInfo {
    call.info()
}

struct ZeroClock;

impl Clock for ZeroClock {
    fn now_ms(&self) -> u64 {
        0
    }
}

struct ElapsedClock {
    start: Instant,
}

impl ElapsedClock {
    fn new() -> Self {
        Self { start: Instant::now() }
    }
}

impl Clock for ElapsedClock {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

struct ManualClock(AtomicU64);

impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

struct VetoGate {
    interest: Interest,
    befores: Arc<AtomicUsize>,
    afters: Arc<AtomicUsize>,
    seen: Arc<Mutex<Option<Outcome>>>,
}

impl Interceptor<()> for VetoGate {
    fn interest(&self) -> &Interest {
        &self.interest
    }

    fn before(&self, _session: &mut (), _call: &mut Call) -> Verdict {
        self.befores.fetch_add(1, Ordering::SeqCst);
        Verdict::Veto { reason: "no".to_string(), by: "gate".to_string() }
    }

    fn after(&self, _call: &CallInfo, out: &Outcome) {
        self.afters.fetch_add(1, Ordering::SeqCst);
        *self.seen.lock().expect("seen") = Some(*out);
    }
}

struct CountGate {
    interest: Interest,
    befores: Arc<AtomicUsize>,
    afters: Arc<AtomicUsize>,
}

impl Interceptor<()> for CountGate {
    fn interest(&self) -> &Interest {
        &self.interest
    }

    fn before(&self, _session: &mut (), _call: &mut Call) -> Verdict {
        self.befores.fetch_add(1, Ordering::SeqCst);
        Verdict::Pass
    }

    fn after(&self, _call: &CallInfo, _out: &Outcome) {
        self.afters.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn veto_still_runs_after() {
    let hooks = Hooks::<()>::new();
    let veto_befores = Arc::new(AtomicUsize::new(0));
    let veto_afters = Arc::new(AtomicUsize::new(0));
    let later_befores = Arc::new(AtomicUsize::new(0));
    let later_afters = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(Mutex::new(None));
    hooks.register(
        &owner("veto"),
        Arc::new(VetoGate {
            interest: interest("*", 0),
            befores: Arc::clone(&veto_befores),
            afters: Arc::clone(&veto_afters),
            seen: Arc::clone(&seen),
        }),
    );
    hooks.register(
        &owner("later"),
        Arc::new(CountGate { interest: interest("*", 10), befores: Arc::clone(&later_befores), afters: Arc::clone(&later_afters) }),
    );

    let mut attempt = call("file.save");
    let verdict = hooks.before(&mut (), &mut attempt, &ZeroClock);
    assert_eq!(verdict, Verdict::Veto { reason: "no".to_string(), by: "gate".to_string() });
    assert_eq!(veto_befores.load(Ordering::SeqCst), 1);
    assert_eq!(later_befores.load(Ordering::SeqCst), 0, "a veto stops later before gates");

    hooks.after(&info(&attempt), &Outcome::Vetoed, &ZeroClock);
    assert_eq!(veto_afters.load(Ordering::SeqCst), 1, "veto must still run after");
    assert_eq!(later_afters.load(Ordering::SeqCst), 1, "interest-matched gates still run after a veto");
    assert_eq!(*seen.lock().expect("seen"), Some(Outcome::Vetoed));
}

struct UnregGate {
    interest: Interest,
    hooks: Hooks<()>,
    owner: Owner,
    befores: Arc<AtomicUsize>,
    afters: Arc<AtomicUsize>,
}

impl Interceptor<()> for UnregGate {
    fn interest(&self) -> &Interest {
        &self.interest
    }

    fn before(&self, _session: &mut (), _call: &mut Call) -> Verdict {
        self.befores.fetch_add(1, Ordering::SeqCst);
        assert_eq!(self.hooks.unregister_owner(&self.owner), 1);
        Verdict::Pass
    }

    fn after(&self, _call: &CallInfo, out: &Outcome) {
        assert_eq!(*out, Outcome::Ok);
        self.afters.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn unregister_during_before_keeps_snapshot_after_and_drops_next_before() {
    let hooks = Hooks::<()>::new();
    let plugin = owner("plug");
    let befores = Arc::new(AtomicUsize::new(0));
    let afters = Arc::new(AtomicUsize::new(0));
    hooks.register(
        &plugin,
        Arc::new(UnregGate {
            interest: interest("*", 0),
            hooks: hooks.clone(),
            owner: plugin.clone(),
            befores: Arc::clone(&befores),
            afters: Arc::clone(&afters),
        }),
    );

    let mut attempt = call("file.save");
    assert!(matches!(hooks.before(&mut (), &mut attempt, &ZeroClock), Verdict::Pass));
    assert_eq!(befores.load(Ordering::SeqCst), 1);
    hooks.after(&info(&attempt), &Outcome::Ok, &ZeroClock);
    assert_eq!(afters.load(Ordering::SeqCst), 1, "snapshotted after runs after unregister");

    assert!(matches!(hooks.before(&mut (), &mut attempt, &ZeroClock), Verdict::Pass));
    hooks.after(&info(&attempt), &Outcome::Ok, &ZeroClock);
    assert_eq!(befores.load(Ordering::SeqCst), 1, "next before does not see the unregistered gate");
    assert_eq!(afters.load(Ordering::SeqCst), 1, "next after does not see the unregistered gate");
}

struct SlowGate {
    interest: Interest,
    runs: Arc<AtomicUsize>,
}

impl Interceptor<()> for SlowGate {
    fn interest(&self) -> &Interest {
        &self.interest
    }

    fn before(&self, _session: &mut (), _call: &mut Call) -> Verdict {
        self.runs.fetch_add(1, Ordering::SeqCst);
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(5) {
            std::thread::sleep(Duration::from_millis(1));
        }
        Verdict::Veto { reason: "too slow".to_string(), by: "slow".to_string() }
    }
}

#[test]
fn slow_gate_is_circuit_broken_and_verdict_is_pass() {
    let hooks = Hooks::<()>::new();
    hooks.begin_frame();
    let runs = Arc::new(AtomicUsize::new(0));
    let seq = hooks.register(&owner("slow"), Arc::new(SlowGate { interest: interest("*", 0), runs: Arc::clone(&runs) }));

    let mut attempt = call("file.save");
    let verdict = hooks.before(&mut (), &mut attempt, &ElapsedClock::new());
    assert_eq!(verdict, Verdict::Pass);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    let reason = hooks.disable_reason(seq).expect("slow gate records why it was disabled");
    assert!(reason.contains("exceeds 2 ms"), "{reason}");

    hooks.begin_frame();
    let again = hooks.before(&mut (), &mut attempt, &ElapsedClock::new());
    hooks.after(&info(&attempt), &Outcome::Ok, &ZeroClock);
    assert_eq!(again, Verdict::Pass);
    assert_eq!(runs.load(Ordering::SeqCst), 1, "a circuit-broken gate does not run on the next call");
}

struct PanicGate {
    interest: Interest,
    runs: Arc<AtomicUsize>,
}

impl Interceptor<()> for PanicGate {
    fn interest(&self) -> &Interest {
        &self.interest
    }

    fn before(&self, _session: &mut (), _call: &mut Call) -> Verdict {
        self.runs.fetch_add(1, Ordering::SeqCst);
        panic!("gate blew up");
    }
}

#[test]
fn panic_in_gate_fails_open_and_gate_does_not_run_again() {
    let hooks = Hooks::<()>::new();
    let runs = Arc::new(AtomicUsize::new(0));
    let seq = hooks.register(&owner("panic"), Arc::new(PanicGate { interest: interest("*", 0), runs: Arc::clone(&runs) }));

    let mut attempt = call("file.save");
    let first = hooks.before(&mut (), &mut attempt, &ZeroClock);
    assert_eq!(first, Verdict::Pass);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    let reason = hooks.disable_reason(seq).expect("panic records why the gate was disabled");
    assert!(reason.contains("gate blew up"), "{reason}");

    hooks.after(&info(&attempt), &Outcome::Ok, &ZeroClock);
    let second = hooks.before(&mut (), &mut attempt, &ZeroClock);
    assert_eq!(second, Verdict::Pass);
    assert_eq!(runs.load(Ordering::SeqCst), 1, "a panicked gate does not run on the next call");
}

struct OrderGate {
    interest: Interest,
    name: &'static str,
    log: Arc<Mutex<Vec<String>>>,
}

impl Interceptor<()> for OrderGate {
    fn interest(&self) -> &Interest {
        &self.interest
    }

    fn before(&self, _session: &mut (), _call: &mut Call) -> Verdict {
        self.log.lock().expect("log").push(format!("b:{}", self.name));
        Verdict::Pass
    }

    fn after(&self, _call: &CallInfo, _out: &Outcome) {
        self.log.lock().expect("log").push(format!("a:{}", self.name));
    }
}

#[test]
fn before_order_is_priority_then_seq_ascending_and_after_is_descending() {
    let hooks = Hooks::<()>::new();
    let log = Arc::new(Mutex::new(Vec::new()));
    // Registration order is c, a, b. Sort key is (priority, seq).
    for (name, priority) in [("c", 5), ("a", 1), ("b", 5)] {
        hooks.register(&owner(name), Arc::new(OrderGate { interest: interest("*", priority), name, log: Arc::clone(&log) }));
    }

    let mut attempt = call("file.save");
    assert_eq!(hooks.before(&mut (), &mut attempt, &ZeroClock), Verdict::Pass);
    hooks.after(&info(&attempt), &Outcome::Ok, &ZeroClock);
    assert_eq!(log.lock().expect("log").as_slice(), ["b:a", "b:c", "b:b", "a:b", "a:c", "a:a"]);
}

struct NameGate {
    interest: Interest,
    name: &'static str,
    log: Arc<Mutex<Vec<&'static str>>>,
}

impl Interceptor<()> for NameGate {
    fn interest(&self) -> &Interest {
        &self.interest
    }

    fn before(&self, _session: &mut (), _call: &mut Call) -> Verdict {
        self.log.lock().expect("log").push(self.name);
        Verdict::Pass
    }
}

#[test]
fn interest_matches_star_prefix_or_exact_id() {
    let hooks = Hooks::<()>::new();
    let log = Arc::new(Mutex::new(Vec::new()));
    let gates = [("star", "*", 2), ("exact", "file.save", 0), ("prefix", "file*", 1), ("other", "edit.cut", -1)];
    for (name, glob, priority) in gates {
        hooks.register(&owner(name), Arc::new(NameGate { interest: interest(glob, priority), name, log: Arc::clone(&log) }));
    }

    for id in ["file.save", "file.savextra", "edit.cut"] {
        let mut attempt = call(id);
        hooks.before(&mut (), &mut attempt, &ZeroClock);
        hooks.after(&info(&attempt), &Outcome::Ok, &ZeroClock);
    }

    assert_eq!(log.lock().expect("log").as_slice(), ["exact", "prefix", "star", "prefix", "star", "other", "star"]);
}

struct TickGate {
    interest: Interest,
    clock: Arc<ManualClock>,
    add_ms: u64,
    runs: Arc<AtomicUsize>,
    veto: bool,
}

impl Interceptor<()> for TickGate {
    fn interest(&self) -> &Interest {
        &self.interest
    }

    fn before(&self, _session: &mut (), _call: &mut Call) -> Verdict {
        self.runs.fetch_add(1, Ordering::SeqCst);
        self.clock.0.fetch_add(self.add_ms, Ordering::SeqCst);
        if self.veto { Verdict::Veto { reason: "frame".to_string(), by: "tick".to_string() } } else { Verdict::Pass }
    }
}

#[test]
fn frame_budget_disables_the_gate_that_crosses_6ms() {
    let hooks = Hooks::<()>::new();
    hooks.begin_frame();
    let clock = Arc::new(ManualClock(AtomicU64::new(0)));
    let mut seqs = Vec::new();
    let mut runs = Vec::new();
    for (index, veto) in [(0, false), (1, false), (2, false), (3, true)] {
        let count = Arc::new(AtomicUsize::new(0));
        let seq = hooks.register(
            &owner("shared"),
            Arc::new(TickGate { interest: interest("*", index), clock: Arc::clone(&clock), add_ms: 2, runs: Arc::clone(&count), veto }),
        );
        seqs.push(seq);
        runs.push(count);
    }

    let mut attempt = call("file.save");
    let verdict = hooks.before(&mut (), &mut attempt, clock.as_ref());
    hooks.after(&info(&attempt), &Outcome::Ok, &ZeroClock);
    assert_eq!(verdict, Verdict::Pass, "crossing the frame budget drops the veto");
    assert!(hooks.disable_reason(seqs[3]).expect("frame reason").contains("frame budget"));
    assert!(hooks.disable_reason(seqs[0]).is_none());
    assert!(hooks.disable_reason(seqs[1]).is_none());
    assert!(hooks.disable_reason(seqs[2]).is_none());

    hooks.begin_frame();
    clock.0.store(0, Ordering::SeqCst);
    let again = hooks.before(&mut (), &mut attempt, clock.as_ref());
    assert_eq!(again, Verdict::Pass);
    let counts: Vec<usize> = runs.iter().map(|count| count.load(Ordering::SeqCst)).collect();
    assert_eq!(counts, vec![2, 2, 2, 1]);
}

#[test]
fn depth_enter_leave_round_trips() {
    let hooks = Hooks::<()>::new();
    assert_eq!(hooks.depth(), 0);
    assert_eq!(hooks.enter(), 1);
    assert_eq!(hooks.enter(), 2);
    assert_eq!(hooks.depth(), 2);
    assert_eq!(hooks.leave(), 1);
    assert_eq!(hooks.leave(), 0);
    assert_eq!(hooks.leave(), 0);
    assert_eq!(hooks.depth(), 0);
}

#[test]
fn with_cause_restores_and_changed_publishes() {
    let hooks = Hooks::<()>::new();
    assert_eq!(hooks.current_cause(), Cause::Untracked);
    let bus = hooks.change_bus::<u32>();
    let sub = bus.subscribe();

    hooks.with_cause(Cause::Undo, || {
        assert_eq!(hooks.current_cause(), Cause::Undo);
        hooks.with_cause(Cause::Redo, || {
            assert_eq!(hooks.current_cause(), Cause::Redo);
        });
        assert_eq!(hooks.current_cause(), Cause::Undo);
        hooks.changed(4, hooks.current_cause(), true, 8u32);
    });
    assert_eq!(hooks.current_cause(), Cause::Untracked);

    assert_eq!(bus.drain(sub), vec![Change { revision: 4, cause: Cause::Undo, merged: true, snapshot: 8, prev: None }]);
}

#[test]
fn observer_coalesce_and_drop_oldest() {
    let queue = ObserverQueue::new();
    let clock = ManualClock(AtomicU64::new(0));
    let coalesced = queue.subscribe(Some(100));
    queue.push("drag", Origin::Ui, &clock);
    clock.0.store(50, Ordering::SeqCst);
    queue.push("drag", Origin::Ui, &clock);
    queue.push("other", Origin::Script, &clock);
    let events = queue.drain(coalesced);
    assert_eq!(events.len(), 2);
    assert!(matches!(&events[0], ObserverEvent::Call { id, origin, at_ms: 0 } if &**id == "drag" && *origin == Origin::Ui));
    assert!(matches!(&events[1], ObserverEvent::Call { id, origin, .. } if &**id == "other" && *origin == Origin::Script));
    assert!(queue.drain(coalesced).is_empty());

    let bounded = queue.subscribe(None);
    for index in 0..1024 {
        queue.push(&index.to_string(), Origin::Execute, &clock);
    }
    queue.push("overflow-a", Origin::Execute, &clock);
    queue.push("overflow-b", Origin::Execute, &clock);
    let drained = queue.drain(bounded);
    assert_eq!(drained.iter().filter(|event| matches!(event, ObserverEvent::Lost { .. })).count(), 1);
    assert_eq!(drained.first().and_then(ObserverEvent::lost_json), Some(json!({ "events.lost": 2 })));
    assert_eq!(drained.len(), 1025);
    assert!(matches!(drained.get(1), Some(ObserverEvent::Call { id, .. }) if &**id == "2"));
    assert!(matches!(drained.last(), Some(ObserverEvent::Call { id, .. }) if &**id == "overflow-b"));
    assert!(queue.drain(bounded).is_empty(), "lost count is consumed by drain");

    let gone = queue.subscribe(None);
    queue.unsubscribe(gone);
    queue.push("after-unsub", Origin::Mcp, &clock);
    assert!(queue.drain(gone).is_empty());
}

#[test]
fn frame_window_resets_itself_without_begin_frame() {
    // 1,000 calls. Each gate call costs 1 ms. 15 ms of other work separates calls, so each call opens
    // a new 16 ms window. Without self-reset the cumulative 1,000 ms would break the 6 ms frame budget.
    let hooks = Hooks::<()>::new();
    let clock = Arc::new(ManualClock(AtomicU64::new(0)));
    let runs = Arc::new(AtomicUsize::new(0));
    let seq = hooks.register(
        &owner("steady"),
        Arc::new(TickGate { interest: interest("*", 0), clock: Arc::clone(&clock), add_ms: 1, runs: Arc::clone(&runs), veto: true }),
    );

    let mut attempt = call("file.save");
    for _ in 0..1_000 {
        let verdict = hooks.before(&mut (), &mut attempt, clock.as_ref());
        assert!(matches!(verdict, Verdict::Veto { .. }), "an enabled gate keeps its veto");
        hooks.after(&info(&attempt), &Outcome::Ok, clock.as_ref());
        clock.0.fetch_add(15, Ordering::SeqCst);
    }
    assert_eq!(hooks.disable_reason(seq), None);
    assert_eq!(runs.load(Ordering::SeqCst), 1_000);
}

#[test]
fn gate_that_spends_over_6ms_inside_one_window_is_disabled_without_begin_frame() {
    // 2 ms per call passes the 2 ms call limit. The fourth call ends at 8 ms of charge inside a 16 ms window.
    let hooks = Hooks::<()>::new();
    let clock = Arc::new(ManualClock(AtomicU64::new(100)));
    let runs = Arc::new(AtomicUsize::new(0));
    let seq = hooks.register(
        &owner("hog"),
        Arc::new(TickGate { interest: interest("*", 0), clock: Arc::clone(&clock), add_ms: 2, runs: Arc::clone(&runs), veto: true }),
    );

    let mut attempt = call("file.save");
    let mut verdicts = Vec::new();
    for _ in 0..4 {
        verdicts.push(hooks.before(&mut (), &mut attempt, clock.as_ref()));
    }
    assert!(verdicts[..3].iter().all(|verdict| matches!(verdict, Verdict::Veto { .. })));
    assert_eq!(verdicts[3], Verdict::Pass, "the call that crosses 6 ms drops its verdict");
    let reason = hooks.disable_reason(seq).expect("hog gate disabled");
    assert!(reason.contains("frame budget"), "{reason}");

    clock.0.fetch_add(1_000, Ordering::SeqCst);
    assert_eq!(hooks.before(&mut (), &mut attempt, clock.as_ref()), Verdict::Pass);
    assert_eq!(runs.load(Ordering::SeqCst), 4, "a disabled gate stays disabled in later windows");
}

#[test]
fn begin_frame_still_restarts_the_window_early() {
    let hooks = Hooks::<()>::new();
    let clock = Arc::new(ManualClock(AtomicU64::new(0)));
    let runs = Arc::new(AtomicUsize::new(0));
    let seq = hooks.register(
        &owner("hint"),
        Arc::new(TickGate { interest: interest("*", 0), clock: Arc::clone(&clock), add_ms: 2, runs: Arc::clone(&runs), veto: false }),
    );
    let mut attempt = call("file.save");
    // Three calls use 6 ms of a 16 ms window, then the hint restarts it. Three more fit.
    for _ in 0..3 {
        hooks.before(&mut (), &mut attempt, clock.as_ref());
    }
    hooks.begin_frame();
    for _ in 0..3 {
        hooks.before(&mut (), &mut attempt, clock.as_ref());
    }
    assert_eq!(hooks.disable_reason(seq), None);
    assert_eq!(runs.load(Ordering::SeqCst), 6);
}

struct PassGate {
    interest: Interest,
}

impl Interceptor<()> for PassGate {
    fn interest(&self) -> &Interest {
        &self.interest
    }
}

#[test]
fn is_idle_tracks_gates_observers_change_subscribers_and_inflight_calls() {
    let hooks = Hooks::<()>::new();
    assert!(hooks.is_idle(), "a fresh Hooks has nothing to serve");

    hooks.register(&owner("g"), Arc::new(PassGate { interest: interest("*", 0) }));
    assert!(!hooks.is_idle(), "a gate makes it busy");
    assert_eq!(hooks.unregister_owner(&owner("g")), 1);
    assert!(hooks.is_idle());

    let sub = hooks.observer_queue().subscribe(None);
    assert!(!hooks.is_idle(), "an observer subscriber makes it busy");
    hooks.observer_queue().unsubscribe(sub);
    hooks.observer_queue().unsubscribe(sub);
    assert!(hooks.is_idle(), "a repeated unsubscribe does not underflow the count");

    let bus_u32 = hooks.change_bus::<u32>();
    let bus_str = hooks.change_bus::<String>();
    assert!(hooks.is_idle(), "a bus without subscribers is idle");
    let a = bus_u32.subscribe();
    let b = bus_str.subscribe();
    assert!(!hooks.is_idle(), "a change subscriber on any bus makes it busy");
    bus_u32.unsubscribe(a);
    bus_u32.unsubscribe(a);
    assert!(!hooks.is_idle(), "the other bus still has a subscriber");
    bus_str.unsubscribe(b);
    assert!(hooks.is_idle());

    // A call in flight (before without after) keeps it busy even after the gate is gone.
    hooks.register(&owner("g2"), Arc::new(PassGate { interest: interest("*", 0) }));
    let mut attempt = call("file.save");
    hooks.before(&mut (), &mut attempt, &ZeroClock);
    hooks.unregister_owner(&owner("g2"));
    assert!(!hooks.is_idle(), "an unpaired before keeps the hook layer busy");
    hooks.after(&info(&attempt), &Outcome::Ok, &ZeroClock);
    assert!(hooks.is_idle());
}

#[test]
fn disabling_the_last_gate_makes_hooks_idle_again() {
    let hooks = Hooks::<()>::new();
    let clock = Arc::new(ManualClock(AtomicU64::new(0)));
    hooks.register(
        &owner("slow"),
        Arc::new(TickGate { interest: interest("*", 0), clock: Arc::clone(&clock), add_ms: 3, runs: Arc::new(AtomicUsize::new(0)), veto: false }),
    );
    let mut attempt = call("file.save");
    hooks.before(&mut (), &mut attempt, clock.as_ref());
    hooks.after(&info(&attempt), &Outcome::Ok, clock.as_ref());
    assert!(hooks.is_idle(), "the 2 ms breaker removed the only gate");
}

// ---- origin and session access -------------------------------------------------------

struct SeeOriginGate {
    interest: Interest,
    seen: Arc<Mutex<Vec<Origin>>>,
}

impl Interceptor<()> for SeeOriginGate {
    fn interest(&self) -> &Interest {
        &self.interest
    }

    fn before(&self, _session: &mut (), call: &mut Call) -> Verdict {
        self.seen.lock().expect("seen").push(call.origin);
        Verdict::Pass
    }
}

#[test]
fn a_gate_sees_origin_plugin_apart_from_execute() {
    let hooks = Hooks::<()>::new();
    let seen = Arc::new(Mutex::new(Vec::new()));
    hooks.register(&owner("watch"), Arc::new(SeeOriginGate { interest: interest("*", 0), seen: Arc::clone(&seen) }));
    for origin in [Origin::Execute, Origin::Plugin] {
        let mut attempt = Call { origin, ..call("file.save") };
        hooks.before(&mut (), &mut attempt, &ZeroClock);
    }
    assert_eq!(seen.lock().expect("seen").as_slice(), [Origin::Execute, Origin::Plugin]);
    assert_ne!(Origin::Plugin, Origin::Script);
}

struct CountSession {
    interest: Interest,
}

impl Interceptor<u32> for CountSession {
    fn interest(&self) -> &Interest {
        &self.interest
    }

    fn before(&self, session: &mut u32, _call: &mut Call) -> Verdict {
        *session += 1;
        Verdict::Pass
    }
}

#[test]
fn a_gate_gets_the_session_by_mutable_reference() {
    let hooks = Hooks::<u32>::new();
    hooks.register(&owner("count"), Arc::new(CountSession { interest: interest("*", 0) }));
    let mut session = 0_u32;
    let mut attempt = call("file.save");
    hooks.before(&mut session, &mut attempt, &ZeroClock);
    hooks.before(&mut session, &mut attempt, &ZeroClock);
    assert_eq!(session, 2, "the gate worked on the caller's session, not a copy");
}

// ---- an owner over its share of the window loses its own gates (per owner) ----------------------------------

fn disabled_log(hooks: &Hooks<()>) -> Arc<Mutex<Vec<GateDisabled>>> {
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&log);
    hooks.set_disable_listener(Arc::new(move |event: &GateDisabled| sink.lock().expect("log").push(event.clone())));
    log
}

/// Four gates of 2 ms fill the 6 ms window and the fourth crosses it. Two more gates, one fast and
/// one slow, wait behind them. Their cost does not matter: the window is closed to them.
fn over_budget_rig() -> (Hooks<()>, Arc<ManualClock>, Vec<u64>, Vec<Arc<AtomicUsize>>) {
    let hooks = Hooks::<()>::new();
    let clock = Arc::new(ManualClock(AtomicU64::new(100)));
    let mut seqs = Vec::new();
    let mut runs = Vec::new();
    for (index, add_ms) in [(0, 2), (1, 2), (2, 2), (3, 2), (4, 0), (5, 1)] {
        let count = Arc::new(AtomicUsize::new(0));
        let seq = hooks.register(
            &owner("shared"),
            Arc::new(TickGate { interest: interest("*", index), clock: Arc::clone(&clock), add_ms, runs: Arc::clone(&count), veto: index >= 3 }),
        );
        seqs.push(seq);
        runs.push(count);
    }
    (hooks, clock, seqs, runs)
}

#[test]
fn gates_behind_an_exhausted_window_are_disabled_not_skipped_silently() {
    let (hooks, clock, seqs, runs) = over_budget_rig();
    let log = disabled_log(&hooks);
    let mut attempt = call("file.save");

    let verdict = hooks.before(&mut (), &mut attempt, clock.as_ref());

    assert_eq!(verdict, Verdict::Pass, "an over-budget chain fails open");
    let counts: Vec<usize> = runs.iter().map(|count| count.load(Ordering::SeqCst)).collect();
    assert_eq!(counts, vec![1, 1, 1, 1, 0, 0], "gates behind the closed window do not run");
    for seq in &seqs[4..] {
        let reason = hooks.disable_reason(*seq).expect("a gate the window shut out is disabled for the session");
        assert!(reason.contains("frame budget"), "{reason}");
    }
    let events = log.lock().expect("log");
    let told: Vec<(u64, DisableCause)> = events.iter().map(|event| (event.seq, event.cause)).collect();
    assert_eq!(
        told,
        vec![(seqs[3], DisableCause::FrameBudget), (seqs[4], DisableCause::FrameExhausted), (seqs[5], DisableCause::FrameExhausted)],
        "the user hears about each switched-off gate exactly once"
    );
    assert_eq!(events[1].owner, owner("shared"));
}

#[test]
fn a_disabled_gate_stays_off_and_is_not_announced_twice() {
    let (hooks, clock, seqs, runs) = over_budget_rig();
    let log = disabled_log(&hooks);
    let mut attempt = call("file.save");
    hooks.before(&mut (), &mut attempt, clock.as_ref());
    let first = log.lock().expect("log").len();

    // A later window has room, yet the shut-out gates stay off. Nothing more is announced.
    clock.0.fetch_add(1_000, Ordering::SeqCst);
    let _ = hooks.before(&mut (), &mut attempt, clock.as_ref());
    assert_eq!(runs[4].load(Ordering::SeqCst), 0);
    assert_eq!(runs[5].load(Ordering::SeqCst), 0);
    assert_eq!(log.lock().expect("log").len(), first);
    assert!(hooks.disable_reason(seqs[4]).is_some());
}

// ---- time is charged per owner; a security gate is never timed out and fails closed -------

#[test]
fn one_owner_cannot_spend_the_time_of_another_owner() {
    let hooks = Hooks::<()>::new();
    let clock = Arc::new(ManualClock(AtomicU64::new(100)));
    let evil_runs = Arc::new(AtomicUsize::new(0));
    let honest_runs = Arc::new(AtomicUsize::new(0));
    let evil = hooks.register(
        &owner("evil"),
        Arc::new(TickGate { interest: interest("*", 0), clock: Arc::clone(&clock), add_ms: 2, runs: evil_runs, veto: false }),
    );
    let honest = hooks.register(
        &owner("honest"),
        Arc::new(TickGate { interest: interest("*", 1), clock: Arc::clone(&clock), add_ms: 0, runs: Arc::clone(&honest_runs), veto: true }),
    );
    let mut attempt = call("file.delete");
    for _ in 0..10 {
        let verdict = hooks.before(&mut (), &mut attempt, clock.as_ref());
        // Inside one 16 ms window, once the evil gate is off the honest veto still holds.
        if hooks.disable_reason(evil).is_some() {
            assert!(matches!(verdict, Verdict::Veto { .. }), "the honest veto must hold: {verdict:?}");
        }
    }
    assert!(hooks.disable_reason(evil).is_some(), "the owner that overran loses its own gate");
    assert_eq!(hooks.disable_reason(honest), None, "the honest owner's gate is not disabled");
    assert!(honest_runs.load(Ordering::SeqCst) >= 4);
}

struct Secure {
    interest: Interest,
    clock: Arc<ManualClock>,
    add_ms: u64,
    panics: bool,
}

impl Interceptor<()> for Secure {
    fn interest(&self) -> &Interest {
        &self.interest
    }

    fn security(&self) -> bool {
        true
    }

    fn before(&self, _session: &mut (), _call: &mut Call) -> Verdict {
        self.clock.0.fetch_add(self.add_ms, Ordering::SeqCst);
        assert!(!self.panics, "boom");
        Verdict::Pass
    }
}

#[test]
fn a_security_gate_is_never_timed_out_and_a_panic_in_it_vetoes() {
    let hooks = Hooks::<()>::new();
    let clock = Arc::new(ManualClock(AtomicU64::new(100)));
    let slow = hooks.register(&owner("host"), Arc::new(Secure { interest: interest("*", 0), clock: Arc::clone(&clock), add_ms: 50, panics: false }));
    let mut attempt = call("file.delete");
    for _ in 0..5 {
        assert_eq!(hooks.before(&mut (), &mut attempt, clock.as_ref()), Verdict::Pass);
    }
    assert_eq!(hooks.disable_reason(slow), None, "a slow security gate stays on");

    let hooks = Hooks::<()>::new();
    let broken = hooks.register(&owner("host"), Arc::new(Secure { interest: interest("*", 0), clock: Arc::clone(&clock), add_ms: 0, panics: true }));
    for _ in 0..3 {
        let verdict = hooks.before(&mut (), &mut attempt, clock.as_ref());
        assert!(matches!(&verdict, Verdict::Veto { reason, .. } if reason.contains("security gate failed")), "{verdict:?}");
    }
    assert_eq!(hooks.disable_reason(broken), None, "a failing security gate fails closed, it is not switched off");
}
