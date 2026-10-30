//! The wasmi engine.
//!
//! - Deadline: fuel is handed out in slices. After each slice the engine reads the injected
//!   [`Clock`], polls the cancel flag, and either resumes the call or stops it.
//! - Memory: a [`ResourceLimiter`] caps linear memory, initial memory included. A grow past the cap
//!   traps with `Trap::OutOfMemory`. A grow past the module's own declared maximum returns -1, as
//!   the Wasm spec says.
//! - Stack: the compile step sets wasmi's recursion depth and value stack height from
//!   `CompileOptions::stack_bytes`.
//! - Host calls: the one import, `craft_host.call`, runs a [`HostDispatch`] closure directly on
//!   guest memory. The guest supplies the output buffer, so the host never re-enters the guest.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use ::wasmi::{Caller, Config, Engine as WEngine, Extern, Func, Linker, Memory, Module as WModule, ResourceLimiter, Store, TrapCode, Val};
use ::wasmi_core::LimiterError;
use craft_plugin_abi::PolicyReport;

use super::{
    BoxFuture, Budget, Caps, Clock, CompileOptions, Engine, EngineId, Export, HostDispatch, Instance, LoadError, Module, Trap, checked_range,
    hostcall::{RawCall, run_host_call},
    require_accepted, require_deadline_enforcement,
};

/// Fuel handed to the guest per slice. One unit is about one Wasm instruction.
pub const DEFAULT_SLICE_FUEL: u64 = 1_000_000;
/// Fuel for instantiation (a start function, if a legacy package has one).
const START_FUEL: u64 = 10_000_000;
/// Table growth cap, the gate limit.
const MAX_TABLE_ELEMENTS: usize = craft_plugin_abi::gate::MAX_TABLE_ELEMENTS as usize;
/// One guest call frame is budgeted at this many bytes when turning `stack_bytes` into a depth.
const FRAME_BUDGET_BYTES: u32 = 64;
const MIN_RECURSION_DEPTH: u32 = 16;
const MIN_STACK_HEIGHT_BYTES: u32 = 4096;

struct MemLimiter {
    cap: usize,
    /// Set when a grow past `cap` was refused. The call ends with `Trap::OutOfMemory`.
    oom: bool,
}

impl ResourceLimiter for MemLimiter {
    fn memory_growing(&mut self, _current: usize, desired: usize, maximum: Option<usize>) -> Result<bool, LimiterError> {
        if desired > self.cap {
            self.oom = true;
            return Err(LimiterError::ResourceLimiterDeniedAllocation);
        }
        Ok(maximum.is_none_or(|m| desired <= m))
    }

    fn table_growing(&mut self, _current: usize, desired: usize, maximum: Option<usize>) -> Result<bool, LimiterError> {
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

struct HostState {
    limits: MemLimiter,
    dispatch: HostDispatch,
    memory: Option<Memory>,
    /// A trap raised by the dispatcher or by host call plumbing. It wins over wasmi's generic error.
    pending: Option<Trap>,
}

/// The wasmi engine. Create one per host and share it.
pub struct WasmiEngine {
    clock: Arc<dyn Clock>,
    slice_fuel: u64,
}

impl WasmiEngine {
    pub fn new(clock: Arc<dyn Clock>) -> WasmiEngine {
        WasmiEngine { clock, slice_fuel: DEFAULT_SLICE_FUEL }
    }

    /// An engine on the system clock.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn system() -> WasmiEngine {
        WasmiEngine::new(Arc::new(super::SystemClock::new()))
    }

    /// Fuel per slice. Smaller slices stop a runaway guest sooner and cost more resumes.
    pub fn with_slice_fuel(mut self, fuel: u64) -> WasmiEngine {
        self.slice_fuel = fuel.max(1);
        self
    }

    /// Compile without a future. The `Engine` trait method wraps this.
    pub fn compile_sync(&self, wasm: &[u8], report: &PolicyReport, opts: CompileOptions) -> Result<Arc<WasmiModule>, LoadError> {
        require_deadline_enforcement(self.id(), self.caps(), opts.trust)?;
        require_accepted(report)?;
        let mut config = Config::default();
        config.consume_fuel(true);
        let depth = (opts.stack_bytes / FRAME_BUDGET_BYTES).max(MIN_RECURSION_DEPTH);
        config.set_max_recursion_depth(depth as usize);
        config.set_min_stack_height(opts.stack_bytes.min(MIN_STACK_HEIGHT_BYTES) as usize);
        config.set_max_stack_height(opts.stack_bytes as usize);
        let engine = WEngine::new(&config);
        let module = WModule::new(&engine, wasm).map_err(|e| LoadError::Compile(e.to_string()))?;
        Ok(Arc::new(WasmiModule { engine, module, clock: Arc::clone(&self.clock), slice_fuel: self.slice_fuel, stack_bytes: opts.stack_bytes }))
    }
}

impl Engine for WasmiEngine {
    fn id(&self) -> EngineId {
        EngineId::Wasmi
    }

