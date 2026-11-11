# GPU backends

| Crate | Role |
|-------|------|
| `craft-gpu` | Traits, `GpuJob`, `ProviderRegistry` |
| `craft-gpu-wgpu` | wgpu → WebGPU (wasm) / Vulkan·DX12·Metal (native) |
| `craft-gpu-null` | In-memory context for CI |

Feature flags on `craft-gpu-wgpu`:

- `adapter-probe` — native only; uses pollster to `request_adapter`/`request_device` and build real [`DeviceCaps`](../crates/gpu/src/caps.rs) from the created device (falls back to the staging path when no GPU, e.g. CI with no backend feature enabled).
- `native-vulkan`, `native-dx12`, `native-metal`, `native-gles` — enable the matching wgpu backend. `adapter-probe` alone compiles the probing code but finds no adapter unless at least one of these is also on.

## Real device, device-truth caps (RFC-0022 requirement 1)

With `adapter-probe` and a `native-*` backend, `WgpuContext::new` calls `Adapter::request_device`, not just `request_adapter`. `DeviceCaps` is built from the created `Device`'s own `limits()` / `features()`, never from the adapter's advertised maximum: an adapter can advertise more than a device grants. `required_features` is the adapter's features intersected with what this context asks for (currently nothing — no provider dispatches a real WGSL job yet, see [gpu-providers.md](./gpu-providers.md)); `required_limits` is the adapter's own limits. Both choices mean `request_device` cannot panic on an unsupported feature or limit here.

Buffers are real `wgpu::Buffer`s (`STORAGE | COPY_SRC | COPY_DST`), not a `Vec<Vec<u8>>` staged on the host.

**The no-adapter path is still defined, not an error.** Without a usable adapter — `adapter-probe` off, no backend feature on, no GPU present, or on `wasm32` (see below) — `WgpuContext` degrades to the same in-memory staging behavior as `craft-gpu-null`. `WgpuContext::has_device()` tells a caller which path it got; `probe_caps()` (no context) stays the quick, adapter-only, advisory estimate it always was.

**`wasm32` gap, acknowledged, not hidden.** RFC-0022 itself describes browser capability probing moving "from static defaults to real adapter queries once async initialization lands in the worker" — that landing hasn't happened yet. `request_adapter`/`request_device` are JS promises; driving them synchronously the way `pollster::block_on` does natively isn't available on `wasm32`, and wiring the async worker-init path is a larger, separate piece of work. Until then, `WgpuContext` on `wasm32` reports the same conservative, adapter-free defaults it always has. This is the RFC's own acknowledged follow-up, not new scope invented for this PR.

## Device loss (RFC-0022 requirement 1)

`WgpuContext` registers `Device::set_device_lost_callback` when it creates a real device. Loss (a driver reset, OS-level eviction, or an explicit `destroy()`) surfaces as `GpuFault::DeviceLost`, from the next buffer call and from `poll_reads()`, never as a panic.

Once a context has reported `GpuFault::DeviceLost`, every `DeviceCaps` it reported before that point is stale. The stated recovery, per the trait docs on [`GpuContext`](../crates/gpu/src/context.rs):

1. fall back to the portable path for the rest of the session, or
2. construct a fresh `GpuContext` and re-probe `caps()` before resuming GPU jobs.

No backend here supports continuing to dispatch jobs against a context that already reported loss, using caps read before it.

## Async readback (RFC-0022 requirement 3)

`GpuContext::read_buffer` used to be synchronous. WebGPU can only map a buffer asynchronously (`map_async`), so the trait's real primitives are now `request_read` (queue a readback, get a `ReadRequest`) and `poll_reads` (drain completions). This mirrors `craft-worker-host`'s own async shape: a `HostMessage::GpuJobs { batch_id, .. }` is answered later by `WorkerMessage::GpuBatchDone { batch_id, outcomes }`, not inline; `request_read`/`poll_reads` is the same split one level down, for one buffer instead of one job batch.

`read_buffer` stays as a blocking convenience with a default body built on those two primitives, for callers that need bytes now (the CPU-stand-in providers in `craft-gpu-providers`):

