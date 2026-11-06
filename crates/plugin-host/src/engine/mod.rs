//! The engine contract for craft-abi 1 (see `crates/plugin-abi/ABI.md`).
//!
//! One `Engine` compiles a gated package into a shared `Module`. A `Module` makes `Instance`s.
//! An `Instance` runs the five craft-abi exports under a `Budget` and reports a `Trap` when the
//! budget or the guest breaks a rule. Engines differ in what they can enforce. They say so with
//! `Caps`, and the loader refuses an untrusted plugin on an engine that cannot stop it in time.
//!
//! The browser engines (`browser`, feature `browser`, `wasm32` only) run a module on the page's own
//! WebAssembly. They cannot meter fuel and cannot stop a running guest on the page thread. Their
//! `Caps` say so, and the loader refuses an untrusted plugin on the page thread engine.
//!
//! This crate never calls `std::time::Instant`. The host injects a [`Clock`], because
//! `wasm32-unknown-unknown` has no monotonic clock in `std`.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use craft_plugin_abi::gate::Violation;
use craft_plugin_abi::{LegacyAbi, Op, PolicyReport, Status};

#[cfg(feature = "browser")]
#[cfg(target_arch = "wasm32")]
pub mod browser;
#[cfg(any(feature = "wasmi", all(feature = "browser", target_arch = "wasm32"), all(feature = "wasmtime", not(target_arch = "wasm32"))))]
mod hostcall;
pub mod js_fault;
mod load;
pub mod memory_patch;
#[cfg(feature = "wasmi")]
pub mod wasmi;
#[cfg(all(feature = "wasmtime", not(target_arch = "wasm32")))]
pub mod wasmtime;

pub use load::load_package;

/// `Send` and `Sync` on native targets. No bound on `wasm32`, where browser engines hold JS handles.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSendSync: Send + Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync + ?Sized> MaybeSendSync for T {}
/// `Send` and `Sync` on native targets. No bound on `wasm32`, where browser engines hold JS handles.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSendSync {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSendSync for T {}

/// `Send` on native targets. No bound on `wasm32`.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + ?Sized> MaybeSend for T {}
/// `Send` on native targets. No bound on `wasm32`.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSend for T {}

/// A boxed future that need not be `Send`. Browser engines compile and instantiate through promises.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// A monotonic time source the host supplies. The value counts from an arbitrary origin.
pub trait Clock: MaybeSendSync {
    fn now(&self) -> Duration;
}

/// `std::time::Instant` clock for native hosts. Web hosts implement [`Clock`] over `performance.now()`.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct SystemClock {
    origin: std::time::Instant,
}

#[cfg(not(target_arch = "wasm32"))]
impl SystemClock {
    pub fn new() -> SystemClock {
        SystemClock { origin: std::time::Instant::now() }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for SystemClock {
    fn default() -> Self {
        SystemClock::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

bitflags::bitflags! {
    /// What an engine can enforce or offer.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Caps: u32 {
        /// Fuel metering: `Budget::work` is enforced.
        const FUEL = 1 << 0;
        /// The host can pause a running guest at a slice boundary and stop it on the deadline.
        const DEADLINE_PREEMPT = 1 << 1;
        /// The host can kill a running guest on the deadline (epoch interruption, worker terminate).
        const DEADLINE_KILL = 1 << 2;
        /// `Budget::memory_bytes` is a hard cap including initial memory.
        const MEMORY_CAP = 1 << 3;
        /// `Budget::stack_bytes` is enforced.
        const STACK_CAP = 1 << 4;
        /// NaN results are canonical, so engines agree bit for bit.
        const CANON_NAN = 1 << 5;
        /// The engine runs `simd128` modules.
        const SIMD128 = 1 << 6;
        /// Calls return synchronously. A worker proxy does not set this.
        const SYNC_CALL = 1 << 7;
    }
}

impl Caps {
    /// True when the engine can stop a guest that runs past its deadline.
    pub fn enforces_deadline(self) -> bool {
        self.intersects(Caps::DEADLINE_PREEMPT | Caps::DEADLINE_KILL)
    }
}

/// Which engine is running a plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EngineId {
    Wasmi,
    Wasmtime,
    BrowserJit,
    BrowserJitWorker,
}

/// Whether the host trusts the plugin author.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Trust {
    Trusted,
    Untrusted,
}

/// The rule for untrusted plugins: an untrusted plugin needs an engine that enforces the deadline.
pub fn require_deadline_enforcement(id: EngineId, caps: Caps, trust: Trust) -> Result<(), LoadError> {
    if trust == Trust::Untrusted && !caps.enforces_deadline() {
        return Err(LoadError::EngineCannotEnforceDeadline { engine: id });
    }
    Ok(())
}

/// Refuse a package the gate did not accept. Shared by the engines that compile it themselves.
#[cfg(any(feature = "wasmi", feature = "browser"))]
pub(crate) fn require_accepted(report: &PolicyReport) -> Result<(), LoadError> {
    if let Some(abi) = report.legacy {
        return Err(LoadError::LegacyNeedsShim(abi));
    }
    if !report.is_accepted() {
        return Err(LoadError::Gate(report.violations.clone()));
    }
    Ok(())
}

/// Limits for one call. Each engine enforces the parts its `Caps` list.
#[derive(Clone, Debug)]
pub struct Budget {
    /// Fuel units. The scale depends on the engine. `None` means no total cap.
    pub work: Option<u64>,
    /// Wall time for one call, measured by the host [`Clock`]. `None` means no deadline.
    pub deadline: Option<Duration>,
    /// Hard memory cap in bytes, including the initial memory.
    pub memory_bytes: u64,
    /// Guest call stack budget in bytes. An instance runs with the value its module was compiled for.
    pub stack_bytes: u32,
    /// Cooperative cancel. Engines poll it at every slice boundary.
    pub cancel: Arc<AtomicBool>,
}

impl Budget {
    /// A budget with no work cap and no deadline. Set the public fields to tighten it.
    pub fn new(memory_bytes: u64, stack_bytes: u32) -> Budget {
        Budget { work: None, deadline: None, memory_bytes, stack_bytes, cancel: Arc::new(AtomicBool::new(false)) }
    }
}

/// Why a call stopped before it returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trap {
    /// `Budget::work` ran out.
    OutOfWork,
    /// `Budget::deadline` passed.
    Deadline,
    /// `Budget::cancel` was set.
    Cancelled,
    /// `memory.grow` went past `Budget::memory_bytes`.
    OutOfMemory,
    /// The guest call stack went past its budget.
    StackOverflow,
    /// The guest ran `unreachable`.
    Unreachable,
    /// A host call dispatcher killed the guest for using an op it may not use.
    Denied(Op),
    /// A pointer and length pair fell outside guest memory.
    BadPointer,
    /// The export is not in this module.
    NoSuchExport(Export),
    /// The call used the wrong number of arguments.
    BadArity { export: Export, expected: usize, got: usize },
    /// The call budget asks for a stack or memory limit this instance cannot enforce.
    BudgetMismatch,
    /// Another guest fault, such as an out of bounds load, a divide by zero, or an indirect call type error.
    GuestFault(String),
    /// A host side failure inside the engine.
    Other(String),
}

