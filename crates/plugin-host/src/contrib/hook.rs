//! `hook` contributions become [`craft_command::Interceptor`]s and change subscribers.
//!
//! ```text
//! phase    permission          where the guest runs                   what the guest returns
//! before   commands.intercept  the thread that calls Hooks::before    {} | {"rewrite": {..}} | {"veto": "why"}
//! after    commands.observe    a delivery thread of this hook         ignored
//! change   commands.observe    a delivery thread of this hook         ignored
//! ```
//!
//! Rules this module keeps:
//!
//! - A hook never blocks an edit. A `before` call gets a 2 ms wall deadline, does not wait for a busy
//!   session instance, and fails open (the command runs) on a trap, a bad answer, a busy instance,
//!   or an open circuit. The user hears about the first fault of a hook once, and about the circuit
//!   once. `Generation::call_with` counts every fault, so three in a row open the circuit.
//! - `after` and `change` calls never run on the thread that executes the command. The interceptor
//!   `after` only pushes a request onto a bounded queue (1024, drop-oldest, then one
//!   `{"events.lost": n}` request). A delivery thread owned by the hook drains the queue. The change
//!   feed is drained by the same thread from the [`ChangeBus`] every 20 ms.
//! - `after` uses `Interceptor::after` and not the [`craft_command::ObserverQueue`], because an
//!   observer item carries no outcome and the request to the plugin has one.
//! - A veto reaches the user through `Services::notify` with the plugin's reason. The verdict then
//!   travels to the app as `Verdict::Veto`, and the app turns it into `CommandError::Vetoed`.
//! - A hook that runs while the same hook is already running on this thread passes at once. Without
//!   that, a hook that calls `cmd.execute` would call itself until the stack ended.
//! - The app registers through [`HookHost::register`] or, with a `PluginManager`, through
//!   `PluginManager::attach_hooks`. A hook whose permission is missing is skipped and listed in
//!   [`HookRegistration::skipped`]. Default deny.
//!
//! Known limits. A plugin whose `after` hook calls a command it observes feeds itself, and the queue
//! bound is the only stop. A `change` hook ignores `match`, because a change is not a command id.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use craft_command::{
    Call, CallInfo, Cause, Change, ChangeBus, DisableCause, GateDisabled, Hooks, Interceptor, Interest, Origin, Outcome, Owner, Verdict,
};
use craft_plugin_abi::Permission;
use craft_plugin_abi::manifest::{Contribution, Kind, Manifest, Phase};
use serde_json::{Value, json};

use crate::engine::Clock;
use crate::invoke::{CallError, CallOpts, Router};
use crate::services::{Services, UserNotice, lend};

/// Wall deadline of one `before` call.
pub const GATE_CALL_LIMIT: Duration = Duration::from_millis(2);
/// Requests a delivery queue holds before it drops the oldest.
pub const QUEUE_CAPACITY: usize = 1024;
/// Gate order key. Plugin gates run after the app's own gates, which use lower numbers.
pub const PLUGIN_PRIORITY: i32 = 1000;
/// How often a `change` hook thread looks at the change bus.
const CHANGE_POLL: Duration = Duration::from_millis(20);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---- JSON of the requests -------------------------------------------------------------------

/// The `origin` string a hook sees.
pub fn origin_name(origin: Origin) -> &'static str {
    match origin {
        Origin::Execute => "execute",
        Origin::Ui => "ui",
        Origin::Shortcut => "shortcut",
        Origin::Control => "control",
        Origin::Mcp => "mcp",
        Origin::Script => "script",
        Origin::Plugin => "plugin",
    }
}

/// The `outcome` string an `after` hook sees.
pub fn outcome_name(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Ok => "ok",
        Outcome::Err => "err",
        Outcome::Vetoed => "vetoed",
        Outcome::Disabled => "disabled",
        Outcome::Panicked => "panicked",
    }
}

fn cause_json(cause: &Cause) -> Value {
    match cause {
        Cause::Command { id, depth, origin } => json!({ "kind": "command", "id": &**id, "depth": depth, "origin": origin_name(*origin) }),
        Cause::Undo => json!({ "kind": "undo" }),
        Cause::Redo => json!({ "kind": "redo" }),
        Cause::Job(name) => json!({ "kind": "job", "name": name }),
        Cause::Load => json!({ "kind": "load" }),
        Cause::Untracked => json!({ "kind": "untracked" }),
    }
}

