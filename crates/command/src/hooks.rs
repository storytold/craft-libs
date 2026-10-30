//! Sync command gates. `before` and `after` share one snapshot so unregister during `before` still runs `after`.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::Owner;
use crate::change::{Cause, Change, ChangeBus};
use crate::mutex_guard;
use crate::observe::ObserverQueue;
use crate::tracked::Tracked;

const CALL_LIMIT_MS: u64 = 2;
const FRAME_LIMIT_MS: u64 = 6;
/// Length of one gate budget window. The window restarts from the injected `Clock`, so an app does not have to call `begin_frame`.
const FRAME_WINDOW_MS: u64 = 16;

/// Who invoked the command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    Execute,
    Ui,
    Shortcut,
    Control,
    Mcp,
    Script,
    /// A plugin ran the command through `cmd.execute`.
    Plugin,
}

/// Which command ids a gate wants, and its order key.
///
/// Matching uses the id only. Params are not cloned to test interest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interest {
    pub globs: Vec<String>,
    pub priority: i32,
}

impl Interest {
    pub fn matches(&self, id: &str) -> bool {
        self.globs.iter().any(|pattern| glob_matches(pattern, id))
    }
}

/// `*` matches every id. A pattern that ends in `*` matches that prefix. Anything else is an exact id.
fn glob_matches(pattern: &str, id: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return id.starts_with(prefix);
    }
    pattern == id
}

/// What a `before` gate decided.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Pass,
    Veto { reason: String, by: String },
    Rewrite(Value),
}

/// Class of result passed to `after`. The app picks this. The hook layer does not inspect the command `Result`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Outcome {
    Ok,
    Err,
    Vetoed,
    Disabled,
    Panicked,
}

/// One command attempt. Gates may rewrite `params`.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub id: Arc<str>,
    pub params: Value,
    pub depth: u32,
    pub origin: Origin,
    pub seq: u64,
}

/// `Call` without params, so `after` does not need the value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallInfo {
    pub id: Arc<str>,
    pub depth: u32,
    pub origin: Origin,
    pub seq: u64,
}

impl Call {
    pub fn info(&self) -> CallInfo {
        CallInfo { id: Arc::clone(&self.id), depth: self.depth, origin: self.origin, seq: self.seq }
    }
}

/// Sync gate. `before` may veto or rewrite. `after` observes.
pub trait Interceptor<S>: Send + Sync {
    fn interest(&self) -> &Interest;

    /// `session` is the app session, borrowed mutably so a gate can lend it to code that must reach
    /// it during the call (a plugin host call such as `cmd.execute` or `doc.query`).
    fn before(&self, _session: &mut S, _call: &mut Call) -> Verdict {
        Verdict::Pass
    }

    fn after(&self, _call: &CallInfo, _out: &Outcome) {}

    /// True for a gate the host trusts and relies on for safety. The command layer never
    /// switches it off for time or a call overrun, and a panic in it vetoes the command (fail
    /// closed) instead of passing it. Only host code may return true: the plugin host's gates keep
    /// the default. The cost: a slow security gate slows the UI, and a gate that always panics
    /// blocks every command it matches.
    fn security(&self) -> bool {
        false
    }
}

/// Why the command layer switched a gate off for the rest of the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DisableCause {
    /// The gate panicked.
    Panic,
    /// One call took longer than 2 ms. The gate's owner knows its own measurement.
    CallOverrun,
    /// The call pushed its owner's share of the 16 ms window past 6 ms.
    FrameBudget,
    /// The owner's share of the window was already past 6 ms when the gate's turn came. The gate did
    /// not run. Only the gate's own owner has spent that time.
    FrameExhausted,
}

/// One gate the command layer switched off. Sent once per gate to the listener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateDisabled {
    pub seq: u64,
    pub owner: Owner,
    pub cause: DisableCause,
    pub reason: String,
}

/// Receives each [`GateDisabled`]. It runs on the thread that called `before` or `after`, with no
/// hook lock held.
pub type DisableListener = Arc<dyn Fn(&GateDisabled) + Send + Sync>;

/// Time source injected by the app. The crate does not call `Instant::now`.
pub trait Clock {
    fn now_ms(&self) -> u64;
}

struct Entry<S> {
    seq: u64,
    owner: Owner,
    priority: i32,
    interceptor: Arc<dyn Interceptor<S> + Send + Sync>,
}

