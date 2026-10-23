# craft-worker-host

Rust protocol types (`HostMessage`, `WorkerMessage`) and browser bootstrap scripts
[`assets/craft-worker-host.js`](../../assets/craft-worker-host.js) (SharedWorker) and
[`assets/craft-dedicated-worker.js`](../../assets/craft-dedicated-worker.js) (fallback) for Craft `-web` apps.
Shared routing lives in [`assets/craft-worker-host-routing.js`](../../assets/craft-worker-host-routing.js).

## Message tags

Serde uses internally tagged JSON with `"type"` in **snake_case**:

| Direction | `type` values |
|-----------|----------------|
| Tab → worker | `init`, `storage_put`, `storage_flush`, `gpu_jobs`, `cancel` |
| Worker → tab | `ready`, `gpu_batch_done`, `storage_flushed`, `error` |

Unit variants (`storage_flush`, `cancel`) are objects with only `"type"`.

## WASM hooks

Before or after loading WASM, assign on the worker global:

```js
globalThis.craftRuntimeHandleGpuBatch = async ({ batch_id, jobs }) => {
  // return GpuJobOutcome[] aligned with `jobs`
};

globalThis.craftRuntimeFlushPending = async (data) => {
  // return { pending_count: number }
};
```

If `craftRuntimeHandleGpuBatch` is unset, the stub completes `gpu_jobs` with empty outcomes.
If `craftRuntimeFlushPending` is unset, `storage_flush` replies with `pending_count` equal to the drained pending batch length.
Loading `opfs-flush.js` from the worker bootstrap installs a default flush hook.

## WebAssembly.Module sharing

`WebAssembly.Module` is structured-cloneable — it can be posted via `postMessage` without
re-compiling the bytecode. Include it in the `init` message payload to avoid redundant
compilation:

```js
// Main thread: compile once, share across workers.
const module = await WebAssembly.compileStreaming(fetch('/craft-runtime.wasm'));

// SharedWorker path: one worker serves all tabs; module compiles once for the process.
const worker = new SharedWorker('/craft-worker-host.js');
worker.port.postMessage({ type: 'init', app_id: 'my-app', capabilities: [], wasm_module: module });

// DedicatedWorker path (fallback): one worker per tab; each tab posts its own clone.
// The structured clone is free at the JS level -- the browser does not recompile.
const dedicated = new Worker('/craft-dedicated-worker.js');
dedicated.postMessage({ type: 'init', app_id: 'my-app', capabilities: [], wasm_module: module });
```

Install `craftRuntimeReceiveWasmModule` in the worker global (e.g. in `craft-worker-host-routing.js`
or your runtime bootstrap) **before** the `init` message arrives to receive the module:

```js
// Inside the worker (or loaded via importScripts):
globalThis.craftRuntimeReceiveWasmModule = (module) => {
  // Pass to WebAssembly.instantiate(module, imports) instead of re-fetching.
};
```

If `craftRuntimeReceiveWasmModule` is unset the module field is ignored; the worker still
replies `ready` and the runtime falls back to its own fetch/compile path.

**SharedWorker advantage:** a single SharedWorker instance is shared across all tabs of the
same origin, so the module is compiled exactly once per browser session regardless of how
many tabs open. DedicatedWorker spawns a new worker per tab; each tab's worker receives its
own structured-clone of the module (no recompile, but separate instantiation per tab).

## CollabCraft static host

Serve `craft-worker-host.js` from the same origin as the Craft app (or another dedicated
static origin your CSP allows). Tabs construct the worker with that URL.

**Content-Security-Policy** must allow the SharedWorker script and worker execution:

- `script-src` must include the origin that serves `craft-worker-host.js` (typically `'self'`).
- `worker-src` must include the same origin (and `'self'`). Without `worker-src`,
  browsers fall back to `script-src`; keep them consistent so SharedWorker load is not
  blocked after deploy.
- Avoid `worker-src 'none'` on pages that use Craft runtime.

Example baseline for app HTML:

```
Content-Security-Policy: default-src 'self'; script-src 'self'; worker-src 'self'; connect-src 'self' wss: https:;
```

Adjust `connect-src` for API, WebSocket, and storage-provider endpoints your app uses.
Precompress with brotli (`craft-worker-host.js.br`) when the static host supports content
negotiation.

## Tests

`cargo test -p craft-worker-host` includes JSON shape assertions for `Init` and `GpuJobs`
that must stay aligned with the JS bootstrap and `craft-runtime` callers.
