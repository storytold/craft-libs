//! Turns one decoded mesh block plus one placement into a [`ScenePart`].
//!
//! Conventions of the result:
//! - Positions, normals and tangents are in world space (the node's world transform applied).
//!   A mirroring transform reverses the triangle winding so faces keep pointing outwards.
//! - UVs: Assimp and the reference app store OpenGL-style UVs (origin bottom-left, V up); `ScenePart`
//!   uses glTF's (origin top-left, V down), so V is flipped: `v' = 1 - v`. Only u and v are kept.
//! - Tangent w is the handedness `sign(dot(cross(n, t), b))` with Assimp's bitangent, which
//!   points along increasing OpenGL V. That is "up" in the image, the direction glTF expects
//!   `cross(n, t) * w` to point, so the V flip does not change w.
//! - Non-finite normals and tangents (Assimp's output for degenerate triangles) are repaired.

use super::faces::triangulate;
use super::mesh_block::{MeshBlock, RawArray};
use super::repair::repair_frames;
use super::scene_mesh::ScenePart;
use super::transform::{Transform, cross, dot, normalize};
use crate::decode::DecodeError;

/// Builds the part for `mesh` placed with `world`. `name` is the part's name.
pub fn build_part(mesh: &MeshBlock, world: &Transform, name: String) -> Result<ScenePart, DecodeError> {
    mesh.check_lengths()?;
    let mut triangles = triangulate(mesh.faces, mesh.face_count, mesh.vertex_count())?;
    if world.determinant() < 0.0 {
        triangles.iter_mut().for_each(|t| t.swap(1, 2));
    }
    let normals: Vec<[f32; 3]> = decode(mesh.normals).into_iter().map(|n| normalize(world.normal(n))).collect();
    let mut part = ScenePart {
        name,
        material: mesh.material_index as usize,
        positions: decode(mesh.positions).into_iter().map(|p| world.point(p)).collect(),
        tangents: tangents(mesh, world, &normals),
        normals,
        uv_sets: mesh.uv_sets.iter().map(flipped_uvs).collect(),
        triangles,
        polygon_starts: Vec::new(),
    };
    repair_frames(&mut part);
    Ok(part)
}

fn decode(array: Option<RawArray>) -> Vec<[f32; 3]> {
    array.map(|a| a.vec3s()).unwrap_or_default()
}

/// World-space tangents with handedness in w; empty without stored tangents or normals.
fn tangents(mesh: &MeshBlock, world: &Transform, normals: &[[f32; 3]]) -> Vec<[f32; 4]> {
    let Some((tangents, bitangents)) = mesh.tangent_frames else {
        return Vec::new();
    };
    if normals.is_empty() {
        return Vec::new();
    }
    let frames = tangents.vec3s().into_iter().zip(bitangents.vec3s()).zip(normals);
    frames
        .map(|((t, b), n)| {
            let t = normalize(world.direction(t));
            let b = world.direction(b);
            let w = if dot(cross(*n, t), b) < 0.0 { -1.0 } else { 1.0 };
            [t[0], t[1], t[2], w]
        })
        .collect()
}

fn flipped_uvs(set: &RawArray) -> Vec<[f32; 2]> {
    set.vec3s().into_iter().map(|uv| [uv[0], 1.0 - uv[1]]).collect()
}
