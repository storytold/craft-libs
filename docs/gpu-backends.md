# GPU backends

| Crate | Role |
|-------|------|
| `craft-gpu` | Traits, `GpuJob`, `ProviderRegistry` |
| `craft-gpu-wgpu` | wgpu → WebGPU (wasm) / Vulkan·DX12·Metal (native) |
| `craft-gpu-null` | In-memory context for CI |

Feature flags on `craft-gpu-wgpu`:

- `adapter-probe`: native only; opens the device through [`craft-gpu-device`](../crates/gpu-device/README.md) and builds real [`DeviceCaps`](../crates/gpu/src/caps.rs) from the created device (falls back to the staging path when no adapter opens). `craft-gpu-device` depends on wgpu with its default features, so this feature also turns on wgpu's default native backends. The context's instance uses `craft_gpu_device::native_descriptor`'s backends (no OpenGL on Windows) unless `WGPU_BACKEND` names others.
- `native-vulkan`, `native-dx12`, `native-metal`, `native-gles`: enable the matching wgpu backend together with `adapter-probe`. On native targets wgpu is a dependency only when `adapter-probe` is on, so these features do nothing without it.

## Real device, device-truth caps (RFC-0022 requirement 1)

With `adapter-probe`, `WgpuContext::new` opens a real device with `craft_gpu_device::open` for `Preference::Auto`: the same adapter choice, driver corrections and fallback order as every other caller of that crate, so the suite has one device selector. `probe_caps()` reports the adapter `craft_gpu_device::pick` chooses, without opening a device on it; when a device fails to open there, `WgpuContext::new` opens the next adapter in `ranked` order or a software one, and its `caps()` name that device. `WgpuContext::from_opened` takes a device the app opened with `craft-gpu-device` (with the user's own `Preference`, or after a `Recovery` step), and `WgpuContext::with_device` takes any device with its `DeviceHealth`. `DeviceCaps` is built from the created `Device`'s own `limits()` / `features()` (`craft_gpu_device::device_caps`), never from the adapter's advertised maximum: an adapter can advertise more than a device grants. `WgpuContext::new` asks for the adapter's features intersected with what this context asks for (currently nothing, since no provider dispatches a real WGSL job yet, see [gpu-providers.md](./gpu-providers.md)), and for the adapter's own limits. Both choices mean `request_device` cannot panic on an unsupported feature or limit here. A context on a device the caller opened takes the features and limits that device was opened with.

Buffers are real `wgpu::Buffer`s (`STORAGE | COPY_SRC | COPY_DST`). The host-staged `Vec<Vec<u8>>` form survives only on the no-device path.

**The no-adapter path is still a defined path.** Without a usable adapter (`adapter-probe` off, no backend feature on, no GPU present, or `wasm32`, see below), `WgpuContext` degrades to the same in-memory staging behavior as `craft-gpu-null`. `WgpuContext::has_device()` tells a caller which path it got; `probe_caps()` (no context) stays the quick, adapter-only, advisory estimate it always was.

**`wasm32` gap, acknowledged, not hidden.** RFC-0022 itself describes browser capability probing moving "from static defaults to real adapter queries once async initialization lands in the worker". That landing hasn't happened yet. `request_adapter`/`request_device` are JS promises. Driving them synchronously the way `pollster::block_on` does natively is unavailable on `wasm32`. Wiring the async worker-init path is a larger, separate piece of work. Until then, `WgpuContext` on `wasm32` reports the same conservative, adapter-free defaults it always has, and the crate does not depend on wgpu on `wasm32`. This is the RFC's own acknowledged follow-up, not new scope invented for this PR.

## Device loss (RFC-0022 requirement 1)

`WgpuContext` reads a device loss from the device's `craft_gpu_device::DeviceHealth`, which sets the device's one device-lost callback and records a loss without panicking; the context sets no callback of its own. Loss (a driver reset, OS-level eviction, an explicit `destroy()`, or a loss another user of the same `DeviceHealth` records) surfaces as `GpuFault::DeviceLost`, from the next buffer call, from `poll_reads()` and from `device_status()`, never as a panic. `WgpuContext::health()` gives that `DeviceHealth`, for `craft_gpu_device::Recovery` and the rest of the app. Any other fault the `DeviceHealth` records (an uncaptured error, or an out-of-memory error the app has not cleared with `DeviceHealth::clear_out_of_memory`) makes `upload_buffer` and `request_read` return `GpuFault::Operation`. `poll_reads()` fails pending reads on recorded faults other than out of memory; an out-of-memory fault leaves those reads eligible to complete. `device_status()` reports a loss only, and `free_buffer` keeps running. An allocation error the context's own error scopes catch is recorded in the `DeviceHealth` as well (`DeviceHealth::caught`).

Once a context has reported `GpuFault::DeviceLost`, every `DeviceCaps` it reported before that point is stale. The stated recovery, per the trait docs on [`GpuContext`](../crates/gpu/src/context.rs):

1. fall back to the portable path for the rest of the session, or
2. construct a fresh `GpuContext` and re-probe `caps()` before resuming GPU jobs.

No backend here supports continuing to dispatch jobs against a context that already reported loss, using caps read before it.

## Async readback (RFC-0022 requirement 3)

`GpuContext::read_buffer` used to be synchronous. WebGPU can only map a buffer asynchronously (`map_async`), so the trait's real primitives are now `request_read` (queue a readback, get a `ReadRequest`) and `poll_reads` (drain completions). This mirrors `craft-worker-host`'s own async shape: a `HostMessage::GpuJobs { batch_id, .. }` is answered later by `WorkerMessage::GpuBatchDone { batch_id, outcomes }`, not inline; `request_read`/`poll_reads` is the same split one level down, for one buffer instead of one job batch.

`read_buffer` stays as a blocking convenience with a default body built on those two primitives, for callers that need bytes now (the CPU-stand-in providers in `craft-gpu-providers`). Both bundled backends override that default body. The default has nowhere to put completions belonging to other outstanding requests. The overrides keep those queued for the next `poll_reads`:

- `craft-gpu-null`: overrides to take only its own completion; every request resolves immediately, with nothing to wait for.
- `craft-gpu-wgpu`, native, with a real device: overrides to block by calling `DeviceHealth::wait` (a `Device::poll` that waits for all submitted work) between polls. It waits on the GPU without busy-spinning the CPU, re-queuing other requests' completions. When the device reports an error other than a loss or running out of memory while the read is pending, or the wait itself fails, `read_buffer` drops the read and returns that error as `GpuFault::Operation`; a loss fails it with `GpuFault::DeviceLost`.
- `craft-gpu-wgpu`, native, staging path (no device): resolves immediately, same as the null context.
- `craft-gpu-wgpu`, `wasm32`: **serves the staging read.** The only backing on this target today is the in-memory staging store, since real device creation needs the async worker-init path that has not landed. Staging reads resolve synchronously. `read_buffer` returns their bytes. Only a read that did not resolve synchronously returns `GpuFault::Operation`. That refusal will matter once a real device backing exists there: `wasm32` is single-threaded and blocking the one thread the browser needs to run `map_async`'s own completion callback on would hang the tab. A caller on a future device backing must drive `request_read` + `poll_reads` from the worker's own message loop.

**Consumed on `craft-libs#662`.** That branch's `RuntimeContext` wrapper now forwards `request_read`, `poll_reads`, `free_buffer` and `device_status` to its inner context. It forwards `read_buffer` too, letting the inner backend's completion-preserving override apply. No logic in `craft-worker-host` or the runtime facade changed shape beyond that: `HostMessage::GpuJobs` still dispatches a whole batch synchronously today, answered per job.

## Named consumers (RFC-0022's "what this RFC decides")

- **First: the runtime slice**, `craft-libs#662` (`craft-worker-host`, the worker-host executing `GpuJob` batches; `craft-runtime`, the facade `RuntimeBuilder`/`Runtime` build on top of `GpuContext`).
- **Second: the render-path apps**, EffectCraft and FilmCraft. Neither has adopted these crates yet (see `docs/pilot-effectcraft.md`, `docs/pilot-filmcraft.md`); both already carry their own hand-rolled wgpu code today:
  - **FilmCraft** (`storytold/filmcraft`, `crates/gpu` = `filmcraft-gpu`): a monitor/preview `GpuCompositor` over `composite.wgsl`, `mask.rs`/`mask.wgsl`, `lut.rs`/`lut.wgsl`, `transition.rs`/`transition.wgsl`, plus its own adapter-selection policy (`adapter.rs`, see device ownership below) and a separate off-screen `ExportRenderer`. The pilot doc orders `craft.mask`, `craft.lut`, `craft.transition`, then `craft.composite`/`craft.readback` as the extraction sequence into `craft-gpu-providers`.
  - **EffectCraft** (`storytold/effectcraft`, `apps/effectcraft-web`): its own `store.rs`/`persist.rs`/`diskcache.rs` and dedicated-worker GPU dispatch (`worker.rs`, `frames.rs`); app-specific jobs route through `effectcraft.*` provider ids once it adopts `craft-runtime`, with the shared `craft.*` ids from this slice used alongside them.

## Device ownership, when egui's wgpu renderer is also running

The question that decides whether a render-path app can adopt `WgpuContext` as-is: who owns the `Device`/`Queue` when egui (via `eframe`/`egui-wgpu`) is also drawing?

**FilmCraft's answer today is "it depends on which device."** Its on-screen monitor compositor adopts egui's own device: both the native desktop app and the web app call `app.set_wgpu(cc.wgpu_render_state.clone())` (or the equivalent on native), so the compositor draws through the exact `Device`/`Queue` `eframe`'s `RenderState` already created. One device, shared, no second adapter opened for the window. Its **export** path is deliberately the opposite: a separate, off-screen `wgpu::Device` with no surface, created once and kept for the process (`export_renderer.rs`), specifically because export runs independent of the UI's lifecycle (worker threads, no window) and must keep working even if the UI's device were ever lost or recreated.

`WgpuContext::with_device` accepts an existing `(Device, Queue)` pair with its `DeviceHealth`, and `WgpuContext::from_opened` accepts a `craft_gpu_device::Opened`. Neither changes `GpuContext`'s trait shape. They take whatever the device was granted: `DeviceCaps` is built from that device, and a job that needs a feature the caps do not name takes its portable path or fails. A render-path app that wants GPU compute on the same frame it draws with egui passes egui's device to `with_device`, with the `DeviceHealth::watch` of that device. A worker-isolated batch job (the first named consumer, `craft-libs#662`'s worker-host) has no window and keeps its own device from `WgpuContext::new`.