impl fmt::Display for Trap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Trap::OutOfWork => f.write_str("work budget exhausted"),
            Trap::Deadline => f.write_str("deadline passed"),
            Trap::Cancelled => f.write_str("cancelled"),
            Trap::OutOfMemory => f.write_str("memory cap exceeded"),
            Trap::StackOverflow => f.write_str("guest stack overflow"),
            Trap::Unreachable => f.write_str("guest executed unreachable"),
            Trap::Denied(op) => write!(f, "op {} denied", op.name()),
            Trap::BadPointer => f.write_str("pointer outside guest memory"),
            Trap::NoSuchExport(e) => write!(f, "module has no export {}", e.name()),
            Trap::BadArity { export, expected, got } => write!(f, "{} takes {expected} arguments, got {got}", export.name()),
            Trap::BudgetMismatch => f.write_str("call budget differs from the budget this instance was built for"),
            Trap::GuestFault(m) => write!(f, "guest fault: {m}"),
            Trap::Other(m) => write!(f, "engine error: {m}"),
        }
    }
}

impl std::error::Error for Trap {}

/// The craft-abi 1 exports the host calls (ABI.md section 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Export {
    /// `() -> i32`
    Abi,
    /// `(len, align) -> i32`
    Alloc,
    /// `(ptr, len, align)`
    Free,
    /// `(entry, req_ptr, req_len, blob_ptr, blob_len) -> i64`
    Invoke,
    /// `(desc_ptr, desc_len) -> i32`
    Process,
    /// `(out_ptr_ptr) -> i32`. Optional.
    StateSave,
    /// `(ptr, len, from_ver_ptr, from_ver_len) -> i32`. Optional.
    StateLoad,
}

impl Export {
    pub const ALL: [Export; 7] = [Export::Abi, Export::Alloc, Export::Free, Export::Invoke, Export::Process, Export::StateSave, Export::StateLoad];

    pub fn name(self) -> &'static str {
        match self {
            Export::Abi => "craft_abi",
            Export::Alloc => "craft_alloc",
            Export::Free => "craft_free",
            Export::Invoke => "craft_invoke",
            Export::Process => "craft_process",
            Export::StateSave => "craft_state_save",
            Export::StateLoad => "craft_state_load",
        }
    }

    /// Number of `i32` arguments.
    pub fn arity(self) -> usize {
        match self {
            Export::Abi => 0,
            Export::Alloc => 2,
            Export::Free => 3,
            Export::Invoke => 5,
            Export::Process => 2,
            Export::StateSave => 1,
            Export::StateLoad => 4,
        }
    }

    /// True for the exports every package must have.
    pub fn is_required(self) -> bool {
        matches!(self, Export::Abi | Export::Alloc | Export::Free)
    }
}

/// How a host call fails. Success is the byte count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostError {
    /// The output buffer was too small. The guest sees `E_TOO_SMALL` and `required` as the count.
    TooSmall { required: usize },
    /// Any other non-zero status. `Status::Ok` and `Status::TooSmall` are not valid here.
    Status(Status),
    /// Stop the guest. The call returns this trap to the caller.
    Trap(Trap),
}

