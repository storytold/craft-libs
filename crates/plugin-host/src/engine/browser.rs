//! The browser engines: the page's own WebAssembly, driven through `js_sys::WebAssembly`.
//!
//! Two engines share one compile step.
//!
//! - [`BrowserEngine`] (`EngineId::BrowserJit`) runs a module on the thread that calls it. Calls are
//!   synchronous. The host call import is a JS closure that runs the [`HostDispatch`](super::HostDispatch)
//!   on a copy of the guest's input and output ranges. Nothing can interrupt a running guest on that
//!   thread, so the engine reports no deadline and the loader refuses untrusted plugins on it.
//! - [`worker::BrowserWorkerEngine`] (`EngineId::BrowserJitWorker`) runs a pure kernel in a Web
//!   Worker. The host stops a late kernel with `Worker.terminate()`. Its calls are async, so it has
//!   no `SYNC_CALL` and no host calls.
//!
//! What a browser cannot do, and what each engine therefore leaves out of its [`Caps`]:
//!
//! | Budget | Browser engines |
//! |---|---|
//! | Fuel | none. `Budget::work` is ignored. |
//! | Deadline | main thread: none. Worker: wall clock, by terminate, which loses the instance. |
//! | Memory | a hard cap, by lowering the declared memory maximum before compile ([`memory_patch`](super::memory_patch)). A grow past it returns -1 to the guest. wasmi traps with `OutOfMemory` instead. |
//! | Stack | engine defined. The depth that overflows differs per browser and per call site. |
//! | NaN | not canonical. Two engines can differ in NaN payload bits. |
//! | Cancel | main thread: checked before a call. Worker: polled while the call runs, ends in terminate. |
//!
//! `Instance::with_mem` copies the whole memory out and back, because safe Rust cannot borrow a JS
//! `ArrayBuffer` in place. Use `read` and `write` for large buffers.

pub mod worker;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use craft_plugin_abi::PolicyReport;
use js_sys::{Array, ArrayBuffer, Function, Object, Reflect, Uint8Array, WebAssembly};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use super::hostcall::{RawCall, decode_call, pack_outcome};
use super::js_fault::classify_js_error;
use super::memory_patch::{MemoryLimits, memory_limits, pages_in, with_memory_max};
use super::{
    BoxFuture, Budget, Caps, Clock, CompileOptions, Engine, EngineId, Export, HostDispatch, HostFn, Instance, LoadError, Module, Trap, checked_range,
    require_accepted, require_deadline_enforcement,
};

/// A [`Clock`] over `performance.now()`, for wasmi on a `wasm32` page.
#[derive(Debug)]
pub struct WebClock {
    origin: web_time::Instant,
}

impl WebClock {
    pub fn new() -> WebClock {
        WebClock { origin: web_time::Instant::now() }
    }
}

impl Default for WebClock {
    fn default() -> Self {
        WebClock::new()
    }
}

impl Clock for WebClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

/// A module with one `v128.const`. `WebAssembly.validate` accepts it only when the browser has simd128.
const SIMD_PROBE: [u8; 31] = [0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 123, 3, 2, 1, 0, 10, 10, 1, 8, 0, 65, 0, 253, 15, 253, 98, 11];

/// True when this browser validates simd128 modules.
pub fn simd128_supported() -> bool {
    WebAssembly::validate(&Uint8Array::from(&SIMD_PROBE[..])).unwrap_or(false)
}

/// Describe a JS exception for an error message.
pub(crate) fn describe(e: &JsValue) -> String {
    match e.dyn_ref::<js_sys::Error>() {
        Some(err) => format!("{}: {}", err.name(), err.message()),
        None => e.as_string().unwrap_or_else(|| format!("{e:?}")),
    }
}

/// The trap for a JS exception.
pub(crate) fn trap_of(e: &JsValue) -> Trap {
    match e.dyn_ref::<js_sys::Error>() {
        Some(err) => classify_js_error(&String::from(err.name()), &String::from(err.message())),
        None => Trap::GuestFault(describe(e)),
    }
}