fn before_request(call: &Call) -> Value {
    json!({ "call": { "id": &*call.id, "params": call.params, "depth": call.depth, "origin": origin_name(call.origin), "seq": call.seq } })
}

fn after_request(info: &CallInfo, outcome: &Outcome) -> Value {
    json!({ "call": { "id": &*info.id, "depth": info.depth, "origin": origin_name(info.origin), "seq": info.seq }, "outcome": outcome_name(outcome) })
}

fn change_request<D>(change: &Change<D>) -> Value {
    json!({ "change": { "revision": change.revision, "cause": cause_json(&change.cause), "merged": change.merged } })
}

// ---- Shared state of one hook contribution --------------------------------------------------

/// Tells the user about a hook fault: the first fault once, the open circuit once.
struct Reporter {
    services: Arc<dyn Services>,
    plugin_id: String,
    plugin_name: String,
    contribution: String,
    consequence: &'static str,
    faulted: AtomicBool,
    circuit: AtomicBool,
}

impl Reporter {
    fn notice(&self, message: String) {
        self.services.notify(UserNotice::new(self.plugin_id.clone(), Some(self.contribution.clone()), &message));
    }

    fn what(&self) -> String {
        format!("The {} hook {} did not finish", self.plugin_name, self.contribution)
    }

    /// A call failed. `Busy` is not a fault and is never reported.
    fn failed(&self, error: &CallError) {
        match error {
            CallError::Busy | CallError::NotLoaded => {}
            CallError::CircuitOpen => {
                if !self.circuit.swap(true, Ordering::Relaxed) {
                    self.notice(format!(
                        "The {} hook {} stays off for this session after repeated faults. {}",
                        self.plugin_name, self.contribution, self.consequence
                    ));
                }
            }
            other => {
                if !self.faulted.swap(true, Ordering::Relaxed) {
                    self.notice(format!("{}: {other}. {}", self.what(), self.consequence));
                }
            }
        }
    }

    /// The answer parsed, but the guest broke the response rules.
    fn malformed(&self, why: &str) {
        if !self.faulted.swap(true, Ordering::Relaxed) {
            self.notice(format!("The {} hook {} sent a bad answer: {why}. {}", self.plugin_name, self.contribution, self.consequence));
        }
    }

    /// The call finished but took longer than the gate limit. The command layer drops the verdict
    /// and switches the gate off, so say so.
    fn slow(&self, elapsed: Duration) {
        if !self.faulted.swap(true, Ordering::Relaxed) {
            self.notice(format!(
                "The {} hook {} took {} ms, over the {} ms limit. The app switches it off for this session. {}",
                self.plugin_name,
                self.contribution,
                elapsed.as_millis(),
                GATE_CALL_LIMIT.as_millis(),
                self.consequence
            ));
        }
    }
}

/// What every call of one hook contribution needs.
struct HookCtx {
    owner: Owner,
    contribution: Arc<Contribution>,
    router: Arc<Router>,
    services: Arc<dyn Services>,
    clock: Arc<dyn Clock>,
    reporter: Reporter,
}

impl HookCtx {
    fn new(
        owner: &Owner,
        manifest: &Manifest,
        c: &Arc<Contribution>,
        router: &Arc<Router>,
        services: &Arc<dyn Services>,
        clock: &Arc<dyn Clock>,
        consequence: &'static str,
    ) -> HookCtx {
        HookCtx {
            owner: owner.clone(),
            contribution: Arc::clone(c),
            router: Arc::clone(router),
            services: Arc::clone(services),
            clock: Arc::clone(clock),
            reporter: Reporter {
                services: Arc::clone(services),
                plugin_id: manifest.id.clone(),
                plugin_name: manifest.name.clone(),
                contribution: c.id.clone(),
                consequence,
                faulted: AtomicBool::new(false),
                circuit: AtomicBool::new(false),
            },
        }
    }

    fn invoke(&self, request: &Value, opts: CallOpts) -> Result<Value, CallError> {
        let generation = self.router.current(&self.owner.plugin_id).ok_or(CallError::NotLoaded)?;
        generation.call_with(&self.contribution, request, &[], 0, opts)
    }
}

// ---- before ---------------------------------------------------------------------------------

