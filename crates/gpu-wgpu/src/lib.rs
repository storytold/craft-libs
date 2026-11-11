//! wgpu-backed [`GpuContext`](craft_gpu::GpuContext).
//!
//! WebGPU on `wasm32`; native APIs when backend features are enabled. With the `adapter-probe`
//! feature (native only, pollster) and at least one `native-*` backend feature,
//! [`WgpuContext::new`] requests a real device and builds [`DeviceCaps`] from what that device
//! actually enabled (`Device::limits()` / `Device::features()`), never from the adapter's
//! advertised maximum. Without a usable adapter (the features above are off, no GPU is present,
//! or on `wasm32` where real device creation needs the async worker-init path noted in
//! `docs/gpu-backends.md`), the context degrades to the same in-memory staging behavior as
//! `craft_gpu_null::NullContext` (not a dependency of this crate; named here only for context);
//! that is RFC-0022's defined no-adapter path, not an error.
//!
//! Readback is asynchronous end to end (`request_read` + `poll_reads`), because WebGPU can only
//! map a buffer that way. Native, with a real device, drives it with `Buffer::map_async` plus
//! `Device::poll`. The blocking `read_buffer` convenience (see
//! [`GpuContext`](craft_gpu::GpuContext)'s trait docs) blocks efficiently on native by polling
//! the device while it waits; on `wasm32` it instead returns a fault, since blocking there would
//! freeze the one thread the map's own completion callback needs to run on.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use craft_gpu::{DeviceCaps, GpuContext, GpuFault, HandleId, ReadOutcome, ReadRequest, WorkerKind};

#[cfg(test)]
mod tests;

fn probe_fallback(backend_name: &str, device_name: &str) -> DeviceCaps {
    DeviceCaps { backend_name: backend_name.into(), device_name: device_name.into(), ..DeviceCaps::default() }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
fn probe_caps_native() -> DeviceCaps {
    let instance = native::instance();
    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) {
        Ok(adapter) => adapter,
        Err(_) => return probe_fallback("wgpu", "no-adapter"),
    };
    let info = adapter.get_info();
    let limits = adapter.limits();
    DeviceCaps {
        backend_name: format!("{:?}", info.backend),
        device_name: info.name,
        max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
        max_buffer_size: limits.max_buffer_size,
        max_texture_dimension_2d: limits.max_texture_dimension_2d,
        features: Vec::new(),
    }
}