impl<S> Clone for Entry<S> {
    fn clone(&self) -> Self {
        Self { seq: self.seq, owner: self.owner.clone(), priority: self.priority, interceptor: Arc::clone(&self.interceptor) }
    }
}

struct State<S> {
    next_seq: u64,
    /// Sorted by `(priority, seq)` ascending.
    live: Arc<[Entry<S>]>,
    /// One snapshot per unpaired `before`, so nested calls keep their own list.
    inflight: Vec<Arc<[Entry<S>]>>,
    disabled: Vec<(u64, String)>,
    depth: u32,
    /// Gate time spent in the current window, per plugin id. One owner cannot spend another's share.
    frame_ms: HashMap<Arc<str>, u64>,
    /// Clock time at which the current budget window began. `None` until the first gate runs or after `begin_frame`.
    window_start_ms: Option<u64>,
    causes: Vec<Cause>,
}

impl<S> State<S> {
    /// Start a new budget window when none is open or the open one is `FRAME_WINDOW_MS` old.
    fn roll_window(&mut self, now: u64) {
        match self.window_start_ms {
            Some(start) if now.saturating_sub(start) < FRAME_WINDOW_MS => {}
            _ => {
                self.window_start_ms = Some(now);
                self.frame_ms.clear();
            }
        }
    }
}

struct Core<S> {
    state: Mutex<State<S>>,
    live_count: AtomicUsize,
    inflight_depth: AtomicUsize,
    /// Change subscribers across every bus this `Hooks` created. Shared with each `ChangeBus`.
    change_subs: Arc<AtomicUsize>,
    observers: ObserverQueue,
    buses: Mutex<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
    listener: Mutex<Option<DisableListener>>,
}

enum Admit {
    Run,
    Skip,
}

enum GateStop {
    Continue,
    Rewritten,
    Veto { reason: String, by: String },
}

/// Interceptors, observer queue, change bus, and the cause stack.
///
/// Methods take `&self`. The session can hold `Hooks<S>` and pass `&S` into `before` at the same time.
/// `before` and `after` are a stack. Pair them on one thread.
pub struct Hooks<S> {
    core: Arc<Core<S>>,
}

impl<S> Clone for Hooks<S> {
    fn clone(&self) -> Self {
        Self { core: Arc::clone(&self.core) }
    }
}

impl<S> Default for Hooks<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Hooks<S> {
    pub fn new() -> Self {
        Self {
            core: Arc::new(Core {
                state: Mutex::new(State {
                    next_seq: 0,
                    live: Arc::from(Vec::<Entry<S>>::new()),
                    inflight: Vec::new(),
                    disabled: Vec::new(),
                    depth: 0,
                    frame_ms: HashMap::new(),
                    window_start_ms: None,
                    causes: Vec::new(),
                }),
                live_count: AtomicUsize::new(0),
                inflight_depth: AtomicUsize::new(0),
                change_subs: Arc::new(AtomicUsize::new(0)),
                observers: ObserverQueue::new(),
                buses: Mutex::new(HashMap::new()),
                listener: Mutex::new(None),
            }),
        }
    }

    pub fn observer_queue(&self) -> &ObserverQueue {
        &self.core.observers
    }

    /// Register a gate. The returned seq is monotonic and is the tie break after `priority`.
    pub fn register(&self, owner: &Owner, interceptor: Arc<dyn Interceptor<S> + Send + Sync>) -> u64 {
        let priority = interceptor.interest().priority;
        let mut state = self.lock();
        let seq = state.next_seq;
        state.next_seq = state.next_seq.saturating_add(1);
        let entry = Entry { seq, owner: owner.clone(), priority, interceptor };
        let mut entries: Vec<Entry<S>> = state.live.iter().cloned().collect();
        let pos = match entries.binary_search_by(|item| (item.priority, item.seq).cmp(&(entry.priority, entry.seq))) {
            Ok(index) | Err(index) => index,
        };
        entries.insert(pos, entry);
        self.store_live(&mut state, entries);
        seq
    }

    pub fn unregister_owner(&self, owner: &Owner) -> usize {
        let mut state = self.lock();
        let before = state.live.len();
        let entries: Vec<Entry<S>> = state.live.iter().filter(|entry| &entry.owner != owner).cloned().collect();
        let removed = before - entries.len();
        if removed > 0 {
            self.store_live(&mut state, entries);
        }
        removed
    }

