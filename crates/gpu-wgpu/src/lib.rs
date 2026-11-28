//! wgpu-backed [`GpuContext`].
//!
//! WebGPU on `wasm32`; native APIs when backend features are enabled. With the `adapter-probe`
//! feature (native only), [`WgpuContext::new`] opens a real device through `craft-gpu-device`
//! (its adapter choice for `Preference::Auto`, its driver corrections and its fallback order) and
//! builds [`DeviceCaps`] from what that device actually enabled (`Device::limits()` /
//! `Device::features()`), never from the adapter's advertised maximum.
//! With the native `adapter-probe` feature, `WgpuContext::from_opened` and `WgpuContext::with_device`
//! take a device the caller opened.
//! A loss is read from the device's `craft_gpu_device::DeviceHealth`, which owns the device's
//! device-lost callback. Without a usable adapter (`adapter-probe` is off, no GPU is present,
//! or on `wasm32` where real device creation needs the async worker-init path noted in
//! `docs/gpu-backends.md`), the context degrades to the same in-memory staging behavior as
//! `craft_gpu_null::NullContext` (not a dependency of this crate; named here only for context);
//! that is RFC-0022's defined no-adapter path, not an error.
//!
//! Readback is asynchronous end to end (`request_read` + `poll_reads`), because WebGPU can only
//! map a buffer that way. Native, with a real device, drives it with `Buffer::map_async` plus
//! `Device::poll`. The blocking `read_buffer` convenience (see
//! [`GpuContext`]'s trait docs) blocks efficiently on native by polling
//! the device while it waits. On `wasm32` the backing is always the in-memory staging store
//! today, whose reads complete synchronously, and `read_buffer` serves them; only a read that
//! did not complete synchronously returns a fault there, since blocking would freeze the one
//! thread a real map's completion callback would need to run on.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use craft_gpu::{DeviceCaps, GpuContext, GpuFault, HandleId, ReadOutcome, ReadRequest, WorkerKind};

#[cfg(test)]
mod tests;

