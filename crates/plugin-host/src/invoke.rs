//! One loaded generation of a plugin and the guest call path.
//!
//! A [`Generation`] owns a compiled module, the grants that were in force when it loaded, and
//! (for `instance: session` contributions) one long lived instance. [`Generation::call`] is the
//! single place that moves bytes into guest memory, runs `craft_invoke`, and reads the answer.
//! It also keeps the call lifecycle promises: `begin_call` and `end_call` always pair, a fault
//! reaches the user, repeated faults open a circuit, and a faulted session instance is dropped.

use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use craft_command::Owner;
use craft_plugin_abi::ProcessDescriptor;
use craft_plugin_abi::manifest::{Contribution, Instance as InstanceKind, Manifest};
use craft_plugin_abi::ops::unpack_invoke_return;
use craft_plugin_abi::{Op, Permission};
use serde_json::Value;

use crate::budget::{BudgetClass, BudgetConfig, FaultTracker, Limits, is_fault};
use crate::dispatch::{DispatchConfig, HostDispatcher, PluginContext};
use crate::engine::{BoxFuture, Budget, Clock, EngineId, Export, Instance, LoadError, Module, Trap};
use crate::services::{CallOutcome, Services, UserNotice};
use crate::store::StateStore;

/// Largest JSON response the host reads from a guest.
pub const MAX_RESPONSE_BYTES: u32 = 16 * 1024 * 1024;
/// `craft_state_save` budget.
pub const STATE_SAVE_DEADLINE: Duration = Duration::from_millis(250);