    /// Tell `listener` about every gate switched off from now on, once per gate. A second call
    /// replaces the first listener.
    pub fn set_disable_listener(&self, listener: DisableListener) {
        *mutex_guard(&self.core.listener) = Some(listener);
    }

    /// Why a gate was removed for the rest of the session, if it was.
    pub fn disable_reason(&self, seq: u64) -> Option<String> {
        self.lock().disabled.iter().find(|(id, _)| *id == seq).map(|(_, reason)| reason.clone())
    }

    /// True when nothing can observe a call: no gate, no call in flight, no observer subscriber and
    /// no change subscriber. An app may then skip `with_cause`, `before`, `enter`, `leave` and `after`
    /// for that call.
    ///
    /// The answer is a snapshot. A gate or subscriber added while an idle call runs sees only later calls.
    pub fn is_idle(&self) -> bool {
        self.core.live_count.load(Ordering::Acquire) == 0
            && self.core.inflight_depth.load(Ordering::Acquire) == 0
            && !self.core.observers.has_subscribers()
            && self.core.change_subs.load(Ordering::Acquire) == 0
    }

    /// Optional hint: start a fresh gate budget window now. Disabled gates stay disabled.
    ///
    /// The budget also restarts by itself every 16 ms of `Clock` time, so apps need not call this.
    pub fn begin_frame(&self) {
        let mut state = self.lock();
        state.frame_ms.clear();
        state.window_start_ms = None;
    }

    /// Run matching `before` gates. The snapshot is fixed at the start of this call.
    ///
    /// A veto stops later `before` gates. `after` still sees the whole snapshot.
    /// A panic, a call over 2 ms, or a call that pushes its owner's share of the 16 ms window past
    /// 6 ms disables that gate, drops its veto or rewrite, and the chain continues as `Pass`. A gate
    /// whose owner has already spent more than 6 ms of the window does not run. It is disabled too.
    /// Time is charged per plugin id, so one owner cannot disable another owner's gates.
    /// The chain fails open, except for a [`Interceptor::security`] gate: it is never timed out and a
    /// panic in it vetoes the command.
    pub fn before(&self, session: &mut S, call: &mut Call, clock: &dyn Clock) -> Verdict {
        if self.core.live_count.load(Ordering::Acquire) == 0 && self.core.inflight_depth.load(Ordering::Acquire) == 0 {
            return Verdict::Pass;
        }
        let snapshot = self.push_snapshot();
        if snapshot.is_empty() {
            return Verdict::Pass;
        }
        let mut rewritten = false;
        for entry in snapshot.iter() {
            if !entry.interceptor.interest().matches(call.id.as_ref()) {
                continue;
            }
            let now = clock.now_ms();
            match self.admit(entry, now) {
                Admit::Run => {}
                Admit::Skip => continue,
            }
            match self.run_before_gate(session, call, entry, clock, now) {
                GateStop::Continue => {}
                GateStop::Rewritten => rewritten = true,
                GateStop::Veto { reason, by } => return Verdict::Veto { reason, by },
            }
        }
        if rewritten { Verdict::Rewrite(call.params.clone()) } else { Verdict::Pass }
    }

    /// Run the snapshot taken by the matching `before`, highest `(priority, seq)` first.
    ///
    /// A gate removed during `before` is still in that snapshot. The next `before` does not see it.
    ///
    /// An `after` gate whose owner has used more than 6 ms of the window misses this `after`. It is
    /// not disabled, because it did not overrun.
    pub fn after(&self, call: &CallInfo, outcome: &Outcome, clock: &dyn Clock) {
        if self.core.live_count.load(Ordering::Acquire) == 0 && self.core.inflight_depth.load(Ordering::Acquire) == 0 {
            return;
        }
        let Some(snapshot) = self.pop_snapshot() else {
            return;
        };
        for entry in snapshot.iter().rev() {
            if !entry.interceptor.interest().matches(call.id.as_ref()) {
                continue;
            }
            let now = clock.now_ms();
            if !self.frame_open(&entry.owner, now) {
                continue;
            }
            self.run_after_gate(call, outcome, entry, clock, now);
        }
    }