fn probe_fallback(backend_name: &str, device_name: &str) -> DeviceCaps {
    DeviceCaps { backend_name: backend_name.into(), device_name: device_name.into(), ..DeviceCaps::default() }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
fn probe_caps_native() -> DeviceCaps {
    native::probe_caps_on(&native::instance())
}

/// Adapter-advertised limits, when an adapter is available; otherwise conservative defaults. This
/// is the quick, device-free probe for UI surfaces that want a number before any context exists
/// (Settings). It is advisory only: these are the adapter's maximums, not a device's granted
/// ones. With `adapter-probe` it reports the adapter `craft_gpu_device::pick` chooses for
/// `Preference::Auto` without opening a device on it; when a device then fails to open there,
/// [`WgpuContext::new`] opens the next adapter in `craft_gpu_device::ranked` order, or a software
/// one, which this probe does not report. [`WgpuContext::caps`] is the authoritative, device-truth
/// source once a context exists.
pub fn probe_caps() -> DeviceCaps {
    #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
    {
        probe_caps_native()
    }

    #[cfg(all(not(target_arch = "wasm32"), not(feature = "adapter-probe")))]
    {
        probe_fallback("wgpu", "probe-not-run")
    }

    #[cfg(target_arch = "wasm32")]
    {
        probe_fallback("webgpu", "browser")
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
mod native {
    use craft_gpu::DeviceCaps;
    use craft_gpu_device::{Capabilities, DeviceHealth, Opened, Preference, Request};

    use crate::{Backing, DeviceBacking, probe_fallback};

    /// Features this context asks a device for. No provider dispatches a real WGSL job on wgpu
    /// yet (`craft-gpu-providers` ships CPU stand-ins, by design, until that lands); extend this
    /// as real compute providers land, and it degrades gracefully regardless: see
    /// [`create_device`].
    const DESIRED_FEATURES: wgpu::Features = wgpu::Features::empty();

    /// An instance over `craft_gpu_device::native_descriptor`'s backends (Vulkan, Metal and
    /// DirectX 12, plus OpenGL except on Windows), honoring `WGPU_BACKEND` when set, so the
    /// same build can be pointed at a software Vulkan adapter (lavapipe/llvmpipe) in a local container run or a CI lane.
    /// The descriptor has no display handle because this context never has a window (no surface
    /// is ever requested); FilmCraft's own off-screen export device uses the same constructor for
    /// the same reason (`filmcraft-gpu`'s `export_renderer.rs`, see `docs/gpu-backends.md`).
    ///
    /// The process holds one instance, made on the first call and returned by every later one,
    /// so `WGPU_BACKEND` is read once. Every context and every probe requests its adapter from
    /// it. An instance per context loads and unloads every installed Vulkan driver each time, and
    /// on Linux with Ubuntu's `mesa-vulkan-drivers` installed, contexts opened that way from
    /// several threads at once crash with SIGSEGV while a lavapipe device is in use.
    pub(crate) fn instance() -> wgpu::Instance {
        static SHARED: std::sync::OnceLock<wgpu::Instance> = std::sync::OnceLock::new();
        SHARED
            .get_or_init(|| {
                let mut desc = craft_gpu_device::native_descriptor();
                if let Some(backends) = wgpu::Backends::from_env() {
                    desc.backends = backends;
                }
                wgpu::Instance::new(desc)
            })
            .clone()
    }

    /// Open a real device on `instance` with `craft-gpu-device`'s choice for
    /// `Preference::Auto`. `None` on the defined no-adapter path (no usable adapter): the caller
    /// degrades to the staging backing, identical in shape to `craft-gpu-null`.
    ///
    /// Never panics: `craft_gpu_device::open` asks only for the features from
    /// [`DESIRED_FEATURES`] the adapter has, and for the adapter's own limits (never more), so
    /// `request_device`'s documented panics ("features/limits not supported by this adapter")
    /// cannot trigger here.
    pub(crate) fn create_device_on(instance: &wgpu::Instance) -> Option<(DeviceCaps, Backing)> {
        let request = Request { label: Some("craft-gpu-wgpu"), optional_features: DESIRED_FEATURES, ..Default::default() };
        let opened = craft_gpu_device::open(instance, None, &Preference::Auto, &request).ok()?;
        Some(backing_of(&opened))
    }

    pub(crate) fn create_device() -> Option<(DeviceCaps, Backing)> {
        create_device_on(&instance())
    }

    pub(crate) fn backing_of(opened: &Opened) -> (DeviceCaps, Backing) {
        backing_with(opened.device.clone(), opened.queue.clone(), opened.health.clone())
    }

    /// The device's own caps, and a backing that reads the loss from `health`.
    pub(crate) fn backing_with(device: wgpu::Device, queue: wgpu::Queue, health: DeviceHealth) -> (DeviceCaps, Backing) {
        let caps = craft_gpu_device::device_caps(&device);
        (caps, Backing::Device(DeviceBacking { device, queue, buffers: Vec::new(), health, pending: Vec::new() }))
    }

    /// The adapter `craft-gpu-device` would open for `Preference::Auto`, with its advertised
    /// limits and no features (no device was opened to grant any).
    pub(crate) fn probe_caps_on(instance: &wgpu::Instance) -> DeviceCaps {
        let (adapters, _) = craft_gpu_device::list(instance, None);
        match craft_gpu_device::pick(&adapters, None, &Preference::Auto) {
            Ok((adapter, _)) => DeviceCaps { features: Vec::new(), ..Capabilities::of_adapter(&adapter).into_device_caps() },
            Err(_) => probe_fallback("wgpu", "no-adapter"),
        }
    }
}

/// No real device: buffers live on the host, exactly like `craft-gpu-null`. RFC-0022's defined
/// degrade path for "no adapter" (feature off, no GPU present, or the `wasm32` async-init gap).
/// Each slot is `None` once freed (or never allocated): handles are stable indices, so a freed
/// slot cannot simply be removed without shifting every later handle's index.
#[cfg(not(all(not(target_arch = "wasm32"), feature = "adapter-probe")))]
enum Backing {
    Staging(Vec<Option<Vec<u8>>>),
}

#[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
enum Backing {
    Staging(Vec<Option<Vec<u8>>>),
    /// A real device. `DeviceCaps` was built from it, not from the adapter it came from.
    Device(DeviceBacking),
}

#[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
struct DeviceBacking {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Each buffer paired with its true, caller-given byte length. wgpu requires every buffer
    /// with copy usage to be sized in multiples of [`COPY_ALIGN`]; the buffer itself is padded to
    /// that, but callers (`buffer_len`, `request_read`'s returned bytes) see the true length,
    /// matching the staging path and `craft-gpu-null` exactly, with no padding visible. `None`
    /// once freed: see the `Backing::Staging` doc for why the slot stays rather than shifting.
    buffers: Vec<Option<(wgpu::Buffer, u64)>>,
    /// The device's health from `craft-gpu-device`, which holds its device-lost callback; its
    /// loss is checked before every call that would otherwise touch the (now invalid) device.
    health: craft_gpu_device::DeviceHealth,
    pending: Vec<PendingRead>,
}

/// wgpu's `COPY_BUFFER_ALIGNMENT`: every buffer with copy usage, and every `write_buffer` /
/// `copy_buffer_to_buffer` size, must be a multiple of this (4 bytes). Uploading or reading back
/// a buffer length that doesn't land on this boundary (`b"xy"`, two bytes) is a validation error
/// on native wgpu, surfaced as a panic from inside `wgpu-core`'s own uncaptured-error path unless
/// padded first; `craft-gpu-null`'s plain `Vec<u8>` has no such constraint, so this padding is
/// purely an artifact of the real backend, kept invisible to every caller of this crate.
#[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
const COPY_ALIGN: u64 = 4;

#[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
struct PendingRead {
    request: ReadRequest,
    handle: HandleId,
    staging: wgpu::Buffer,
    /// True length to truncate the mapped (padded) bytes back down to.
    true_len: u64,
    state: std::sync::Arc<std::sync::Mutex<Option<Result<(), String>>>>,
}

#[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
fn resolve_pending(pending: PendingRead) -> ReadOutcome {
    let PendingRead { request, handle, staging, true_len, state } = pending;
    let result = match state.lock() {
        Ok(mut guard) => match guard.take() {
            Some(Ok(())) => match staging.get_mapped_range(0..true_len.next_multiple_of(COPY_ALIGN)) {
                Ok(view) => {
                    let mut bytes = view.to_vec();
                    drop(view);
                    staging.unmap();
                    bytes.truncate(usize::try_from(true_len).unwrap_or(usize::MAX));
                    Ok(bytes)
                }
                Err(err) => Err(GpuFault::Operation(err.to_string())),
            },
            Some(Err(message)) => Err(GpuFault::Operation(message)),
            None => Err(GpuFault::Operation("read_buffer: map_async callback has not run yet".into())),
        },
        Err(_) => Err(GpuFault::Operation("read_buffer: map state lock poisoned".into())),
    };
    ReadOutcome { request, handle, result }
}

/// wgpu-backed context. See the module docs for the real-device-vs-staging split.
pub struct WgpuContext {
    caps: DeviceCaps,
    worker_kind: WorkerKind,
    backing: Backing,
    next_request: u64,
    ready: Vec<ReadOutcome>,
}

impl WgpuContext {
    pub fn new(worker_kind: WorkerKind) -> Self {
        #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
        {
            if let Some((caps, backing)) = native::create_device() {
                return Self { caps, worker_kind, backing, next_request: 0, ready: Vec::new() };
            }
            // request_adapter and/or request_device failed: fall back to the conservative
            // fallback, never probe_caps(). probe_caps() would re-query the adapter here, and a
            // second, successful adapter query would report the adapter's advertised limits as
            // if they came from a granted device -- the exact thing probe_caps's own doc comment
            // says caps() must never do for a context with no device.
            Self { caps: probe_fallback("wgpu", "no-device"), worker_kind, backing: Backing::Staging(Vec::new()), next_request: 0, ready: Vec::new() }
        }
        #[cfg(not(all(not(target_arch = "wasm32"), feature = "adapter-probe")))]
        Self { caps: probe_caps(), worker_kind, backing: Backing::Staging(Vec::new()), next_request: 0, ready: Vec::new() }
    }

    /// Force the staging (no-device) path with caller-supplied caps. For tests, and for callers
    /// that intentionally want the portable path (FilmCraft's `?webgl` / `?nogpuworkers`, per
    /// `docs/pilot-filmcraft.md`) rather than a real device's own caps.
    pub fn with_caps(worker_kind: WorkerKind, caps: DeviceCaps) -> Self {
        Self { caps, worker_kind, backing: Backing::Staging(Vec::new()), next_request: 0, ready: Vec::new() }
    }

    /// A context on a device `craft_gpu_device::open` (or `open_async`) opened, sharing its
    /// device, queue and [`DeviceHealth`](craft_gpu_device::DeviceHealth). The caps are the
    /// device's own (`Opened::device_caps`). A loss the health records, from the device-lost
    /// callback it set or from any other user of the device, is this context's
    /// `GpuFault::DeviceLost`.
    #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
    pub fn from_opened(worker_kind: WorkerKind, opened: &craft_gpu_device::Opened) -> Self {
        let (caps, backing) = native::backing_of(opened);
        Self { caps, worker_kind, backing, next_request: 0, ready: Vec::new() }
    }

    /// A context on a device the caller opened, such as the one egui-wgpu's render state holds.
    /// `health` is the device's [`DeviceHealth::watch`](craft_gpu_device::DeviceHealth::watch);
    /// wgpu keeps one device-lost callback per device, so this context sets none of its own. The
    /// caps are the device's own: the features and limits it was opened with, whatever they are.
    #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
    pub fn with_device(worker_kind: WorkerKind, device: wgpu::Device, queue: wgpu::Queue, health: craft_gpu_device::DeviceHealth) -> Self {
        let (caps, backing) = native::backing_with(device, queue, health);
        Self { caps, worker_kind, backing, next_request: 0, ready: Vec::new() }
    }

    /// The health of this context's device, when it has one: share it with the rest of the app,
    /// and with `craft_gpu_device::Recovery` after a loss.
    #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
    pub fn health(&self) -> Option<&craft_gpu_device::DeviceHealth> {
        match &self.backing {
            Backing::Device(d) => Some(&d.health),
            Backing::Staging(_) => None,
        }
    }

    /// True when this context holds a real device (`request_device` succeeded), false on the
    /// staging/no-adapter path. Settings UI can use this to show "no GPU" honestly instead of
    /// implying a device that was never created; tests use it to skip rather than fail when the
    /// host genuinely has no usable adapter (see `crates/gpu-wgpu/src/tests.rs`).
    pub fn has_device(&self) -> bool {
        #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
        {
            matches!(self.backing, Backing::Device(_))
        }

        #[cfg(not(all(not(target_arch = "wasm32"), feature = "adapter-probe")))]
        {
            false
        }
    }

    /// True once the device has reported loss. Every [`DeviceCaps`] read before this point is
    /// stale; see [`GpuContext`]'s trait docs for the recovery this context expects.
    pub fn is_device_lost(&self) -> bool {
        #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
        if let Backing::Device(d) = &self.backing {
            return d.health.device_status().is_err();
        }
        false
    }

    fn take_lost_fault(&self) -> Option<GpuFault> {
        #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
        if let Backing::Device(d) = &self.backing {
            return d.health.device_status().err();
        }
        None
    }

    /// `read_buffer`'s wait for the device between polls, for the work submitted so far.
    /// `DeviceHealth::wait` does not wait once the device reported an error other than out of
    /// memory, stops after its current 100 ms slice when another user of the device records
    /// such an error, and fails when its poll fails. Then the read of `request` is dropped and
    /// fails with that error, instead of polling in a loop until a map that may never complete;
    /// a loss is left to the next `poll_reads`, which fails every pending read with it.
    #[cfg(not(target_arch = "wasm32"))]
    #[cfg_attr(not(feature = "adapter-probe"), allow(unused_variables, clippy::unused_self))]
    fn wait_between_reads(&mut self, request: ReadRequest) -> Option<GpuFault> {
        #[cfg(feature = "adapter-probe")]
        if let Backing::Device(d) = &mut self.backing
            // a recorded fault other than out of memory: no empty submission to wait on
            && (d.health.fault().is_some_and(|f| !f.is_out_of_memory()) || !d.health.wait(&d.device, d.queue.submit([])))
        {
            d.health.probe(&d.device);
            if d.health.device_status().is_err() {
                return None;
            }
            d.pending.retain(|p| p.request != request);
            return Some(
                d.health.fault().map(GpuFault::from).unwrap_or_else(|| GpuFault::Operation("read_buffer: waiting for the device failed".into())),
            );
        }
        None
    }

    /// Why no new work may go to the device: a loss as `GpuFault::DeviceLost`, and any other
    /// fault its health recorded (an uncaptured error, or an out-of-memory error the app has not
    /// cleared with `DeviceHealth::clear_out_of_memory`) as `GpuFault::Operation`.
    fn take_work_fault(&self) -> Option<GpuFault> {
        #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
        if let Backing::Device(d) = &self.backing {
            return d.health.fault().map(GpuFault::from);
        }
        None
    }
}

impl GpuContext for WgpuContext {
    fn caps(&self) -> DeviceCaps {
        self.caps.clone()
    }

    fn worker_kind(&self) -> WorkerKind {
        self.worker_kind
    }

    #[cfg_attr(not(all(not(target_arch = "wasm32"), feature = "adapter-probe")), allow(unused_variables))]
    fn upload_buffer(&mut self, label: &str, bytes: &[u8]) -> Result<HandleId, GpuFault> {
        if let Some(fault) = self.take_work_fault() {
            return Err(fault);
        }
        match &mut self.backing {
            Backing::Staging(buffers) => {
                let id = u32::try_from(buffers.len()).map_err(|_| GpuFault::Operation("too many buffers".into()))?;
                buffers.push(Some(bytes.to_vec()));
                Ok(HandleId(id))
            }
            #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
            Backing::Device(d) => {
                let true_len = bytes.len() as u64;
                let padded_len = true_len.next_multiple_of(COPY_ALIGN);
                if padded_len > self.caps.max_buffer_size {
                    return Err(GpuFault::InvalidJob(format!(
                        "upload_buffer: {true_len} bytes (padded to {padded_len}) exceeds the device's max_buffer_size {}",
                        self.caps.max_buffer_size
                    )));
                }
                // The size check above only catches a request past the device's advertised
                // maximum; it proves nothing about whether the device can actually allocate
                // padded_len bytes right now (free VRAM varies). wgpu 30.0.1 sends an allocation
                // failure to the default uncaptured-error handler, which panics, unless a scope
                // captures it first. Pop this scope immediately, before write_buffer below ever
                // touches `buffer`: a buffer that failed to create is invalid, and writing to an
                // invalid buffer raises its own VALIDATION error, a kind this OutOfMemory-only
                // scope does not catch, so it must reach the default handler and panic regardless
                // of when this scope is popped. Checking right here is what keeps that from
                // happening at all.
                let create_scope = d.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
                let buffer = d.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: padded_len,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                if let Some(err) = pollster::block_on(create_scope.pop()) {
                    // The scope kept it from the device's health; every user of the device sees it.
                    d.health.caught(&d.device, &err);
                    return Err(GpuFault::Operation(format!("upload_buffer: device reported an allocation error creating the buffer: {err}")));
                }
                // write_buffer makes its own staging allocation to carry `bytes` to the device, a
                // second allocation that can also run out of memory; it gets its own scope rather
                // than reusing the one already closed above.
                let write_scope = d.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
                if padded_len == true_len {
                    d.queue.write_buffer(&buffer, 0, bytes);
                } else {
                    let mut padded = bytes.to_vec();
                    padded.resize(usize::try_from(padded_len).unwrap_or(bytes.len()), 0);
                    d.queue.write_buffer(&buffer, 0, &padded);
                }
                // write_buffer only records the copy into wgpu's PendingWrites; wgpu-core flushes
                // that queue at the sooner of the caller's next real command-buffer submission or
                // the next map_async on an affected buffer, and keeps the buffer named by the
                // write alive internally until then. Without a submission here, a free_buffer
                // right after this call could never actually release the GPU allocation behind
                // it, since nothing would ever flush PendingWrites to let it go. An empty submit
                // is still a submission as far as that flush is concerned, so it is enough.
                d.queue.submit(std::iter::empty());
                if let Some(err) = pollster::block_on(write_scope.pop()) {
                    d.health.caught(&d.device, &err);
                    return Err(GpuFault::Operation(format!("upload_buffer: device reported an allocation error writing the buffer: {err}")));
                }
                let id = u32::try_from(d.buffers.len()).map_err(|_| GpuFault::Operation("too many buffers".into()))?;
                d.buffers.push(Some((buffer, true_len)));
                Ok(HandleId(id))
            }
        }
    }

    fn buffer_len(&self, handle: HandleId) -> Result<usize, GpuFault> {
        if let Some(fault) = self.take_lost_fault() {
            return Err(fault);
        }
        match &self.backing {
            Backing::Staging(buffers) => {
                buffers.get(handle.0 as usize).and_then(Option::as_ref).map(Vec::len).ok_or_else(|| GpuFault::InvalidJob("bad handle".into()))
            }
            #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
            Backing::Device(d) => {
                d.buffers.get(handle.0 as usize).and_then(Option::as_ref).ok_or_else(|| GpuFault::InvalidJob("bad handle".into())).and_then(
                    |(_, true_len)| usize::try_from(*true_len).map_err(|_| GpuFault::Operation("buffer size exceeds host address space".into())),
                )
            }
        }
    }

    fn request_read(&mut self, handle: HandleId) -> Result<ReadRequest, GpuFault> {
        if let Some(fault) = self.take_work_fault() {
            return Err(fault);
        }
        match &mut self.backing {
            Backing::Staging(buffers) => {
                let bytes =
                    buffers.get(handle.0 as usize).and_then(Option::as_ref).cloned().ok_or_else(|| GpuFault::InvalidJob("bad handle".into()))?;
                let id = self.next_request;
                self.next_request = self.next_request.wrapping_add(1);
                let request = ReadRequest(id);
                self.ready.push(ReadOutcome { request, handle, result: Ok(bytes) });
                Ok(request)
            }
            #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
            Backing::Device(d) => {
                let (buffer, true_len) =
                    d.buffers.get(handle.0 as usize).and_then(Option::as_ref).ok_or_else(|| GpuFault::InvalidJob("bad handle".into()))?;
                let true_len = *true_len;
                if true_len == 0 {
                    // An empty buffer has nothing to copy or map (wgpu refuses an empty map
                    // range); resolve it immediately, same as the staging path.
                    let id = self.next_request;
                    self.next_request = self.next_request.wrapping_add(1);
                    let request = ReadRequest(id);
                    self.ready.push(ReadOutcome { request, handle, result: Ok(Vec::new()) });
                    return Ok(request);
                }
                let copy_len = true_len.next_multiple_of(COPY_ALIGN);
                if copy_len > self.caps.max_buffer_size {
                    return Err(GpuFault::Operation(format!(
                        "request_read: staging copy of {copy_len} bytes exceeds the device's max_buffer_size {}",
                        self.caps.max_buffer_size
                    )));
                }
                // Same reasoning as upload_buffer's error scope: an allocation failure for the
                // read-staging buffer must become a GpuFault, not reach the default
                // uncaptured-error handler and panic. Pop immediately, before copy_buffer_to_buffer
                // below ever names `staging` as a copy destination: a staging buffer that failed
                // to create is invalid, and copying into an invalid buffer raises its own
                // VALIDATION error, a kind this OutOfMemory-only scope does not catch, so it must
                // reach the default handler and panic regardless of when this scope is popped.
                let create_scope = d.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
                let staging = d.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("craft-gpu-wgpu.read-staging"),
                    size: copy_len,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                if let Some(err) = pollster::block_on(create_scope.pop()) {
                    d.health.caught(&d.device, &err);
                    return Err(GpuFault::Operation(format!("request_read: device reported an allocation error creating the staging buffer: {err}")));
                }
                // The copy and its submission get their own scope rather than reusing the one
                // already closed above, same split as upload_buffer's create vs. write.
                let copy_scope = d.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
                let mut encoder = d.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("craft-gpu-wgpu.read-copy") });
                encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, copy_len);
                d.queue.submit(Some(encoder.finish()));
                if let Some(err) = pollster::block_on(copy_scope.pop()) {
                    d.health.caught(&d.device, &err);
                    return Err(GpuFault::Operation(format!("request_read: device reported an allocation error copying the buffer: {err}")));
                }
                let state = std::sync::Arc::new(std::sync::Mutex::new(None));
                {
                    let state = std::sync::Arc::clone(&state);
                    staging.map_async(wgpu::MapMode::Read, 0..copy_len, move |result| {
                        if let Ok(mut slot) = state.lock() {
                            *slot = Some(result.map_err(|e| e.to_string()));
                        }
                    });
                }
                let id = self.next_request;
                self.next_request = self.next_request.wrapping_add(1);
                let request = ReadRequest(id);
                d.pending.push(PendingRead { request, handle, staging, true_len, state });
                Ok(request)
            }
        }
    }

    /// Replaces the slot with `None` rather than removing it: handles are stable indices, and
    /// removing would shift every later handle's index out from under its caller. Dropping a
    /// `Some((wgpu::Buffer, _))` here releases that buffer's GPU-side allocation.
    fn free_buffer(&mut self, handle: HandleId) {
        match &mut self.backing {
            Backing::Staging(buffers) => {
                if let Some(slot) = buffers.get_mut(handle.0 as usize) {
                    *slot = None;
                }
            }
            #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
            Backing::Device(d) => {
                if let Some(slot) = d.buffers.get_mut(handle.0 as usize) {
                    *slot = None;
                }
            }
        }
    }

    /// Covers the gap `poll_reads` has when nothing is pending: that method can only attach a
    /// device-lost fault to a read it has something to attach it to, so a caller whose event
    /// loop only polls, with no outstanding read, could never observe loss through it alone.
    fn device_status(&self) -> Result<(), GpuFault> {
        if let Some(fault) = self.take_lost_fault() {
            return Err(fault);
        }
        Ok(())
    }

    #[cfg_attr(not(all(not(target_arch = "wasm32"), feature = "adapter-probe")), allow(unused_mut))]
    fn poll_reads(&mut self) -> Vec<ReadOutcome> {
        let mut outcomes = std::mem::take(&mut self.ready);
        #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
        if let Backing::Device(d) = &mut self.backing {
            // A non-blocking poll that also runs after an error other than a loss was recorded,
            // and records a loss instead of panicking when the device is gone.
            d.health.probe(&d.device);
            let fault = d.health.polling_fault().map(GpuFault::from);
            if let Some(fault) = fault {
                outcomes.extend(d.pending.drain(..).map(|p| ReadOutcome { request: p.request, handle: p.handle, result: Err(fault.clone()) }));
                return outcomes;
            }
            let mut remaining = Vec::with_capacity(d.pending.len());
            for pending in d.pending.drain(..) {
                let is_ready = pending.state.lock().map(|g| g.is_some()).unwrap_or(false);
                if is_ready {
                    outcomes.push(resolve_pending(pending));
                } else {
                    remaining.push(pending);
                }
            }
            d.pending = remaining;
        }
        outcomes
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn read_buffer(&mut self, handle: HandleId) -> Result<Vec<u8>, GpuFault> {
        let request = self.request_read(handle)?;
        loop {
            let mut matched = None;
            let mut others = Vec::new();
            for outcome in self.poll_reads() {
                if outcome.request == request {
                    matched = Some(outcome.result);
                } else {
                    others.push(outcome);
                }
            }
            // Completions belonging to other outstanding requests go back on the queue for the
            // caller's next poll_reads instead of being dropped.
            self.ready.extend(others);
            if let Some(result) = matched {
                return result;
            }
            // Block on the GPU between polls instead of busy-spinning the CPU. A no-op when
            // `backing` is `Staging` (nothing to wait for; the request already resolved above)
            // or when wgpu is itself WebGPU under the hood, which self-polls.
            if let Some(fault) = self.wait_between_reads(request) {
                return Err(fault);
            }
        }
    }

    /// On `wasm32` the backing is always the in-memory staging store today (real device
    /// creation needs the async worker-init path and does not exist on this target yet), and a
    /// staging read resolves synchronously, so serving it blocks nothing. The refusal below
    /// guards the future wasm device backing: WebGPU can only map a buffer asynchronously, and
    /// `wasm32` is single-threaded, so blocking on a real device would stop the only thread the
    /// browser needs to run `map_async`'s own completion callback on, hanging the tab. Callers
    /// on a device backing drive [`request_read`](GpuContext::request_read) and
    /// [`poll_reads`](GpuContext::poll_reads) from the worker's message loop instead; see
    /// `docs/gpu-backends.md`.
    #[cfg(target_arch = "wasm32")]
    fn read_buffer(&mut self, handle: HandleId) -> Result<Vec<u8>, GpuFault> {
        let request = self.request_read(handle)?;
        match self.ready.iter().position(|o| o.request == request) {
            Some(i) => self.ready.remove(i).result,
            None => Err(GpuFault::Operation(
                "read_buffer on wasm32: the read did not resolve synchronously. Use request_read + poll_reads from the worker's message loop \
                 instead."
                    .into(),
            )),
        }
    }
}
