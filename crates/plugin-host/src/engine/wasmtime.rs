//! The wasmtime engine. Native only. Feature `wasmtime`.
//!
//! It keeps the semantics of the wasmi engine and maps traps the same way.
//!
//! - Work: `consume_fuel`. `Budget::work` is the fuel of one call. Fuel units are not wasmi fuel
//!   units, so a work cap is portable only as an order of magnitude.
//! - Deadline and cancel: epoch interruption. A timer thread bumps the engine epoch every tick
//!   while at least one call runs, and sleeps when none does. Each epoch tick runs a callback in
//!   the store. The callback reads the injected [`Clock`] and the cancel flag, exactly like the
//!   wasmi slice boundary, and ends the call with `Trap::Deadline` or `Trap::Cancelled`. Overshoot
//!   is at most one tick.
//! - Memory: a [`ResourceLimiter`] caps linear memory, initial memory included. A grow past the cap
//!   traps with `Trap::OutOfMemory`. A grow past the module's own declared maximum returns -1.
//! - Stack: `max_wasm_stack` is set from `CompileOptions::stack_bytes` when the module is compiled.
//!   Native frames are larger than wasmi frames, so the same stack budget allows fewer nested calls
//!   here than on wasmi. Both engines stop runaway recursion with `Trap::StackOverflow`.
//! - Floats: NaN canonicalization is on, so results match wasmi's deterministic profile.
//! - Allocation: the pooling allocator, sized by [`WasmtimeEngine::with_pool_instances`].
//!
//! The host call body is shared with wasmi (`hostcall.rs`).

use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use ::wasmtime::{
    Caller, Config, Engine as WEngine, EngineWeak, Extern, Func, InstanceAllocationStrategy, Linker, Memory, Module as WModule,
    PoolingAllocationConfig, ResourceLimiter, Store, UpdateDeadline, Val,
};
use craft_plugin_abi::PolicyReport;

use super::hostcall::{RawCall, run_host_call};
use super::{
    BoxFuture, Budget, Caps, Clock, CompileOptions, Engine, EngineId, Export, HostDispatch, Instance, LoadError, Module, Trap, checked_range,
    require_deadline_enforcement,
};

/// Epoch tick length. A deadline or cancel is noticed within one tick.
pub const DEFAULT_TICK: Duration = Duration::from_millis(1);
/// Instances the pooling allocator keeps slots for.
pub const DEFAULT_POOL_INSTANCES: u32 = 64;
/// Fuel for instantiation (a start function, if a package has one).
const START_FUEL: u64 = 10_000_000;
/// Fuel when `Budget::work` is `None`. wasmtime stores fuel as an `i64`.
const UNLIMITED_FUEL: u64 = i64::MAX as u64;
/// Table growth cap, the gate limit.
const MAX_TABLE_ELEMENTS: usize = craft_plugin_abi::gate::MAX_TABLE_ELEMENTS as usize;
/// The pool reserves this much address space for each memory. It matches the largest declared
/// maximum the native gate policy accepts (`HostPolicy::native`).
const POOL_MEMORY_BYTES: usize = 1024 * 1024 * 1024;
/// Epoch deadline used while no call runs. Large enough that a stray tick cannot reach it.
const IDLE_DEADLINE_TICKS: u64 = 1 << 62;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A poisoned lock only means another thread panicked. The tick state stays valid.
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---- the epoch ticker --------------------------------------------------------------------

struct TickState {
    /// Calls running now.
    active: usize,
    closed: bool,
    engines: Vec<EngineWeak>,
}

struct TickShared {
    state: Mutex<TickState>,
    wake: Condvar,
    tick: Duration,
}

/// One timer thread for every wasmtime engine a `WasmtimeEngine` compiled. It runs only while a
/// call runs. It ends when the last module and instance that hold it are dropped.
struct Ticker {
    shared: Arc<TickShared>,
}

impl Ticker {
    fn spawn(tick: Duration) -> Result<Ticker, LoadError> {
        let shared =
            Arc::new(TickShared { state: Mutex::new(TickState { active: 0, closed: false, engines: Vec::new() }), wake: Condvar::new(), tick });
        let thread_shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("craft-wasmtime-epoch".into())
            .spawn(move || tick_loop(&thread_shared))
            .map_err(|e| LoadError::Instantiate(format!("cannot start the epoch timer thread: {e}")))?;
        Ok(Ticker { shared })
    }

