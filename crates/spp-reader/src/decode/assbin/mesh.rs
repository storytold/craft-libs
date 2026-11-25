//! Per-mesh summary: name, counts and material slot, read without decoding vertex data.

use super::mesh_block::MeshBlock;
use crate::decode::DecodeError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshSummary {
    pub name: String,
    pub vertices: u32,
    pub faces: u32,
    pub uv_channels: u32,
    pub material_index: u32,
}

impl MeshSummary {
    /// Parses the content of one mesh block.
    pub fn parse(block: &[u8]) -> Result<Self, DecodeError> {
        let mesh = MeshBlock::parse(block)?;
        Ok(MeshSummary {
            // Counts were read from u32 fields, so they fit.
            vertices: mesh.vertex_count() as u32,
            faces: mesh.face_count,
            uv_channels: mesh.uv_sets.len() as u32,
            material_index: mesh.material_index,
            name: mesh.name,
        })
    }
}