/// The bytes written into the guest supplied output buffer, or why the call failed.
pub type HostResult = Result<usize, HostError>;

/// A host call dispatcher. Arguments: the op, the request bytes, and the guest's output buffer.
/// The engine reads the request and hands out the buffer straight from guest memory, with no copy
/// unless the guest made them overlap. The dispatcher must not call back into the guest.
pub trait HostFn: Fn(Op, &[u8], &mut [u8]) -> HostResult + MaybeSendSync {}
impl<F: Fn(Op, &[u8], &mut [u8]) -> HostResult + MaybeSendSync> HostFn for F {}

/// Shared handle to a dispatcher. Not `Send` on `wasm32`.
pub type HostDispatch = Arc<dyn HostFn>;

/// Why a package did not load.
#[derive(Clone, Debug, PartialEq)]
pub enum LoadError {
    /// The policy gate rejected the package.
    Gate(Vec<Violation>),
    /// The package uses a v1 ABI. It loads through a shim, not through this path.
    LegacyNeedsShim(LegacyAbi),
    /// An untrusted plugin needs an engine that can stop it at a deadline.
    EngineCannotEnforceDeadline { engine: EngineId },
    /// The engine rejected the bytes.
    Compile(String),
    /// Instantiation failed: link error, start function trap, or missing required export.
    Instantiate(String),
    /// The initial memory is larger than `Budget::memory_bytes`.
    MemoryOverBudget,
    /// The instance budget asks for a stack size the module was not compiled for.
    StackBudgetMismatch { compiled: u32, requested: u32 },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Gate(v) => {
                f.write_str("policy gate rejected the package:")?;
                for x in v {
                    write!(f, " [{x}]")?;
                }
                Ok(())
            }
            LoadError::LegacyNeedsShim(a) => write!(f, "{a:?} package needs its legacy shim"),
            LoadError::EngineCannotEnforceDeadline { engine } => {
                write!(f, "{engine:?} cannot enforce a deadline, so it may not run an untrusted plugin")
            }
            LoadError::Compile(m) => write!(f, "compile failed: {m}"),
            LoadError::Instantiate(m) => write!(f, "instantiate failed: {m}"),
            LoadError::MemoryOverBudget => f.write_str("initial memory is larger than the memory budget"),
            LoadError::StackBudgetMismatch { compiled, requested } => {
                write!(f, "module compiled for a {compiled} byte stack, budget asks for {requested}")
            }
        }
    }
}

impl std::error::Error for LoadError {}

/// Options that bind a module at compile time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompileOptions {
    pub trust: Trust,
    /// Guest call stack budget. Engines fix it when they compile, so every instance uses it.
    pub stack_bytes: u32,
}

/// An execution engine.
pub trait Engine: MaybeSendSync {
    fn id(&self) -> EngineId;
    fn caps(&self) -> Caps;
    /// Compile a package the gate accepted. Fails with `LoadError::Gate` on a rejected report and
    /// with `LoadError::EngineCannotEnforceDeadline` for an untrusted plugin on a weak engine.
    fn compile<'a>(&'a self, wasm: &'a [u8], report: &'a PolicyReport, opts: CompileOptions) -> BoxFuture<'a, Result<Arc<dyn Module>, LoadError>>;
}

/// A compiled module. Shared. Hot reload swaps the `Arc`.
pub trait Module: MaybeSendSync {
    fn instantiate<'a>(&'a self, host: HostDispatch, budget: &'a Budget) -> BoxFuture<'a, Result<Box<dyn Instance>, LoadError>>;
}

/// A running module with its own memory.
pub trait Instance: MaybeSend {
    /// Call an export. `args` are `i32` values. The result is the export's return value, with an
    /// `i32` result sign extended and a unit result as 0.
    fn call(&mut self, export: Export, args: &[i32], budget: &Budget) -> Result<i64, Trap>;
    /// Linear memory size in bytes.
    fn mem_len(&self) -> usize;
    /// Run `f` on guest memory in place. No copy on wasmi and wasmtime.
    fn with_mem(&mut self, f: &mut dyn FnMut(&mut [u8])) -> Result<(), Trap>;
    /// Copy `src` into guest memory at `ptr`. `Trap::BadPointer` if the range is outside memory.
    fn write(&mut self, ptr: u32, src: &[u8]) -> Result<(), Trap>;
    /// Copy guest memory at `ptr` into `dst`. `Trap::BadPointer` if the range is outside memory.
    fn read(&self, ptr: u32, dst: &mut [u8]) -> Result<(), Trap>;
}

/// The range `ptr..ptr + len` inside a memory of `mem_len` bytes, or `Trap::BadPointer`.
pub fn checked_range(ptr: u32, len: usize, mem_len: usize) -> Result<std::ops::Range<usize>, Trap> {
    let start = ptr as usize;
    let end = start.checked_add(len).ok_or(Trap::BadPointer)?;
    if end > mem_len {
        return Err(Trap::BadPointer);
    }
    Ok(start..end)
}