    fn register(&self, engine: &WEngine) {
        lock(&self.shared.state).engines.push(engine.weak());
    }

    /// Mark one call as running. The timer thread ticks until the guard drops.
    fn begin(&self) -> ActiveCall<'_> {
        let mut st = lock(&self.shared.state);
        st.active += 1;
        if st.active == 1 {
            self.shared.wake.notify_all();
        }
        ActiveCall { ticker: self }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        lock(&self.shared.state).closed = true;
        self.shared.wake.notify_all();
    }
}

struct ActiveCall<'a> {
    ticker: &'a Ticker,
}

impl Drop for ActiveCall<'_> {
    fn drop(&mut self) {
        let mut st = lock(&self.ticker.shared.state);
        st.active = st.active.saturating_sub(1);
    }
}

fn tick_loop(shared: &TickShared) {
    let mut st = lock(&shared.state);
    loop {
        while st.active == 0 && !st.closed {
            st = shared.wake.wait(st).unwrap_or_else(PoisonError::into_inner);
        }
        if st.closed {
            return;
        }
        let (guard, _) = shared.wake.wait_timeout(st, shared.tick).unwrap_or_else(PoisonError::into_inner);
        st = guard;
        if st.closed {
            return;
        }
        if st.active > 0 {
            st.engines.retain(|weak| match weak.upgrade() {
                Some(engine) => {
                    engine.increment_epoch();
                    true
                }
                None => false,
            });
        }
    }
}

// ---- store state -------------------------------------------------------------------------

struct MemLimiter {
    cap: usize,
    /// Set when a grow past `cap` was refused. The call ends with `Trap::OutOfMemory`.
    oom: bool,
}

impl ResourceLimiter for MemLimiter {
    fn memory_growing(&mut self, _current: usize, desired: usize, maximum: Option<usize>) -> ::wasmtime::Result<bool> {
        if desired > self.cap {
            self.oom = true;
            return Err(::wasmtime::Error::msg("memory budget exceeded"));
        }
        Ok(maximum.is_none_or(|m| desired <= m))
    }

    fn table_growing(&mut self, _current: usize, desired: usize, maximum: Option<usize>) -> ::wasmtime::Result<bool> {
        Ok(desired <= MAX_TABLE_ELEMENTS && maximum.is_none_or(|m| desired <= m))
    }

    fn instances(&self) -> usize {
        1
    }

    fn tables(&self) -> usize {
        craft_plugin_abi::gate::MAX_TABLES
    }

    fn memories(&self) -> usize {
        1
    }
}

/// What the epoch callback checks on each tick.
struct StopCheck {
    clock: Arc<dyn Clock>,
    started: Duration,
    deadline: Option<Duration>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

impl StopCheck {
    fn check(&self) -> Result<(), Trap> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Trap::Cancelled);
        }
        if let Some(limit) = self.deadline
            && self.clock.now().saturating_sub(self.started) >= limit
        {
            return Err(Trap::Deadline);
        }
        Ok(())
    }
}

struct HostState {
    limits: MemLimiter,
    dispatch: HostDispatch,
    memory: Option<Memory>,
    /// A trap raised by the dispatcher, the epoch callback, or host call plumbing. It wins over
    /// wasmtime's generic error.
    pending: Option<Trap>,
    stop: StopCheck,
}

// ---- engine ------------------------------------------------------------------------------

/// The wasmtime engine. Create one per host and share it.
pub struct WasmtimeEngine {
    clock: Arc<dyn Clock>,
    ticker: Arc<Ticker>,
    pool_instances: u32,
}

impl WasmtimeEngine {
    /// An engine with the default tick. Fails when the timer thread cannot start.
    pub fn new(clock: Arc<dyn Clock>) -> Result<WasmtimeEngine, LoadError> {
        WasmtimeEngine::with_tick(clock, DEFAULT_TICK)
    }