    pub fn enter(&self) -> u32 {
        let mut state = self.lock();
        state.depth = state.depth.saturating_add(1);
        state.depth
    }

    pub fn leave(&self) -> u32 {
        let mut state = self.lock();
        state.depth = state.depth.saturating_sub(1);
        state.depth
    }

    pub fn depth(&self) -> u32 {
        self.lock().depth
    }

    /// Push `cause` for the body, then restore the previous cause, including on panic.
    pub fn with_cause<R>(&self, cause: Cause, body: impl FnOnce() -> R) -> R {
        self.lock().causes.push(cause);
        let _guard = CauseGuard { hooks: self };
        body()
    }

    pub fn current_cause(&self) -> Cause {
        self.lock().causes.last().cloned().unwrap_or(Cause::Untracked)
    }

    /// Publish one change on the bus for `D`. `prev` is `None`. Callers with a previous snapshot use [`ChangeBus::publish`].
    pub fn changed<D>(&self, revision: u64, cause: Cause, merged: bool, snapshot: D)
    where
        D: Clone + Send + Sync + 'static,
    {
        self.change_bus::<D>().publish(Change { revision, cause, merged, snapshot, prev: None });
    }

    /// Emit one `Untracked` change when `tracked` is dirty, then clear the flag. A clean value emits nothing.
    pub fn flush<D>(&self, tracked: &mut Tracked<D>, revision: u64)
    where
        D: Clone + Send + Sync + 'static,
    {
        if !tracked.is_dirty() {
            return;
        }
        let snapshot = tracked.clone_value();
        self.changed(revision, Cause::Untracked, false, snapshot);
        tracked.mark_bumped();
    }

