use serde::{Deserialize, Serialize};

/// Limits and labels reported to Settings UI and worker RPC (`GpuCaps`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCaps {
    pub backend_name: String,
    pub device_name: String,
    pub max_storage_buffer_binding_size: u64,
    pub max_buffer_size: u64,
    pub max_texture_dimension_2d: u32,
}

impl Default for DeviceCaps {
    fn default() -> Self {
        Self {
            backend_name: "null".into(),
            device_name: "null".into(),
            max_storage_buffer_binding_size: 128 * 1024 * 1024,
            max_buffer_size: 256 * 1024 * 1024,
            max_texture_dimension_2d: 8192,
        }
    }
}