async fn compile_js(wasm: &[u8]) -> Result<WebAssembly::Module, LoadError> {
    let module = JsFuture::from(WebAssembly::compile(&Uint8Array::from(wasm))).await.map_err(|e| LoadError::Compile(describe(&e)))?;
    module.dyn_into::<WebAssembly::Module>().map_err(|_| LoadError::Compile("WebAssembly.compile did not return a Module".into()))
}

/// A compiled package: the browser's `Module`, plus what the memory cap and the stack check need.
/// Both engines compile through this type.
pub(crate) struct Compiled {
    wasm: Vec<u8>,
    js: WebAssembly::Module,
    limits: MemoryLimits,
    stack_bytes: u32,
    /// Modules recompiled with a lower memory maximum, by page count.
    capped: RefCell<HashMap<u32, WebAssembly::Module>>,
}

impl Compiled {
    /// Gate result, compile, and read the declared memory.
    async fn new(wasm: &[u8], report: &PolicyReport, opts: CompileOptions) -> Result<Compiled, LoadError> {
        require_accepted(report)?;
        let limits = memory_limits(wasm)
            .map_err(|e| LoadError::Compile(e.to_string()))?
            .ok_or_else(|| LoadError::Compile("the module declares no memory".into()))?;
        let js = compile_js(wasm).await?;
        Ok(Compiled { wasm: wasm.to_vec(), js, limits, stack_bytes: opts.stack_bytes, capped: RefCell::new(HashMap::new()) })
    }

    /// The module to instantiate under `budget`: the original when its declared maximum already
    /// fits the budget, else a copy with the maximum lowered to the budget.
    pub(crate) async fn for_budget(&self, budget: &Budget) -> Result<WebAssembly::Module, LoadError> {
        if budget.stack_bytes != self.stack_bytes {
            return Err(LoadError::StackBudgetMismatch { compiled: self.stack_bytes, requested: budget.stack_bytes });
        }
        let cap = pages_in(budget.memory_bytes);
        if self.limits.min_pages > cap {
            return Err(LoadError::MemoryOverBudget);
        }
        if self.limits.max_pages.is_some_and(|m| m <= cap) {
            return Ok(self.js.clone());
        }
        if let Some(hit) = self.capped.borrow().get(&cap) {
            return Ok(hit.clone());
        }
        let patched = with_memory_max(&self.wasm, cap).map_err(|e| LoadError::Compile(e.to_string()))?;
        let module = compile_js(&patched).await?;
        self.capped.borrow_mut().insert(cap, module.clone());
        Ok(module)
    }
}

/// The engine that runs a module on the calling thread.
#[derive(Debug)]
pub struct BrowserEngine {
    simd128: bool,
}

impl BrowserEngine {
    /// Probes the browser once for simd128.
    pub fn new() -> BrowserEngine {
        BrowserEngine { simd128: simd128_supported() }
    }
}

impl Default for BrowserEngine {
    fn default() -> Self {
        BrowserEngine::new()
    }
}

impl Engine for BrowserEngine {
    fn id(&self) -> EngineId {
        EngineId::BrowserJit
    }

    /// `SYNC_CALL`, `MEMORY_CAP`, and `SIMD128` when the browser has it. Never a deadline, fuel,
    /// a stack cap or canonical NaN.
    fn caps(&self) -> Caps {
        let mut caps = Caps::SYNC_CALL | Caps::MEMORY_CAP;
        if self.simd128 {
            caps |= Caps::SIMD128;
        }
        caps
    }

    fn compile<'a>(&'a self, wasm: &'a [u8], report: &'a PolicyReport, opts: CompileOptions) -> BoxFuture<'a, Result<Arc<dyn Module>, LoadError>> {
        Box::pin(async move {
            require_deadline_enforcement(self.id(), self.caps(), opts.trust)?;
            let compiled = Compiled::new(wasm, report, opts).await?;
            Ok(Arc::new(BrowserModule { compiled }) as Arc<dyn Module>)
        })
    }
}

