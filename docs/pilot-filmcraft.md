# Pilot: wire filmcraft-web to craft-runtime

Lab tracking: [wdunn001/craft-libs#10](https://github.com/wdunn001/craft-libs/issues/10)  
Upstream epic: [storytold/craft-libs#132](https://github.com/storytold/craft-libs/issues/132)

Repin order (from [web-runtime.md](./web-runtime.md)): **craft-libs → collab-craft → FilmCraft `-web`**.

This checklist describes how `storytold/filmcraft` `apps/filmcraft-web` and `crates/gpu` (`filmcraft-gpu`) map onto craft-libs crates. Implementation belongs in craft-libs / collab-craft / a FilmCraft branch; **no FilmCraft changes are required to land this document**.

Sources reviewed (verified on GitHub `storytold/filmcraft` `main`, 2026-10-14):

| Area | Path |
|------|------|
| OPFS shim | [`apps/filmcraft-web/src/opfs.rs`](https://github.com/storytold/filmcraft/blob/main/apps/filmcraft-web/src/opfs.rs) |
| Autosave / recovery | [`apps/filmcraft-web/src/recovery.rs`](https://github.com/storytold/filmcraft/blob/main/apps/filmcraft-web/src/recovery.rs) |
| Web shell | [`apps/filmcraft-web/src/lib.rs`](https://github.com/storytold/filmcraft/blob/main/apps/filmcraft-web/src/lib.rs) |
| GPU crate | [`crates/gpu/`](https://github.com/storytold/filmcraft/tree/main/crates/gpu) |

---

## Prerequisites (craft-libs)

Do not start the FilmCraft pilot until these lab issues are merge-ready:

| Issue | Crate | FilmCraft needs |
|-------|--------|-----------------|
| [#4](https://github.com/wdunn001/craft-libs/issues/4) | `craft-tier-store` | Hot mirror + warm OPFS flush; key policy for `recovery/*`, `media/*` |
| [#5](https://github.com/wdunn001/craft-libs/issues/5) | `craft-worker-host` | SharedWorker RPC + `craft-worker-host.js` on CollabCraft static host |
| [#6](https://github.com/wdunn001/craft-libs/issues/6) | `craft-gpu` | `GpuJob`, `ProviderRegistry`, `GpuContext` |
| [#7](https://github.com/wdunn001/craft-libs/issues/7) | `craft-gpu-providers` | Shared WGSL providers (`craft.*` ids) |
| [#8](https://github.com/wdunn001/craft-libs/issues/8) | `craft-runtime` | Facade: tier store coordinator, worker messages, GPU dispatch |

Dependency rule ([gpu-providers.md](./gpu-providers.md)): **`craft-gpu-providers` must not depend on `filmcraft-*` or `*-engine`.**

---

## filmcraft-gpu layout (today)

Package: `filmcraft-gpu` ([`crates/gpu/Cargo.toml`](https://github.com/storytold/filmcraft/blob/main/crates/gpu/Cargo.toml)).

| File | Role | Stays in FilmCraft initially |
|------|------|------------------------------|
| `lib.rs` | `GpuCompositor`, layer upload, `FramePlan` execution, `composite.wgsl` pipelines, accum/output readback | Core compositor stays app-side until `craft.composite` exists; **readback helpers** (`read_output`, `read_accumulator`, `f16_to_f32*`) are extraction candidates |
| `composite.wgsl` | YUV/RGB sampling, blend modes, resolve | Move with composite provider |
| `mask.rs` + `mask.wgsl` | `GpuMask`: coverage + masked mix (CPU oracle: `filmcraft_render::mask`) | **Extract to `craft-gpu-providers`** |
| `lut.rs` + `lut.wgsl` | `GpuLut`: 3D LUT tetrahedral (CPU oracle: `filmcraft_color::Lut3d`) | **Extract to `craft-gpu-providers`** |
| `transition.rs` + `transition.wgsl` | `PushStage`: push transition on accum format | **Extract** (decouple `EffectInstance` → versioned job params first) |
| `fx.rs` + `fx.wgsl` | Layer effect compute stage | Remains `filmcraft-gpu` / `filmcraft.*` provider |
| `adapter.rs` | WebGPU adapter selection | App or `craft-gpu-wgpu` glue |
| `export_renderer.rs` | Export timing / GPU export path | App-specific orchestration |

---

## craft-gpu-providers: extraction order (first wave)

Among **composite, mask, lut, transition**, land providers in this order (lowest coupling and clearest oracles first):

### 1. `craft.mask` — `mask.rs` + `mask.wgsl`

- Self-contained compute pipeline; inputs are flat mask buffers + dimensions.
- Params: encode `FlatMask`-equivalent data in `GpuJobParams` (no `filmcraft_render` types in the provider crate).
- FilmCraft keeps a thin adapter that serializes masks from the engine into job blobs.
- Golden tests: reuse existing CPU parity tests in `filmcraft-gpu` as the oracle during extraction.

### 2. `craft.lut` — `lut.rs` + `lut.wgsl`

- Same pattern as mask: LUT table + domain metadata in the job blob.
- Depends only on numeric LUT payload, not on `Lut3d` struct in the provider crate.

### 3. `craft.transition` — `transition.rs` + `transition.wgsl`

- Small render pass, but today reads `filmcraft_project::EffectInstance` (`direction`, `motion_blur`, progress).
- **Before move:** define a stable param struct (direction index, blur 0..1, progress 0..1, width, height) in the provider schema; FilmCraft maps project effects to that blob.
- Output format must match compositor accum (`Rgba16Float` / premultiplied), same as `PushStage` today.

### 4. `craft.composite` — `lib.rs` compositor core + `composite.wgsl`

- Largest surface: `FramePlan`, uploads, blend/backdrop passes, optional `fx` stage integration, transitions hook.
- **Split recommended:**
  - **`craft.readback`** (issue #7): texture/buffer readback + `f16_to_f32` path from `lib.rs` (used by export and tests).
  - **`craft.composite`**: layer draw + resolve; still fed by FilmCraft-prepared textures/handles via `GpuContext` until a later phase inlines upload.
- FilmCraft registers **`filmcraft.compositor.frame`** (or similar) for plan-specific work until the generic composite provider subsumes it.

Register new providers in `craft_gpu_providers::register_all` only when behavior-preserving tests pass; worker code continues to call `ProviderRegistry::dispatch` only ([gpu-providers.md](./gpu-providers.md)).

---

## OPFS and tier-store mapping

Today [`opfs.rs`](https://github.com/storytold/filmcraft/blob/main/apps/filmcraft-web/src/opfs.rs) is a **wasm-only OPFS client**:

| `opfs` API | Behavior | craft-libs target |
|------------|----------|-------------------|
| `available()` | Probes `navigator.storage.getDirectory` | Runtime init capability flag |
| `write` / `write_then` | Per-path queue, coalesce newest bytes, single writer task | `MirrorStore::put` + `take_pending` → JS warm-tier flush ([`tier-store/src/mirror.rs`](../crates/tier-store/src/mirror.rs)) |
| `read` | Async read whole file | Warm-tier read into mirror (runtime glue) |
| `remove` | Delete file | Warm-tier delete |
| `store_blob` | Stream `Blob` to OPFS without wasm copy | Media tier: `media/<name>` keys |
| `files_in` | List directory | Recovery: enumerate `media/` at startup |

[`recovery.rs`](https://github.com/storytold/filmcraft/blob/main/apps/filmcraft-web/src/recovery.rs) **stays FilmCraft-specific** but should call the runtime store instead of `crate::opfs` directly:

| OPFS path (today) | `StoreKey` suggestion | `KeyClass` ([policy](../crates/tier-store/src/policy.rs)) |
|-------------------|----------------------|-----------------------------------------------------------|
| `recovery/snapshot.fcproj` | `config/recovery/snapshot.fcproj` | Config |
| `recovery/meta.json` | `config/recovery/meta.json` | Config |
| `media/<name>` | `media/<name>` | Media |

Autosave policy (`recovery_policy`, revision/dirty/clean) remains in `filmcraft-web`; only persistence I/O moves behind `craft-runtime` + tier-store coordinator.

---

## filmcraft-web → craft-runtime checklist

### A. Dependencies and features

- [ ] Add path/git dependency on published `craft-runtime` with features: `web`, `shared-worker`, optional `cloud-delegate`.
- [ ] Add `craft-gpu-wgpu` (or let runtime re-export `WgpuContext`) for the same WebGPU path eframe uses today.
- [ ] Ship CollabCraft static asset `assets/craft-worker-host.js` (+ brotli) per [web-runtime.md](./web-runtime.md).

### B. Storage / autosave (#4, #8)

- [ ] Replace direct `opfs::write*` calls in `recovery.rs` with `MirrorStore` puts keyed as above.
- [ ] Wire `HostMessage::StorageFlush` (or runtime equivalent) to drain pending writes to OPFS on idle / visibility hidden.
- [ ] Keep `keep_media` / `restore_media` semantics: blob store under `media/`, register into virtual `fs` table.
- [ ] Preserve query flags (behavior documented in `lib.rs` startup):

  | Flag | Effect |
  | --- | --- |
  | `?fresh` | Skips recovery; starts from a blank project. |
  | `?norecover` | Suppresses the recovery dialog even when a dirty autosave exists. |
  | `?craftRuntime` | Enables the craft-runtime code path (feature flag on). Off by default until storage + worker smoke tests pass. |
  | `?webgl` | Forces WebGL2 backend; bypasses WebGPU adapter selection. Use in CI or on hardware without WebGPU. `gpu_jobs` requiring compute shaders are nooped or return an error outcome. |
  | `?nogpuworkers` | Disables GPU dispatch to the SharedWorker / DedicatedWorker. Jobs run on the main thread (or are nooped). Useful for exercising the storage layer in isolation. |

  Flags may be combined: `?craftRuntime&nogpuworkers` exercises storage + flush with no GPU overhead.
- [ ] Expose `opfs` availability via `set_info("opfs", …)` using runtime probe.

### C. Worker / GPU (#5, #6, #7, #8)

- [ ] Instantiate `Runtime::builder().wgpu_context().with_default_providers().build()` (or SharedWorker-owned runtime).
- [ ] Register FilmCraft providers on init: compositor frame jobs, `fx`, etc. (`filmcraft.*` ids).
- [ ] Route preview/export GPU batches that today call `GpuCompositor` directly through `GpuJob` batches where worker isolation wins (cooperative frame pump may stay on main thread initially).
- [ ] After #7 lands: switch mask/LUT/transition call sites to `craft.mask`, `craft.lut`, `craft.transition` jobs when worker path is enabled.

### D. UI shell (unchanged scope)

- [ ] Keep eframe `WebApp`, `Session`, WebCodecs, audio, import, `window.filmcraft` API on main thread.
- [ ] Frame cooperative pump (`FrameServer::pump`) unchanged until worker offload is proven for compositor jobs.

### E. Verification gates

- [ ] `cargo fmt`, `clippy -D warnings`, native + `wasm32` check for craft-libs consumers.
- [ ] Manual: dirty autosave → reload → recovery dialog; media restored from OPFS.
- [ ] GPU: mask/LUT CPU vs GPU parity (existing `filmcraft-gpu` tests) against new provider ids.
- [ ] SharedWorker: tab refresh leaves worker alive; second tab shares wasm module per #5.
- [ ] DedicatedWorker smoke: block `craft-worker-host.js` (DevTools → Network → Block URL pattern) or point the worker URL at `craft-dedicated-worker.js`; confirm `WorkerKind::DedicatedWorker` in `Ready` caps and the same `GpuBatchDone` outcomes as the SharedWorker path. See [web-runtime.md § SharedWorker vs dedicated worker](./web-runtime.md) for the broker-only pattern when WebGPU is unavailable inside a SharedWorker.

---

## What remains in FilmCraft after the pilot

- `filmcraft-engine`, `filmcraft-render`, `FramePlan` planning, WebCodecs decode, egui UI.
- `filmcraft-gpu`: `GpuCompositor` orchestration until `craft.composite` covers the plan graph; `fx` stage; export orchestration.
- App-specific provider ids (`filmcraft.compositor.frame`, …) registered at wasm init.
- Recovery **policy** and project encode (`filmcraft_format::encode`).

---

## Related docs

- [web-runtime.md](./web-runtime.md) — repin order and static assets  
- [gpu-backends.md](./gpu-backends.md) — `craft-gpu-wgpu` / null  
- [gpu-providers.md](./gpu-providers.md) — provider id namespaces and dependency lint  

---

## Known console noise

These messages appear on a clean load and are **not actionable** unless the text or frequency changes:

| Message prefix | Source | Why it appears | Action |
| --- | --- | --- | --- |
| `Permissions-Policy: unrecognized-feature "…"` | Browser | The HTTP response from CollabCraft (or your static host) includes Permissions-Policy directives that the current browser version does not recognize. | None. File a task only if a new directive was intentionally added and the browser should support it. |
| `powerPreference: "high-performance" is not honored` (or similar) | Browser / WebGPU | Some browsers log this when the WebGPU adapter request specifies `powerPreference` but the platform has only one discrete GPU or the hint is otherwise ignored. GPU init proceeds normally. | None. |
| `[egui] Setting theme to Dark` / `[egui] Setting theme to Light` | egui | egui reads the OS color scheme on first paint and logs the resolved theme. Expected on every cold start; eframe's `WebApp` receives a `SetTheme` event before the first frame. | None. |
