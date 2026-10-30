//! The Worker engine: a pure kernel in a Web Worker that the host can terminate.
//!
//! The Worker holds the instance. The page talks to it by `postMessage`, one request at a time.
//! While a request is open the page races the reply against a short timer. On each tick it checks
//! the cancel flag and the deadline, and a late or cancelled call ends in `Worker.terminate()`.
//! Termination loses the instance, which is the price of a deadline the page cannot get otherwise.
//!
//! The guest has no host. Its `craft_host.call` import answers `E_UNSUPPORTED` to every op. Control
//! plane plugins, which need the document, stay on wasmi.
//!
//! Copies per call: bytes go page heap -> transferred `ArrayBuffer` -> guest memory, and the same
//! way back. That is four copies, against two on the page thread engine.
//!
//! The worker script is a string in this file, loaded from a Blob URL. A page whose CSP forbids
//! `blob:` workers makes [`BrowserWorkerModule::spawn`] fail with `LoadError::Instantiate`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use craft_plugin_abi::PolicyReport;
use craft_plugin_abi::Status;
use craft_plugin_abi::descriptor::{DESCRIPTOR_V1_SIZE, ProcessDescriptor};
use craft_plugin_abi::ops::pack_call_return;
use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Blob, BlobPropertyBag, ErrorEvent, MessageEvent, Url, Worker};

use super::{Compiled, describe, simd128_supported};
use crate::engine::js_fault::classify_js_error;
use crate::engine::{
    BoxFuture, Budget, Caps, CompileOptions, Engine, EngineId, Export, HostDispatch, Instance, LoadError, Module, Trap, require_deadline_enforcement,
};

/// How often the page looks at the cancel flag and the deadline while a request is open.
const TICK_MS: u64 = 10;
/// Deadline for starting the worker and instantiating the module when the budget has none.
const SPAWN_DEADLINE: Duration = Duration::from_secs(5);

/// The worker side. One message in, one message out.
///
/// - `init {module, unsupported}`: instantiate. Reply `{ok, memLen}`.
/// - `call {name, args}`: reply `{ok, value, memLen}`; `{ok:false, missing}` when the export is absent.
/// - `write {ptr, bytes}` and `read {ptr, len}`: bounds checked in the worker, reply `{ok, bytes?}` or `{ok:false, badPointer}`.
///
/// A JS exception becomes `{ok:false, name, message}`, which the page maps with `classify_js_error`.
const WORKER_JS: &str = r#"
let inst = null;
let mem = null;
self.onmessage = (ev) => {
  const m = ev.data;
  try {
    if (m.t === 'init') {
      const unsupported = m.unsupported;
      inst = new WebAssembly.Instance(m.module, { craft_host: { call: () => unsupported } });
      mem = inst.exports.memory;
      self.postMessage({ ok: true, memLen: mem.buffer.byteLength });
    } else if (m.t === 'call') {
      const f = inst.exports[m.name];
      if (typeof f !== 'function') { self.postMessage({ ok: false, missing: true }); return; }
      const value = f(...m.args);
      self.postMessage({ ok: true, value: value === undefined ? 0 : value, memLen: mem.buffer.byteLength });
    } else if (m.t === 'write') {
      const bytes = new Uint8Array(m.bytes);
      if (m.ptr + bytes.length > mem.buffer.byteLength) { self.postMessage({ ok: false, badPointer: true }); return; }
      new Uint8Array(mem.buffer).set(bytes, m.ptr);
      self.postMessage({ ok: true });
    } else if (m.t === 'read') {
      if (m.ptr + m.len > mem.buffer.byteLength) { self.postMessage({ ok: false, badPointer: true }); return; }
      const out = new Uint8Array(mem.buffer).slice(m.ptr, m.ptr + m.len);
      self.postMessage({ ok: true, bytes: out.buffer }, [out.buffer]);
    }
  } catch (e) {
    self.postMessage({ ok: false, name: String(e && e.name), message: String(e && e.message) });
  }
};
"#;

/// The Worker engine. Compiles on the page, runs on a worker it can kill.
#[derive(Debug)]
pub struct BrowserWorkerEngine {
    simd128: bool,
}

impl BrowserWorkerEngine {
    pub fn new() -> BrowserWorkerEngine {
        BrowserWorkerEngine { simd128: simd128_supported() }
    }
}

impl Default for BrowserWorkerEngine {
    fn default() -> Self {
        BrowserWorkerEngine::new()
    }
}

impl Engine for BrowserWorkerEngine {
    fn id(&self) -> EngineId {
        EngineId::BrowserJitWorker
    }

    /// `DEADLINE_KILL`, `MEMORY_CAP`, and `SIMD128` when the browser has it. No `SYNC_CALL`: every
    /// call is a message. Never fuel, a stack cap or canonical NaN.
    fn caps(&self) -> Caps {
        let mut caps = Caps::DEADLINE_KILL | Caps::MEMORY_CAP;
        if self.simd128 {
            caps |= Caps::SIMD128;
        }
        caps
    }

