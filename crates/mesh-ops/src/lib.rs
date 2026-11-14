//! # craft-mesh-ops
//!
//! Operations on triangle meshes: area-weighted smooth normals, MikkTSpace tangents, welding,
//! automatic UV unwrapping, the UV-space [`UvGBuffer`] that maps texels to surface points,
//! built-in [`primitives`] with UVs, a [`Bvh`] for ray casts and box queries (feature `bvh`) and
//! the [`OrbitCamera`].
//!
//! The functions at the crate root take plain slices: positions and normals as `[f32; 3]`, UVs as
//! `[f32; 2]` in the glTF convention (origin top-left, v down), tangents as `[f32; 4]` with the
//! bitangent sign in `w`, and triangle indices as `u32`. They check the input against
//! [`Limits`] and reject a non-finite value or an index past the last vertex before any work.
//! The [`Mesh`] type holds the same arrays for callers that keep several materials together.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(feature = "bvh")]
pub mod bvh;
pub mod camera;
pub mod gbuffer;
pub mod mesh;
pub mod primitives;
pub mod unwrap;

#[cfg(feature = "bvh")]
pub use bvh::{Bvh, Hit};
pub use camera::{OrbitCamera, Ray};
pub use gbuffer::{NO_TRI, Surface, UvGBuffer, surface_at};
pub use glam;
pub use mesh::{MAX_TRIANGLES, Mesh, SubMesh};

use glam::Vec3;

/// Why an operation did not run.
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum MeshOpsError {
    /// The input is malformed: mismatched array lengths, an index out of range, a non-finite
    /// value or an invalid parameter.
    #[error("invalid mesh: {0}")]
    Invalid(String),
    /// A count is over its [`Limits`] field.
    #[error("{what} {got} is over the limit of {limit}")]
    TooLarge { what: &'static str, got: u64, limit: u64 },
    /// Memory for the result could not be reserved.
    #[error("out of memory for {0}")]
    OutOfMemory(&'static str),
}

pub type Result<T> = std::result::Result<T, MeshOpsError>;

/// Caps on the work one call does. Every slice function and `try_` constructor checks its input
/// against these before allocating. The defaults follow `craft-mesh` for whole meshes and are
/// this crate's own values for the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Vertices in one input or output mesh. Default 20,000,000.
    pub max_vertices: usize,
    /// Triangles in one input or output mesh. Default 20,000,000.
    pub max_triangles: usize,
    /// Segments, rings or subdivisions along one direction of a primitive. Default 2,048.
    pub max_segments: usize,
    /// Width or height of a UV G-buffer. Default 8,192.
    pub max_gbuffer_side: u32,
    /// Texels in a UV G-buffer. Default 2^26 (8,192 × 8,192).
    pub max_gbuffer_texels: u64,
    /// Triangles one UV unwrap processes. Default 4,000,000.
    pub max_unwrap_triangles: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_vertices: 20_000_000,
            max_triangles: 20_000_000,
            max_segments: 2_048,
            max_gbuffer_side: 8_192,
            max_gbuffer_texels: 1 << 26,
            max_unwrap_triangles: 4_000_000,
        }
    }
}

/// `Err(TooLarge)` when `got` is over `limit`.
pub(crate) fn check_limit(what: &'static str, got: usize, limit: usize) -> Result<()> {
    if got > limit { Err(MeshOpsError::TooLarge { what, got: got as u64, limit: limit as u64 }) } else { Ok(()) }
}

/// An empty vector with room for `len` elements, or `OutOfMemory`.
pub(crate) fn try_vec<T>(len: usize, what: &'static str) -> Result<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| MeshOpsError::OutOfMemory(what))?;
    Ok(v)
}

/// `len` copies of `value`, or `OutOfMemory`.
pub(crate) fn try_filled<T: Clone>(len: usize, value: T, what: &'static str) -> Result<Vec<T>> {
    let mut v = try_vec(len, what)?;
    v.resize(len, value);
    Ok(v)
}

/// `OutOfMemory` for `what` in place of a failed reservation.
pub(crate) fn reserved(r: std::result::Result<(), std::collections::TryReserveError>, what: &'static str) -> Result<()> {
    r.map_err(|_| MeshOpsError::OutOfMemory(what))
}

/// The result of [`unwrap_uvs`]: vertices are split where charts meet, so the output has its own
/// vertex list.
#[derive(Clone, Debug, PartialEq)]
pub struct Unwrapped {
    /// The input vertex each output vertex copies.
    pub source: Vec<u32>,
    /// One UV per output vertex, in 0..1.
    pub uvs: Vec<[f32; 2]>,
    /// Triangles over the output vertices, in the input's triangle order.
    pub indices: Vec<u32>,
    /// Number of charts.
    pub charts: usize,
}

