//! The wasmi plumbing the three v1 hosts share: load under limits, instantiate, and call an export
//! in fuel slices while a wall clock and an abort flag are checked between slices.
//!
//! Each v1 host did this itself, with the same two loops. The numbers (fuel, memory, recursion, wall
//! time) differ per ABI and live in the adapters. The rules that do not differ live here.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use wasmi::{
    Config, EnforcedLimits, Engine, Instance, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TrapCode, TypedFunc,
    TypedResumableCall, WasmParams, WasmResults,
};

use super::{Result, ShimError};
use crate::engine::Clock;

/// Fuel handed to the guest per slice. Between slices the wall clock and the abort flag are read.
pub const FUEL_SLICE: u64 = 20_000_000;

/// Table cap of the PhotoCraft and VectorCraft hosts.
const MAX_TABLES: u32 = 4;
const MAX_TABLE_ELEMENTS: usize = 100_000;

/// How a v1 host configured wasmi.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Profile {
    /// Cap on linear memory, initial memory included.
    pub max_memory_bytes: usize,
    /// `EnforcedLimits::strict()`, 4 tables of 100,000 elements, one memory. EffectCraft used none of these.
    pub strict: bool,
    /// `Config::set_max_recursion_depth`. EffectCraft kept the wasmi default.
    pub max_recursion_depth: Option<usize>,
}

/// When a run must stop: a wall time limit on the injected clock, and an abort flag that parallel
/// bands share so the first failure stops its siblings and stays the reported cause.
#[derive(Clone)]
pub struct Deadline {
    clock: Arc<dyn Clock>,
    /// Clock reading at which the run is over. `None` means no wall limit.
    at: Option<Duration>,
    abort: Arc<Mutex<Option<ShimError>>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Deadline {
    /// A run that may take `wall` from now. `None` is unlimited.
    pub fn new(clock: Arc<dyn Clock>, wall: Option<Duration>) -> Deadline {
        let at = wall.map(|w| clock.now().saturating_add(w));
        Deadline { clock, at, abort: Arc::new(Mutex::new(None)) }
    }

    /// A run with no wall limit and its own abort flag. For calls whose v1 host had no deadline.
    pub fn none() -> Deadline {
        Deadline { clock: Arc::new(NeverClock), at: None, abort: Arc::new(Mutex::new(None)) }
    }

    /// Record `error` as the cause that stops every run sharing this deadline. The first one wins.
    pub fn abort(&self, error: ShimError) {
        lock(&self.abort).get_or_insert(error);
    }

    pub(crate) fn check(&self) -> Result<()> {
        if let Some(error) = lock(&self.abort).clone() {
            return Err(error);
        }
        if self.at.is_some_and(|at| self.clock.now() >= at) {
            let error = ShimError::Limit("time budget".into());
            self.abort(error.clone());
            return Err(error);
        }
        Ok(())
    }
}

/// The clock of a deadline that has no limit. It is never read for a limit.
struct NeverClock;

impl Clock for NeverClock {
    fn now(&self) -> Duration {
        Duration::ZERO
    }
}

/// A compiled module and the profile it runs under.
pub(crate) struct Machine {
    engine: Engine,
    module: Module,
    profile: Profile,
}

pub(crate) fn wasm_err(e: &wasmi::Error) -> ShimError {
    if e.as_trap_code() == Some(TrapCode::OutOfFuel) { ShimError::Limit("instruction budget".into()) } else { ShimError::Trap(e.to_string()) }
}

impl Machine {
    /// Compile `bytes`. A v1 module imports nothing, so any import is an ABI error.
    pub(crate) fn new(bytes: &[u8], profile: Profile) -> Result<Machine> {
        let mut config = Config::default();
        config.consume_fuel(true);
        if profile.strict {
            config.enforced_limits(EnforcedLimits::strict());
        }
        if let Some(depth) = profile.max_recursion_depth {
            config.set_max_recursion_depth(depth.max(16));
        }
        let engine = Engine::new(&config);
        let module = Module::new(&engine, bytes).map_err(|e| ShimError::Module(e.to_string()))?;
        if let Some(i) = module.imports().next() {
            return Err(ShimError::Abi(format!(
                "plug-ins can't import anything: the module imports `{}.{}`, but plug-ins get no host imports (no WASI, files, clock or network)",
                i.module(),
                i.name()
            )));
        }
        Ok(Machine { engine, module, profile })
    }