thread_local! {
    /// Ids of the gates running on this thread.
    static RUNNING_GATES: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

/// Marks a gate as running on this thread until it drops.
struct Reentry(u64);

impl Reentry {
    /// `None` when this gate already runs on this thread.
    fn enter(gate: u64) -> Option<Reentry> {
        RUNNING_GATES.with(|g| {
            let mut g = g.borrow_mut();
            if g.contains(&gate) {
                return None;
            }
            g.push(gate);
            Some(Reentry(gate))
        })
    }
}

impl Drop for Reentry {
    fn drop(&mut self) {
        RUNNING_GATES.with(|g| g.borrow_mut().retain(|id| *id != self.0));
    }
}

/// A `before` hook as a gate.
struct BeforeGate<S> {
    id: u64,
    interest: Interest,
    ctx: HookCtx,
    _session: PhantomData<fn(&S)>,
}

impl<S> BeforeGate<S> {
    fn verdict(&self, call: &Call, answer: Result<Value, CallError>) -> Verdict {
        let value = match answer {
            Ok(v) => v,
            Err(e) => {
                self.ctx.reporter.failed(&e);
                return Verdict::Pass;
            }
        };
        match parse_verdict(&value) {
            Ok(Answer::Pass) => Verdict::Pass,
            Ok(Answer::Rewrite(params)) => Verdict::Rewrite(params),
            Ok(Answer::Veto(reason)) => {
                self.ctx.services.notify(UserNotice::new(
                    self.ctx.owner.plugin_id.to_string(),
                    Some(self.ctx.contribution.id.clone()),
                    &format!("{} blocked {}: {reason}", self.ctx.reporter.plugin_name, call.id),
                ));
                Verdict::Veto { reason, by: self.ctx.reporter.plugin_name.clone() }
            }
            Err(why) => {
                self.ctx.reporter.malformed(&why);
                Verdict::Pass
            }
        }
    }
}

impl<S: 'static> Interceptor<S> for BeforeGate<S> {
    fn interest(&self) -> &Interest {
        &self.interest
    }

    fn before(&self, session: &mut S, call: &Call) -> Verdict {
        let Some(_running) = Reentry::enter(self.id) else {
            return Verdict::Pass;
        };
        let started = self.ctx.clock.now();
        let opts = CallOpts { deadline: Some(GATE_CALL_LIMIT), quiet: true, no_wait: true };
        let request = before_request(call);
        // The gate keeps the real session. The app lends it to host calls only while the guest runs.
        let Some(answer) = lend(&*self.ctx.services, session, || self.ctx.invoke(&request, opts)) else {
            return Verdict::Pass;
        };
        let elapsed = self.ctx.clock.now().saturating_sub(started);
        // The command layer counts whole milliseconds, drops the verdict past 2, and switches the
        // gate off. A call that failed has already been reported, so only a finished call is "slow".
        if answer.is_ok() && elapsed.as_millis() > GATE_CALL_LIMIT.as_millis() {
            self.ctx.reporter.slow(elapsed);
            return Verdict::Pass;
        }
        self.verdict(call, answer)
    }
}

enum Answer {
    Pass,
    Rewrite(Value),
    Veto(String),
}

/// A `before` answer. `{}`, `null` and an empty response pass. A veto wins over a rewrite.
fn parse_verdict(value: &Value) -> Result<Answer, String> {
    let object = match value {
        Value::Null => return Ok(Answer::Pass),
        Value::Object(o) => o,
        _ => return Err("the answer is not an object".into()),
    };
    if let Some(reason) = object.get("veto") {
        return match reason.as_str().map(str::trim) {
            // The reason reaches the user and the caller of the command. One safe, short line.
            Some(r) if !r.is_empty() => Ok(Answer::Veto(craft_plugin_abi::text::clean_notice(r))),
            _ => Err("`veto` must be a non-empty string".into()),
        };
    }
    match object.get("rewrite") {
        None => Ok(Answer::Pass),
        Some(params @ Value::Object(_)) => Ok(Answer::Rewrite(params.clone())),
        Some(_) => Err("`rewrite` must be an object".into()),
    }
}

// ---- after and change: delivery threads -----------------------------------------------------

/// Where a `change` hook reads. The bus is generic over the snapshot, so this hides it.
trait ChangeSource: Send + Sync {
    /// Requests for every change published since the last call, oldest first.
    fn drain(&self) -> Vec<Value>;
    fn close(&self);
}

struct BusSource<D> {
    bus: Arc<ChangeBus<D>>,
    subscriber: u64,
}

impl<D: Clone + Send + Sync + 'static> ChangeSource for BusSource<D> {
    fn drain(&self) -> Vec<Value> {
        self.bus.drain(self.subscriber).iter().map(change_request).collect()
    }

    fn close(&self) {
        self.bus.unsubscribe(self.subscriber);
    }
}

