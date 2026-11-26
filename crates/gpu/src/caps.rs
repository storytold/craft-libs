use serde::{Deserialize, Serialize};

/// Limits and labels reported to Settings UI and worker RPC (`GpuCaps`).
///
/// Per RFC-0022's acceptance requirements, a backend that has a device fills every field here
/// from that created device, never from the adapter it was created on: an adapter advertises more
/// than a device may grant (`required_features`/`required_limits` at `request_device` time narrow
/// both). `craft-gpu-wgpu` reports [`Default`]-shaped conservative values only on the defined
/// no-adapter path (no GPU present, the feature compiled out, or the `wasm32` worker-init gap
/// described in `docs/gpu-backends.md`), never as a stand-in for a real device's own caps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCaps {
    pub backend_name: String,
    pub device_name: String,
    pub max_storage_buffer_binding_size: u64,
    pub max_buffer_size: u64,
    pub max_texture_dimension_2d: u32,
    /// Feature names actually enabled on the created device (`wgpu::Features::iter_names` on
    /// native wgpu), empty on backends with no device. A fast path gates on membership here,
    /// never on what the adapter merely advertised.
    #[serde(default)]
    pub features: Vec<String>,
}

impl Default for DeviceCaps {
    fn default() -> Self {
        Self {
            backend_name: "null".into(),
            device_name: "null".into(),
            max_storage_buffer_binding_size: 128 * 1024 * 1024,
            max_buffer_size: 256 * 1024 * 1024,
            max_texture_dimension_2d: 8192,
            features: Vec::new(),
        }
    }
}