/// A compiled module for [`BrowserEngine`].
pub struct BrowserModule {
    compiled: Compiled,
}

impl Module for BrowserModule {
    fn instantiate<'a>(&'a self, host: HostDispatch, budget: &'a Budget) -> BoxFuture<'a, Result<Box<dyn Instance>, LoadError>> {
        Box::pin(async move {
            let module = self.compiled.for_budget(budget).await?;
            Ok(Box::new(BrowserInstance::new(&module, host, self.compiled.stack_bytes)?) as Box<dyn Instance>)
        })
    }
}

/// State the `craft_host.call` closure shares with its instance.
struct Shared {
    memory: RefCell<Option<WebAssembly::Memory>>,
    /// A trap the dispatcher raised. It wins over the generic JS exception that unwinds the guest.
    pending: RefCell<Option<Trap>>,
}

/// Bytes of the guest memory `range`, copied out.
fn copy_out(view: &Uint8Array, range: &std::ops::Range<usize>) -> Result<Vec<u8>, Trap> {
    let (start, end) = (u32::try_from(range.start).map_err(|_| Trap::BadPointer)?, u32::try_from(range.end).map_err(|_| Trap::BadPointer)?);
    let mut bytes = vec![0u8; range.len()];
    if !bytes.is_empty() {
        view.subarray(start, end).copy_to(&mut bytes);
    }
    Ok(bytes)
}

/// One `craft_host.call`. Reads the input range and the output range out of guest memory, runs the
/// dispatcher on the copies, and writes the output range back. The decode and packing rules are the
/// ones wasmi and wasmtime use (`hostcall`).
fn host_call(dispatch: &dyn HostFn, memory: &WebAssembly::Memory, call: &RawCall) -> Result<i64, Trap> {
    let view = Uint8Array::new(&memory.buffer());
    let decoded = match decode_call(call, view.length() as usize) {
        Ok(d) => d,
        Err(packed) => return Ok(packed),
    };
    let input = copy_out(&view, &decoded.input)?;
    let mut output = copy_out(&view, &decoded.output)?;
    let out_cap = output.len();
    let result = dispatch(decoded.op, &input, &mut output);
    if !output.is_empty() {
        let start = u32::try_from(decoded.output.start).map_err(|_| Trap::BadPointer)?;
        view.subarray(start, start + out_cap as u32).copy_from(&output);
    }
    pack_outcome(result, out_cap)
}

/// The JS function behind `craft_host.call`. It returns an `Err` to throw into the guest.
type HostImport = Closure<dyn FnMut(i32, i32, i32, i32, i32) -> Result<i64, JsValue>>;

/// A running module on the calling thread.
pub struct BrowserInstance {
    memory: WebAssembly::Memory,
    funcs: [Option<Function>; Export::ALL.len()],
    shared: Rc<Shared>,
    stack_bytes: u32,
    /// The JS closure behind the import. The instance owns it so the import stays valid.
    _import: HostImport,
}