#[derive(Default)]
struct Queue {
    items: VecDeque<Value>,
    lost: u64,
    in_flight: bool,
    closed: bool,
}

impl Queue {
    fn push(&mut self, request: Value) {
        if self.items.len() == QUEUE_CAPACITY {
            self.items.pop_front();
            self.lost = self.lost.saturating_add(1);
        }
        self.items.push_back(request);
    }

    /// The next request to deliver. A lost count goes first, as one item.
    fn pop(&mut self) -> Option<Value> {
        if self.lost > 0 {
            let n = std::mem::take(&mut self.lost);
            return Some(json!({ "events.lost": n }));
        }
        self.items.pop_front()
    }
}

/// One delivery thread: the queue, the optional change source, and the hook it calls.
struct Worker {
    queue: Mutex<Queue>,
    wake: Condvar,
    source: Option<Box<dyn ChangeSource>>,
    ctx: HookCtx,
}

impl Worker {
    fn push(&self, request: Value) {
        let mut q = lock(&self.queue);
        if q.closed {
            return;
        }
        q.push(request);
        drop(q);
        self.wake.notify_all();
    }

    /// Move the change bus into the queue. The bus keeps no lost count, so a flood is cut here.
    fn pull(&self) {
        let Some(source) = &self.source else {
            return;
        };
        let batch = source.drain();
        if batch.is_empty() {
            return;
        }
        let mut q = lock(&self.queue);
        if q.closed {
            return;
        }
        for request in batch {
            q.push(request);
        }
    }

    fn close(&self) {
        lock(&self.queue).closed = true;
        if let Some(source) = &self.source {
            source.close();
        }
        self.wake.notify_all();
    }

    fn run(&self) {
        loop {
            self.pull();
            let next = {
                let mut q = lock(&self.queue);
                if q.closed {
                    return;
                }
                match q.pop() {
                    Some(request) => {
                        q.in_flight = true;
                        Some(request)
                    }
                    None => {
                        // Sleep until a push, or until it is time to look at the change bus.
                        let _wait = match &self.source {
                            Some(_) => self.wake.wait_timeout(q, CHANGE_POLL).unwrap_or_else(std::sync::PoisonError::into_inner).0,
                            None => self.wake.wait(q).unwrap_or_else(std::sync::PoisonError::into_inner),
                        };
                        None
                    }
                }
            };
            let Some(request) = next else {
                continue;
            };
            if let Err(e) = self.ctx.invoke(&request, CallOpts { deadline: None, quiet: true, no_wait: false }) {
                self.ctx.reporter.failed(&e);
            }
            lock(&self.queue).in_flight = false;
            self.wake.notify_all();
        }
    }

    /// Wait until nothing is queued and nothing runs, or `timeout` passes.
    fn wait_idle(&self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            self.pull();
            let q = lock(&self.queue);
            if q.items.is_empty() && q.lost == 0 && !q.in_flight {
                return true;
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return false;
            }
            let slice = left.min(Duration::from_millis(5));
            drop(self.wake.wait_timeout(q, slice).unwrap_or_else(std::sync::PoisonError::into_inner));
        }
    }
}

/// An `after` hook as a gate whose only job is to queue the call.
struct AfterTap<S> {
    interest: Interest,
    worker: Arc<Worker>,
    _session: PhantomData<fn(&S)>,
}

impl<S: 'static> Interceptor<S> for AfterTap<S> {
    fn interest(&self) -> &Interest {
        &self.interest
    }

    fn after(&self, call: &CallInfo, outcome: &Outcome) {
        self.worker.push(after_request(call, outcome));
    }
}

// ---- Registration ---------------------------------------------------------------------------

/// A hook contribution that was not registered, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedHook {
    pub contribution: String,
    pub reason: String,
}

/// What [`HookRegistrar::register`] did with one plugin generation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookRegistration {
    /// Contribution ids that are live.
    pub registered: Vec<String>,
    /// Hooks the host left out.
    pub skipped: Vec<SkippedHook>,
}

/// What `PluginManager` needs from the hook layer. [`HookHost`] implements it.
pub trait HookRegistrar: Send + Sync {
    /// Register every `hook` contribution of `manifest` that `grants` allow.
    fn register(&self, owner: &Owner, manifest: &Manifest, grants: &[Permission]) -> HookRegistration;
    /// Remove the hooks of `owner`. Returns how many were live.
    fn unregister(&self, owner: &Owner) -> usize;
}

