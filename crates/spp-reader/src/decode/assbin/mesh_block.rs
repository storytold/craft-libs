//! The layout of one mesh block, located but not decoded: each vertex array as a raw slice.
//!
//! Layout (after the block size): name, `u32` primitive types, `u32` material index,
//! `u32` component bits, then the vertex arrays in Assimp's order (positions, normals,
//! tangents, bitangents, colour sets, UV sets), each prefixed with its own `u32` element count,
//! UV sets additionally with a `u32` component count first. Then `u32` face count, the faces,
//! and bones (not read).

use super::RECORD;
use super::block::{read_count, read_string, read_u32};
use crate::container::bytes::ByteReader;
use crate::decode::DecodeError;

const HAS_POSITIONS: u32 = 0x1;
const HAS_NORMALS: u32 = 0x2;
const HAS_TANGENTS_AND_BITANGENTS: u32 = 0x4;
const TEXCOORD_BASE: u32 = 0x100;
const COLOR_BASE: u32 = 0x1_0000;
/// Assimp's limit for both colour sets and texture coordinate sets.
const MAX_SETS: u32 = 8;

const VEC3_LEN: usize = 12;
const COLOR4_LEN: usize = 16;

/// A counted array of `f32` vectors, still as bytes.
#[derive(Debug, Clone, Copy)]
pub struct RawArray<'a> {
    pub count: usize,
    pub bytes: &'a [u8],
}

impl RawArray<'_> {
    /// Decodes the array as 3-component vectors.
    pub fn vec3s(&self) -> Vec<[f32; 3]> {
        self.bytes.as_chunks::<VEC3_LEN>().0.iter().map(vec3).collect()
    }
}

fn vec3(&[x0, x1, x2, x3, y0, y1, y2, y3, z0, z1, z2, z3]: &[u8; VEC3_LEN]) -> [f32; 3] {
    [[x0, x1, x2, x3], [y0, y1, y2, y3], [z0, z1, z2, z3]].map(f32::from_le_bytes)
}

pub struct MeshBlock<'a> {
    pub name: String,
    pub material_index: u32,
    pub positions: Option<RawArray<'a>>,
    pub normals: Option<RawArray<'a>>,
    /// Tangents and bitangents, always stored together.
    pub tangent_frames: Option<(RawArray<'a>, RawArray<'a>)>,
    pub color_sets: Vec<RawArray<'a>>,
    /// Stored as 3D vectors whatever the component count; the reference app's are 2D.
    pub uv_sets: Vec<RawArray<'a>>,
    pub face_count: u32,
    /// The faces and everything after them.
    pub faces: &'a [u8],
}

impl<'a> MeshBlock<'a> {
    pub fn parse(block: &'a [u8]) -> Result<Self, DecodeError> {
        let mut r = ByteReader::new(block);
        let name = read_string(&mut r)?;
        let _primitive_types = read_u32(&mut r)?;
        let material_index = read_u32(&mut r)?;
        let components = read_u32(&mut r)?;
        let has = |bit: u32| components & bit != 0;
        let positions = optional(&mut r, has(HAS_POSITIONS), VEC3_LEN)?;
        let normals = optional(&mut r, has(HAS_NORMALS), VEC3_LEN)?;
        let tangent_frames = match has(HAS_TANGENTS_AND_BITANGENTS) {
            true => Some((array(&mut r, VEC3_LEN)?, array(&mut r, VEC3_LEN)?)),
            false => None,
        };
        let color_sets = (0..set_count(components, COLOR_BASE)).map(|_| array(&mut r, COLOR4_LEN)).collect::<Result<_, _>>()?;
        let uv_sets = (0..set_count(components, TEXCOORD_BASE))
            .map(|_| {
                let _components_per_uv = read_u32(&mut r)?;
                array(&mut r, VEC3_LEN)
            })
            .collect::<Result<_, _>>()?;
        let face_count = read_u32(&mut r)?;
        Ok(MeshBlock { name, material_index, positions, normals, tangent_frames, color_sets, uv_sets, face_count, faces: r.rest() })
    }

    /// Number of vertices: the length of the position array.
    pub fn vertex_count(&self) -> usize {
        self.positions.map_or(0, |p| p.count)
    }

    /// Fails unless every vertex array has one entry per vertex.
    pub fn check_lengths(&self) -> Result<(), DecodeError> {
        let n = self.vertex_count();
        let tangents = self.tangent_frames.iter().flat_map(|(t, b)| [t, b]);
        let all = self.normals.iter().chain(tangents).chain(&self.color_sets).chain(&self.uv_sets);
        match all.map(|a| a.count).find(|&count| count != n) {
            Some(count) => Err(DecodeError::invalid(RECORD, format!("mesh '{}' has a vertex array of {count} entries for {n} vertices", self.name))),
            None => Ok(()),
        }
    }
}

fn optional<'a>(r: &mut ByteReader<'a>, present: bool, item_len: usize) -> Result<Option<RawArray<'a>>, DecodeError> {
    present.then(|| array(r, item_len)).transpose()
}

fn array<'a>(r: &mut ByteReader<'a>, item_len: usize) -> Result<RawArray<'a>, DecodeError> {
    let count = read_count(r, item_len)?;
    // `read_count` checked that `count * item_len` fits in the remaining data.
    let bytes = r.bytes(count * item_len).map_err(DecodeError::truncated(RECORD))?;
    Ok(RawArray { count, bytes })
}

/// Number of consecutive sets present, as Assimp stops at the first missing one.
fn set_count(components: u32, base: u32) -> u32 {
    (0..MAX_SETS).take_while(|&n| components & (base << n) != 0).count() as u32
}
