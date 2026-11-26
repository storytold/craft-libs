use serde::{Deserialize, Serialize};

/// Opaque resource handle in a job graph (resolved in the active [`GpuContext`](crate::GpuContext)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HandleId(pub u32);

/// Stable provider name (`craft.readback`, `effectcraft.compositor.frame`, …).
pub type ProviderId = String;

/// Versioned parameter blob; schema owned by the provider crate, not craft-gpu.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuJobParams(pub Vec<u8>);

/// Serializable work unit for generic worker dispatch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuJob {
    pub provider_id: ProviderId,
    pub params: GpuJobParams,
    pub inputs: Vec<HandleId>,
    pub outputs: Vec<HandleId>,
}

/// Result handles or side-channel bytes (provider-defined interpretation).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuJobOutcome {
    pub outputs: Vec<HandleId>,
    pub data: Option<Vec<u8>>,
}