/// The permission a hook phase needs.
pub fn required_permission(phase: Phase) -> Permission {
    match phase {
        Phase::Before => Permission::CommandsIntercept,
        Phase::After | Phase::Change => Permission::CommandsObserve,
    }
}

/// What the user hears when the command layer switches a gate off.
struct GateLabel {
    owner: Owner,
    plugin_name: String,
    contribution: String,
    consequence: &'static str,
}

type Labels = Arc<Mutex<HashMap<u64, GateLabel>>>;

/// Tell the user about a gate the command layer switched off. A slow call (`CallOverrun`) is left
/// out: the hook reported it with its own measured time before the command layer acted.
fn announce(services: &dyn Services, labels: &Labels, event: &GateDisabled) {
    let (plugin_name, contribution, consequence) = match lock(labels).get(&event.seq) {
        Some(label) => (label.plugin_name.clone(), label.contribution.clone(), label.consequence),
        None => return,
    };
    let why = match event.cause {
        DisableCause::CallOverrun => return,
        DisableCause::Panic => format!("failed: {}", event.reason),
        DisableCause::FrameBudget => "used more than its plugin's share of the hook time in one frame".to_owned(),
        DisableCause::FrameExhausted => "found no time left in this frame, because the hooks of the same plugin used it up".to_owned(),
    };
    services.notify(UserNotice::new(
        event.owner.plugin_id.to_string(),
        Some(contribution.clone()),
        &format!("The {plugin_name} hook {contribution} {why}. The app switches it off for this session. {consequence}"),
    ));
}

/// Registers plugin hooks with one app's [`Hooks`]. `S` is the app session type and `D` is the
/// snapshot type of its change bus.
pub struct HookHost<S, D> {
    hooks: Hooks<S>,
    router: Arc<Router>,
    services: Arc<dyn Services>,
    clock: Arc<dyn Clock>,
    labels: Labels,
    live: Mutex<HashMap<Owner, Vec<Arc<Worker>>>>,
    next_gate: AtomicU64,
    _snapshot: PhantomData<fn() -> D>,
}

impl<S: 'static, D: Clone + Send + Sync + 'static> HookHost<S, D> {
    /// `clock` times `before` calls. Pass the clock the plugin manager uses.
    ///
    /// Takes the disable listener of `hooks`, so a gate the command layer switches off (frame
    /// budget, panic) reaches the user through `Services::notify`, once per gate.
    pub fn new(hooks: Hooks<S>, router: Arc<Router>, services: Arc<dyn Services>, clock: Arc<dyn Clock>) -> Arc<HookHost<S, D>> {
        let labels: Labels = Arc::new(Mutex::new(HashMap::new()));
        let (sink, known) = (Arc::clone(&services), Arc::clone(&labels));
        hooks.set_disable_listener(Arc::new(move |event: &GateDisabled| announce(&*sink, &known, event)));
        Arc::new(HookHost {
            hooks,
            router,
            services,
            clock,
            labels,
            live: Mutex::new(HashMap::new()),
            next_gate: AtomicU64::new(1),
            _snapshot: PhantomData,
        })
    }

    fn label(&self, seq: u64, owner: &Owner, manifest: &Manifest, c: &Contribution, consequence: &'static str) {
        let label = GateLabel { owner: owner.clone(), plugin_name: manifest.name.clone(), contribution: c.id.clone(), consequence };
        lock(&self.labels).insert(seq, label);
    }

    pub fn hooks(&self) -> &Hooks<S> {
        &self.hooks
    }

    /// Wait until every `after` and `change` hook has delivered what it was given. For tests and
    /// for an app that shuts down. False when `timeout` passed first.
    pub fn flush(&self, timeout: Duration) -> bool {
        let workers: Vec<Arc<Worker>> = lock(&self.live).values().flatten().cloned().collect();
        workers.iter().all(|w| w.wait_idle(timeout))
    }

    fn spawn(&self, worker: &Arc<Worker>, name: String) -> Result<(), String> {
        let w = Arc::clone(worker);
        std::thread::Builder::new().name(name).spawn(move || w.run()).map(drop).map_err(|e| format!("could not start the delivery thread: {e}"))
    }