    /// An engine whose epoch tick is `tick`. A shorter tick stops a runaway guest sooner and wakes
    /// the timer thread more often.
    pub fn with_tick(clock: Arc<dyn Clock>, tick: Duration) -> Result<WasmtimeEngine, LoadError> {
        Ok(WasmtimeEngine { clock, ticker: Arc::new(Ticker::spawn(tick.max(Duration::from_micros(50)))?), pool_instances: DEFAULT_POOL_INSTANCES })
    }

    /// An engine on the system clock.
    pub fn system() -> Result<WasmtimeEngine, LoadError> {
        WasmtimeEngine::new(Arc::new(super::SystemClock::new()))
    }

    /// How many instances of one module can be alive at once. The pool reserves address space for
    /// each. Instantiation fails with `LoadError::Instantiate` when the pool is full.
    pub fn with_pool_instances(mut self, instances: u32) -> WasmtimeEngine {
        self.pool_instances = instances.max(1);
        self
    }

    fn wasmtime_config(&self, stack_bytes: u32) -> Config {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.epoch_interruption(true);
        config.max_wasm_stack(stack_bytes as usize);
        config.cranelift_nan_canonicalization(true);
        config.wasm_relaxed_simd(false);
        let mut pool = PoolingAllocationConfig::new();
        pool.total_core_instances(self.pool_instances);
        pool.total_memories(self.pool_instances);
        pool.total_tables(self.pool_instances.saturating_mul(craft_plugin_abi::gate::MAX_TABLES as u32));
        pool.max_memory_size(POOL_MEMORY_BYTES);
        pool.table_elements(MAX_TABLE_ELEMENTS);
        config.allocation_strategy(InstanceAllocationStrategy::Pooling(pool));
        config
    }

    /// Compile without a future. The `Engine` trait method wraps this.
    pub fn compile_sync(&self, wasm: &[u8], report: &PolicyReport, opts: CompileOptions) -> Result<Arc<WasmtimeModule>, LoadError> {
        require_deadline_enforcement(self.id(), self.caps(), opts.trust)?;
        if let Some(abi) = report.legacy {
            return Err(LoadError::LegacyNeedsShim(abi));
        }
        if !report.is_accepted() {
            return Err(LoadError::Gate(report.violations.clone()));
        }
        let engine = WEngine::new(&self.wasmtime_config(opts.stack_bytes)).map_err(|e| LoadError::Compile(format!("{e:#}")))?;
        let module = WModule::new(&engine, wasm).map_err(|e| LoadError::Compile(format!("{e:#}")))?;
        self.ticker.register(&engine);
        Ok(Arc::new(WasmtimeModule {
            engine,
            module,
            clock: Arc::clone(&self.clock),
            ticker: Arc::clone(&self.ticker),
            stack_bytes: opts.stack_bytes,
        }))
    }
}

impl Engine for WasmtimeEngine {
    fn id(&self) -> EngineId {
        EngineId::Wasmtime
    }

    fn caps(&self) -> Caps {
        Caps::FUEL | Caps::DEADLINE_KILL | Caps::MEMORY_CAP | Caps::STACK_CAP | Caps::CANON_NAN | Caps::SIMD128 | Caps::SYNC_CALL
    }

    fn compile<'a>(&'a self, wasm: &'a [u8], report: &'a PolicyReport, opts: CompileOptions) -> BoxFuture<'a, Result<Arc<dyn Module>, LoadError>> {
        Box::pin(std::future::ready(self.compile_sync(wasm, report, opts).map(|m| m as Arc<dyn Module>)))
    }
}

/// A compiled wasmtime module.
pub struct WasmtimeModule {
    engine: WEngine,
    module: WModule,
    clock: Arc<dyn Clock>,
    ticker: Arc<Ticker>,
    stack_bytes: u32,
}