    fn compile<'a>(&'a self, wasm: &'a [u8], report: &'a PolicyReport, opts: CompileOptions) -> BoxFuture<'a, Result<Arc<dyn Module>, LoadError>> {
        Box::pin(async move { self.compile_worker(wasm, report, opts).await.map(|m| m as Arc<dyn Module>) })
    }
}

impl BrowserWorkerEngine {
    /// Compile, keeping the concrete type so the caller can [`spawn`](BrowserWorkerModule::spawn).
    pub async fn compile_worker(&self, wasm: &[u8], report: &PolicyReport, opts: CompileOptions) -> Result<Arc<BrowserWorkerModule>, LoadError> {
        require_deadline_enforcement(self.id(), self.caps(), opts.trust)?;
        Ok(Arc::new(BrowserWorkerModule { compiled: Compiled::new(wasm, report, opts).await? }))
    }
}

/// A compiled module for [`BrowserWorkerEngine`].
pub struct BrowserWorkerModule {
    compiled: Compiled,
}

impl BrowserWorkerModule {
    /// Start a worker and instantiate the module in it. The budget's deadline (else five seconds)
    /// covers the start, so a module whose start code never returns is killed.
    pub async fn spawn(&self, budget: &Budget) -> Result<WorkerInstance, LoadError> {
        let module = self.compiled.for_budget(budget).await?;
        let mut worker = WorkerInstance::start(self.compiled.stack_bytes)?;
        let init = Object::new();
        set(&init, "t", &"init".into()).map_err(LoadError::Instantiate)?;
        set(&init, "module", &module).map_err(LoadError::Instantiate)?;
        let unsupported = pack_call_return(Status::Unsupported, 0);
        set(&init, "unsupported", &JsValue::from(unsupported)).map_err(LoadError::Instantiate)?;
        let mut start = budget.clone();
        start.deadline = Some(budget.deadline.unwrap_or(SPAWN_DEADLINE));
        match worker.request(&init, None, &start).await {
            Ok(reply) => worker.mem_len = field_f64(&reply, "memLen") as usize,
            Err(Trap::Deadline) => return Err(LoadError::Instantiate("the module did not start before the deadline".into())),
            Err(trap) => return Err(LoadError::Instantiate(trap.to_string())),
        }
        Ok(worker)
    }
}

impl Module for BrowserWorkerModule {
    /// Always fails. `Instance` is synchronous and a worker is not. Use [`BrowserWorkerModule::spawn`].
    fn instantiate<'a>(&'a self, _host: HostDispatch, _budget: &'a Budget) -> BoxFuture<'a, Result<Box<dyn Instance>, LoadError>> {
        Box::pin(std::future::ready(Err(LoadError::Instantiate(
            "the Worker engine has no synchronous Instance; call BrowserWorkerModule::spawn".into(),
        ))))
    }
}

fn set(target: &Object, key: &str, value: &JsValue) -> Result<(), String> {
    Reflect::set(target, &key.into(), value).map(|_| ()).map_err(|e| describe(&e))
}

fn field(reply: &JsValue, key: &str) -> JsValue {
    Reflect::get(reply, &key.into()).unwrap_or(JsValue::UNDEFINED)
}

fn field_f64(reply: &JsValue, key: &str) -> f64 {
    field(reply, key).as_f64().unwrap_or(0.0)
}

fn field_true(reply: &JsValue, key: &str) -> bool {
    field(reply, key).is_truthy()
}

/// `setTimeout` as a promise that resolves with `undefined`. Works in a window and in a worker.
fn sleep(ms: u64) -> Promise {
    Promise::new(&mut |resolve, reject| {
        let global = js_sys::global();
        let timer = Reflect::get(&global, &"setTimeout".into()).ok().and_then(|f| f.dyn_into::<Function>().ok());
        let armed = timer.map(|t| t.call2(&global, &resolve, &JsValue::from_f64(ms as f64)));
        if !matches!(armed, Some(Ok(_))) {
            // No timer: the call cannot honor a deadline. Fail loudly instead of waiting forever.
            let _ = reject.call1(&JsValue::NULL, &JsValue::from_str("setTimeout is unavailable"));
        }
    })
}

/// A running worker with the module instantiated inside it.
pub struct WorkerInstance {
    /// `None` after a deadline or cancel terminated the worker.
    worker: Option<Worker>,
    url: String,
    /// Where the next reply goes. One request is open at a time.
    slot: Rc<RefCell<Option<Function>>>,
    stack_bytes: u32,
    mem_len: usize,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_error: Closure<dyn FnMut(ErrorEvent)>,
}

