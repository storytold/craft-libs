//! `/paint/scene.assbin`: the project mesh as an Assimp binary dump.
//!
//! The reference app writes its own variant of Assimp's format (see the `block` and `scene` modules).
//! [`AssbinScene`] is the cheap summary (counts and names); [`AssbinGeometry`] decodes everything.

mod block;
mod camera;
#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod corpus_tests;
mod faces;
mod geometry;
#[cfg(test)]
mod geometry_tests;
mod header;
mod material;
mod mesh;
mod mesh_block;
mod mesh_meta;
mod node;
mod normalization;
mod part;
mod repair;
mod scene;
mod scene_mesh;
#[cfg(test)]
pub(crate) mod test_writer;
#[cfg(test)]
mod tests;
mod transform;

pub use camera::AssbinCamera;
pub use geometry::AssbinGeometry;
pub use header::{AssbinHeader, HEADER_LEN};
pub use material::MaterialSummary;
pub use mesh::MeshSummary;
pub use mesh_meta::MeshMetadata;
pub use normalization::Normalization;
pub use scene_mesh::{SceneMesh, ScenePart};

use crate::decode::DecodeError;
use scene::{SceneBlocks, scene_bytes};

const RECORD: &str = "scene.assbin";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssbinScene {
    pub meshes: Vec<MeshSummary>,
    pub materials: Vec<MaterialSummary>,
}

impl AssbinScene {
    /// Reads names and counts only; vertex data is skipped, not decoded.
    pub fn parse(data: &[u8]) -> Result<Self, DecodeError> {
        let scene = scene_bytes(data)?;
        let blocks = SceneBlocks::split(&scene)?;
        Ok(AssbinScene { meshes: parse_all(&blocks.meshes, MeshSummary::parse)?, materials: parse_all(&blocks.materials, MaterialSummary::parse)? })
    }
}

fn parse_all<T>(blocks: &[&[u8]], parse: impl Fn(&[u8]) -> Result<T, DecodeError>) -> Result<Vec<T>, DecodeError> {
    blocks.iter().map(|b| parse(b)).collect()
}
