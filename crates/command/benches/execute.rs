//! Cost of the hook paths in `craft-command`.
//!
//! `hooks_*` time `Hooks::before` plus `Hooks::after` alone. `execute_*` replay the
//! normative `Session::execute` order as an app runs it (resolve, `Arc<str>` id, cause
//! push, `before`, depth enter/leave, command body, `after`) against the shared `Registry`.
//! `execute_baseline` is the same sequence with every hook call removed, so
//! `execute_<config> - execute_baseline` is the hook cost and `execute_baseline - body` is the
//! non-hook dispatch cost.
//!
//! Gate benches call `Hooks::begin_frame` once per iteration (one call per frame, as the apps do) and
//! assert the gate is still enabled afterwards. Without the reset, the ms clock charges 1 ms per tick
//! crossing to the 6 ms frame budget and the circuit breaker disables the gate mid-run.
//! `begin_frame` is also benched on its own, so its cost can be subtracted.
//!
//! Observer note: no engine code pushes to `ObserverQueue` yet, so the observer configuration
//! is `subscribe` plus one `push` per call at the call site.

use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use craft_command::{Call, CallInfo, Cause, Clock, CommandSpec, Hooks, Interceptor, Interest, Origin, Outcome, Owner, Registry, Verdict};
use criterion::{Criterion, criterion_group, criterion_main};
use serde_json::{Value, json};

struct Session {
    counter: u64,
}

type Spec = CommandSpec<Session, String>;

fn always(_: &Session) -> Result<(), String> {
    Ok(())
}

fn body(session: &mut Session, params: &Value) -> Result<Value, String> {
    session.counter = session.counter.wrapping_add(1);
    Ok(black_box(params).clone())
}

static SPECS: [Spec; 1] = [CommandSpec {
    id: "view.zoomIn",
    label: "Zoom In",
    menu: &["View"],
    shortcut: None,
    alt_shortcuts: &[],
    params: "{}",
    enabled: always,
    run: body,
    journal: false,
}];

/// Wall clock in ms from an `Instant`, like the apps' `SystemClock` (one clock read per call).
struct BenchClock(Instant);