    fn register_one(&self, owner: &Owner, manifest: &Manifest, c: &Arc<Contribution>, phase: Phase) -> Result<Option<Arc<Worker>>, String> {
        let ctx = |consequence| HookCtx::new(owner, manifest, c, &self.router, &self.services, &self.clock, consequence);
        match phase {
            Phase::Before => {
                let interest = Interest { globs: c.match_globs.clone().unwrap_or_default(), priority: PLUGIN_PRIORITY };
                let gate = BeforeGate::<S> {
                    id: self.next_gate.fetch_add(1, Ordering::Relaxed),
                    interest,
                    ctx: ctx("The edit was not blocked."),
                    _session: PhantomData,
                };
                let seq = self.hooks.register(owner, Arc::new(gate));
                self.label(seq, owner, manifest, c, "The edit was not blocked.");
                Ok(None)
            }
            Phase::After => {
                let worker = Arc::new(Worker {
                    queue: Mutex::new(Queue::default()),
                    wake: Condvar::new(),
                    source: None,
                    ctx: ctx("Later events still arrive until it fails again."),
                });
                self.spawn(&worker, format!("hook-after-{}-{}", owner.plugin_id, c.id))?;
                let globs = c.match_globs.clone().unwrap_or_else(|| vec!["*".to_owned()]);
                let tap =
                    AfterTap::<S> { interest: Interest { globs, priority: PLUGIN_PRIORITY }, worker: Arc::clone(&worker), _session: PhantomData };
                let seq = self.hooks.register(owner, Arc::new(tap));
                self.label(seq, owner, manifest, c, "It gets no more events this session.");
                Ok(Some(worker))
            }
            Phase::Change => {
                let bus = self.hooks.change_bus::<D>();
                let subscriber = bus.subscribe();
                let source: Box<dyn ChangeSource> = Box::new(BusSource { bus, subscriber });
                let worker = Arc::new(Worker {
                    queue: Mutex::new(Queue::default()),
                    wake: Condvar::new(),
                    source: Some(source),
                    ctx: ctx("Later changes still arrive until it fails again."),
                });
                if let Err(e) = self.spawn(&worker, format!("hook-change-{}-{}", owner.plugin_id, c.id)) {
                    worker.close();
                    return Err(e);
                }
                Ok(Some(worker))
            }
        }
    }
}

impl<S: 'static, D: Clone + Send + Sync + 'static> HookRegistrar for HookHost<S, D> {
    fn register(&self, owner: &Owner, manifest: &Manifest, grants: &[Permission]) -> HookRegistration {
        let mut out = HookRegistration::default();
        let mut workers = Vec::new();
        for c in manifest.contributions.iter().filter(|c| c.kind == Kind::Hook) {
            // Manifest validation guarantees a phase on a hook. A manifest that skipped it is left out.
            let Some(phase) = c.phase else {
                out.skipped.push(SkippedHook { contribution: c.id.clone(), reason: "the hook has no phase".into() });
                continue;
            };
            let needed = required_permission(phase);
            if !grants.contains(&needed) {
                out.skipped.push(SkippedHook { contribution: c.id.clone(), reason: format!("missing permission {needed}") });
                continue;
            }
            match self.register_one(owner, manifest, &Arc::new(c.clone()), phase) {
                Ok(worker) => {
                    workers.extend(worker);
                    out.registered.push(c.id.clone());
                }
                Err(reason) => out.skipped.push(SkippedHook { contribution: c.id.clone(), reason }),
            }
        }
        if !workers.is_empty() {
            lock(&self.live).entry(owner.clone()).or_default().extend(workers);
        }
        out
    }

    fn unregister(&self, owner: &Owner) -> usize {
        let workers = lock(&self.live).remove(owner).unwrap_or_default();
        for w in &workers {
            w.close();
        }
        lock(&self.labels).retain(|_, label| &label.owner != owner);
        self.hooks.unregister_owner(owner) + workers.iter().filter(|w| w.source.is_some()).count()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_veto_reason_is_one_safe_short_line() {
        let hostile = format!("no\n[ERROR] forged\u{202E}{}", "y".repeat(100_000));
        let Ok(Answer::Veto(reason)) = parse_verdict(&json!({ "veto": hostile })) else { panic!("expected a veto") };
        assert!(reason.starts_with("no [ERROR] forged"), "{reason:?}");
        assert!(!reason.chars().any(craft_plugin_abi::text::is_unsafe_char));
        assert_eq!(reason.chars().count(), craft_plugin_abi::text::MAX_NOTICE_CHARS + 3);
    }
}