impl WasmtimeModule {
    /// Instantiate without a future. The `Module` trait method wraps this.
    pub fn instantiate_sync(&self, host: HostDispatch, budget: &Budget) -> Result<WasmtimeInstance, LoadError> {
        if budget.stack_bytes != self.stack_bytes {
            return Err(LoadError::StackBudgetMismatch { compiled: self.stack_bytes, requested: budget.stack_bytes });
        }
        let cap = usize::try_from(budget.memory_bytes).unwrap_or(usize::MAX);
        let stop = StopCheck { clock: Arc::clone(&self.clock), started: Duration::ZERO, deadline: None, cancel: Arc::clone(&budget.cancel) };
        let state = HostState { limits: MemLimiter { cap, oom: false }, dispatch: host, memory: None, pending: None, stop };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        store.epoch_deadline_callback(|mut ctx| {
            let data = ctx.data_mut();
            match data.stop.check() {
                Ok(()) => Ok(UpdateDeadline::Continue(1)),
                Err(trap) => {
                    data.pending = Some(trap);
                    Err(::wasmtime::Error::msg("the host stopped the guest"))
                }
            }
        });
        let start_fuel = budget.work.map_or(START_FUEL, |w| w.min(START_FUEL));
        store.set_fuel(start_fuel).map_err(|e| LoadError::Instantiate(format!("{e:#}")))?;
        store.set_epoch_deadline(IDLE_DEADLINE_TICKS);
        let mut linker = Linker::<HostState>::new(&self.engine);
        linker.func_wrap("craft_host", "call", host_call).map_err(|e| LoadError::Instantiate(format!("{e:#}")))?;
        let instance = match linker.instantiate(&mut store, &self.module) {
            Ok(i) => i,
            Err(_) if store.data().limits.oom => return Err(LoadError::MemoryOverBudget),
            Err(e) => return Err(LoadError::Instantiate(format!("{e:#}"))),
        };
        let memory = instance.get_memory(&mut store, "memory").ok_or_else(|| LoadError::Instantiate("the module does not export `memory`".into()))?;
        store.data_mut().memory = Some(memory);
        let mut funcs: [Option<Func>; Export::ALL.len()] = [const { None }; Export::ALL.len()];
        for export in Export::ALL {
            let func = instance.get_func(&mut store, export.name());
            if func.is_none() && export.is_required() {
                return Err(LoadError::Instantiate(format!("the module does not export `{}`", export.name())));
            }
            funcs[export as usize] = func;
        }
        Ok(WasmtimeInstance { store, memory, funcs, ticker: Arc::clone(&self.ticker), stack_bytes: self.stack_bytes, stats: CallStats::default() })
    }
}

impl Module for WasmtimeModule {
    fn instantiate<'a>(&'a self, host: HostDispatch, budget: &'a Budget) -> BoxFuture<'a, Result<Box<dyn Instance>, LoadError>> {
        Box::pin(std::future::ready(self.instantiate_sync(host, budget).map(|i| Box::new(i) as Box<dyn Instance>)))
    }
}

/// What the last call cost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallStats {
    /// Fuel burned.
    pub fuel_used: u64,
}

/// A running wasmtime instance.
pub struct WasmtimeInstance {
    store: Store<HostState>,
    memory: Memory,
    funcs: [Option<Func>; Export::ALL.len()],
    ticker: Arc<Ticker>,
    stack_bytes: u32,
    stats: CallStats,
}

impl WasmtimeInstance {
    /// Fuel of the most recent call, finished or trapped.
    pub fn last_call_stats(&self) -> CallStats {
        self.stats
    }

    /// The trap for a wasmtime error. A trap the host raised wins over wasmtime's generic error.
    fn error_of(&mut self, e: &::wasmtime::Error) -> Trap {
        if let Some(t) = self.store.data_mut().pending.take() {
            return t;
        }
        if self.store.data().limits.oom {
            return Trap::OutOfMemory;
        }
        match e.downcast_ref::<::wasmtime::Trap>() {
            Some(::wasmtime::Trap::UnreachableCodeReached) => Trap::Unreachable,
            Some(::wasmtime::Trap::StackOverflow) => Trap::StackOverflow,
            Some(::wasmtime::Trap::OutOfFuel) => Trap::OutOfWork,
            Some(code) => Trap::GuestFault(code.to_string()),
            None => Trap::Other(format!("{e:#}")),
        }
    }

