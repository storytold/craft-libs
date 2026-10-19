# craft-runtime (web)

Portable browser runtime for Craft WASM apps: tier storage, SharedWorker RPC, and GPU provider dispatch. **Host-agnostic** — it depends on **`craft-cloud` contracts** and a **`craft-cloud-client`-shaped HTTP gateway**, not on any one product name.

## Layer model (open-ended)

| Layer | Typical crate / repo | Role |
| --- | --- | --- |
| Contracts | **craft-libs → `craft-cloud`** | `Files`, `Entry`, errors — no IO, no vendor |
| HTTP gateway | **Your service** | Session, `/api/v1`-style routes, provider held server-side |
| Browser transport | **`craft-cloud-client`** (today lives in collab-craft) | Route builders + `fetch`; any gateway that speaks the same JSON and routes can be used |
| Tab + worker | **craft-libs → `craft-runtime`**, `craft-worker-host`, `craft-tier-store`, `craft-gpu-*` | Mirror/OPFS glue, worker protocol, GPU registry |

Apps never embed provider credentials or WebDAV. They call the gateway; the gateway implements `craft_cloud::Provider`.

## Reference stack

One deployment shape that exercises these libraries (not a hard-coded limit):

1. **`craft-cloud`** — contracts in craft-libs
2. **collab-craft** — HTTP service + session + provider wiring
3. **`craft-cloud-client`** — wasm/desktop client for those routes (collab-craft repo)
4. **`-web` apps** — EffectCraft, FilmCraft, … on SharedWorker + `craft-runtime`

**Repin order:** craft-libs → collab-craft (when API/contracts change) → application `-web` crates.

Serve app bundles plus runtime JS from the **web root** of the CollabCraft (or equivalent) static host — not a subdirectory named like an app:

| Asset | Notes |
| --- | --- |
| `craft-worker-host.js` | SharedWorker bootstrap (+ `.br` / `.gz` siblings when the host negotiates compression) |
| Same origin for `craft-worker-host-routing.js`, `craft-dedicated-worker.js`, `opfs-flush.js` | Publish beside the app shell |

**Do not** publish under `/runtime/` on CollabCraft: every directory under the web root is treated as a hosted app, so static worker scripts would redirect to sign-in.

Worker `init` uses same-origin `/api/v1` when the app is served from CollabCraft, or `craft-cloud-url` / tab-provided base when static is elsewhere.

## SharedWorker vs dedicated worker

| Asset | When to use |
| --- | --- |
| `assets/craft-worker-host.js` | **Default:** one SharedWorker serves every tab (`WorkerKind::SharedWorker`). |
| `assets/craft-dedicated-worker.js` | **Fallback:** same JSON protocol on a per-tab DedicatedWorker when SharedWorker cannot create a WebGPU device (`WorkerKind::DedicatedWorker`). |

Both scripts `importScripts('craft-worker-host-routing.js')` and call the same hooks on `globalThis`:

- `craftRuntimeHandleGpuBatch` — async `gpu_jobs` handler; must return an outcomes array aligned with `jobs`.
- `craftRuntimeFlushPending` — optional `storage_flush` handler; returns `{ pending_count }` (default from `opfs-flush.js` when loaded).

**Broker-only SharedWorker:** if WebGPU is unavailable inside SharedWorker but tabs still need a single storage mirror and RPC endpoint, keep the SharedWorker for `init`, `storage_put`, and `storage_flush` only. Route `gpu_jobs` to a DedicatedWorker or the main thread (whichever holds the `GpuContext`) until the worker reports `Ready` caps from a real backend. The tab selects worker URL from runtime `WorkerKind`; message tags do not change.

Ship `craft-dedicated-worker.js` (+ brotli) beside `craft-worker-host.js` on the static host.

## Worker message flow

1. `init` → `ready` (caps)
2. `storage_put` → mirror update (no reply)
3. `storage_flush` → warm tier via JS hook `craftRuntimeFlushPending` (default [`opfs-flush.js`](../assets/opfs-flush.js)) → `storage_flushed { pending_count, pending? }`

Tier key layout and OPFS paths: [tier-store.md](./tier-store.md).
4. `gpu_jobs` → `gpu_batch_done` or `error`

Cloud hydration/flush over HTTP is **`craft-cloud-client` from the worker**, not provider code in craft-libs. See [collabcraft-range-api.md](./collabcraft-range-api.md) for range streaming on the gateway.

## Build and test

Rust `fmt` / `test` for this workspace need a Linux host with `cargo` (and Node if you run the protocol smokes locally):

```bash
./scripts/test-lab.sh
```

## Verify (deploy)

Against a running CollabCraft (or compatible) gateway:

- `GET /healthz` → `ok`
- Unauthenticated `GET /api/v1/me` → `unauthenticated`
- Static worker: `curl -sS -D - -o /dev/null -H 'Accept-Encoding: br' https://<your-host>/craft-worker-host.js` → **200** with brotli when precompressed assets are published (headers show status and `Content-Encoding`)

After each image or asset change, recreate the service containers and republish the static worker scripts to the web root.
