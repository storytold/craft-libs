//! The node tree: which meshes are placed where.
//!
//! A node block holds: name, 16 `f32` local transform (row-major), `u32` child count,
//! `u32` mesh count, the mesh index array (`u32` count + indices), the child array
//! (`u32` count + child node blocks), then metadata (not read; the reference app stores its
//! normalisation there, e.g. the normalisation centre and radius).

use super::RECORD;
use super::block::{read_block, read_count, read_string, read_u32};
use super::transform::Transform;
use crate::container::bytes::ByteReader;
use crate::decode::DecodeError;

const MAX_DEPTH: usize = 128;
/// Upper bound on mesh placements, against node trees that repeat a mesh without end.
const MAX_INSTANCES: usize = 1 << 16;
/// Smallest child block: size field, empty name, matrix and four counts.
const MIN_NODE_LEN: usize = 8 + 4 + 64 + 16;

/// One placement of a mesh: the mesh index and the node's world transform.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshInstance {
    pub mesh: u32,
    pub node: String,
    pub world: Transform,
}

/// Every mesh placement in the tree, in depth-first order.
pub fn mesh_instances(root_node: &[u8]) -> Result<Vec<MeshInstance>, DecodeError> {
    let mut out = Vec::new();
    visit(root_node, &Transform::IDENTITY, 0, &mut out)?;
    Ok(out)
}

fn visit(block: &[u8], parent: &Transform, depth: usize, out: &mut Vec<MeshInstance>) -> Result<(), DecodeError> {
    if depth > MAX_DEPTH {
        return Err(DecodeError::invalid(RECORD, "node tree nested too deeply"));
    }
    let mut r = ByteReader::new(block);
    let name = read_string(&mut r)?;
    let world = parent.then_local(&read_matrix(&mut r)?);
    let child_count = read_u32(&mut r)? as usize;
    let mesh_count = read_u32(&mut r)? as usize;
    let meshes = read_count(&mut r, 4)?;
    expect_equal("mesh", mesh_count, meshes)?;
    for _ in 0..meshes {
        if out.len() >= MAX_INSTANCES {
            return Err(DecodeError::invalid(RECORD, "too many mesh placements"));
        }
        out.push(MeshInstance { mesh: read_u32(&mut r)?, node: name.clone(), world });
    }
    let children = read_count(&mut r, MIN_NODE_LEN)?;
    expect_equal("child", child_count, children)?;
    for _ in 0..children {
        visit(read_block(&mut r)?, &world, depth + 1, out)?;
    }
    Ok(())
}

fn read_matrix(r: &mut ByteReader) -> Result<Transform, DecodeError> {
    let mut v = [0.0f32; 16];
    for x in &mut v {
        *x = r.f32_le().map_err(DecodeError::truncated(RECORD))?;
    }
    Ok(Transform::from_rows(v))
}

fn expect_equal(what: &str, declared: usize, stored: usize) -> Result<(), DecodeError> {
    if declared == stored { Ok(()) } else { Err(DecodeError::invalid(RECORD, format!("node declares {declared} {what} entries but stores {stored}"))) }
}