impl Drop for WorkerInstance {
    fn drop(&mut self) {
        if let Some(w) = self.worker.take() {
            w.terminate();
        }
        let _ = Url::revoke_object_url(&self.url);
    }
}

impl WorkerInstance {
    fn start(stack_bytes: u32) -> Result<WorkerInstance, LoadError> {
        let fail = |what: &str, e: &JsValue| LoadError::Instantiate(format!("{what}: {}", describe(e)));
        let parts = Array::of1(&JsValue::from_str(WORKER_JS));
        let props = BlobPropertyBag::new();
        props.set_type("text/javascript");
        let blob = Blob::new_with_str_sequence_and_options(&parts, &props).map_err(|e| fail("worker script", &e))?;
        let url = Url::create_object_url_with_blob(&blob).map_err(|e| fail("worker url", &e))?;
        let worker = Worker::new(&url).map_err(|e| {
            let _ = Url::revoke_object_url(&url);
            fail("start worker", &e)
        })?;
        let slot: Rc<RefCell<Option<Function>>> = Rc::new(RefCell::new(None));
        let on_message = {
            let slot = Rc::clone(&slot);
            Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
                if let Some(resolve) = slot.borrow_mut().take() {
                    let _ = resolve.call1(&JsValue::NULL, &ev.data());
                }
            })
        };
        let on_error = {
            let slot = Rc::clone(&slot);
            Closure::<dyn FnMut(ErrorEvent)>::new(move |ev: ErrorEvent| {
                if let Some(resolve) = slot.borrow_mut().take() {
                    let reply = Object::new();
                    let _ = Reflect::set(&reply, &"ok".into(), &JsValue::FALSE);
                    let _ = Reflect::set(&reply, &"name".into(), &"WorkerError".into());
                    let _ = Reflect::set(&reply, &"message".into(), &JsValue::from_str(&ev.message()));
                    let _ = resolve.call1(&JsValue::NULL, &reply);
                }
            })
        };
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        Ok(WorkerInstance { worker: Some(worker), url, slot, stack_bytes, mem_len: 0, _on_message: on_message, _on_error: on_error })
    }

    /// Guest memory size in bytes, as of the last reply.
    pub fn mem_len(&self) -> usize {
        self.mem_len
    }

    /// True once a deadline or cancel has terminated the worker. Every later call fails.
    pub fn is_terminated(&self) -> bool {
        self.worker.is_none()
    }

    /// Kill the worker now.
    pub fn terminate(&mut self) {
        if let Some(w) = self.worker.take() {
            w.terminate();
        }
    }

    /// Send `msg` and wait for the reply. Ends in `Trap::Deadline` or `Trap::Cancelled`, and
    /// terminates the worker, when the budget runs out first. A reply with `ok: false` becomes a trap.
    async fn request(&mut self, msg: &Object, transfer: Option<&Array>, budget: &Budget) -> Result<JsValue, Trap> {
        let started = web_time::Instant::now();
        let reply = {
            let worker = self.worker.as_ref().ok_or_else(|| Trap::Other("the worker was terminated by an earlier deadline or cancel".into()))?;
            let slot = Rc::clone(&self.slot);
            let reply = Promise::new(&mut |resolve, _| {
                *slot.borrow_mut() = Some(resolve);
            });
            let posted = match transfer {
                Some(t) => worker.post_message_with_transfer(msg, t),
                None => worker.post_message(msg),
            };
            posted.map_err(|e| Trap::Other(format!("post to worker: {}", describe(&e))))?;
            reply
        };
        loop {
            let remaining = budget.deadline.map(|d| d.saturating_sub(started.elapsed()));
            let wait = remaining.map_or(TICK_MS, |r| (r.as_millis() as u64).min(TICK_MS));
            let raced = Promise::race(&Array::of2(&reply, &sleep(wait)));
            let value = JsFuture::from(raced).await.map_err(|e| Trap::Other(format!("worker wait: {}", describe(&e))))?;
            if !value.is_undefined() {
                return self.parse_reply(&value);
            }
            let stop = if budget.cancel.load(Ordering::Relaxed) {
                Some(Trap::Cancelled)
            } else if budget.deadline.is_some_and(|d| started.elapsed() >= d) {
                Some(Trap::Deadline)
            } else {
                None
            };
            if let Some(trap) = stop {
                self.terminate();
                self.slot.borrow_mut().take();
                return Err(trap);
            }
        }
    }

    fn parse_reply(&mut self, reply: &JsValue) -> Result<JsValue, Trap> {
        if field_true(reply, "ok") {
            if !field(reply, "memLen").is_undefined() {
                self.mem_len = field_f64(reply, "memLen") as usize;
            }
            return Ok(reply.clone());
        }
        if field_true(reply, "badPointer") {
            return Err(Trap::BadPointer);
        }
        if field_true(reply, "missing") {
            return Ok(reply.clone());
        }
        let name = field(reply, "name").as_string().unwrap_or_default();
        let message = field(reply, "message").as_string().unwrap_or_default();
        Err(classify_js_error(&name, &message))
    }

    /// Call an export. Same argument rules as [`Instance::call`].
    pub async fn call(&mut self, export: Export, args: &[i32], budget: &Budget) -> Result<i64, Trap> {
        if args.len() != export.arity() {
            return Err(Trap::BadArity { export, expected: export.arity(), got: args.len() });
        }
        if budget.stack_bytes != self.stack_bytes {
            return Err(Trap::BudgetMismatch);
        }
        if budget.cancel.load(Ordering::Relaxed) {
            return Err(Trap::Cancelled);
        }
        let msg = Object::new();
        let js_args = Array::new();
        for a in args {
            js_args.push(&JsValue::from(*a));
        }
        for (key, value) in [("t", JsValue::from_str("call")), ("name", JsValue::from_str(export.name())), ("args", js_args.into())] {
            set(&msg, key, &value).map_err(Trap::Other)?;
        }
        let reply = self.request(&msg, None, budget).await?;
        if field_true(&reply, "missing") {
            return Err(Trap::NoSuchExport(export));
        }
        let value = field(&reply, "value");
        if export == Export::Invoke {
            return i64::try_from(value).map_err(|_| Trap::Other("craft_invoke did not return an i64".into()));
        }
        Ok(value.as_f64().map_or(0, |v| i64::from(v as i32)))
    }

    /// Copy `src` into guest memory at `ptr`.
    pub async fn write(&mut self, ptr: u32, src: &[u8], budget: &Budget) -> Result<(), Trap> {
        let bytes = Uint8Array::from(src);
        let msg = Object::new();
        for (key, value) in [("t", JsValue::from_str("write")), ("ptr", JsValue::from(ptr)), ("bytes", bytes.buffer().into())] {
            set(&msg, key, &value).map_err(Trap::Other)?;
        }
        self.request(&msg, Some(&Array::of1(&bytes.buffer())), budget).await.map(|_| ())
    }

    /// Copy `len` bytes of guest memory at `ptr` out.
    pub async fn read(&mut self, ptr: u32, len: usize, budget: &Budget) -> Result<Vec<u8>, Trap> {
        let msg = Object::new();
        for (key, value) in [("t", JsValue::from_str("read")), ("ptr", JsValue::from(ptr)), ("len", JsValue::from_f64(len as f64))] {
            set(&msg, key, &value).map_err(Trap::Other)?;
        }
        let reply = self.request(&msg, None, budget).await?;
        Ok(Uint8Array::new(&field(&reply, "bytes")).to_vec())
    }

    /// `craft_alloc(len, 4)`. A null pointer is `Trap::OutOfMemory`.
    async fn alloc(&mut self, len: usize, budget: &Budget) -> Result<u32, Trap> {
        let len = i32::try_from(len).map_err(|_| Trap::OutOfMemory)?;
        match self.call(Export::Alloc, &[len, 4], budget).await? {
            0 => Err(Trap::OutOfMemory),
            p => u32::try_from(p).map_err(|_| Trap::BadPointer),
        }
    }

    /// Run one `craft_process` job the way the host does it: allocate in the guest, copy the input
    /// and the params in, write the descriptor, call, copy the output out. `desc` supplies every
    /// field but the four guest pointers and `params_len`, which this method sets. The deadline in
    /// `budget` covers the whole job. Returns the export's status code and `out_len` output bytes.
    pub async fn process(
        &mut self,
        desc: &ProcessDescriptor,
        input: &[u8],
        params: &[u8],
        out_len: usize,
        budget: &Budget,
    ) -> Result<(i64, Vec<u8>), Trap> {
        let started = web_time::Instant::now();
        let step = || Budget { deadline: budget.deadline.map(|d| d.saturating_sub(started.elapsed())), ..budget.clone() };
        let desc_ptr = self.alloc(DESCRIPTOR_V1_SIZE, &step()).await?;
        let in_ptr = self.alloc(input.len().max(1), &step()).await?;
        let out_ptr = self.alloc(out_len.max(1), &step()).await?;
        let params_ptr = self.alloc(params.len().max(1), &step()).await?;
        self.write(in_ptr, input, &step()).await?;
        self.write(params_ptr, params, &step()).await?;
        let full = ProcessDescriptor { in_ptr, out_ptr, params_ptr, params_len: params.len() as u32, ..desc.clone() };
        self.write(desc_ptr, &full.encode(), &step()).await?;
        let code = self.call(Export::Process, &[desc_ptr as i32, DESCRIPTOR_V1_SIZE as i32], &step()).await?;
        let out = self.read(out_ptr, out_len, &step()).await?;
        Ok((code, out))
    }
}
