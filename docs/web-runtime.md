# craft-runtime (web)

Portable browser runtime for Craft WASM apps: tier storage, SharedWorker RPC, and GPU provider dispatch. **Host-agnostic** — it depends on **`craft-cloud` contracts** and a **`craft-cloud-client`-shaped HTTP gateway**, not on any one product name.

## Layer model (open-ended)

| Layer | Typical crate / repo | Role |
| --- | --- | --- |
| Contracts | **craft-libs → `craft-cloud`** | `Files`, `Entry`, errors — no IO, no vendor |
| HTTP gateway | **Your service** (see lab stack below) | Session, `/api/v1`-style routes, provider held server-side |
| Browser transport | **`craft-cloud-client`** (today lives in collab-craft) | Route builders + `fetch`; any gateway that speaks the same JSON and routes can be used |
| Tab + worker | **craft-libs → `craft-runtime`**, `craft-worker-host`, `craft-tier-store`, `craft-gpu-*` | Mirror/OPFS glue, worker protocol, GPU registry |

Apps never embed provider credentials or WebDAV. They call the gateway; the gateway implements `craft_cloud::Provider`.

## Lab stack (what we run)

This is **our** deployment, not a hard-coded limit of the libraries:

1. **`craft-cloud`** — contracts in craft-libs  
2. **collab-craft** — HTTP service + session + provider wiring ([craft.quasarke.net](https://craft.quasarke.net))  
3. **`craft-cloud-client`** — wasm/desktop client for those routes (collab-craft repo)  
4. **`-web` apps** — EffectCraft, FilmCraft, … on SharedWorker + `craft-runtime`

**Repin order:** craft-libs → collab-craft (when API/contracts change) → application `-web` crates.

Static host on CollabCraft ships app bundles plus `assets/craft-worker-host.js` (+ brotli). Worker `init` gets a cloud base URL: same-origin when the app is served from CollabCraft, or `craft-cloud-url` / tab-provided base when static is elsewhere.

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

Cloud hydration/flush over HTTP is **`craft-cloud-client` from the worker**, not provider code in craft-libs. See [collabcraft-range-api.md](./collabcraft-range-api.md) for range streaming on the gateway we ship.

## Build and test (lab hosts — not the laptop)

Rust `fmt` / `test` for this workspace run on **Linux builders in the lab**:

| Host | Role |
| --- | --- |
| **192.168.1.88** (vin ez) | Primary build box |
| **192.168.1.57** (ai-node-02) | Secondary builder |

On the host, with a checkout of craft-libs (and collab-craft when touching the gateway):

```bash
./scripts/test-lab.sh
```

Optional on any machine with Node only (protocol JSON smoke, no `cargo`):

```bash
node tests/worker-host-protocol.mjs
```

## Verify (deploy)

Deploy **collab-craft** and static assets to **craft.quasarke.net**, then smoke SharedWorker `init`, authenticated file routes, and one app storage/GPU path. That deployment is the integration oracle.