/// Why a guest call produced no value.
#[derive(Clone, Debug, PartialEq)]
pub enum CallError {
    /// Repeated faults opened the circuit for this contribution.
    CircuitOpen,
    /// The engine stopped the guest: budget, trap, or bad pointer.
    Trap(Trap),
    /// The module would not instantiate.
    Instantiate(LoadError),
    /// The engine did not finish the future on the first poll. The control plane needs `Caps::SYNC_CALL`.
    EngineNotSync,
    /// `craft_alloc` returned 0 or the request was too large to address.
    GuestAlloc,
    /// `craft_invoke` returned a negative status.
    GuestStatus(i64),
    /// The response was not JSON or was too large.
    BadResponse(String),
    /// The plugin answered `{"error": message}`. Not a fault.
    Plugin(String),
    /// The plugin has no active generation: it was disabled, removed, or failed to load.
    NotLoaded,
    /// `CallOpts::no_wait` was set and the session instance was in use. Nothing ran.
    Busy,
    /// The contribution cannot run: unknown kind, or an entry index that does not fit `i32`.
    NotCallable(String),
    /// The caller's job is wrong: parameters that break the schema, a pixel format the contribution
    /// does not take, a buffer of the wrong size. No guest code ran. Not a fault.
    BadJob(String),
    /// A v1 plugin broke a limit or its ABI. The shim names the reason. Counts toward the circuit.
    Legacy(String),
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::NotLoaded => f.write_str("the plugin is not loaded"),
            CallError::Busy => f.write_str("the plugin instance is busy with another call"),
            CallError::CircuitOpen => f.write_str("the plugin was switched off for this session after repeated faults"),
            CallError::Trap(t) => write!(f, "{t}"),
            CallError::Instantiate(e) => write!(f, "{e}"),
            CallError::EngineNotSync => f.write_str("the engine cannot run a synchronous call"),
            CallError::GuestAlloc => f.write_str("the guest could not allocate memory for the request"),
            CallError::GuestStatus(s) => write!(f, "craft_invoke returned status {s}"),
            CallError::BadResponse(m) => write!(f, "bad response: {m}"),
            CallError::Plugin(m) => f.write_str(m),
            CallError::NotCallable(m) => write!(f, "not callable: {m}"),
            CallError::BadJob(m) => write!(f, "bad job: {m}"),
            CallError::Legacy(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for CallError {}

impl CallError {
    /// True when the call counts toward the circuit: the guest or the budget broke a rule.
    /// A plugin's own `{"error"}` and a user cancel do not.
    fn is_fault(&self) -> bool {
        match self {
            CallError::Plugin(_)
            | CallError::CircuitOpen
            | CallError::NotCallable(_)
            | CallError::BadJob(_)
            | CallError::NotLoaded
            | CallError::Busy => false,
            CallError::Trap(t) => is_fault(t),
            _ => true,
        }
    }
}

/// Per-call changes to how [`Generation::call_with`] runs a guest call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallOpts {
    /// Wall deadline for the whole call. It can only tighten the budget class deadline.
    pub deadline: Option<Duration>,
    /// Do not send the user a notice for a fault. The caller reports it. Faults still count toward
    /// the circuit and still reach `Services::end_call`.
    pub quiet: bool,
    /// Do not wait for the session instance. When another call holds it the result is
    /// [`CallError::Busy`] and no guest code runs. A call on the UI thread uses this, so a slow
    /// call on a worker cannot stall a frame.
    pub no_wait: bool,
}

/// Everything a generation shares with the manager.
pub struct Shared {
    pub services: Arc<dyn Services>,
    pub state: Arc<dyn StateStore>,
    pub clock: Arc<dyn Clock>,
    pub faults: Arc<FaultTracker>,
    pub budgets: BudgetConfig,
    pub dispatch: DispatchConfig,
    pub engine: EngineId,
}

struct Runner {
    instance: Box<dyn Instance>,
    cancel: Arc<AtomicBool>,
}

/// Run a future that must finish on its first poll.
pub(crate) fn poll_ready<T>(mut fut: BoxFuture<'_, T>) -> Result<T, CallError> {
    let mut cx = Context::from_waker(Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(v) => Ok(v),
        Poll::Pending => Err(CallError::EngineNotSync),
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// FNV-1a. A fixed seed per plugin id for the deterministic `random.bytes` stream.
fn seed_of(id: &str) -> u64 {
    id.bytes().fold(0xCBF2_9CE4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3))
}

/// Wall clock accounting across the several exports one logical call makes.
struct Timer<'a> {
    clock: &'a dyn Clock,
    start: Duration,
    total: Option<Duration>,
}

impl Timer<'_> {
    /// The base budget with the deadline cut to what is left, or `Trap::Deadline` when nothing is.
    fn budget(&self, base: &Budget) -> Result<Budget, Trap> {
        let mut b = base.clone();
        if let Some(total) = self.total {
            let used = self.clock.now().saturating_sub(self.start);
            if used >= total {
                return Err(Trap::Deadline);
            }
            b.deadline = Some(total - used);
        }
        Ok(b)
    }
}

impl Runner {
    fn call(&mut self, export: Export, args: &[i32], timer: &Timer<'_>, base: &Budget) -> Result<i64, CallError> {
        let budget = timer.budget(base).map_err(CallError::Trap)?;
        self.instance.call(export, args, &budget).map_err(CallError::Trap)
    }

    /// Allocate in the guest and copy `bytes` in. Returns `(ptr, len)`; an empty slice is `(0, 0)`.
    fn put(&mut self, bytes: &[u8], timer: &Timer<'_>, base: &Budget) -> Result<(i32, i32), CallError> {
        if bytes.is_empty() {
            return Ok((0, 0));
        }
        let len = i32::try_from(bytes.len()).map_err(|_| CallError::GuestAlloc)?;
        let ptr = self.call(Export::Alloc, &[len, 1], timer, base)? as i32;
        if ptr == 0 {
            return Err(CallError::GuestAlloc);
        }
        self.instance.write(ptr as u32, bytes).map_err(CallError::Trap)?;
        Ok((ptr, len))
    }

    fn free(&mut self, buf: (i32, i32), timer: &Timer<'_>, base: &Budget) -> Result<(), CallError> {
        if buf.1 > 0 {
            self.call(Export::Free, &[buf.0, buf.1, 1], timer, base)?;
        }
        Ok(())
    }
}

/// Which generation of each plugin takes new calls. Calls already running keep their own `Arc`.
#[derive(Default)]
pub struct Router {
    current: std::sync::RwLock<std::collections::HashMap<String, Arc<Generation>>>,
}