    /// Bus for snapshot type `D`. The first call creates it. Later calls return the same bus.
    pub fn change_bus<D>(&self) -> Arc<ChangeBus<D>>
    where
        D: Clone + Send + Sync + 'static,
    {
        let mut buses = mutex_guard(&self.core.buses);
        let key = TypeId::of::<D>();
        if let Some(existing) = buses.get(&key)
            && let Ok(bus) = Arc::clone(existing).downcast::<ChangeBus<D>>()
        {
            return bus;
        }
        let bus = Arc::new(ChangeBus::<D>::with_counter(Arc::clone(&self.core.change_subs)));
        buses.insert(key, Arc::clone(&bus) as Arc<dyn Any + Send + Sync>);
        bus
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State<S>> {
        mutex_guard(&self.core.state)
    }

    fn store_live(&self, state: &mut State<S>, entries: Vec<Entry<S>>) {
        let count = entries.len();
        state.live = Arc::from(entries);
        self.core.live_count.store(count, Ordering::Release);
    }

    fn push_snapshot(&self) -> Arc<[Entry<S>]> {
        let mut state = self.lock();
        let snapshot = Arc::clone(&state.live);
        state.inflight.push(Arc::clone(&snapshot));
        self.core.inflight_depth.fetch_add(1, Ordering::Release);
        snapshot
    }

    fn pop_snapshot(&self) -> Option<Arc<[Entry<S>]>> {
        let mut state = self.lock();
        let snapshot = state.inflight.pop();
        if snapshot.is_some() {
            self.core.inflight_depth.fetch_sub(1, Ordering::Release);
        }
        snapshot
    }

    /// Roll the budget window forward to `now`, then say whether it still has room.
    fn frame_open(&self, owner: &Owner, now: u64) -> bool {
        let mut state = self.lock();
        state.roll_window(now);
        state.frame_ms.get(&owner.plugin_id).copied().unwrap_or(0) <= FRAME_LIMIT_MS
    }

    /// Decide whether a `before` gate may run at `now`. Rolls the budget window forward first.
    ///
    /// A window already past the frame limit shuts the gate out: it is disabled for the
    /// session and announced, never skipped silently. A gate switched off while this snapshot was in
    /// flight does not run either.
    fn admit(&self, entry: &Entry<S>, now: u64) -> Admit {
        let used = {
            let mut state = self.lock();
            state.roll_window(now);
            if state.disabled.iter().any(|(id, _)| *id == entry.seq) {
                return Admit::Skip;
            }
            // A security gate always runs. Its time is not charged.
            let spent = state.frame_ms.get(&entry.owner.plugin_id).copied().unwrap_or(0);
            if spent <= FRAME_LIMIT_MS || entry.interceptor.security() {
                return Admit::Run;
            }
            spent
        };
        self.disable(
            entry,
            DisableCause::FrameExhausted,
            format!("frame budget exhausted ({used} ms of {FRAME_LIMIT_MS} ms used before this gate could run)"),
        );
        Admit::Skip
    }

    /// Charge `delta` against the frame. `Some` means this gate must be disabled and its verdict dropped.
    fn budget_break(&self, entry: &Entry<S>, delta: u64) -> Option<(DisableCause, String)> {
        if entry.interceptor.security() {
            return None;
        }
        let mut state = self.lock();
        let spent = state.frame_ms.entry(Arc::clone(&entry.owner.plugin_id)).or_insert(0);
        let next = spent.saturating_add(delta);
        *spent = next;
        if delta > CALL_LIMIT_MS {
            return Some((DisableCause::CallOverrun, format!("clock delta {delta} ms exceeds 2 ms")));
        }
        if next > FRAME_LIMIT_MS {
            return Some((DisableCause::FrameBudget, format!("frame budget exceeded ({next} ms)")));
        }
        None
    }

    /// Switch `entry` off for the session and tell the listener, once per gate.
    fn disable(&self, entry: &Entry<S>, cause: DisableCause, reason: String) {
        let seq = entry.seq;
        let first = {
            let mut state = self.lock();
            let entries: Vec<Entry<S>> = state.live.iter().filter(|live| live.seq != seq).cloned().collect();
            if entries.len() != state.live.len() {
                self.store_live(&mut state, entries);
            }
            if state.disabled.iter().all(|(id, _)| *id != seq) {
                state.disabled.push((seq, reason.clone()));
                true
            } else {
                false
            }
        };
        if !first {
            return;
        }
        let listener = mutex_guard(&self.core.listener).clone();
        if let Some(listener) = listener {
            listener(&GateDisabled { seq, owner: entry.owner.clone(), cause, reason });
        }
    }

    fn run_before_gate(&self, session: &mut S, call: &mut Call, entry: &Entry<S>, clock: &dyn Clock, started: u64) -> GateStop {
        let interceptor = Arc::clone(&entry.interceptor);
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| interceptor.before(session, call)));
        let delta = clock.now_ms().saturating_sub(started);
        let break_reason = self.budget_break(entry, delta);
        match result {
            Err(payload) if entry.interceptor.security() => {
                // Fail closed: a host gate that cannot answer does not let the command through.
                GateStop::Veto { reason: format!("security gate failed: {}", panic_message(payload)), by: entry.owner.plugin_id.to_string() }
            }
            Err(payload) => {
                self.disable(entry, DisableCause::Panic, panic_message(payload));
                GateStop::Continue
            }
            Ok(verdict) => {
                if let Some((cause, reason)) = break_reason {
                    self.disable(entry, cause, reason);
                    GateStop::Continue
                } else {
                    match verdict {
                        Verdict::Pass => GateStop::Continue,
                        Verdict::Rewrite(value) => {
                            call.params = value;
                            GateStop::Rewritten
                        }
                        Verdict::Veto { reason, by } => GateStop::Veto { reason, by },
                    }
                }
            }
        }
    }

    fn run_after_gate(&self, call: &CallInfo, outcome: &Outcome, entry: &Entry<S>, clock: &dyn Clock, started: u64) {
        let interceptor = Arc::clone(&entry.interceptor);
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| interceptor.after(call, outcome)));
        let delta = clock.now_ms().saturating_sub(started);
        let break_reason = self.budget_break(entry, delta);
        match result {
            Err(_) if entry.interceptor.security() => {}
            Err(payload) => self.disable(entry, DisableCause::Panic, panic_message(payload)),
            Ok(()) => {
                if let Some((cause, reason)) = break_reason {
                    self.disable(entry, cause, reason);
                }
            }
        }
    }
}

fn panic_message(payload: Box<dyn Any + Send>) -> String {
    let detail = if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown".to_owned()
    };
    format!("panic: {detail}")
}

struct CauseGuard<'a, S> {
    hooks: &'a Hooks<S>,
}

impl<S> Drop for CauseGuard<'_, S> {
    fn drop(&mut self) {
        self.hooks.lock().causes.pop();
    }
}