    fn run(&mut self, func: Func, inputs: &[Val], outputs: &mut [Val], budget: &Budget) -> Result<(), Trap> {
        self.stats = CallStats::default();
        {
            let data = self.store.data_mut();
            data.pending = None;
            data.limits.oom = false;
            data.limits.cap = usize::try_from(budget.memory_bytes).unwrap_or(usize::MAX);
            let started = data.stop.clock.now();
            data.stop.started = started;
            data.stop.deadline = budget.deadline;
            data.stop.cancel = Arc::clone(&budget.cancel);
            data.stop.check()?;
        }
        let given = budget.work.map_or(UNLIMITED_FUEL, |w| w.min(UNLIMITED_FUEL));
        if given == 0 {
            return Err(Trap::OutOfWork);
        }
        self.store.set_fuel(given).map_err(|e| Trap::Other(format!("{e:#}")))?;
        self.store.set_epoch_deadline(1);
        let result = {
            let _active = self.ticker.begin();
            func.call(&mut self.store, inputs, outputs)
        };
        self.stats.fuel_used = given.saturating_sub(self.store.get_fuel().unwrap_or(0));
        self.store.set_epoch_deadline(IDLE_DEADLINE_TICKS);
        result.map_err(|e| self.error_of(&e))
    }
}

impl Instance for WasmtimeInstance {
    fn call(&mut self, export: Export, args: &[i32], budget: &Budget) -> Result<i64, Trap> {
        if args.len() != export.arity() {
            return Err(Trap::BadArity { export, expected: export.arity(), got: args.len() });
        }
        if budget.stack_bytes != self.stack_bytes {
            return Err(Trap::BudgetMismatch);
        }
        let func = self.funcs[export as usize].ok_or(Trap::NoSuchExport(export))?;
        let mut inputs = [const { Val::I32(0) }; 5];
        for (slot, arg) in inputs.iter_mut().zip(args) {
            *slot = Val::I32(*arg);
        }
        let mut outputs = [if export == Export::Invoke { Val::I64(0) } else { Val::I32(0) }];
        let results = if export == Export::Free { 0 } else { 1 };
        self.run(func, &inputs[..args.len()], &mut outputs[..results], budget)?;
        match outputs[..results].first() {
            None => Ok(0),
            Some(Val::I32(v)) => Ok(i64::from(*v)),
            Some(Val::I64(v)) => Ok(*v),
            Some(other) => Err(Trap::Other(format!("export {} returned {other:?}", export.name()))),
        }
    }

    fn mem_len(&self) -> usize {
        self.memory.data_size(&self.store)
    }

    fn with_mem(&mut self, f: &mut dyn FnMut(&mut [u8])) -> Result<(), Trap> {
        f(self.memory.data_mut(&mut self.store));
        Ok(())
    }

    fn write(&mut self, ptr: u32, src: &[u8]) -> Result<(), Trap> {
        let mem = self.memory.data_mut(&mut self.store);
        let range = checked_range(ptr, src.len(), mem.len())?;
        mem.get_mut(range).ok_or(Trap::BadPointer)?.copy_from_slice(src);
        Ok(())
    }

    fn read(&self, ptr: u32, dst: &mut [u8]) -> Result<(), Trap> {
        let mem = self.memory.data(&self.store);
        let range = checked_range(ptr, dst.len(), mem.len())?;
        dst.copy_from_slice(mem.get(range).ok_or(Trap::BadPointer)?);
        Ok(())
    }
}

/// The `craft_host.call` import.
fn host_call(mut caller: Caller<'_, HostState>, op: i32, in_ptr: i32, in_len: i32, out_ptr: i32, out_cap: i32) -> ::wasmtime::Result<i64> {
    let dispatch = Arc::clone(&caller.data().dispatch);
    let memory = match caller.data().memory {
        Some(m) => m,
        None => match caller.get_export("memory").and_then(Extern::into_memory) {
            Some(m) => m,
            None => return Err(::wasmtime::Error::msg("craft_host.call: the guest has no exported memory")),
        },
    };
    let mem = memory.data_mut(&mut caller);
    let args = RawCall { op: op as u32, in_ptr: in_ptr as u32, in_len: in_len as u32, out_ptr: out_ptr as u32, out_cap: out_cap as u32 };
    match run_host_call(&*dispatch, mem, &args) {
        Ok(packed) => Ok(packed),
        Err(trap) => {
            caller.data_mut().pending = Some(trap);
            Err(::wasmtime::Error::msg("craft_host.call: the host stopped the guest"))
        }
    }
}