impl Router {
    pub fn new() -> Arc<Router> {
        Arc::new(Router::default())
    }

    pub fn current(&self, plugin: &str) -> Option<Arc<Generation>> {
        self.current.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(plugin).cloned()
    }

    /// Point new calls at `generation`. Returns the one it replaced.
    pub fn route(&self, generation: Arc<Generation>) -> Option<Arc<Generation>> {
        let id = generation.owner.plugin_id.to_string();
        self.current.write().unwrap_or_else(std::sync::PoisonError::into_inner).insert(id, generation)
    }

    /// Stop routing to a plugin. Returns the generation that was current.
    pub fn unroute(&self, plugin: &str) -> Option<Arc<Generation>> {
        self.current.write().unwrap_or_else(std::sync::PoisonError::into_inner).remove(plugin)
    }
}

/// What a generation runs: a craft-abi 1 module on an [`crate::engine::Engine`], or a v1 plugin
/// behind its shim. A v1 plugin has no `craft_invoke`, so only the data plane reaches it.
#[derive(Clone)]
pub(crate) enum Backend {
    Craft(Arc<dyn Module>),
    #[cfg(feature = "legacy-v1")]
    Legacy(Arc<crate::shim::LegacyPlugin>),
}

/// One loaded version of a plugin.
pub struct Generation {
    pub owner: Owner,
    pub manifest: Arc<Manifest>,
    backend: Backend,
    grants: Vec<Permission>,
    limits: Limits,
    shared: Arc<Shared>,
    session: Mutex<Option<Runner>>,
}

impl Generation {
    pub fn new(
        owner: Owner,
        manifest: Arc<Manifest>,
        module: Arc<dyn Module>,
        grants: Vec<Permission>,
        limits: Limits,
        shared: Arc<Shared>,
    ) -> Generation {
        Generation { owner, manifest, backend: Backend::Craft(module), grants, limits, shared, session: Mutex::new(None) }
    }

    /// A generation for a v1 plugin. It holds no grants: a v1 plugin has no host ops. `limits` is
    /// what the plugin list reports for memory; the shim enforces its own v1 numbers.
    #[cfg(feature = "legacy-v1")]
    pub fn new_legacy(
        owner: Owner,
        manifest: Arc<Manifest>,
        plugin: Arc<crate::shim::LegacyPlugin>,
        limits: Limits,
        shared: Arc<Shared>,
    ) -> Generation {
        Generation { owner, manifest, backend: Backend::Legacy(plugin), grants: Vec::new(), limits, shared, session: Mutex::new(None) }
    }

    pub(crate) fn backend(&self) -> &Backend {
        &self.backend
    }