    fn caps(&self) -> Caps {
        let mut caps = Caps::FUEL | Caps::DEADLINE_PREEMPT | Caps::MEMORY_CAP | Caps::STACK_CAP | Caps::SYNC_CALL;
        if cfg!(feature = "deterministic") {
            caps |= Caps::CANON_NAN;
        }
        caps
    }

    fn compile<'a>(&'a self, wasm: &'a [u8], report: &'a PolicyReport, opts: CompileOptions) -> BoxFuture<'a, Result<Arc<dyn Module>, LoadError>> {
        Box::pin(std::future::ready(self.compile_sync(wasm, report, opts).map(|m| m as Arc<dyn Module>)))
    }
}

/// A compiled wasmi module.
pub struct WasmiModule {
    engine: WEngine,
    module: WModule,
    clock: Arc<dyn Clock>,
    slice_fuel: u64,
    stack_bytes: u32,
}

impl WasmiModule {
    /// Instantiate without a future. The `Module` trait method wraps this.
    pub fn instantiate_sync(&self, host: HostDispatch, budget: &Budget) -> Result<WasmiInstance, LoadError> {
        if budget.stack_bytes != self.stack_bytes {
            return Err(LoadError::StackBudgetMismatch { compiled: self.stack_bytes, requested: budget.stack_bytes });
        }
        let cap = usize::try_from(budget.memory_bytes).unwrap_or(usize::MAX);
        let state = HostState { limits: MemLimiter { cap, oom: false }, dispatch: host, memory: None, pending: None };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        let start_fuel = budget.work.map_or(START_FUEL, |w| w.min(START_FUEL));
        store.set_fuel(start_fuel).map_err(|e| LoadError::Instantiate(e.to_string()))?;
        let mut linker = Linker::<HostState>::new(&self.engine);
        linker.func_wrap("craft_host", "call", host_call).map_err(|e| LoadError::Instantiate(e.to_string()))?;
        let instance = match linker.instantiate_and_start(&mut store, &self.module) {
            Ok(i) => i,
            Err(_) if store.data().limits.oom => return Err(LoadError::MemoryOverBudget),
            Err(e) => return Err(LoadError::Instantiate(e.to_string())),
        };
        let memory = instance.get_memory(&store, "memory").ok_or_else(|| LoadError::Instantiate("the module does not export `memory`".into()))?;
        store.data_mut().memory = Some(memory);
        let mut funcs: [Option<Func>; Export::ALL.len()] = [const { None }; Export::ALL.len()];
        for export in Export::ALL {
            let func = instance.get_func(&store, export.name());
            if func.is_none() && export.is_required() {
                return Err(LoadError::Instantiate(format!("the module does not export `{}`", export.name())));
            }
            funcs[export as usize] = func;
        }
        Ok(WasmiInstance {
            store,
            memory,
            funcs,
            clock: Arc::clone(&self.clock),
            slice_fuel: self.slice_fuel,
            stack_bytes: self.stack_bytes,
            stats: CallStats::default(),
        })
    }
}

impl Module for WasmiModule {
    fn instantiate<'a>(&'a self, host: HostDispatch, budget: &'a Budget) -> BoxFuture<'a, Result<Box<dyn Instance>, LoadError>> {
        Box::pin(std::future::ready(self.instantiate_sync(host, budget).map(|i| Box::new(i) as Box<dyn Instance>)))
    }
}

/// What the last call cost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallStats {
    /// Fuel slices the call used, counting the first.
    pub slices: u32,
    /// Fuel burned.
    pub fuel_used: u64,
}

/// A running wasmi instance.
pub struct WasmiInstance {
    store: Store<HostState>,
    memory: Memory,
    funcs: [Option<Func>; Export::ALL.len()],
    clock: Arc<dyn Clock>,
    slice_fuel: u64,
    stack_bytes: u32,
    stats: CallStats,
}

impl WasmiInstance {
    /// Slices and fuel of the most recent call, finished or trapped.
    pub fn last_call_stats(&self) -> CallStats {
        self.stats
    }

