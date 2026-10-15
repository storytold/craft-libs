# craft-runtime (web)

Browser Craft apps use **craft-tier-store** (hot mirror + warm/cold tiers), **craft-worker-host**
(SharedWorker RPC), and **craft-gpu** providers dispatched through **craft-runtime**.

Tracking: [storytold/craft-libs#132](https://github.com/storytold/craft-libs/issues/132),
lab tasks wdunn001/craft-libs#4–#10.

## Repin order

1. craft-libs (this repo)
2. collab-craft / CollabCraft service
3. Application `-web` crates

## Static assets

Ship `assets/craft-worker-host.js` (+ brotli sibling) from CollabCraft static host.

## Local test (Windows without MSVC)

```powershell
node tests/worker-host-protocol.mjs
.\scripts\test-local.ps1
```

Podman runs `cargo fmt --check` and all runtime crate tests inside `rust:bookworm`.

## Worker message flow

1. `init` → `ready` (caps)
2. `storage_put` → (no reply); read via mirror on worker
3. `storage_flush` → `storage_flushed { pending_count }`
4. `gpu_jobs` → `gpu_batch_done` or `error`
