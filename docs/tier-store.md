# craft-tier-store (browser tiers)

Portable tier model for Craft `-web` apps: **hot** mirror in WASM/worker memory, **warm** OPFS (and parallel app-specific cache trees), **cold** File System Access volumes, **cloud** byte ranges via `craft-cloud` refs. This crate has **no IO** — hosts implement persistence in JS (OPFS, FS Access) or HTTP (`craft-cloud-client` in collab-craft, not in craft-libs).

**Out of scope here:** `craft-nextcloud`, collab HTTP, and desktop sync. Cloud identity only: [`CloudBlobRef`](../crates/tier-store/src/cloud_ref.rs) + [`craft-cloud`](../crates/cloud/).

## Tier namespaces

| Tier | Where bytes live | Key namespace | Eviction / policy |
| --- | --- | --- | --- |
| **Hot** | `MirrorStore` / `VersionedMirror` in worker WASM | Same string as warm: `config/*`, `files/*`, `cache/*`, `render/*` | LRU byte budget on `cache/` only ([`DefaultTierPolicy`](../crates/tier-store/src/policy.rs)) |
| **Warm** | OPFS under app warm root (EffectCraft: `effectcraft-store/v1/<StoreKey>`) | `StoreKey` path segments | App flush loop; coalesced pending ([`mirror.rs`](../crates/tier-store/src/mirror.rs)) |
| **Warm (parallel)** | Separate OPFS tree for render disk cache (EffectCraft: `effectcraft-cache/v1/…`) | **Not** `StoreKey` today — u128 / layer hex names | App-owned; do not rename without migration ([pilot-effectcraft](./pilot-effectcraft.md)) |
| **Cold** | User-picked directories (FS Access) | Logical keys still `files/…`; physical path under [`ExternalVolume`](../crates/tier-store/src/external_volume.rs) `handle_id` | Config stays warm-only; media/export paths may map to cold ([`TierCoordinator`](../crates/tier-store/src/coordinator.rs)) |
| **Cloud** | Remote blob via gateway | Hydration targets `StoreKey::slice_cache(hash, offset, len)` | Range reads; see [collabcraft-range-api.md](./collabcraft-range-api.md) |

## EffectCraft `StoreKey` mapping

Engine-visible keys (must not change for the pilot):

| Pattern | Example | `KeyClass` |
| --- | --- | --- |
| `config/<name>` | `config/session.ecproj` | Config |
| `files/<path>` | `files/clip.mp4` (POSIX path without leading slash in key) | Media |
| `cache/slice/<hex>/<offset>+<length>` | [`StoreKey::slice_cache`](../crates/tier-store/src/key.rs) | Cache |
| `render/…` | render temps | RenderTemp |

Warm OPFS relative path (default coordinator): `effectcraft-store/v1/{key}` — e.g. `effectcraft-store/v1/config/prefs.json`.

Disk cache remains **`effectcraft-cache/v1`** with engine entry names; not folded into `StoreKey` until a dedicated migration.

## Slice cache keys

[`StoreKey::slice_cache`](../crates/tier-store/src/key.rs) builds stable warm keys for cloud range hydration:

```text
cache/slice/{content_hash_hex}/{offset}+{length}
```

Always classified as **Cache** (prefix `cache/`). OPFS layout uses the same path under the warm root.

## Cold tier (FS Access)

[`ExternalVolume`](../crates/tier-store/src/external_volume.rs) stores **metadata only**:

- `handle_id` — stable id for a permission grant persisted in IndexedDB (host responsibility).
- `label` — optional display name from the directory picker.

Policy (documented default coordinator):

- **`config/*`** — warm OPFS only; never mapped to cold.
- **`files/*` and other media** — may flush to `volumes/{handle_id}/…` when the app binds a volume to a project folder.

No picker or IDB code in craft-libs.

## Flush path (worker → warm OPFS)

1. Tab sends `storage_put` → `MirrorStore::put`.
2. Tab or scheduler sends `storage_flush`.
3. Runtime drains `take_pending()` (latest bytes per key, first-touch order).
4. If a [`FlushSink`](../crates/runtime/src/flush.rs) is configured, it consumes pending (tests, custom hosts).
5. Otherwise pending entries are attached to `storage_flushed` for JS ([`opfs-flush.js`](../assets/opfs-flush.js) via `craftRuntimeFlushPending`).

SharedWorker bootstrap: [`craft-worker-host.js`](../assets/craft-worker-host.js).

## Cloud refs

[`CloudBlobRef`](../crates/tier-store/src/cloud_ref.rs) aligns with [`craft-cloud`](../crates/cloud/) file identity. Streaming writes target slice-cache keys; HTTP lives in the worker through `craft-cloud-client` (collab-craft repo).

## Build and test

Rust workspace tests for the runtime crates need a Linux host with `cargo`:

```bash
./scripts/test-lab.sh
```

## Related docs

- [web-runtime.md](./web-runtime.md) — SharedWorker message flow
- [pilot-effectcraft.md](./pilot-effectcraft.md) — key preservation checklist
- [collabcraft-range-api.md](./collabcraft-range-api.md) — cloud range hydration