    fn map_error(&self, e: &::wasmi::Error) -> Trap {
        if self.store.data().limits.oom {
            return Trap::OutOfMemory;
        }
        match e.as_trap_code() {
            Some(TrapCode::UnreachableCodeReached) => Trap::Unreachable,
            Some(TrapCode::StackOverflow) => Trap::StackOverflow,
            Some(TrapCode::OutOfFuel) => Trap::OutOfWork,
            Some(TrapCode::GrowthOperationLimited) => Trap::OutOfMemory,
            Some(code) => Trap::GuestFault(code.to_string()),
            None => Trap::Other(e.to_string()),
        }
    }

    fn check_stop(&self, budget: &Budget, started: Duration) -> Result<(), Trap> {
        if budget.cancel.load(Ordering::Relaxed) {
            return Err(Trap::Cancelled);
        }
        if let Some(limit) = budget.deadline
            && self.clock.now().saturating_sub(started) >= limit
        {
            return Err(Trap::Deadline);
        }
        Ok(())
    }

    fn set_fuel(&mut self, fuel: u64) -> Result<(), Trap> {
        self.store.set_fuel(fuel).map_err(|e| Trap::Other(e.to_string()))
    }

    fn fuel_burned(&self, given: u64) -> u64 {
        given.saturating_sub(self.store.get_fuel().unwrap_or(0))
    }

    /// Run `func` in fuel slices until it returns, traps, or a budget stops it.
    fn run(&mut self, func: Func, inputs: &[Val], outputs: &mut [Val], budget: &Budget) -> Result<(), Trap> {
        self.stats = CallStats::default();
        {
            let data = self.store.data_mut();
            data.pending = None;
            data.limits.oom = false;
            data.limits.cap = usize::try_from(budget.memory_bytes).unwrap_or(usize::MAX);
        }
        let started = self.clock.now();
        self.check_stop(budget, started)?;
        let mut left = budget.work;
        let mut given = self.next_slice(&mut left, 0)?;
        self.set_fuel(given)?;
        self.stats.slices = 1;
        let mut call = func.call_resumable(&mut self.store, inputs, outputs).map_err(|e| self.error_of(&e))?;
        loop {
            match call {
                ::wasmi::ResumableCall::Finished => {
                    self.stats.fuel_used += self.fuel_burned(given);
                    return Ok(());
                }
                ::wasmi::ResumableCall::HostTrap(inv) => {
                    self.stats.fuel_used += self.fuel_burned(given);
                    return Err(self.error_of(inv.host_error()));
                }
                ::wasmi::ResumableCall::OutOfFuel(inv) => {
                    self.stats.fuel_used += given;
                    self.check_stop(budget, started)?;
                    given = self.next_slice(&mut left, inv.required_fuel())?;
                    self.set_fuel(given)?;
                    self.stats.slices += 1;
                    call = inv.resume(&mut self.store, outputs).map_err(|e| self.error_of(&e))?;
                }
            }
        }
    }

    /// The trap for a wasmi error. A trap the host raised wins over wasmi's generic error.
    fn error_of(&mut self, e: &::wasmi::Error) -> Trap {
        match self.store.data_mut().pending.take() {
            Some(t) => t,
            None => self.map_error(e),
        }
    }

    /// Fuel for the next slice, taken from `left`. `Trap::OutOfWork` when the total cap cannot pay for it.
    fn next_slice(&self, left: &mut Option<u64>, required: u64) -> Result<u64, Trap> {
        let want = self.slice_fuel.max(required);
        match left {
            None => Ok(want),
            Some(l) => {
                if *l == 0 || required > *l {
                    return Err(Trap::OutOfWork);
                }
                let give = want.min(*l);
                *l -= give;
                Ok(give)
            }
        }
    }
}

impl Instance for WasmiInstance {
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
fn host_call(mut caller: Caller<'_, HostState>, op: i32, in_ptr: i32, in_len: i32, out_ptr: i32, out_cap: i32) -> Result<i64, ::wasmi::Error> {
    let dispatch = Arc::clone(&caller.data().dispatch);
    let memory = match caller.data().memory {
        Some(m) => m,
        None => match caller.get_export("memory").and_then(Extern::into_memory) {
            Some(m) => m,
            None => return Err(::wasmi::Error::new("craft_host.call: the guest has no exported memory")),
        },
    };
    let mem = memory.data_mut(&mut caller);
    let args = RawCall { op: op as u32, in_ptr: in_ptr as u32, in_len: in_len as u32, out_ptr: out_ptr as u32, out_cap: out_cap as u32 };
    match run_host_call(&*dispatch, mem, &args) {
        Ok(packed) => Ok(packed),
        Err(trap) => {
            caller.data_mut().pending = Some(trap);
            Err(::wasmi::Error::new("craft_host.call: the host stopped the guest"))
        }
    }
}