- `craft-gpu-null`: resolves every request immediately — nothing to wait for.
- `craft-gpu-wgpu`, native, with a real device: blocks by calling `Device::poll(PollType::wait_indefinitely())` between polls, so it waits on the GPU instead of busy-spinning the CPU.
- `craft-gpu-wgpu`, native, staging path (no device): resolves immediately, same as the null context.
- `craft-gpu-wgpu`, `wasm32`: **returns `GpuFault::Operation`, not bytes.** `wasm32` is single-threaded; blocking the one thread the browser needs to run `map_async`'s own completion callback on would hang the tab, not return. A caller there must drive `request_read` + `poll_reads` from the worker's own message loop instead of calling `read_buffer`.

**Known follow-up on `craft-libs#662`.** That branch's `RuntimeContext` wrapper (`crates/runtime/src/runtime.rs`) implements `GpuContext` by matching on its `Null`/`Wgpu` variants, including an explicit `read_buffer` arm. When #662 rebases past this PR, that block needs two added match arms (`request_read`, `poll_reads`) delegating to each inner context the same way the existing methods do, and its own `read_buffer` arm can simply be deleted in favor of the trait's default (or kept, delegating the same way). No logic in `craft-worker-host` or the runtime facade changes: `HostMessage::GpuJobs` still dispatches a whole batch synchronously today, same as before this PR.

## Named consumers (RFC-0022's "what this RFC decides")

- **First: the runtime slice**, `craft-libs#662` (`craft-worker-host`, the worker-host executing `GpuJob` batches; `craft-runtime`, the facade `RuntimeBuilder`/`Runtime` build on top of `GpuContext`).
- **Second: the render-path apps**, EffectCraft and FilmCraft. Neither has adopted these crates yet (see `docs/pilot-effectcraft.md`, `docs/pilot-filmcraft.md`); both already carry their own hand-rolled wgpu code today:
  - **FilmCraft** (`storytold/filmcraft`, `crates/gpu` = `filmcraft-gpu`): a monitor/preview `GpuCompositor` over `composite.wgsl`, `mask.rs`/`mask.wgsl`, `lut.rs`/`lut.wgsl`, `transition.rs`/`transition.wgsl`, plus its own adapter-selection policy (`adapter.rs`, see device ownership below) and a separate off-screen `ExportRenderer`. The pilot doc orders `craft.mask`, `craft.lut`, `craft.transition`, then `craft.composite`/`craft.readback` as the extraction sequence into `craft-gpu-providers`.
  - **EffectCraft** (`storytold/effectcraft`, `apps/effectcraft-web`): its own `store.rs`/`persist.rs`/`diskcache.rs` and dedicated-worker GPU dispatch (`worker.rs`, `frames.rs`); app-specific jobs route through `effectcraft.*` provider ids once it adopts `craft-runtime`, with the shared `craft.*` ids from this slice used alongside them.

## Device ownership, when egui's wgpu renderer is also running

The question that decides whether a render-path app can adopt `WgpuContext` as-is: who owns the `Device`/`Queue` when egui (via `eframe`/`egui-wgpu`) is also drawing?

**FilmCraft's answer today is "it depends on which device."** Its on-screen monitor compositor adopts egui's own device: both the native desktop app and the web app call `app.set_wgpu(cc.wgpu_render_state.clone())` (or the equivalent on native), so the compositor draws through the exact `Device`/`Queue` `eframe`'s `RenderState` already created — one device, shared, no second adapter opened for the window. Its **export** path is deliberately the opposite: a separate, off-screen `wgpu::Device` with no surface, created once and kept for the process (`export_renderer.rs`), specifically because export runs independent of the UI's lifecycle (worker threads, no window) and must keep working even if the UI's device were ever lost or recreated.

`WgpuContext::new` in this PR always opens its own device (or degrades to staging) and has no constructor that accepts an externally-supplied `(Device, Queue)`. That is **documented here as the named follow-up, not implemented in this PR**: accepting an existing device/queue pair (with `DeviceCaps` built from that pair, the same device-truth way `create_device` builds it today) is a small, additive constructor — it does not change `GpuContext`'s trait shape — but it needs a decision this PR's scope doesn't force: whether the shared constructor should require the caller to prove the features/limits it needs were already granted (since a second caller can't call `request_device` again on a device that already exists), or whether it should accept whatever was granted and fail a job if it needs more. FilmCraft's own split (share for render, separate for export) is a reasonable default for this layer too: a render-path app wanting GPU compute on the same frame it draws with egui should share; a worker-isolated batch job (the first named consumer, `craft-libs#662`'s worker-host) has no window and no reason to share, and keeps its own device exactly as implemented here.