/// Axis-aligned bounds of `positions`, or `None` when there are none. A non-finite coordinate is
/// an error.
pub fn bounds(positions: &[[f32; 3]]) -> Result<Option<([f32; 3], [f32; 3])>> {
    let mut found: Option<(Vec3, Vec3)> = None;
    for p in positions {
        let p = Vec3::from_array(*p);
        if !p.is_finite() {
            return Err(MeshOpsError::Invalid("non-finite position".into()));
        }
        found = Some(found.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p))));
    }
    Ok(found.map(|(lo, hi)| (lo.to_array(), hi.to_array())))
}

/// Area-weighted smooth normals, one per vertex. A vertex no triangle uses gets +Y.
pub fn smooth_normals(positions: &[[f32; 3]], indices: &[u32], limits: &Limits) -> Result<Vec<[f32; 3]>> {
    let mut mesh = Mesh::from_slices(positions, None, None, indices, limits)?;
    mesh.try_compute_normals()?;
    let mut out = try_vec(mesh.normals.len(), "normals")?;
    out.extend(mesh.normals.iter().map(|n| n.to_array()));
    Ok(out)
}

/// MikkTSpace tangents, one per vertex: xyz tangent and the bitangent sign in `w`, so that
/// `cross(normal, tangent) * w` points up the image (glTF 2.0). Meshes MikkTSpace cannot handle
/// (every triangle degenerate, no UV area, more than [`mesh::MAX_MIKKTSPACE_VALENCE`] triangles
/// around one vertex) get per-vertex tangents instead. The normals are unit vectors, as glTF
/// 2.0 stores them: each tangent is made perpendicular to its normal with a projection that is
/// exact only at unit length.
///
/// A vertex whose triangles need different tangent frames (UVs mirrored across it) cannot have
/// one tangent: that is an `Invalid` error naming the vertex. [`Mesh::compute_tangents`] splits
/// such vertices instead.
pub fn tangents(positions: &[[f32; 3]], normals: &[[f32; 3]], uvs: &[[f32; 2]], indices: &[u32], limits: &Limits) -> Result<Vec<[f32; 4]>> {
    let mut mesh = Mesh::from_slices(positions, Some(normals), Some(uvs), indices, limits)?;
    mesh.try_compute_tangents()?;
    if let Some(k) = mesh.indices.iter().position(|&v| v as usize >= positions.len()) {
        let vertex = indices.get(k).copied().unwrap_or_default();
        return Err(MeshOpsError::Invalid(format!("vertex {vertex} is shared by triangles with different tangent frames")));
    }
    let mut out = try_vec(mesh.tangents.len(), "tangents")?;
    out.extend(mesh.tangents.iter().map(|t| t.to_array()));
    Ok(out)
}

/// Welded vertex id per vertex, as [`unwrap::weld_ids`] gives it, for finite positions.
pub fn weld(positions: &[[f32; 3]], limits: &Limits) -> Result<Vec<u32>> {
    check_limit("vertex count", positions.len(), limits.max_vertices)?;
    let mut p = try_vec(positions.len(), "positions")?;
    for q in positions {
        let q = Vec3::from_array(*q);
        if !q.is_finite() {
            return Err(MeshOpsError::Invalid("non-finite position".into()));
        }
        p.push(q);
    }
    unwrap::weld_ids(&p)
}

/// An automatic non-overlapping UV layout for one material, as [`unwrap::auto_unwrap`] makes it.
/// The vertex limit applies to the input and to the output, which has more vertices where
/// charts meet. A padding that is not finite is an error; one under 1 texel counts as 1.
pub fn unwrap_uvs(positions: &[[f32; 3]], indices: &[u32], padding_texels: f32, resolution: u32, limits: &Limits) -> Result<Unwrapped> {
    if !padding_texels.is_finite() {
        return Err(MeshOpsError::Invalid(format!("padding {padding_texels} is not finite")));
    }
    check_limit("triangle count", indices.len() / 3, limits.max_unwrap_triangles)?;
    let mut mesh = Mesh::from_slices(positions, None, None, indices, limits)?;
    let (charts, source) = unwrap::unwrap_core(&mut mesh, padding_texels, resolution, limits.max_vertices)?;
    let mut uvs = try_vec(mesh.uvs.len(), "UVs")?;
    uvs.extend(mesh.uvs.iter().map(|uv| uv.to_array()));
    Ok(Unwrapped { source, uvs, indices: mesh.indices, charts })
}
