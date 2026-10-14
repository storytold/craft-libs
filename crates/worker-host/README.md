# craft-worker-host

Rust protocol types (`HostMessage`, `WorkerMessage`) and the browser bootstrap script
[`assets/craft-worker-host.js`](../../assets/craft-worker-host.js) for Craft `-web` apps.

## Message tags

Serde uses internally tagged JSON with `"type"` in **snake_case**:

| Direction | `type` values |
|-----------|----------------|
| Tab → worker | `init`, `storage_flush`, `gpu_jobs`, `cancel` |
| Worker → tab | `ready`, `gpu_batch_done`, `error` |

Unit variants (`storage_flush`, `cancel`) are objects with only `"type"`.

## WASM hook

Before or after loading WASM, assign:

```js
globalThis.craftRuntimeHandleGpuBatch = async ({ batch_id, jobs }) => {
  // return GpuJobOutcome[] aligned with `jobs`
};
```

If unset, the stub completes `gpu_jobs` with empty outcomes.

## craft.quasarke.net (CollabCraft static host)

Serve `craft-worker-host.js` from the same origin as the Craft app (or a dedicated
static path on `https://craft.quasarke.net`). Tabs construct the worker with that URL.

**Content-Security-Policy** must allow the SharedWorker script and worker execution:

- `script-src` must include the origin that serves `craft-worker-host.js` (typically
  `'self'` on `craft.quasarke.net`).
- `worker-src` must include the same origin (and `'self'`). Without `worker-src`,
  browsers fall back to `script-src`; keep them consistent so SharedWorker load is not
  blocked after deploy.
- Avoid `worker-src 'none'` on pages that use Craft runtime.

Example baseline for app HTML on `craft.quasarke.net`:

```
Content-Security-Policy: default-src 'self'; script-src 'self'; worker-src 'self'; connect-src 'self' wss: https:;
```

Adjust `connect-src` for API, WebSocket, and Nextcloud endpoints your app uses. Precompress
with brotli (`craft-worker-host.js.br`) when the static host supports content negotiation.

## Tests

`cargo test -p craft-worker-host` includes JSON shape assertions for `Init` and `GpuJobs`
that must stay aligned with the JS bootstrap and `craft-runtime` callers.
