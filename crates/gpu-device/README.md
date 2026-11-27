# craft-gpu-device

Choosing and opening a wgpu device for a Craft app: an adapter choice the user can set, fixes for
drivers that crash, a software fallback, a capability report, and device health with recovery
after a device loss. The crate depends on `craft-gpu`, `wgpu` 30, `pollster` and `log`, with
`serde` as an optional feature (on by default). It depends on no app crate.

It uses the workspace's `wgpu`, which turns on no native backend, and the WebGPU backend on
`wasm32`. The `native-vulkan`, `native-dx12`, `native-metal` and `native-gles` features turn on
wgpu's backends, as the features of the same names in `craft-gpu-wgpu` do. Cargo merges the
features, so an app can turn them on through either crate.

The `gpu-device` feature of `craft` exports this crate as `craft::gpu_device`.

`native_instance` shares one instance across calls on native targets. It reads wgpu's environment
settings for flags and backend options when that instance is first created. `native_descriptor`
reads those settings on every call.

Licensed under MIT OR Apache-2.0.

## Contents

| Item | What |
|---|---|
| `Candidate` | one listed adapter as plain data: name, backend, device type, PCI vendor, device and bus, driver, feature count, and whether it can draw to the window's surface |
| `list(instance, surface)` | every adapter the instance lists, as `wgpu::Adapter`s and as `Candidate`s |
| `native_instance`, `native_descriptor` | a wgpu instance over Vulkan, Metal and DirectX 12, plus OpenGL except on Windows; wgpu's `WGPU_*` flag variables apply |
| `choose(candidates, power)` | wgpu's own adapter rule with two corrections: the newest version of a driver that lists one GPU twice, and DirectX 12 instead of Vulkan on Intel Gen 9 to Gen 11 integrated GPUs (`intel_gen9_to_gen11`) |
| `Preference` | `Auto` (high performance), `LowPower`, `Adapter(AdapterKey)` or `Software`; serde form `{"kind":"auto"}`, `{"kind":"adapter","name":…,"backend":…,"vendor":…,"device":…,"pciBusId":…,"driver":…}` |
| `AdapterKey` | what identifies an adapter across runs: backend, PCI vendor, device and bus, the Vulkan driver's name, and the name when the backend reports no PCI ids |
| `select(candidates, preference)` | the adapter to draw with, as a `Selection` with its `Notice`s |
| `ranked(candidates, preference)` | the order in which to try opening a device: the selection, then the other usable adapters by type with software after hardware, never an older version of a driver listed twice, or the Vulkan entry of an Intel Gen 9 to Gen 11 GPU on Intel's Windows driver |
| `open(instance, surface, preference, request)` | native only, like `list`: opens a device through `ranked`, then wgpu's fallback adapter; returns `Opened` (adapter, device, queue, `Capabilities`, `DeviceHealth`, notices) or `OpenError` |
| `list_async`, `open_async` | `list` and `open` as futures, for a `wasm32` web worker as well as native; in a browser `list_async` gives the high-performance adapter, the one `open_async` opens for `Auto` and a named adapter |
| `pick(adapters, surface, preference)` | `select` over listed adapters, for egui-wgpu's `native_adapter_selector`; the error is an `OpenError` (convert it with `map_err(\|e\| e.to_string())` for that selector) |
| `Capabilities`, `Vendor` | NV12 and P010 (sampled only in wgpu 30), 16-bit norm, `f16` and timestamp support, the largest texture and buffer, the vendor, and whether the adapter is a software one, read from the `craft_gpu::DeviceCaps` it holds (its fields are readable through `Deref`; `From` converts both ways) |
| `DeviceHealth`, `Fault`, `WaitOutcome` | records a device loss or an uncaptured error instead of panicking; polls and bounded waits that do not touch a lost device |
| `Recovery`, `RecoveryPolicy` | the preference to open after a loss: the user's own, and `Software` once the device was lost 3 times in 10 minutes (both values can be set) |
| `device_caps(device)`, `Opened::device_caps` | the `craft_gpu::DeviceCaps` of a device: its own limits, enabled feature names, vendor id and software flag, in the form `craft-gpu-wgpu` reports |
| `DeviceHealth::device_status` | a recorded loss as `craft_gpu::GpuFault::DeviceLost`, in the form of `GpuContext::device_status`; `Fault` converts to `GpuFault` (a loss to `DeviceLost`, an error to `Operation`) |
| `Notice` | the adapter asked for is missing, no software adapter is listed, only a software adapter can draw, an adapter failed to open, or drawing moved to software after losses; the `Display` text is English, for the log |

## Failure cases

- With no usable adapter, `select` returns `None`, and `open` and `pick` return
  `OpenError::NoAdapter`, or `OpenError::UnsafeDriver` when only a driver that can end the
  process can draw.
