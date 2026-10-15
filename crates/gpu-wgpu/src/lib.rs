//! wgpu-backed [`GpuContext`](craft_gpu::GpuContext).
//!
//! WebGPU on `wasm32`; native APIs when backend features are enabled. Adapter probing is
//! optional on native via the `adapter-probe` feature (pollster).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use craft_gpu::{DeviceCaps, GpuContext, GpuFault, HandleId, WorkerKind};

fn probe_fallback(backend_name: &str, device_name: &str) -> DeviceCaps {
    DeviceCaps { backend_name: backend_name.into(), device_name: device_name.into(), ..DeviceCaps::default() }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
fn probe_caps_native() -> DeviceCaps {
    let instance = wgpu::Instance::default();
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
    }
}

/// Report device limits when an adapter is available; otherwise conservative defaults.
pub fn probe_caps() -> DeviceCaps {
    #[cfg(all(not(target_arch = "wasm32"), feature = "adapter-probe"))]
    {
        return probe_caps_native();
    }

    #[cfg(all(not(target_arch = "wasm32"), not(feature = "adapter-probe")))]
    {
        return probe_fallback("wgpu", "probe-not-run");
    }

    #[cfg(target_arch = "wasm32")]
    {
        probe_fallback("webgpu", "browser")
    }
}

/// wgpu-backed context; buffer IO uses staging until full device wiring lands (craft-libs#6).
pub struct WgpuContext {
    caps: DeviceCaps,
    worker_kind: WorkerKind,
    staging: Vec<Vec<u8>>,
}

impl WgpuContext {
    pub fn new(worker_kind: WorkerKind) -> Self {
        Self { caps: probe_caps(), worker_kind, staging: Vec::new() }
    }

    pub fn with_caps(worker_kind: WorkerKind, caps: DeviceCaps) -> Self {
        Self { caps, worker_kind, staging: Vec::new() }
    }
}

impl GpuContext for WgpuContext {
    fn caps(&self) -> DeviceCaps {
        self.caps.clone()
    }

    fn worker_kind(&self) -> WorkerKind {
        self.worker_kind
    }

    fn upload_buffer(&mut self, _label: &str, bytes: &[u8]) -> Result<HandleId, GpuFault> {
        let id = HandleId(u32::try_from(self.staging.len()).map_err(|_| GpuFault::Operation("too many buffers".into()))?);
        self.staging.push(bytes.to_vec());
        Ok(id)
    }

    fn buffer_len(&self, handle: HandleId) -> Result<usize, GpuFault> {
        self.staging.get(handle.0 as usize).map(|b| b.len()).ok_or_else(|| GpuFault::InvalidJob("bad handle".into()))
    }

    fn read_buffer(&self, handle: HandleId) -> Result<Vec<u8>, GpuFault> {
        self.staging.get(handle.0 as usize).cloned().ok_or_else(|| GpuFault::InvalidJob("bad handle".into()))
    }
}