/// Adapter-advertised limits, when an adapter is available; otherwise conservative defaults. This
/// is the quick, device-free probe for UI surfaces that want a number before any context exists
/// (Settings). It is advisory only: these are the adapter's maximums, not a device's granted
/// ones. [`WgpuContext::caps`] is the authoritative, device-truth source once a context exists.
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
    use std::sync::{Arc, Mutex};

    use craft_gpu::{DeviceCaps, GpuFault};

    use crate::{Backing, DeviceBacking};

    /// Features this context asks a device for. No provider dispatches a real WGSL job on wgpu
    /// yet (`craft-gpu-providers` ships CPU stand-ins, by design, until that lands); extend this
    /// as real compute providers land, and it degrades gracefully regardless: see
    /// [`create_device`].
    const DESIRED_FEATURES: wgpu::Features = wgpu::Features::empty();

    /// An instance over whichever native backends were compiled in (`native-vulkan`,
    /// `native-dx12`, `native-metal`, `native-gles`), honoring `WGPU_BACKEND` when set, so the
    /// same build can be pointed at a software Vulkan adapter (lavapipe/llvmpipe) in CI.
    /// `new_without_display_handle` because this context never has a window (no surface is ever
    /// requested); FilmCraft's own off-screen export device uses the same constructor for the
    /// same reason (`filmcraft-gpu`'s `export_renderer.rs`, see `docs/gpu-backends.md`).
    pub(crate) fn instance() -> wgpu::Instance {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        if let Some(backends) = wgpu::Backends::from_env() {
            desc.backends = backends;
        }
        wgpu::Instance::new(desc)
    }

    /// Request a real device. `None` on the defined no-adapter path (no usable adapter): the
    /// caller degrades to the staging backing, identical in shape to `craft-gpu-null`.
    ///
    /// Never panics: `required_features` is intersected with what the adapter actually
    /// advertises, and `required_limits` is set to the adapter's own limits (never more), so
    /// `request_device`'s documented panics ("features/limits not supported by this adapter")
    /// cannot trigger here.
    pub(crate) fn create_device() -> Option<(DeviceCaps, Backing)> {
        let instance = instance();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        let features = DESIRED_FEATURES & adapter.features();
        let limits = adapter.limits();
        let desc =
            wgpu::DeviceDescriptor { label: Some("craft-gpu-wgpu"), required_features: features, required_limits: limits, ..Default::default() };
        let (device, queue) = pollster::block_on(adapter.request_device(&desc)).ok()?;

        let lost = Arc::new(Mutex::new(None));
        {
            let lost = Arc::clone(&lost);
            device.set_device_lost_callback(move |reason, message| {
                if let Ok(mut slot) = lost.lock() {
                    *slot = Some(GpuFault::DeviceLost(format!("{reason:?}: {message}")));
                }
            });
        }

        let info = device.adapter_info();
        let device_limits = device.limits();
        let caps = DeviceCaps {
            backend_name: format!("{:?}", info.backend),
            device_name: info.name,
            max_storage_buffer_binding_size: device_limits.max_storage_buffer_binding_size,
            max_buffer_size: device_limits.max_buffer_size,
            max_texture_dimension_2d: device_limits.max_texture_dimension_2d,
            features: device.features().iter_names().map(|(name, _)| name.to_string()).collect(),
        };
        let backing = Backing::Device(DeviceBacking { device, queue, buffers: Vec::new(), lost, pending: Vec::new() });
        Some((caps, backing))
    }
}

/// No real device: buffers live on the host, exactly like `craft-gpu-null`. RFC-0022's defined
/// degrade path for "no adapter" (feature off, no GPU present, or the `wasm32` async-init gap).
#[cfg(not(all(not(target_arch = "wasm32"), feature = "adapter-probe")))]
enum Backing {
    Staging(Vec<Vec<u8>>),
}

