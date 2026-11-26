//! # craft-mesh-ops
//!
//! Triangle mesh operations: the [`Mesh`] type with normals and MikkTSpace tangents, built-in
//! [`primitives`] (material ball, sphere, cube, …), a [`Bvh`] for ray casts and box queries,
//! the UV-space [`UvGBuffer`] that maps texels to surface points, and the [`OrbitCamera`].
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
