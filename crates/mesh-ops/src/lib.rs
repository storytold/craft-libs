//! # craft-mesh-ops
//!
//! Triangle mesh operations: the [`Mesh`] type with smooth and [`flat_normals`] and MikkTSpace
//! tangents, built-in [`primitives`] (material ball, sphere, cube, …), a [`Bvh`] for ray casts and
//! box queries, the UV-space [`UvGBuffer`] that maps texels to surface points, and the
//! [`OrbitCamera`].
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod bvh;
pub mod camera;
pub mod gbuffer;
pub mod mesh;
pub mod primitives;
pub mod unwrap;

pub use bvh::{Bvh, Hit};
pub use camera::{OrbitCamera, Ray};
pub use gbuffer::{NO_TRI, Surface, UvGBuffer, surface_at};
pub use glam;
pub use mesh::{BaseColor, MAX_TRIANGLES, Mesh, SubMesh};

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum GeomError {
    #[error("invalid mesh: {0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, GeomError>;

/// The result of [`flat_normals`]: one vertex per triangle corner.
#[derive(Clone, Debug, PartialEq)]
pub struct FlatNormals {
    /// The position of each output vertex.
    pub positions: Vec<glam::Vec3>,
    /// The unit normal of each output vertex's triangle, +Z for a triangle without area.
    pub normals: Vec<glam::Vec3>,
    /// Triangles over the output vertices, in the input's triangle order: `0, 1, 2, ...`.
    pub indices: Vec<u32>,
    /// The input vertex each output vertex copies, for carrying other per-vertex data (UVs,
    /// colors, skin joints and weights) over to the output.
    pub source: Vec<u32>,
}

/// Flat normals: each triangle corner becomes its own vertex, with the unit normal of its
/// triangle on the side from which the corners run counterclockwise. The cross product of the
/// edges is taken in `f64`. A triangle without area gets +Z, and vertices no triangle uses are
/// left out.
///
/// An index count that is not a multiple of 3, an index past the last vertex, a non-finite
/// position or more than [`MAX_TRIANGLES`] triangles is `Invalid`. These checks read the
/// borrowed slices before any output is allocated.
pub fn flat_normals(positions: &[glam::Vec3], indices: &[u32]) -> Result<FlatNormals> {
    let corners = mesh::flat_corners(positions, indices)?;
    Ok(FlatNormals { positions: corners.positions, normals: corners.normals, indices: corners.indices, source: corners.source })
}