#[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
enum Backing {
    Staging(Vec<Vec<u8>>),
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
    /// matching the staging path and `craft-gpu-null` exactly, with no padding visible.
    buffers: Vec<(wgpu::Buffer, u64)>,
    /// Set by the device-lost callback registered in [`native::create_device`]; checked before
    /// every call that would otherwise touch the (now invalid) device.
    lost: std::sync::Arc<std::sync::Mutex<Option<GpuFault>>>,
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
        }
        Self { caps: probe_caps(), worker_kind, backing: Backing::Staging(Vec::new()), next_request: 0, ready: Vec::new() }
    }

    /// Force the staging (no-device) path with caller-supplied caps. For tests, and for callers
    /// that intentionally want the portable path (FilmCraft's `?webgl` / `?nogpuworkers`, per
    /// `docs/pilot-filmcraft.md`) rather than a real device's own caps.
    pub fn with_caps(worker_kind: WorkerKind, caps: DeviceCaps) -> Self {
        Self { caps, worker_kind, backing: Backing::Staging(Vec::new()), next_request: 0, ready: Vec::new() }
    }

    /// True when this context holds a real device (`request_device` succeeded), false on the
    /// staging/no-adapter path. Settings UI can use this to show "no GPU" honestly instead of
    /// implying a device that was never created; tests use it to skip rather than fail when the
    /// host genuinely has no usable adapter (see `crates/gpu-wgpu/src/tests.rs`).
    pub fn has_device(&self) -> bool {
        #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
        if matches!(self.backing, Backing::Device(_)) {
            return true;
        }
        false
    }

    /// True once the device has reported loss. Every [`DeviceCaps`] read before this point is
    /// stale; see [`GpuContext`]'s trait docs for the recovery this context expects.
    pub fn is_device_lost(&self) -> bool {
        #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
        if let Backing::Device(d) = &self.backing {
            return d.lost.lock().map(|g| g.is_some()).unwrap_or(false);
        }
        false
    }

    fn take_lost_fault(&self) -> Option<GpuFault> {
        #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
        if let Backing::Device(d) = &self.backing {
            return d.lost.lock().ok().and_then(|g| g.clone());
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
        if let Some(fault) = self.take_lost_fault() {
            return Err(fault);
        }
        match &mut self.backing {
            Backing::Staging(buffers) => {
                let id = u32::try_from(buffers.len()).map_err(|_| GpuFault::Operation("too many buffers".into()))?;
                buffers.push(bytes.to_vec());
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
                let buffer = d.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: padded_len,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                if padded_len == true_len {
                    d.queue.write_buffer(&buffer, 0, bytes);
                } else {
                    let mut padded = bytes.to_vec();
                    padded.resize(usize::try_from(padded_len).unwrap_or(bytes.len()), 0);
                    d.queue.write_buffer(&buffer, 0, &padded);
                }
                let id = u32::try_from(d.buffers.len()).map_err(|_| GpuFault::Operation("too many buffers".into()))?;
                d.buffers.push((buffer, true_len));
                Ok(HandleId(id))
            }
        }
    }

    fn buffer_len(&self, handle: HandleId) -> Result<usize, GpuFault> {
        if let Some(fault) = self.take_lost_fault() {
            return Err(fault);
        }
        match &self.backing {
            Backing::Staging(buffers) => buffers.get(handle.0 as usize).map(Vec::len).ok_or_else(|| GpuFault::InvalidJob("bad handle".into())),
            #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
            Backing::Device(d) => {
                d.buffers.get(handle.0 as usize).ok_or_else(|| GpuFault::InvalidJob("bad handle".into())).and_then(|(_, true_len)| {
                    usize::try_from(*true_len).map_err(|_| GpuFault::Operation("buffer size exceeds host address space".into()))
                })
            }
        }
    }

    fn request_read(&mut self, handle: HandleId) -> Result<ReadRequest, GpuFault> {
        if let Some(fault) = self.take_lost_fault() {
            return Err(fault);
        }
        match &mut self.backing {
            Backing::Staging(buffers) => {
                let bytes = buffers.get(handle.0 as usize).cloned().ok_or_else(|| GpuFault::InvalidJob("bad handle".into()))?;
                let id = self.next_request;
                self.next_request = self.next_request.wrapping_add(1);
                let request = ReadRequest(id);
                self.ready.push(ReadOutcome { request, handle, result: Ok(bytes) });
                Ok(request)
            }
            #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
            Backing::Device(d) => {
                let (buffer, true_len) = d.buffers.get(handle.0 as usize).ok_or_else(|| GpuFault::InvalidJob("bad handle".into()))?;
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
                let staging = d.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("craft-gpu-wgpu.read-staging"),
                    size: copy_len,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let mut encoder = d.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("craft-gpu-wgpu.read-copy") });
                encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, copy_len);
                d.queue.submit(Some(encoder.finish()));
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

    #[cfg_attr(not(all(not(target_arch = "wasm32"), feature = "adapter-probe")), allow(unused_mut))]
    fn poll_reads(&mut self) -> Vec<ReadOutcome> {
        let mut outcomes = std::mem::take(&mut self.ready);
        #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
        if let Backing::Device(d) = &mut self.backing {
            if let Some(fault) = d.lost.lock().ok().and_then(|g| g.clone()) {
                outcomes.extend(d.pending.drain(..).map(|p| ReadOutcome { request: p.request, handle: p.handle, result: Err(fault.clone()) }));
                return outcomes;
            }
            let _ = d.device.poll(wgpu::PollType::Poll);
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
            #[cfg(feature = "adapter-probe")]
            if let Backing::Device(d) = &self.backing {
                let _ = d.device.poll(wgpu::PollType::wait_indefinitely());
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