impl BrowserInstance {
    fn new(module: &WebAssembly::Module, host: HostDispatch, stack_bytes: u32) -> Result<BrowserInstance, LoadError> {
        let shared = Rc::new(Shared { memory: RefCell::new(None), pending: RefCell::new(None) });
        let import = {
            let shared = Rc::clone(&shared);
            HostImport::new(move |op, in_ptr, in_len, out_ptr, out_cap| {
                let Some(memory) = shared.memory.borrow().clone() else {
                    return Err(JsValue::from_str("craft_host.call ran before the module memory was bound"));
                };
                let raw = RawCall { op: op as u32, in_ptr: in_ptr as u32, in_len: in_len as u32, out_ptr: out_ptr as u32, out_cap: out_cap as u32 };
                host_call(&*host, &memory, &raw).map_err(|trap| {
                    *shared.pending.borrow_mut() = Some(trap);
                    JsValue::from_str("craft_host.call: the host stopped the guest")
                })
            })
        };
        let inst_err = |what: &str, e: &JsValue| LoadError::Instantiate(format!("{what}: {}", describe(e)));
        let host_ns = Object::new();
        Reflect::set(&host_ns, &"call".into(), import.as_ref()).map_err(|e| inst_err("import object", &e))?;
        let imports = Object::new();
        Reflect::set(&imports, &"craft_host".into(), &host_ns).map_err(|e| inst_err("import object", &e))?;
        let instance = WebAssembly::Instance::new(module, &imports).map_err(|e| inst_err("instantiate", &e))?;
        let exports = instance.exports();
        let memory = Reflect::get(&exports, &"memory".into())
            .ok()
            .and_then(|m| m.dyn_into::<WebAssembly::Memory>().ok())
            .ok_or_else(|| LoadError::Instantiate("the module does not export `memory`".into()))?;
        *shared.memory.borrow_mut() = Some(memory.clone());
        let mut funcs: [Option<Function>; Export::ALL.len()] = [const { None }; Export::ALL.len()];
        for export in Export::ALL {
            let func = Reflect::get(&exports, &export.name().into()).ok().and_then(|f| f.dyn_into::<Function>().ok());
            if func.is_none() && export.is_required() {
                return Err(LoadError::Instantiate(format!("the module does not export `{}`", export.name())));
            }
            funcs[export as usize] = func;
        }
        Ok(BrowserInstance { memory, funcs, shared, stack_bytes, _import: import })
    }

    fn view(&self) -> Uint8Array {
        Uint8Array::new(&self.memory.buffer())
    }
}

/// The value a call returned, as the `i64` the trait promises.
fn call_result(export: Export, value: &JsValue) -> Result<i64, Trap> {
    if export == Export::Free {
        return Ok(0);
    }
    if export == Export::Invoke {
        return i64::try_from(value.clone()).map_err(|_| Trap::Other(format!("export {} did not return an i64", export.name())));
    }
    value.as_f64().map(|v| i64::from(v as i32)).ok_or_else(|| Trap::Other(format!("export {} did not return an i32", export.name())))
}

impl Instance for BrowserInstance {
    fn call(&mut self, export: Export, args: &[i32], budget: &Budget) -> Result<i64, Trap> {
        if args.len() != export.arity() {
            return Err(Trap::BadArity { export, expected: export.arity(), got: args.len() });
        }
        if budget.stack_bytes != self.stack_bytes {
            return Err(Trap::BudgetMismatch);
        }
        let func = self.funcs[export as usize].clone().ok_or(Trap::NoSuchExport(export))?;
        if budget.cancel.load(Ordering::Relaxed) {
            return Err(Trap::Cancelled);
        }
        self.shared.pending.borrow_mut().take();
        let js_args = Array::new();
        for a in args {
            js_args.push(&JsValue::from(*a));
        }
        match func.apply(&JsValue::UNDEFINED, &js_args) {
            Ok(value) => call_result(export, &value),
            Err(e) => Err(self.shared.pending.borrow_mut().take().unwrap_or_else(|| trap_of(&e))),
        }
    }

    fn mem_len(&self) -> usize {
        self.memory.buffer().unchecked_into::<ArrayBuffer>().byte_length() as usize
    }

    fn with_mem(&mut self, f: &mut dyn FnMut(&mut [u8])) -> Result<(), Trap> {
        let view = self.view();
        let mut copy = vec![0u8; view.length() as usize];
        view.copy_to(&mut copy);
        f(&mut copy);
        view.copy_from(&copy);
        Ok(())
    }

    fn write(&mut self, ptr: u32, src: &[u8]) -> Result<(), Trap> {
        let view = self.view();
        let range = checked_range(ptr, src.len(), view.length() as usize)?;
        if !src.is_empty() {
            view.subarray(range.start as u32, range.end as u32).copy_from(src);
        }
        Ok(())
    }

    fn read(&self, ptr: u32, dst: &mut [u8]) -> Result<(), Trap> {
        let view = self.view();
        let range = checked_range(ptr, dst.len(), view.length() as usize)?;
        if !dst.is_empty() {
            view.subarray(range.start as u32, range.end as u32).copy_to(dst);
        }
        Ok(())
    }
}