    /// A fresh instance. Its start function runs on `start_fuel`.
    pub(crate) fn instantiate(&self, start_fuel: u64, deadline: &Deadline) -> Result<Live> {
        deadline.check()?;
        let mut limits = StoreLimitsBuilder::new().memory_size(self.profile.max_memory_bytes).instances(1);
        if self.profile.strict {
            limits = limits.memories(1).tables(MAX_TABLES as usize).table_elements(MAX_TABLE_ELEMENTS);
        }
        let mut store = Store::new(&self.engine, limits.build());
        store.limiter(|l| l);
        store.set_fuel(start_fuel).map_err(|e| wasm_err(&e))?;
        let linker = Linker::<StoreLimits>::new(&self.engine);
        // A start function that traps, runs out of fuel, or asks for too much memory fails here.
        let instance = linker.instantiate_and_start(&mut store, &self.module).map_err(|e| ShimError::Abi(format!("instantiation failed: {e}")))?;
        let memory = instance
            .get_memory(&store, "memory")
            .ok_or_else(|| ShimError::Abi("the module must export its linear memory as `memory`, but exports no `memory`".into()))?;
        Ok(Live { store, instance, memory })
    }
}

/// One live instance: a store with its own memory limit, and the module's exports.
pub(crate) struct Live {
    pub(crate) store: Store<StoreLimits>,
    instance: Instance,
    pub(crate) memory: Memory,
}

impl Live {
    pub(crate) fn func<P: WasmParams, R: WasmResults>(&self, name: &str) -> Result<TypedFunc<P, R>> {
        self.instance
            .get_typed_func::<P, R>(&self.store, name)
            .map_err(|_| ShimError::Abi(format!("the module exports no `{name}` (or with the wrong signature)")))
    }

    /// Call `f`, taking fuel from `pool` in slices and giving back what the call did not burn.
    /// The wall clock and the abort flag are read between slices. An empty pool is an
    /// `instruction budget` limit.
    pub(crate) fn call<P: WasmParams, R: WasmResults>(&mut self, f: &TypedFunc<P, R>, params: P, pool: &mut u64, deadline: &Deadline) -> Result<R> {
        let give = (*pool).min(FUEL_SLICE);
        *pool -= give;
        self.store.set_fuel(give).map_err(|e| wasm_err(&e))?;
        let mut call = f.call_resumable(&mut self.store, params).map_err(|e| wasm_err(&e))?;
        loop {
            match call {
                TypedResumableCall::Finished(r) => {
                    *pool += self.store.get_fuel().unwrap_or(0);
                    return Ok(r);
                }
                TypedResumableCall::HostTrap(_) => return Err(ShimError::Trap("unexpected host trap".into())),
                TypedResumableCall::OutOfFuel(inv) => {
                    let need = inv.required_fuel();
                    if *pool == 0 || need > *pool {
                        return Err(ShimError::Limit("instruction budget".into()));
                    }
                    deadline.check()?;
                    let give = (*pool).min(FUEL_SLICE.max(need));
                    *pool -= give;
                    self.store.set_fuel(give).map_err(|e| wasm_err(&e))?;
                    call = inv.resume(&mut self.store).map_err(|e| wasm_err(&e))?;
                }
            }
        }
    }

    /// Linear memory as bytes.
    pub(crate) fn mem(&self) -> &[u8] {
        self.memory.data(&self.store)
    }

    pub(crate) fn mem_mut(&mut self) -> &mut [u8] {
        self.memory.data_mut(&mut self.store)
    }

    /// `len << 32 | ptr` as a guest returns it: the block it names, copied out, at most `max` bytes.
    pub(crate) fn read_packed(&self, packed: i64, what: &str, max: usize) -> Result<Vec<u8>> {
        let packed = packed as u64;
        let (ptr, len) = ((packed & 0xffff_ffff) as usize, (packed >> 32) as usize);
        if len > max {
            return Err(ShimError::Limit(format!("output size ({what} is {len} bytes, limit {max})")));
        }
        let bytes = self.mem().get(ptr..ptr.saturating_add(len));
        bytes.map(<[u8]>::to_vec).ok_or_else(|| ShimError::Abi(format!("{what} points outside memory")))
    }
}