- In a browser, WebGPU offers one adapter for a power preference, so a named adapter cannot be
  chosen. `open_async` asks for the adapter with `Preference::power()` as the power preference,
  and gives the automatic choice with `Notice::AdapterMissing` for a named adapter. wgpu 30
  does not pass the fallback-adapter option to the browser, so `Preference::Software` opens the
  browser's adapter, with `Notice::NoSoftwareAdapter` unless that adapter is a software one, and
  `Recovery` cannot move drawing to software there. `Capabilities` and
  `Recovery` otherwise work there as they do on native, and so does `DeviceHealth` apart from `wait`,
  `wait_timeout` and `wait_timeout_cancelable`, which are native only. On the browser backend,
  wgpu 30 panics on an uncaptured `GPUInternalError` before the handler `DeviceHealth::watch`
  installs runs, so such an error is not recorded. Validation and out-of-memory errors are.
- An adapter that lacks `Request::required_features`, whose limits are lower than
  `Request::limits`, or whose `request_device` fails, adds a `Notice::DeviceFailed`, and `open`
  tries the next one. When every adapter fails, the error lists each failure.
- A named adapter that is no longer listed, or can no longer draw to the surface, gives the
  automatic choice and `Notice::AdapterMissing`.
- A driver that ends the process cannot be caught. `choose` and `ranked` never use an older
  version of a driver that lists one GPU twice, or the Vulkan entry of an Intel Gen 9 to Gen 11
  GPU on Intel's Windows driver. When those are the only adapters that can draw and wgpu's
  fallback adapter does not open either, `open` returns `OpenError::UnsafeDriver` with their
  names. When a
  driver reports no bus id, a second GPU of the same model on the same driver looks like an
  older version, and is used through its other backend (DirectX 12 on Windows, OpenGL
  elsewhere).
- Only an error whose cause is wgpu's "Parent device is lost" counts as a loss. The same words in
  a resource's label do not count. An error about a destroyed buffer or texture is recorded as an
  error.
- `wait`, `wait_timeout` and `wait_timeout_cancelable` wait for one submission index. To wait for
  everything submitted so far, pass `queue.submit([])`; work submitted later does not extend the
  wait.
- `wait` has no deadline. It polls in slices of at most 100 ms, and a loss or an error other than
  out of memory that another clone of the `DeviceHealth` records ends it after the current slice.
- `DeviceHealth::watch` replaces the device's uncaptured-error handler. Polls catch the panic that
  `Device::poll` raises on a lost device and record the loss.
- wgpu keeps one device-lost callback per device, and a later `set_device_lost_callback`
  replaces the one `DeviceHealth::watch` set. A `craft-gpu` context on a watched device reads
  `DeviceHealth::device_status` instead of setting its own callback.
- After an out-of-memory error, polls and waits keep running, so staging memory is reclaimed.
  The app reads `DeviceHealth::out_of_memory_count` when it handles the error, frees memory, and
  passes the count to `DeviceHealth::clear_out_of_memory`. The device is then healthy again,
  unless it was lost, another error was recorded, or a new out-of-memory error came in after the
  count was read.

## After a device loss

`DeviceHealth::device_status` returns `GpuFault::DeviceLost` once a loss is recorded, and the
caller starts recovery then. `DeviceHealth::device_lost` turns true only when wgpu itself reported
the loss; it stays false for a loss the health layer recorded on its own, such as a bounded wait
that reached its deadline. Nothing made with the lost device can be used again, and wgpu keeps
none of its contents. The caller then:

1. Drops the old `Opened` and everything made with its device: buffers, textures and their
   views, samplers, bind group layouts and bind groups, shader modules, pipeline layouts and
   pipelines, query sets, unsubmitted command encoders, and any renderer that holds them (an
   egui-wgpu `Renderer`, for example). Work submitted before the loss does not complete, and
   waits on it return `false`.
2. Calls `Recovery::lost` with the time and opens a new device with `open` or `open_async`,
   using the returned preference and the same `Request`. When the step has a notice
   (`Notice::SoftwareAfterLosses`), the app shows it only if the new `Opened::caps.software()` is
   true. Otherwise no software adapter opened (in a browser, or with none listed), and the new
   `Opened::notices` say what was opened instead.
3. Reads the new `Opened::caps` before choosing pipelines again. The new device can be another
   adapter, or a software one, with other features and limits: for example, without NV12 the
   planes are drawn into `R8Unorm` and `Rg8Unorm` textures instead.
4. Configures each window surface again with the new device (`Surface::configure`). The surface
   itself is kept when it was passed to `open`, which then opens only an adapter that can draw
   to it. The new adapter may not support the old configuration (an HDR format, for example), so
   the caller reads `Surface::get_capabilities` with the new adapter and picks a format, present
   mode and alpha mode from it.
5. Builds the pipelines for the surface format picked in step 4, and the bind groups and
   renderers, again, and uploads the textures and buffers again from the app's own copies of
   their data.
6. Makes any `craft-gpu` context again on the new device, with the new `Opened::device_caps`, and
   submits again the jobs and frames that did not complete.
7. Uses the new `Opened::health`. The old `DeviceHealth` keeps reporting the loss.

## Tests

The selection tests run on `Candidate` lists from real machines. The `open` tests use wgpu's noop
backend (the `noop` feature, enabled for tests only), so they do not need a GPU. One test compares
`choose` with wgpu's own `request_adapter` on the machine that runs it, and passes when no adapter
is listed.