    /// The v1 plugin behind this generation, if it is one.
    #[cfg(feature = "legacy-v1")]
    pub fn legacy(&self) -> Option<&crate::shim::LegacyPlugin> {
        match &self.backend {
            Backend::Legacy(p) => Some(p),
            Backend::Craft(_) => None,
        }
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    pub fn grants(&self) -> &[Permission] {
        &self.grants
    }

    fn make_runner(&self) -> Result<Runner, CallError> {
        self.make_runner_with(Arc::new(AtomicBool::new(false)))
    }

    /// A fresh instance whose cancel flag is `cancel`. The host op `host.cancelled` and the engine
    /// budget both read it.
    fn make_runner_with(&self, cancel: Arc<AtomicBool>) -> Result<Runner, CallError> {
        // Without `wasmi` the only variant is `Craft`, and the match is irrefutable.
        #[cfg_attr(not(feature = "legacy-v1"), allow(clippy::infallible_destructuring_match))]
        let module = match &self.backend {
            Backend::Craft(m) => m,
            #[cfg(feature = "legacy-v1")]
            Backend::Legacy(_) => return Err(CallError::NotCallable("a v1 plugin has no craft-abi 1 entry points".into())),
        };
        let declared_ops: HashSet<Op> = self.manifest.requires.iter().flat_map(|r| r.ops.iter()).filter_map(|n| Op::from_name(n)).collect();
        let ctx = PluginContext {
            owner: self.owner.clone(),
            declared_ops,
            grants: self.grants.clone(),
            engine: self.shared.engine,
            random_seed: seed_of(&self.owner.plugin_id),
        };
        let dispatcher = HostDispatcher::new(
            ctx,
            Arc::clone(&self.shared.services),
            Arc::clone(&self.shared.state),
            Arc::clone(&self.shared.clock),
            Arc::clone(&cancel),
            self.shared.dispatch.clone(),
        );
        let budget = self.shared.budgets.budget(BudgetClass::Interactive, self.limits, 1, Arc::clone(&cancel));
        let instance = poll_ready(module.instantiate(dispatcher.handle(), &budget))?.map_err(CallError::Instantiate)?;
        Ok(Runner { instance, cancel })
    }

    /// Run `f` on a fresh instance, or on the session instance. A session instance that fails is dropped.
    fn with_runner<R>(&self, kind: InstanceKind, no_wait: bool, f: impl FnOnce(&mut Runner) -> Result<R, CallError>) -> Result<R, CallError> {
        match kind {
            InstanceKind::Fresh => {
                let mut runner = self.make_runner()?;
                f(&mut runner)
            }
            InstanceKind::Session => {
                let mut slot = if no_wait {
                    match self.session.try_lock() {
                        Ok(guard) => guard,
                        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
                        Err(std::sync::TryLockError::WouldBlock) => return Err(CallError::Busy),
                    }
                } else {
                    lock(&self.session)
                };
                let mut runner = match slot.take() {
                    Some(r) => r,
                    None => self.make_runner()?,
                };
                let out = f(&mut runner);
                // A guest that trapped or broke its protocol has unknown state. Keep it only on success.
                if out.as_ref().err().is_none_or(|e| !e.is_fault()) {
                    *slot = Some(runner);
                }
                out
            }
        }
    }

    /// Call a control plane contribution through `craft_invoke`. `units` sizes a `render` budget.
    pub fn call(&self, c: &Contribution, request: &Value, blob: &[u8], units: u64) -> Result<Value, CallError> {
        self.call_with(c, request, blob, units, CallOpts::default())
    }

    /// [`Generation::call`] with per-call options.
    pub fn call_with(&self, c: &Contribution, request: &Value, blob: &[u8], units: u64, opts: CallOpts) -> Result<Value, CallError> {
        self.guarded(c, opts, || self.call_inner(c, request, blob, units, opts))
    }

    /// The promises around one logical guest call: an open circuit refuses it,
    /// `begin_call` and `end_call` pair, a fault reaches the user and counts toward the circuit.
    /// `run` does the work. A data plane job with many bands is one logical call.
    pub(crate) fn guarded<T>(&self, c: &Contribution, opts: CallOpts, run: impl FnOnce() -> Result<T, CallError>) -> Result<T, CallError> {
        let plugin: &str = &self.owner.plugin_id;
        if self.shared.faults.is_open(plugin, &c.id) {
            return Err(CallError::CircuitOpen);
        }
        let services = &self.shared.services;
        let scope = services.begin_call(&self.owner, &c.id);
        let result = run();
        match &result {
            Ok(_) => {
                services.end_call(scope, &CallOutcome::Completed);
                self.shared.faults.record_success(plugin, &c.id);
            }
            Err(CallError::Trap(Trap::Cancelled) | CallError::Busy) => services.end_call(scope, &CallOutcome::Cancelled),
            Err(e) => {
                services.end_call(scope, &CallOutcome::Faulted(e.to_string()));
                if e.is_fault() {
                    let verdict = self.shared.faults.record_fault(plugin, &c.id);
                    if opts.quiet {
                        return result;
                    }
                    let name = c.label.as_deref().unwrap_or(&c.id);
                    let mut message = format!("{name} failed and its changes were undone: {e}");
                    if verdict.tripped_now {
                        message.push_str(&format!(". It stays off for this session after {} faults in a row.", verdict.consecutive));
                    }
                    services.notify(UserNotice::new(plugin, Some(c.id.clone()), &message));
                }
            }
        }
        result
    }

    /// One band through `craft_process` on a fresh instance. The caller built
    /// the descriptor except for the three guest pointers, which this method fills in after it has
    /// copied the input and the params into guest memory and reserved the output. The fresh instance
    /// is dropped afterward, so nothing is freed. Returns the output bytes, `input.len()` of them.
    ///
    /// A non-zero return from the guest is the plugin's own error: `CallError::Plugin`, no circuit count.
    pub(crate) fn run_process_band(
        &self,
        c: &Contribution,
        desc: &ProcessDescriptor,
        input: &[u8],
        params: &[u8],
        units: u64,
        cancel: Arc<AtomicBool>,
    ) -> Result<Vec<u8>, CallError> {
        let mut runner = self.make_runner_with(cancel)?;
        let base = self.shared.budgets.budget(BudgetClass::of(c), self.limits, units, Arc::clone(&runner.cancel));
        let timer = Timer { clock: &*self.shared.clock, start: self.shared.clock.now(), total: base.deadline };
        let in_buf = runner.put(input, &timer, &base)?;
        let params_buf = runner.put(params, &timer, &base)?;
        let out_len = i32::try_from(input.len()).map_err(|_| CallError::GuestAlloc)?;
        let out_ptr = runner.call(Export::Alloc, &[out_len, 1], &timer, &base)? as i32;
        if out_ptr == 0 && out_len > 0 {
            return Err(CallError::GuestAlloc);
        }
        let mut desc = desc.clone();
        desc.in_ptr = in_buf.0 as u32;
        desc.out_ptr = out_ptr as u32;
        desc.params_ptr = params_buf.0 as u32;
        desc.params_len = params_buf.1 as u32;
        let desc_buf = runner.put(&desc.encode(), &timer, &base)?;
        let rc = runner.call(Export::Process, &[desc_buf.0, desc_buf.1], &timer, &base)? as i32;
        if rc != 0 {
            return Err(CallError::Plugin(format!("craft_process returned {rc}")));
        }
        let mut out = vec![0u8; input.len()];
        runner.instance.read(out_ptr as u32, &mut out).map_err(CallError::Trap)?;
        Ok(out)
    }

    fn call_inner(&self, c: &Contribution, request: &Value, blob: &[u8], units: u64, opts: CallOpts) -> Result<Value, CallError> {
        if !c.kind.is_control_plane() {
            return Err(CallError::NotCallable(format!("kind {} does not run through craft_invoke", c.kind.name())));
        }
        let entry = i32::try_from(c.entry).map_err(|_| CallError::NotCallable(format!("entry {} does not fit i32", c.entry)))?;
        let req = serde_json::to_vec(request).map_err(|e| CallError::BadResponse(format!("request does not serialize: {e}")))?;
        self.with_runner(c.instance(), opts.no_wait, |runner| {
            runner.cancel.store(false, Ordering::Relaxed);
            let mut base = self.shared.budgets.budget(BudgetClass::of(c), self.limits, units, Arc::clone(&runner.cancel));
            if let Some(limit) = opts.deadline {
                base.deadline = Some(base.deadline.map_or(limit, |d| d.min(limit)));
            }
            let timer = Timer { clock: &*self.shared.clock, start: self.shared.clock.now(), total: base.deadline };
            let req_buf = runner.put(&req, &timer, &base)?;
            let blob_buf = runner.put(blob, &timer, &base)?;
            let ret = runner.call(Export::Invoke, &[entry, req_buf.0, req_buf.1, blob_buf.0, blob_buf.1], &timer, &base)?;
            runner.free(req_buf, &timer, &base)?;
            runner.free(blob_buf, &timer, &base)?;
            read_response(runner, ret, &timer, &base)
        })
    }

    /// `craft_state_save` on the session instance, if there is one and it exports the function.
    /// `Ok(None)` means there is nothing to save.
    pub fn save_state(&self) -> Result<Option<Vec<u8>>, CallError> {
        let mut slot = lock(&self.session);
        let Some(runner) = slot.as_mut() else {
            return Ok(None);
        };
        let mut base = self.shared.budgets.budget(BudgetClass::Interactive, self.limits, 1, Arc::clone(&runner.cancel));
        base.deadline = Some(STATE_SAVE_DEADLINE);
        let timer = Timer { clock: &*self.shared.clock, start: self.shared.clock.now(), total: base.deadline };
        let scratch = runner.put(&[0u8; 4], &timer, &base)?;
        let ret = match runner.call(Export::StateSave, &[scratch.0], &timer, &base) {
            Err(CallError::Trap(Trap::NoSuchExport(_))) => {
                runner.free(scratch, &timer, &base)?;
                return Ok(None);
            }
            other => other?,
        };
        let len = i32::try_from(ret).ok().filter(|l| *l >= 0).ok_or(CallError::GuestStatus(ret))?;
        let mut word = [0u8; 4];
        runner.instance.read(scratch.0 as u32, &mut word).map_err(CallError::Trap)?;
        let ptr = u32::from_le_bytes(word);
        let mut data = vec![0u8; len as usize];
        if len > 0 {
            runner.instance.read(ptr, &mut data).map_err(CallError::Trap)?;
            runner.free((ptr as i32, len), &timer, &base)?;
        }
        runner.free(scratch, &timer, &base)?;
        Ok(Some(data))
    }

    /// `craft_state_load` on this generation's session instance, which is created if needed.
    /// `Ok(false)` means the module has no `craft_state_load`.
    pub fn load_state(&self, data: &[u8], from_version: &str) -> Result<bool, CallError> {
        let mut slot = lock(&self.session);
        if slot.is_none() {
            *slot = Some(self.make_runner()?);
        }
        let Some(runner) = slot.as_mut() else {
            return Err(CallError::GuestAlloc);
        };
        let mut base = self.shared.budgets.budget(BudgetClass::Interactive, self.limits, 1, Arc::clone(&runner.cancel));
        base.deadline = Some(STATE_SAVE_DEADLINE);
        let timer = Timer { clock: &*self.shared.clock, start: self.shared.clock.now(), total: base.deadline };
        let d = runner.put(data, &timer, &base)?;
        let v = runner.put(from_version.as_bytes(), &timer, &base)?;
        let ret = match runner.call(Export::StateLoad, &[d.0, d.1, v.0, v.1], &timer, &base) {
            Err(CallError::Trap(Trap::NoSuchExport(_))) => return Ok(false),
            other => other?,
        };
        runner.free(d, &timer, &base)?;
        runner.free(v, &timer, &base)?;
        if ret != 0 {
            return Err(CallError::GuestStatus(ret));
        }
        Ok(true)
    }

    /// True when a session instance exists.
    pub fn has_session(&self) -> bool {
        lock(&self.session).is_some()
    }

    /// Create the session instance now, so `craft_state_load` has a place to run.
    pub fn ensure_session(&self) -> Result<(), CallError> {
        let mut slot = lock(&self.session);
        if slot.is_none() {
            *slot = Some(self.make_runner()?);
        }
        Ok(())
    }
}

fn read_response(runner: &mut Runner, ret: i64, timer: &Timer<'_>, base: &Budget) -> Result<Value, CallError> {
    let (ptr, len) = unpack_invoke_return(ret).map_err(CallError::GuestStatus)?;
    if len == 0 {
        return Ok(Value::Null);
    }
    if len > MAX_RESPONSE_BYTES {
        return Err(CallError::BadResponse(format!("{len} bytes is over the {MAX_RESPONSE_BYTES} byte cap")));
    }
    let mut bytes = vec![0u8; len as usize];
    runner.instance.read(ptr, &mut bytes).map_err(CallError::Trap)?;
    runner.free((ptr as i32, len as i32), timer, base)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| CallError::BadResponse(e.to_string()))?;
    if let Some(message) = value.get("error").and_then(Value::as_str) {
        return Err(CallError::Plugin(message.to_owned()));
    }
    Ok(value)
}