impl Clock for BenchClock {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.0.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

struct PassGate {
    interest: Interest,
}

impl Interceptor<Session> for PassGate {
    fn interest(&self) -> &Interest {
        &self.interest
    }
}

fn owner() -> Owner {
    Owner { plugin_id: Arc::from("bench"), generation: 1 }
}

fn hooks_with_gate() -> Hooks<Session> {
    let hooks = Hooks::new();
    hooks.register(&owner(), Arc::new(PassGate { interest: Interest { globs: vec!["*".to_string()], priority: 0 } }));
    hooks
}

fn hooks_with_observer() -> Hooks<Session> {
    let hooks = Hooks::new();
    hooks.observer_queue().subscribe(None);
    hooks
}

fn call_for(id: &Arc<str>) -> Call {
    Call { id: Arc::clone(id), params: Value::Null, depth: 1, origin: Origin::Execute, seq: 1 }
}

/// Mirrors `Session::execute` and `execute_resolved` in an app. `HOOKED` false drops `before`/`after`.
fn execute_once<const HOOKED: bool, const FAST: bool>(
    session: &mut Session,
    registry: &Registry<Session, String>,
    hooks: &Hooks<Session>,
    clock: &BenchClock,
    observe: bool,
    id: &str,
    params: Value,
) -> Result<Value, String> {
    let Some(cmd) = registry.resolve(id) else {
        return Err("unknown".to_string());
    };
    // Fast path: nothing can observe this call, so skip cause, clone, id allocation, gates and depth.
    if FAST && hooks.is_idle() {
        return match &cmd {
            craft_command::CommandEntry::Static(spec) => {
                (spec.enabled)(session)?;
                (spec.run)(session, &params)
            }
            craft_command::CommandEntry::Dynamic(_) => Err("dynamic".to_string()),
        };
    }
    if observe {
        hooks.observer_queue().push(id, Origin::Execute, clock);
    }
    let id_arc: Arc<str> = Arc::from(id);
    let hooks = hooks.clone();
    hooks.with_cause(Cause::Command { id: Arc::clone(&id_arc), depth: 1, origin: Origin::Execute }, || {
        let mut call = Call { id: Arc::clone(&id_arc), params, depth: 1, origin: Origin::Execute, seq: 1 };
        let verdict = if HOOKED { hooks.before(session, &mut call, clock) } else { Verdict::Pass };
        let params = call.params.clone();
        let info = call.info();
        if let Verdict::Veto { reason, by } = verdict {
            return Err(format!("{by}: {reason}"));
        }
        let ran = match &cmd {
            craft_command::CommandEntry::Static(spec) => {
                (spec.enabled)(session)?;
                hooks.enter();
                let out = (spec.run)(session, &params);
                hooks.leave();
                out
            }
            craft_command::CommandEntry::Dynamic(_) => Err("dynamic".to_string()),
        };
        if HOOKED {
            hooks.after(&info, &if ran.is_ok() { Outcome::Ok } else { Outcome::Err }, clock);
        }
        ran
    })
}

fn bench_hooks(c: &mut Criterion) {
    let mut group = c.benchmark_group("hooks");
    let clock = BenchClock(Instant::now());
    let mut session = Session { counter: 0 };
    let id: Arc<str> = Arc::from("view.zoomIn");

    let none = Hooks::<Session>::new();
    group.bench_function("before_after_none", |b| {
        b.iter(|| {
            let mut call = call_for(&id);
            let verdict = none.before(black_box(&mut session), &mut call, &clock);
            none.after(&call.info(), &Outcome::Ok, &clock);
            black_box(verdict)
        });
    });
    // Only the two calls the 20 ns target names: the atomic load and branch in each.
    let info: CallInfo = call_for(&id).info();
    group.bench_function("before_none_only", |b| {
        let mut call = call_for(&id);
        b.iter(|| black_box(none.before(black_box(&mut session), &mut call, &clock)));
    });
    group.bench_function("after_none_only", |b| {
        b.iter(|| none.after(black_box(&info), &Outcome::Ok, &clock));
    });

    // Fixed per-call hook-layer work that runs even with no gate registered.
    group.bench_function("fixed_with_cause", |b| {
        b.iter(|| none.with_cause(Cause::Command { id: Arc::clone(&id), depth: 1, origin: Origin::Execute }, || black_box(1u32)));
    });
    group.bench_function("fixed_enter_leave", |b| {
        b.iter(|| {
            none.enter();
            none.leave()
        });
    });
    group.bench_function("fixed_arc_str_from_id", |b| b.iter(|| Arc::<str>::from(black_box("view.zoomIn"))));
    group.bench_function("fixed_hooks_clone", |b| b.iter(|| black_box(none.clone())));

    let observer = hooks_with_observer();
    group.bench_function("is_idle_true", |b| b.iter(|| black_box(none.is_idle())));
    group.bench_function("is_idle_false", |b| b.iter(|| black_box(observer.is_idle())));
    group.bench_function("before_after_observer_hooks_only", |b| {
        b.iter(|| {
            let mut call = call_for(&id);
            let verdict = observer.before(black_box(&mut session), &mut call, &clock);
            observer.after(&call.info(), &Outcome::Ok, &clock);
            black_box(verdict)
        });
    });
    group.bench_function("observer_push", |b| {
        let queue = observer.observer_queue();
        b.iter(|| queue.push(black_box("view.zoomIn"), Origin::Execute, &clock));
    });

    let gate = hooks_with_gate();
    group.bench_function("begin_frame", |b| b.iter(|| gate.begin_frame()));
    group.bench_function("before_after_gate", |b| {
        b.iter(|| {
            gate.begin_frame();
            let mut call = call_for(&id);
            let verdict = gate.before(black_box(&mut session), &mut call, &clock);
            gate.after(&call.info(), &Outcome::Ok, &clock);
            black_box(verdict)
        });
    });
    assert!(gate.disable_reason(0).is_none(), "gate was disabled during the bench");
    group.finish();
}

fn bench_execute(c: &mut Criterion) {
    let mut group = c.benchmark_group("execute");
    let clock = BenchClock(Instant::now());
    let registry: Registry<Session, String> = Registry::new(&SPECS);
    let mut session = Session { counter: 0 };
    let none = Hooks::<Session>::new();
    let observer = hooks_with_observer();
    let gate = hooks_with_gate();

    group.bench_function("body_only", |b| {
        let params = Value::Null;
        b.iter(|| black_box(body(&mut session, black_box(&params))));
    });
    group.bench_function("resolve_only", |b| {
        b.iter(|| black_box(registry.resolve(black_box("view.zoomIn"))).is_some());
    });
    group.bench_function("baseline_no_hook_calls", |b| {
        b.iter(|| black_box(execute_once::<false, false>(&mut session, &registry, &none, &clock, false, "view.zoomIn", Value::Null)));
    });
    group.bench_function("none", |b| {
        b.iter(|| black_box(execute_once::<true, false>(&mut session, &registry, &none, &clock, false, "view.zoomIn", Value::Null)));
    });
    group.bench_function("none_fast_path", |b| {
        b.iter(|| black_box(execute_once::<true, true>(&mut session, &registry, &none, &clock, false, "view.zoomIn", Value::Null)));
    });
    // Same fast-path code with a busy `Hooks`: the idle check must fall through to the full path.
    group.bench_function("observer_with_fast_path_code", |b| {
        b.iter(|| black_box(execute_once::<true, true>(&mut session, &registry, &observer, &clock, true, "view.zoomIn", Value::Null)));
    });
    group.bench_function("gate_with_fast_path_code", |b| {
        b.iter(|| {
            gate.begin_frame();
            black_box(execute_once::<true, true>(&mut session, &registry, &gate, &clock, false, "view.zoomIn", Value::Null))
        });
    });
    group.bench_function("observer", |b| {
        b.iter(|| black_box(execute_once::<true, false>(&mut session, &registry, &observer, &clock, true, "view.zoomIn", Value::Null)));
    });
    group.bench_function("gate", |b| {
        b.iter(|| {
            gate.begin_frame();
            black_box(execute_once::<true, false>(&mut session, &registry, &gate, &clock, false, "view.zoomIn", Value::Null))
        });
    });
    // Params with a 4-key object, to expose a params clone in the gate or observer path.
    let params = json!({"a": 1, "b": "two", "c": [1, 2, 3], "d": {"e": true}});
    group.bench_function("gate_object_params", |b| {
        b.iter(|| {
            gate.begin_frame();
            black_box(execute_once::<true, false>(&mut session, &registry, &gate, &clock, false, "view.zoomIn", params.clone()))
        });
    });
    assert!(gate.disable_reason(0).is_none(), "gate was disabled during the bench");
    group.finish();
}

criterion_group!(benches, bench_hooks, bench_execute);
criterion_main!(benches);
